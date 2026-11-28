//! 電子ドラムの音源（今は Roland V71・V51・V31）と MIDI でやり取りするための、画面にも OS にも依存しない部分。
//!
//! - [`roland`] は Roland の SysEx（Data Request 1・Data Set 1・Identity）の組み立てと読み取り
//! - [`nibble`] は 4 ビットずつに分けて送る値（`0000 aaaa` の並び）の読み書き
//! - [`vseries`] は V71・V51・V31 の番地表のうち、このアプリが使う所（3 機種で同じ）
//! - [`journal`] は音源の値を書き換える前に元の値を残し、あとで書き戻すための記録
//! - [`listen`] は音源の音を録ったものを調べる所（測る道具で、書き換えが効くまでの時間などを見る）
//!
//! これらの音源は書き換えた値をそのまま本体に保存する（一時的に変える場所が無い）。
//! なので書き換える前に必ず元の値を読み、[`journal::Journal`] に残してから書く。
//! 番地と値の形は Roland の各機種の「MIDI Implementation」による（[`vseries`]）。

pub mod journal;
pub mod listen;
pub mod nibble;
pub mod roland;
pub mod vseries;
