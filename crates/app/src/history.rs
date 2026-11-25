//! プレイの記録。OS のデータのフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の records.sqlite に残し、
//! 結果の画面に出す苦手な小節、練習でクリアした区間の克服、叩き方のくせを、直近の記録から出す。
//! 記録が開けないときは残さずに遊べるようにし、苦手な小節は今回のプレイだけから出す。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dtx::TempoMap;

use engine::analysis::{
    HISTORY_PLAYS, NoteResult, Tendency, measure_weakness, overcome, play_section_score,
    section_weakness, tendencies, weak_phrases,
};
use engine::lag::{Advice, LagSample, advise};
use engine::practice::Section;
use library::Stats;
use records::{
    BestScore, ChartKey, ChartProgress, ClearRecord, PlayScore, PlayTime, Records, SessionKind,
    SessionLag, time_kind,
};

use crate::chart_picture::Marks;
use crate::game::{Play, WeakMap};
use crate::progress::estimate_ms;
use crate::result_view::Rank;

/// 結果の画面に出す苦手フレーズの数
const TOP_PHRASES: usize = 5;

/// 苦手度に使う練習の周の数。練習の周は区間しか叩かないので、普通のプレイより多めに読む
const RECENT_PRACTICE_LAPS: usize = 100;

/// 叩き方のくせに使う、すべての曲の直近のセッションの数
pub const TENDENCY_SESSIONS: usize = 200;

/// 叩き方のくせを出すのに要る打数（レーンと拍の中の位置の組ごと）
pub const TENDENCY_MIN_HITS: usize = 20;

/// 補正のおすすめに読む、すべての曲の直近のセッションの数
const LAG_SESSIONS: usize = 300;

/// おすすめを出すときの、この曲の名前（ほかの曲はフォルダの名前で見分ける）
const THIS_SONG: &str = "\0this";

/// 記録のセッションのずれを、補正を足し戻した生のずれにする。`charts` はこの曲の譜面
fn lag_sample(l: SessionLag, charts: &[PathBuf]) -> LagSample {
    let path = Path::new(&l.chart_path);
    // 同じ曲の難易度は同じフォルダに置くので、ほかの曲はフォルダで見分ける
    let song = if charts.iter().any(|c| c == path) {
        THIS_SONG.to_owned()
    } else {
        path.parent().map_or_else(
            || l.chart_path.clone(),
            |d| d.to_string_lossy().into_owned(),
        )
    };
    let applied = json_number(&l.settings, "input_offset_ms").unwrap_or(0.0)
        + json_number(&l.settings, "song_offset_ms").unwrap_or(0.0);
    LagSample {
        song,
        hits: l.hits,
        raw_ms: l.mean_ms + applied,
    }
}

/// 記録の設定（[`Play::session`] が書く平たい JSON）から数を 1 つ読む
fn json_number(json: &str, key: &str) -> Option<f64> {
    let at = json.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = &json[at..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

/// 練習でクリアした区間を、普通のプレイで確かめた結果
#[derive(Debug, Clone, PartialEq)]
pub struct OvercomeCheck {
    pub section: Section,
    /// 練習でクリアした速度（%、一番速いもの）
    pub speed: u32,
    /// 練習の前の区間の苦手度
    pub before: Option<f64>,
    /// 今回のプレイを入れた区間の苦手度
    pub now: Option<f64>,
    /// 今回のプレイだけの区間の点
    pub this_play: f64,
    pub overcome: bool,
}

pub struct History {
    records: Option<Records>,
}

impl History {
    pub fn path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("DTXmaniaNXT").join("records.sqlite"))
    }

    pub fn open() -> Self {
        let records = Self::path().and_then(|path| {
            if let Some(dir) = path.parent()
                && let Err(e) = std::fs::create_dir_all(dir)
            {
                log::warn!("records are off: {}: {e}", dir.display());
                return None;
            }
            match Records::open(&path) {
                Ok(r) => {
                    log::info!("records: {}", path.display());
                    Some(r)
                }
                Err(e) => {
                    log::warn!("records are off: {}: {e}", path.display());
                    None
                }
            }
        });
        Self { records }
    }

    /// 遊んだ記録と、遊んだ時間を残す。`score` は普通のプレイのスコアと達成率（演奏の画面が数えたもの）、
    /// `end_ms` はやめた曲の時刻。返すのはセッションの番号（残せなかったら None）。
    /// 早送りしたプレイはセッションを残さないが、叩いていれば時間は残す
    pub fn save(&mut self, play: &Play, score: Option<PlayScore>, end_ms: f64) -> Option<i64> {
        let records = self.records.as_mut()?;
        let id = play.session().and_then(|mut session| {
            // 自動で叩くレーンがあるプレイは、ベストの記録（スコア）に入れない
            if play.practice.is_none() && !play.has_auto() {
                session.score = score;
            }
            let path = play.path.to_string_lossy();
            let chart = ChartKey {
                path: &path,
                title: &play.title,
                artist: &play.artist,
                hash: play.chart_hash,
            };
            records
                .save(&chart, &session)
                .map_err(|e| log::warn!("cannot save the record: {e}"))
                .ok()
        });
        if let Some(ms) = play.played_ms(end_ms)
            && let Err(e) = records.add_play_time(play.time_kind, play.started_at, ms, id)
        {
            log::warn!("cannot save the play time: {e}");
        }
        id
    }

    /// 曲の一覧の譜面の隣にある NX の score.ini を、まだなら記録に引き継ぐ。引き継いだ譜面の数を返す
    pub fn import_nx_scores(&mut self, library: &library::Library) -> usize {
        let Some(records) = self.records.as_mut() else {
            return 0;
        };
        let done = match records.imported_files() {
            Ok(d) => d,
            Err(e) => {
                log::warn!("cannot read the records: {e}");
                return 0;
            }
        };
        let now = unix_now();
        let mut count = 0;
        for song in library.all_songs() {
            for chart in song.charts.iter().flatten() {
                let ini_path = crate::nx_import::score_ini_path(&chart.path);
                let file = ini_path.to_string_lossy();
                if done.contains(file.as_ref()) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&ini_path) else {
                    continue;
                };
                let ini = nx::ScoreIni::parse(&dtx::decode(&bytes).0);
                let modified = std::fs::metadata(&ini_path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(now, |d| d.as_secs() as i64);
                let sessions = crate::nx_import::sessions(&ini, modified);
                // 譜面が読めなければ、次に開いたときにもう一度
                let Ok(chart_bytes) = std::fs::read(&chart.path) else {
                    continue;
                };
                let path = chart.path.to_string_lossy();
                let title = if chart.head.title.is_empty() {
                    &song.title
                } else {
                    &chart.head.title
                };
                let key = ChartKey {
                    path: &path,
                    title,
                    artist: &chart.head.artist,
                    hash: records::hash_bytes(&chart_bytes),
                };
                match records.import(&file, &key, &sessions, now) {
                    Ok(()) if !sessions.is_empty() => count += 1,
                    Ok(()) => {}
                    Err(e) => log::warn!("{file}: cannot import: {e}"),
                }
            }
        }
        if count > 0 {
            log::info!("imported NX scores of {count} charts");
        }
        count
    }

    /// この曲の補正（ミリ秒）。決めていないか、記録が読めなければ 0
    pub fn song_offset(&self, path: &Path) -> f64 {
        let Some(records) = self.records.as_ref() else {
            return 0.0;
        };
        records
            .song_offset(&path.to_string_lossy())
            .map_err(|e| log::warn!("cannot read the records: {e}"))
            .ok()
            .flatten()
            .unwrap_or(0.0)
    }

    /// この曲の補正を、曲の譜面 `charts` すべてに残す
    pub fn set_song_offset(&mut self, charts: &[PathBuf], offset_ms: f64) {
        let Some(records) = self.records.as_mut() else {
            log::warn!("records are off: the song offset is not kept");
            return;
        };
        let paths: Vec<String> = charts
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        if let Err(e) = records.set_song_offset(&refs, offset_ms, unix_now()) {
            log::warn!("cannot save the song offset: {e}");
        }
    }

    /// 補正のおすすめ。`charts` はこの曲の譜面（難易度すべて）、`global_now` と `song_now` は今の補正。
    /// 記録が読めなければ、記録の無いときと同じ（何もすすめない）
    pub fn lag_advice(&self, charts: &[PathBuf], global_now: f64, song_now: f64) -> Advice {
        let samples = self
            .records
            .as_ref()
            .and_then(|r| {
                r.recent_lags(LAG_SESSIONS)
                    .map_err(|e| log::warn!("cannot read the records: {e}"))
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|l| lag_sample(l, charts))
            .collect::<Vec<_>>();
        advise(
            &samples,
            THIS_SONG,
            global_now.round() as i64,
            song_now.round() as i64,
        )
    }

    /// 譜面のこれまでのベスト。`before`（セッションの番号）があれば、それより前のプレイだけから。
    /// 記録が読めなければ None
    pub fn best_score(&self, path: &Path, before: Option<i64>) -> Option<BestScore> {
        let records = self.records.as_ref()?;
        records
            .best_score(&path.to_string_lossy(), before)
            .map_err(|e| log::warn!("cannot read the records: {e}"))
            .ok()
    }

    /// 譜面ごとの記録（選曲画面の並べ替え、ランプ、ランク、スキル）。記録が読めなければ空
    pub fn chart_stats(&self) -> Stats {
        let mut out = Stats::new();
        let Some(records) = self.records.as_ref() else {
            return out;
        };
        match records.last_played() {
            Ok(rows) => {
                for (path, at) in rows {
                    out.entry(PathBuf::from(path)).or_default().last_played = Some(at);
                }
            }
            Err(e) => log::warn!("cannot read the records: {e}"),
        }
        match records.cleared_plays() {
            Ok(rows) => {
                for p in rows {
                    let s = out.entry(PathBuf::from(&p.chart_path)).or_default();
                    s.best_rate = Some(s.best_rate.map_or(p.rate, |r| r.max(p.rate)));
                    let rank = Rank::of(&p.tally).number();
                    s.best_rank = Some(s.best_rank.map_or(rank, |r| r.min(rank)));
                }
            }
            Err(e) => log::warn!("cannot read the records: {e}"),
        }
        out
    }

    /// 苦手な小節。記録から出し、記録が読めなければ `fallback`（今回のプレイ）から出す。
    /// どちらも無ければ全部「記録なし」
    pub fn weak_map(
        &self,
        path: &Path,
        notes_per_measure: &[usize],
        fallback: Option<&[NoteResult]>,
    ) -> WeakMap {
        self.weak_map_before(path, notes_per_measure, fallback, None)
    }

    /// 苦手な小節。`before`（UNIX 秒）があれば、それより前に始めた記録だけから
    fn weak_map_before(
        &self,
        path: &Path,
        notes_per_measure: &[usize],
        fallback: Option<&[NoteResult]>,
        before: Option<i64>,
    ) -> WeakMap {
        let path = path.to_string_lossy();
        let laps = self
            .records
            .as_ref()
            .and_then(|r| {
                match r.recent_results(&path, HISTORY_PLAYS, RECENT_PRACTICE_LAPS, before) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        log::warn!("cannot read the records: {e}");
                        None
                    }
                }
            })
            .unwrap_or_default();
        let (plays, practice_laps, results): (usize, usize, Vec<Vec<NoteResult>>) =
            match (laps.is_empty(), fallback) {
                (true, Some(f)) => (1, 0, vec![f.to_vec()]),
                _ => (
                    laps.iter().filter(|l| l.kind == SessionKind::Play).count(),
                    laps.iter()
                        .filter(|l| l.kind == SessionKind::Practice)
                        .count(),
                    laps.into_iter().map(|l| l.results).collect(),
                ),
            };
        let weakness = measure_weakness(&results, notes_per_measure.len());
        let phrases = weak_phrases(&weakness, notes_per_measure, TOP_PHRASES);
        WeakMap {
            weakness,
            phrases,
            plays,
            practice_laps,
        }
    }

    /// よくミスする音。直近のプレイと練習（苦手な小節と同じ周）で、音符ごとに POOR・MISS になった回数
    pub fn note_marks(&self, path: &Path) -> Marks {
        let path = path.to_string_lossy();
        let Some(r) = self.records.as_ref() else {
            return Marks::new();
        };
        match r.recent_note_misses(&path, HISTORY_PLAYS, RECENT_PRACTICE_LAPS, None) {
            Ok(v) => v.into_iter().map(|m| ((m.at, m.lane), m.misses)).collect(),
            Err(e) => {
                log::warn!("cannot read the records: {e}");
                Marks::new()
            }
        }
    }

    /// 譜面の、練習でクリアした区間の記録（古い順）
    pub fn clears(&self, path: &Path) -> Vec<ClearRecord> {
        let Some(r) = &self.records else {
            return Vec::new();
        };
        r.clears(&path.to_string_lossy()).unwrap_or_else(|e| {
            log::warn!("cannot read the clears: {e}");
            Vec::new()
        })
    }

    /// 練習でクリアしてまだ克服していない区間を、今回の普通のプレイ（`started_at` に始めた）で確かめる。
    /// 区間の音符を全部叩いたものだけ確かめ、克服していたら記録に残す。
    /// `now` は今回のプレイを入れた苦手な小節、`results` は今回のプレイの判定
    pub fn check_overcome(
        &mut self,
        path: &Path,
        started_at: i64,
        notes_per_measure: &[usize],
        results: &[NoteResult],
        now: &WeakMap,
    ) -> Vec<OvercomeCheck> {
        // 同じ区間のクリアはまとめ、一番前のクリアの前を「練習の前」にする
        let mut pending: Vec<(Section, i64, u32)> = Vec::new();
        for c in self.clears(path) {
            if c.overcome.is_some() || c.cleared_at >= started_at {
                continue;
            }
            let section = Section::new(c.first, c.last);
            let speed = (c.speed * 100.0).round() as u32;
            match pending.iter_mut().find(|p| p.0 == section) {
                Some(p) => p.2 = p.2.max(speed),
                None => pending.push((section, c.cleared_at, speed)),
            }
        }
        let mut checks = Vec::new();
        for (section, first_clear, speed) in pending {
            let (a, b) = (section.first, section.last);
            let notes: usize = (a..=b)
                .filter_map(|m| notes_per_measure.get(m as usize))
                .sum();
            let played = results
                .iter()
                .filter(|r| (a..=b).contains(&r.measure))
                .count();
            if notes == 0 || played < notes {
                continue;
            }
            let Some(this_play) = play_section_score(results, a, b) else {
                continue;
            };
            let earlier = self.weak_map_before(path, notes_per_measure, None, Some(first_clear));
            let before = (earlier.plays + earlier.practice_laps > 0)
                .then(|| section_weakness(&earlier.weakness, notes_per_measure, a, b))
                .flatten();
            let after = section_weakness(&now.weakness, notes_per_measure, a, b);
            let done = overcome(before, after, Some(this_play));
            if done
                && let Some(r) = self.records.as_mut()
                && let Err(e) =
                    r.mark_overcome(&path.to_string_lossy(), a, b, started_at, before, after)
            {
                log::warn!("cannot save the overcome mark: {e}");
            }
            checks.push(OvercomeCheck {
                section,
                speed,
                before,
                now: after,
                this_play,
                overcome: done,
            });
        }
        checks
    }

    /// 上達の記録に出す、遊んだ時間と譜面ごとの記録。記録が読めなければ None。
    /// 遊んだ時間の表を足す前のセッションは、ここで譜面から長さを見積もって入れる（1 度だけ）
    pub fn progress(&mut self) -> Option<(Vec<PlayTime>, Vec<ChartProgress>)> {
        let records = self.records.as_mut()?;
        fill_play_times(records);
        let read =
            || -> records::Result<_> { Ok((records.play_times(None)?, records.chart_progress()?)) };
        read()
            .map_err(|e| log::warn!("cannot read the progress: {e}"))
            .ok()
    }

    /// すべての曲を通した叩き方のくせ（等速で叩いた直近の記録から）
    pub fn tendencies(&self) -> Vec<Tendency> {
        let Some(r) = &self.records else {
            return Vec::new();
        };
        match r.recent_timed_hits(TENDENCY_SESSIONS) {
            Ok(hits) => tendencies(&hits, TENDENCY_MIN_HITS),
            Err(e) => {
                log::warn!("cannot read the hits: {e}");
                Vec::new()
            }
        }
    }
}

/// 遊んだ時間の記録が無いセッションに、譜面から見積もった長さを入れる。
/// 譜面が読めなければ 0 を入れる（次に開いたときにまた読みに行かないように）
fn fill_play_times(records: &mut Records) {
    let sessions = match records.untimed_sessions() {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return,
        Err(e) => {
            log::warn!("cannot read the sessions: {e}");
            return;
        }
    };
    let mut tempos: HashMap<String, Option<TempoMap>> = HashMap::new();
    let rows: Vec<(&str, i64, i64, Option<i64>)> = sessions
        .iter()
        .map(|s| {
            let tempo = tempos.entry(s.chart_path.clone()).or_insert_with(|| {
                std::fs::read(&s.chart_path)
                    .ok()
                    .map(|bytes| TempoMap::build(&dtx::load(&bytes)))
            });
            let ms = tempo.as_ref().map_or(0, |t| estimate_ms(s, t));
            let kind = match s.kind {
                SessionKind::Practice => time_kind::PRACTICE,
                _ => time_kind::PLAY,
            };
            (kind, s.started_at, ms, Some(s.id))
        })
        .collect();
    match records.add_play_times(&rows) {
        Ok(()) => log::info!("estimated the play time of {} old sessions", rows.len()),
        Err(e) => log::warn!("cannot save the play times: {e}"),
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_numbers_from_the_session_settings() {
        let s = "{\"input_offset_ms\":-31,\"windows_ms\":[34,67,84,117],\"drum_sounds\":true,\"song_offset_ms\":12.5}";
        assert_eq!(json_number(s, "input_offset_ms"), Some(-31.0));
        assert_eq!(json_number(s, "song_offset_ms"), Some(12.5));
        assert_eq!(json_number(s, "missing"), None);
        assert_eq!(json_number("{}", "input_offset_ms"), None);
    }

    #[test]
    fn lag_samples_add_back_the_offsets() {
        let lag = |path: &str, settings: &str| SessionLag {
            chart_path: path.into(),
            started_at: 0,
            settings: settings.into(),
            hits: 100,
            mean_ms: 4.0,
        };
        let charts = [
            PathBuf::from("songs/a/bas.dtx"),
            PathBuf::from("songs/a/mas.dtx"),
        ];
        let s = lag_sample(
            lag(
                "songs/a/mas.dtx",
                "{\"input_offset_ms\":3,\"song_offset_ms\":6}",
            ),
            &charts,
        );
        assert_eq!(s.song, THIS_SONG);
        assert_eq!(s.raw_ms, 13.0);
        // ほかの曲はフォルダで見分ける。前の版の記録（この曲の補正が無い）は 0
        let s = lag_sample(lag("songs/b/ext.dtx", "{\"input_offset_ms\":-2}"), &charts);
        assert_eq!(s.song, Path::new("songs/b").to_string_lossy());
        assert_eq!(s.raw_ms, 2.0);
    }
}
