//! 曲を読み込むあいだの画面（上流 `CStageSongLoading`）。
//! スキンの 6_ の画像で、ジャケット、曲名、アーティスト、パートと難易度のパネル、レベルを出す。
//! 上流と同じく、読み込みが終わっても「Now loading」の音が鳴り終わるまでは出しておく。

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use egui::Color32;
use library::ChartHead;

use crate::game::Play;
use crate::gfx::{Canvas, Gfx, Glyphs, Opt, fit_scale, src};
use crate::play_view::{difficulty_cell, rotate_about_center};
use crate::practice::PracticeOptions;
use crate::text::Style;

/// 読み込みの画面
pub struct Loading {
    pub path: PathBuf,
    /// 練習なら区間とつなぎ方
    pub practice: Option<PracticeOptions>,
    head: ChartHead,
    preimage: Option<PathBuf>,
    /// 6_Difficulty.png の切り出しの左上
    diff_panel: (f32, f32),
    started: Instant,
    /// 「Now loading」の音の長さ
    wait: Duration,
    /// 描いた回数（1 回描いてから読み込む。でないと読み込みのあいだ画面が出ない）
    frames: u32,
    /// 読み終えた曲
    pub play: Option<Box<Play>>,
}

impl Loading {
    /// `label` は難易度の名前（選曲画面で選んだもの）
    pub fn new(
        path: PathBuf,
        practice: Option<PracticeOptions>,
        label: &str,
        wait: Duration,
    ) -> Self {
        let head = library::scan::read_head(&path).unwrap_or_default();
        let preimage = path
            .parent()
            .zip(Some(head.preimage.trim()))
            .filter(|(_, name)| !name.is_empty())
            .and_then(|(dir, name)| library::scan::resolve(dir, name));
        let diff_panel = difficulty_cell("6", label).unwrap_or((0.0, 0.0));
        Self {
            path,
            practice,
            head,
            preimage,
            diff_panel,
            started: Instant::now(),
            wait,
            frames: 0,
            play: None,
        }
    }

    /// 画面を 1 回は出したので、読み込んでよい
    pub fn ready_to_load(&self) -> bool {
        self.frames >= 1 && self.play.is_none()
    }

    /// 読み込みが終わり、音も鳴り終えた
    pub fn done(&self) -> bool {
        self.play.is_some() && self.started.elapsed() >= self.wait
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx) {
        self.frames = self.frames.saturating_add(1);
        match g.skin.tex("Graphics/6_background.jpg") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::BLACK),
        }
        self.draw_jacket(c, g);
        let edge = |px: f32| Style {
            px,
            color: Color32::WHITE,
            bold: false,
            shadow: None,
            edge: Some((Color32::BLACK, px / 12.0)),
            gradient: None,
        };
        if let Some(t) = g.texts.get(&self.head.title, &edge(40.0)) {
            t.draw_fit(c, 190.0, 285.0, 625.0);
        }
        if let Some(t) = g.texts.get(&self.head.artist, &edge(30.0)) {
            t.draw_fit(c, 190.0, 360.0, 625.0);
        }
        // ドラムのパートのパネル（6_Part.png の 1 段目）と難易度のパネル
        if let Some(t) = g.skin.tex("Graphics/6_Part.png") {
            c.part(&t, 191.0, 52.0, src(0.0, 0.0, 262.0, 50.0));
        }
        if let Some(t) = g.skin.tex("Graphics/6_Difficulty.png") {
            let (x, y) = self.diff_panel;
            c.part(&t, 191.0, 102.0, src(x, y, 262.0, 50.0));
        }
        // レベル（上流の XG 表示：整数の部分、点、小数の 2 桁）
        if let Some(level) = self.head.level.filter(|&l| l > 0.0) {
            let t = g.skin.tex("Graphics/6_LevelNumber.png");
            let (whole, frac) = level_parts(level);
            if let Some(t) = &t {
                c.part(t, 282.0, 243.0, src(1000.0, 92.0, 30.0, 38.0));
            }
            BIG.draw(c, t.as_ref(), 187.0, 152.0, &whole.to_string());
            BIG.draw(c, t.as_ref(), 307.0, 152.0, &format!("{frac:02}"));
        }
    }

    /// ジャケット（上流は 384 に縮めて (206, 66) へ動かし、画面の中心のまわりに 0.28 ラジアン回す）
    fn draw_jacket(&self, c: &Canvas, g: &mut Gfx) {
        let default = "Graphics/5_preimage default.png";
        let jacket = match &self.preimage {
            Some(p) => g
                .images
                .get(p)
                .or_else(|| g.images.failed(p).then(|| g.skin.tex(default)).flatten()),
            None => g.skin.tex(default),
        };
        let Some(t) = jacket else {
            return;
        };
        let f = fit_scale(&t, 384.0);
        let (cx, cy) = rotate_about_center(206.0, 66.0, 0.28);
        c.draw_rot(
            &t,
            cx,
            cy,
            0.28,
            Opt {
                sx: f,
                sy: f,
                ..Default::default()
            },
        );
    }
}

/// レベル 8.25 を (8, 25) に分ける
fn level_parts(level: f64) -> (u32, u32) {
    let hundredths = (level * 100.0).round().max(0.0) as u32;
    (hundredths / 100, hundredths % 100)
}

/// 6_LevelNumber.png の大きい数字（上流 `st大文字位置`：100×130 が横に並び、送りは 90）
static BIG: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(
        (0..10u8)
            .map(|d| {
                let x = f32::from(d) * 100.0;
                (char::from(b'0' + d), src(x, 0.0, 100.0, 130.0), 90.0)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    ),
    missing: 90.0,
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_split_like_the_xg_display() {
        assert_eq!(level_parts(8.25), (8, 25));
        assert_eq!(level_parts(6.5), (6, 50));
        assert_eq!(level_parts(9.99), (9, 99));
        // 浮動小数の誤差で 7.0 が 6.99 にならない
        assert_eq!(level_parts(6.999_999_9), (7, 0));
    }
}
