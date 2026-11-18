//! 曲の一覧。曲のフォルダを上流 DTXManiaNX と同じ決まりで探し、BOX と曲の木にする。
//!
//! [`setdef`] は set.def の読み方、[`scan`] はフォルダの探し方。

pub mod scan;
pub mod setdef;

pub use scan::{ChartHead, Library, Node, Progress, Song, SongBox, SongChart, scan};
pub use setdef::{DEFAULT_LABELS, LEVELS, SetDefBlock, parse_set_def};
