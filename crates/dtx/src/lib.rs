//! DTX 譜面の読み込み。
//!
//! 行の解釈は上流 DTXManiaNX（`CDTX.cs`）の動きと、DTX Web Editor（`src/dtx`）の実装にそろえている。
//! 上流のコードは移植せず、ふるまいだけを合わせて書き直したもの。

mod encoding;
mod lane;
mod number;
mod parse;
mod rim;
mod sound;
mod timing;

pub use encoding::{DtxEncoding, decode};
pub use lane::DrumLane;
pub use number::{parse_base36x2, parse_hex2, parse_measure, parse_nx_number};
pub use parse::{Chart, Chip, Warning, parse};
pub use rim::is_rim_sound_name;
pub use sound::{AutoChannel, AutoSound, LaneSound};
pub use timing::{DrumNote, NotePos, TempoMap};

/// チャンネル番号（よく使うもの）
pub mod ch {
    pub const BGM: u8 = 0x01;
    pub const BAR_LENGTH: u8 = 0x02;
    pub const BPM: u8 = 0x03;
    pub const BPM_EX: u8 = 0x08;
}

/// バイト列を読んで譜面にする（文字コードの判定から）
pub fn load(bytes: &[u8]) -> Chart {
    let (text, _) = decode(bytes);
    parse(&text)
}
