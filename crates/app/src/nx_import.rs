//! NX が譜面の隣に書いた `score.ini` の記録を、NXT の記録に引き継ぐ。
//!
//! 引き継ぐのは、ドラムの達成率が一番高かったプレイ（`[HiSkill.Drums]`）と、スコアが一番高かったプレイ
//! （`[HiScore.Drums]`、達成率の一番と別のときだけ）。選曲の画面のランクとスキル、結果の画面の New Record と
//! 前回のベストにそのまま効く。音符ごとの結果は無いので、苦手の分析には使わない。
//! 自動で叩いたレーンがあった記録は、NXT と同じくベストに入れない。

use std::path::{Path, PathBuf};

use chrono::TimeZone;
use engine::judge::Tally;
use nx::{ScoreIni, ScoreRecord};
use records::{Lap, PROGRESS_SECTIONS, PlayScore, Session, SessionKind};

/// 引き継いだセッションの「遊んだときの設定」に書くもの
const SETTINGS: &str = r#"{"imported":"DTXManiaNX score.ini"}"#;

/// 譜面の隣の score.ini の場所（NX と同じく「譜面のファイルの名前.score.ini」）
pub fn score_ini_path(chart: &Path) -> PathBuf {
    let mut name = chart.as_os_str().to_owned();
    name.push(".score.ini");
    PathBuf::from(name)
}

/// score.ini の記録を、残すセッションにする。`fallback_at` は日時が書いていない記録の時刻（UNIX 秒）
pub fn sessions(ini: &ScoreIni, fallback_at: i64) -> Vec<Session> {
    let usable = |r: &&ScoreRecord| !r.auto_play && r.is_played();
    let cleared = |r: &ScoreRecord| ini.clear_count > 0 && r.score > 0 && r.rate > 0.0;
    let mut out = Vec::new();
    let skill = ini.hi_skill.as_ref().filter(usable);
    if let Some(r) = skill {
        out.push(session(r, cleared(r), fallback_at));
    }
    if let Some(r) = ini.hi_score.as_ref().filter(usable)
        && r.score > 0
        && skill.is_none_or(|s| s.score != r.score || s.date_time != r.date_time)
    {
        out.push(session(r, cleared(r), fallback_at));
    }
    out
}

fn session(r: &ScoreRecord, cleared: bool, fallback_at: i64) -> Session {
    let progress = if r.progress.chars().count() == PROGRESS_SECTIONS {
        r.progress.clone()
    } else {
        "0".repeat(PROGRESS_SECTIONS)
    };
    Session {
        kind: SessionKind::Imported,
        started_at: r.date_time.and_then(local_unix).unwrap_or(fallback_at),
        settings: SETTINGS.to_owned(),
        laps: vec![Lap {
            speed: 1.0,
            completed: cleared,
            passed: None,
            tally: Tally {
                perfect: r.perfect,
                great: r.great,
                good: r.good,
                poor: r.poor,
                miss: r.miss,
                combo: 0,
                max_combo: r.max_combo,
                stray: 0,
            },
            offset_mean_ms: None,
            offset_sd_ms: None,
            hits: Vec::new(),
            strays: Vec::new(),
        }],
        clear: None,
        score: Some(PlayScore {
            score: r.score,
            rate: r.rate,
            progress,
            cleared,
        }),
    }
}

/// この PC の時刻の日時を UNIX 秒にする（夏時間の切り替えで 2 つあるときは早いほう）
fn local_unix([y, mo, d, h, mi, s]: [u32; 6]) -> Option<i64> {
    chrono::Local
        .with_ymd_and_hms(i32::try_from(y).ok()?, mo, d, h, mi, s)
        .earliest()
        .map(|t| t.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(score: i64, rate: f64, date: Option<[u32; 6]>) -> ScoreRecord {
        ScoreRecord {
            score,
            rate,
            perfect: 10,
            great: 2,
            good: 1,
            poor: 0,
            miss: 3,
            max_combo: 9,
            total_chips: 16,
            auto_play: false,
            date_time: date,
            progress: "2".repeat(64),
        }
    }

    #[test]
    fn best_rate_and_best_score_become_cleared_plays() {
        let ini = ScoreIni {
            play_count: 8,
            clear_count: 6,
            hi_score: Some(record(620_000, 77.5, Some([2026, 1, 9, 21, 5, 7]))),
            hi_skill: Some(record(617_188, 78.4, Some([2026, 5, 30, 0, 13, 58]))),
        };
        let s = sessions(&ini, 1);
        assert_eq!(s.len(), 2);
        assert!(s.iter().all(|s| s.kind == SessionKind::Imported));
        let best = s[0].score.as_ref().unwrap();
        assert_eq!((best.score, best.rate, best.cleared), (617_188, 78.4, true));
        assert_eq!(s[0].laps[0].tally.max_combo, 9);
        assert_eq!(
            s[0].started_at,
            local_unix([2026, 5, 30, 0, 13, 58]).unwrap()
        );
        assert_eq!(s[1].score.as_ref().unwrap().score, 620_000);
        // 同じプレイなら 1 つだけ
        let same = ScoreIni {
            hi_score: ini.hi_skill.clone(),
            ..ini
        };
        assert_eq!(sessions(&same, 1).len(), 1);
    }

    #[test]
    fn failed_and_auto_plays() {
        // 落ちたプレイは進み具合だけ（最後まで遊んだことにしない）。日時が無ければ渡した時刻
        let mut failed = record(0, 0.0, None);
        failed.progress = format!("3111{}", "0".repeat(60));
        let ini = ScoreIni {
            play_count: 1,
            clear_count: 0,
            hi_score: Some(record(0, 0.0, None)),
            hi_skill: Some(failed),
        };
        let s = sessions(&ini, 42);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].started_at, 42);
        assert!(!s[0].score.as_ref().unwrap().cleared);
        // 自動で叩いたレーンがあった記録は入れない
        let mut auto = record(500_000, 70.0, None);
        auto.auto_play = true;
        let ini = ScoreIni {
            clear_count: 1,
            hi_skill: Some(auto),
            ..Default::default()
        };
        assert!(sessions(&ini, 1).is_empty());
    }

    #[test]
    fn score_ini_sits_next_to_the_chart() {
        assert_eq!(
            score_ini_path(Path::new("songs/a/b.dtx")),
            Path::new("songs/a/b.dtx.score.ini")
        );
    }
}
