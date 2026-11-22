//! 譜面の動画（`#AVI`）を、演奏画面の背景の上・レーンの下に流す（上流 `CActPerfAVI`）。
//!
//! 動画は動画ごとのスレッドで読み（crate `video`）、ここでは毎フレーム「今の動画の時刻」を渡して、
//! できた絵を貼り替えるだけ。今は Windows だけ（ほかの OS では何も出さない）。

use std::path::PathBuf;

use egui::ColorImage;

use crate::game::Play;
use crate::gfx::{Canvas, Opt, Tex};

/// 動画の絵の大きさの上限（画面と同じ）。大きい動画は読むときに縮める
const MAX_SIZE: (u32, u32) = (1280, 720);
/// 流し始めるこれだけ前から、最初の絵を作っておく
const PREROLL_MS: f64 = 1000.0;
/// 1 曲で開いておく動画の数の上限（いくつも使う譜面は少ない）
const MAX_FILES: usize = 8;
/// これより横長なら幅を画面に合わせ、縦長なら高さを合わせる（上流 `fClipアスペクト比`）
const WIDE: f32 = 1.77;

/// 流し始める譜面の時刻と、どの動画か（`Movies::players` の番号）
type Cue = (f64, usize);

pub struct Movies {
    players: Vec<video::Player<ColorImage>>,
    cues: Vec<Cue>,
    /// 貼っている絵と、それがどの動画の何枚目か
    tex: Option<(usize, u64, Tex)>,
}

impl Movies {
    /// 譜面の動画を開く。動画が無いか、この OS で読めなければ None
    pub fn new(play: &Play) -> Option<Self> {
        if play.movies.is_empty() {
            return None;
        }
        if !video::SUPPORTED {
            log::info!("この OS ではまだ譜面の動画を流せません");
            return None;
        }
        let (files, cues) = cue_files(&play.movies);
        let players = files
            .iter()
            .map(|f| video::Player::open(f, MAX_SIZE, to_image))
            .collect();
        Some(Self {
            players,
            cues,
            tex: None,
        })
    }

    /// 譜面の時刻 `chart_ms` の絵を描く
    pub fn draw(&mut self, c: &Canvas, chart_ms: f64) {
        let (now, next) = around(&self.cues, chart_ms);
        for (i, p) in self.players.iter().enumerate() {
            let at = match (now, next) {
                (Some((ms, j)), _) if j == i => Some(chart_ms - ms),
                (_, Some((ms, j))) if j == i && ms - chart_ms < PREROLL_MS => Some(0.0),
                _ => None,
            };
            p.show_at(at);
        }
        let Some((_, i)) = now else {
            return;
        };
        let Some((serial, img)) = self.players[i].frame() else {
            return;
        };
        if let Some((ti, ts, tex)) = &mut self.tex {
            if (*ti, *ts) != (i, serial) {
                tex.set(img);
                (*ti, *ts) = (i, serial);
            }
        } else {
            self.tex = Some((i, serial, Tex::shared(c.painter().ctx(), "movie", img)));
        }
        let Some((_, _, tex)) = &self.tex else {
            return;
        };
        let (x, y, s) = place(tex.w, tex.h);
        c.draw_opt(
            tex,
            x,
            y,
            Opt {
                sx: s,
                sy: s,
                ..Default::default()
            },
        );
    }
}

/// 動画のファイルを重ねずに並べ、流し始めの時刻をファイルの番号にする（多すぎる分は捨てる）
fn cue_files(movies: &[(f64, PathBuf)]) -> (Vec<PathBuf>, Vec<Cue>) {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut cues = Vec::new();
    for (ms, path) in movies {
        let i = match files.iter().position(|f| f == path) {
            Some(i) => i,
            None if files.len() < MAX_FILES => {
                files.push(path.clone());
                files.len() - 1
            }
            None => continue,
        };
        cues.push((*ms, i));
    }
    (files, cues)
}

/// 今流れている動画（最後に流し始めたもの）と、次に流し始める動画
fn around(cues: &[Cue], chart_ms: f64) -> (Option<Cue>, Option<Cue>) {
    let n = cues.partition_point(|q| q.0 <= chart_ms);
    (n.checked_sub(1).map(|i| cues[i]), cues.get(n).copied())
}

/// 絵を置く左上と拡大率。横長は幅を 1280 に合わせて上下の真ん中、それより縦長は高さを 720 に合わせて左右の真ん中
fn place(w: f32, h: f32) -> (f32, f32, f32) {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let s = if w / h >= WIDE { 1280.0 / w } else { 720.0 / h };
    ((1280.0 - w * s) / 2.0, (720.0 - h * s) / 2.0, s)
}

/// 読むスレッドで、絵を egui の形にしておく
fn to_image(f: video::Frame) -> ColorImage {
    // 不透明なので、掛け合わせ済みとして読んでも同じ
    ColorImage::from_rgba_premultiplied([f.width as usize, f.height as usize], &f.rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_files_share_a_player() {
        let a = PathBuf::from("a.mp4");
        let b = PathBuf::from("b.avi");
        let movies = [(0.0, a.clone()), (1000.0, b.clone()), (2000.0, a.clone())];
        let (files, cues) = cue_files(&movies);
        assert_eq!(files, [a, b]);
        assert_eq!(cues, [(0.0, 0), (1000.0, 1), (2000.0, 0)]);
    }

    #[test]
    fn finds_the_playing_and_the_next_movie() {
        let cues = [(1000.0, 0), (5000.0, 1)];
        assert_eq!(around(&cues, 0.0), (None, Some((1000.0, 0))));
        assert_eq!(
            around(&cues, 1000.0),
            (Some((1000.0, 0)), Some((5000.0, 1)))
        );
        assert_eq!(around(&cues, 9000.0), (Some((5000.0, 1)), None));
    }

    #[test]
    fn wide_movies_fit_the_width_and_others_the_height() {
        assert_eq!(place(1280.0, 720.0), (0.0, 0.0, 1.0));
        let (x, y, s) = place(1920.0, 800.0);
        assert_eq!(x, 0.0);
        assert!((y - 93.33).abs() < 0.01 && (s - 2.0 / 3.0).abs() < 1e-6);
        let (x, y, s) = place(640.0, 480.0);
        assert_eq!((x, y, s), (160.0, 0.0, 1.5));
    }
}
