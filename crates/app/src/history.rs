//! プレイの記録。OS のデータのフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の records.sqlite に残し、
//! 結果の画面に出す苦手な小節を、直近の記録から出す。
//! 記録が開けないときは残さずに遊べるようにし、苦手な小節は今回のプレイだけから出す。

use std::path::{Path, PathBuf};

use engine::analysis::{HISTORY_PLAYS, NoteResult, measure_weakness, weak_phrases};
use records::{ChartKey, Records, SessionKind};

use crate::game::{Play, WeakMap};

/// 結果の画面に出す苦手フレーズの数
const TOP_PHRASES: usize = 5;

/// 苦手度に使う練習の周の数。練習の周は区間しか叩かないので、普通のプレイより多めに読む
const RECENT_PRACTICE_LAPS: usize = 100;

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

    /// 遊んだ記録を残す
    pub fn save(&mut self, play: &Play) {
        let (Some(records), Some(session)) = (self.records.as_mut(), play.session()) else {
            return;
        };
        let path = play.path.to_string_lossy();
        let chart = ChartKey {
            path: &path,
            title: &play.title,
            artist: &play.artist,
            hash: play.chart_hash,
        };
        if let Err(e) = records.save(&chart, &session) {
            log::warn!("cannot save the record: {e}");
        }
    }

    /// 苦手な小節。記録から出し、記録が読めなければ `fallback`（今回のプレイ）から出す。
    /// どちらも無ければ全部「記録なし」
    pub fn weak_map(
        &self,
        path: &Path,
        notes_per_measure: &[usize],
        fallback: Option<&[NoteResult]>,
    ) -> WeakMap {
        let path = path.to_string_lossy();
        let laps = self
            .records
            .as_ref()
            .and_then(
                |r| match r.recent_results(&path, HISTORY_PLAYS, RECENT_PRACTICE_LAPS) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        log::warn!("cannot read the records: {e}");
                        None
                    }
                },
            )
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
}
