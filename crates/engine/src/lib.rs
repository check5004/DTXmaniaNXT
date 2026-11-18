//! DTXmaniaNXT のゲームの中核。画面にも OS にも依存しない部分だけを置く。
//!
//! - [`judge`] は叩いた時刻と音符の時刻を比べて判定する
//! - [`clock`] は音の再生位置と MIDI の受信時刻を、同じ「曲の時刻」にそろえる
//! - [`input`] は電子ドラムの MIDI を、レーンと叩き方にそろえる（Roland の既定値つき）

pub mod clock;
pub mod input;
pub mod judge;

pub use dtx::DrumLane;
