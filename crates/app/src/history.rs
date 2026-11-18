//! プレイの記録。OS のデータのフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の records.sqlite に残し、
//! 結果の画面に出す苦手な小節を、直近の記録から出す。
//! 記録が開けないときは残さずに遊べるようにし、苦手な小節は今回のプレイだけから出す。

use std::path::PathBuf;

use engine::analysis::{HISTORY_PLAYS, measure_weakness, weak_phrases};
use records::{ChartKey, Records};

use crate::game::{Outcome, Play, WeakMap};

/// 結果の画面に出す苦手フレーズの数
const TOP_PHRASES: usize = 5;

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

    /// 結果の画面に出す苦手な小節。記録が読めなければ今回のプレイだけから出す
    pub fn weak_map(&self, o: &Outcome) -> WeakMap {
        let path = o.path.to_string_lossy();
        let plays = self
            .records
            .as_ref()
            .and_then(|r| match r.recent_results(&path, HISTORY_PLAYS) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::warn!("cannot read the records: {e}");
                    None
                }
            })
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| vec![o.results.clone()]);
        let weakness = measure_weakness(&plays, o.measures);
        let phrases = weak_phrases(&weakness, &o.notes_per_measure, TOP_PHRASES);
        WeakMap {
            weakness,
            phrases,
            plays: plays.len(),
        }
    }
}
