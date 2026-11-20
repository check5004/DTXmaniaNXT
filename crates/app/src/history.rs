//! プレイの記録。OS のデータのフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の records.sqlite に残し、
//! 結果の画面に出す苦手な小節、練習でクリアした区間の克服、叩き方のくせを、直近の記録から出す。
//! 記録が開けないときは残さずに遊べるようにし、苦手な小節は今回のプレイだけから出す。

use std::path::{Path, PathBuf};

use engine::analysis::{
    HISTORY_PLAYS, NoteResult, Tendency, measure_weakness, overcome, play_section_score,
    section_weakness, tendencies, weak_phrases,
};
use engine::practice::Section;
use records::{BestScore, ChartKey, ClearRecord, PlayScore, Records, SessionKind};

use crate::game::{Play, WeakMap};

/// 結果の画面に出す苦手フレーズの数
const TOP_PHRASES: usize = 5;

/// 苦手度に使う練習の周の数。練習の周は区間しか叩かないので、普通のプレイより多めに読む
const RECENT_PRACTICE_LAPS: usize = 100;

/// 叩き方のくせに使う、すべての曲の直近のセッションの数
pub const TENDENCY_SESSIONS: usize = 200;

/// 叩き方のくせを出すのに要る打数（レーンと拍の中の位置の組ごと）
pub const TENDENCY_MIN_HITS: usize = 20;

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

    /// 遊んだ記録を残す。`score` は普通のプレイのスコアと達成率（演奏の画面が数えたもの）。
    /// 返すのはセッションの番号（残せなかったら None）
    pub fn save(&mut self, play: &Play, score: Option<PlayScore>) -> Option<i64> {
        let (Some(records), Some(mut session)) = (self.records.as_mut(), play.session()) else {
            return None;
        };
        if play.practice.is_none() {
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
