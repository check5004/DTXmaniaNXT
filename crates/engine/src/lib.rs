//! DTXmaniaNXT のゲームの中核。画面にも OS にも依存しない部分だけを置く。
//!
//! - [`judge`] は叩いた時刻と音符の時刻を比べて判定する
//! - [`clock`] は音の再生位置と MIDI の受信時刻を、同じ「曲の時刻」にそろえる
//! - [`input`] は電子ドラムの MIDI を、レーンと叩き方にそろえる（Roland の既定値つき）
//! - [`analysis`] は記録から苦手な小節とフレーズを出す
//! - [`practice`] は練習の区間を周回ぶん並べ、セットごとに苦手の拍を数えて次の一手を決める

pub mod analysis;
pub mod clock;
pub mod input;
pub mod judge;
pub mod practice;

pub use dtx::DrumLane;
