//! タイトルの画面（上流 `CStageTitle`）と、終わるときの画面（上流 `CStageEnd`）。
//! スキンの 2_ の画像で、背景とメニュー（GAME START / CONFIGURATION / EXIT）を出す。
//! 曲を探しているあいだは左上に「曲を探しています」の画像を明滅させる（上流 `CActEnumSongs`）。

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use egui::Color32;

use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::text::Style;

/// メニューの 1 行の高さと幅、左上（上流 `MENU_H` など）
const MENU_H: f32 = 39.0;
const MENU_W: f32 = 227.0;
const MENU_X: f32 = 506.0;
const MENU_Y: f32 = 513.0;

/// 白いフェードの長さ（上流 `CActFIFOWhite`：0〜100 を 5 ミリ秒ずつ）
const FADE: Duration = Duration::from_millis(500);

/// 終わるときの画面を出しておく一番短い時間（上流 `CStageEnd`：1 秒、かつ音が鳴り終わるまで）
const END_MIN: Duration = Duration::from_millis(1000);

/// メニューの項目
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleItem {
    GameStart,
    Config,
    Exit,
}

impl TitleItem {
    const ALL: [TitleItem; 3] = [TitleItem::GameStart, TitleItem::Config, TitleItem::Exit];
}

pub struct TitleView {
    cursor: usize,
    /// カーソルを動かしているアニメ（向き：上が -1、始めた時刻、始めの値 0〜100）
    moving: Option<(i32, Instant, f64)>,
    /// カーソルの光るアニメの起点
    flash: Instant,
    /// 白からのフェードの始まり
    entered: Instant,
    /// フェードを終えてタイトルの音を鳴らした
    sounded: bool,
    /// 決めた項目と、白へのフェードの始まり
    chosen: Option<(TitleItem, Instant)>,
}

impl TitleView {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            cursor: 0,
            moving: None,
            flash: now,
            entered: now,
            sounded: false,
            chosen: None,
        }
    }

    /// 画面に入り直す（白からのフェードから。カーソルはそのまま）
    pub fn enter(&mut self) {
        let now = Instant::now();
        self.entered = now;
        self.flash = now;
        self.moving = None;
        self.sounded = false;
        self.chosen = None;
    }

    /// 操作を受け付けるか（フェードの途中は受け付けない）
    pub fn ready(&self) -> bool {
        self.chosen.is_none() && self.entered.elapsed() >= FADE
    }

    /// フェードを終えたらタイトルの音を鳴らす（1 回だけ true）
    pub fn take_title_sound(&mut self) -> bool {
        if !self.sounded && self.entered.elapsed() >= FADE {
            self.sounded = true;
            return true;
        }
        false
    }

    /// カーソルを動かす。動いたら true（上流と同じく端で止まる）
    pub fn move_by(&mut self, delta: i32) -> bool {
        let next = self.cursor as i32 + delta.signum();
        if !self.ready() || next < 0 || next >= TitleItem::ALL.len() as i32 {
            return false;
        }
        self.cursor = next as usize;
        // 逆向きに動いている途中なら、残りから続ける
        let start = match self.moving {
            Some((dir, _, _)) if dir != delta.signum() => 100.0 - self.move_value(),
            _ => 0.0,
        };
        self.moving = Some((delta.signum(), Instant::now(), start));
        true
    }

    /// 選んでいる項目
    pub fn selected(&self) -> TitleItem {
        TitleItem::ALL[self.cursor]
    }

    /// 決める。EXIT のほかは白へフェードしてから移る
    pub fn decide(&mut self) -> Option<TitleItem> {
        if !self.ready() {
            return None;
        }
        let item = self.selected();
        if item != TitleItem::Exit {
            self.chosen = Some((item, Instant::now()));
        }
        Some(item)
    }

    /// 白へのフェードを終えた項目
    pub fn faded_out(&self) -> Option<TitleItem> {
        self.chosen
            .filter(|(_, at)| at.elapsed() >= FADE)
            .map(|(item, _)| item)
    }

    /// カーソルの移動のアニメの値（0〜100。1 ミリ秒に 1 進む）
    fn move_value(&self) -> f64 {
        self.moving.map_or(100.0, |(_, at, start)| {
            (start + at.elapsed().as_secs_f64() * 1000.0).min(100.0)
        })
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, scanning: bool) {
        match g.skin.tex("Graphics/2_background.jpg") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(20, 24, 40)),
        }
        // 版（上流は左上 (2, 2) に白い文字で出す）
        let version = concat!("DTXmaniaNXT ", env!("CARGO_PKG_VERSION"));
        let style = Style::edged(14.0, Color32::WHITE, Color32::BLACK, 1.0);
        if let Some(t) = g.texts.get(version, &style) {
            t.draw(c, 2.0, 2.0, 1.0, 1.0);
        }
        if let Some(t) = g.skin.tex("Graphics/2_menu.png") {
            let mut y = MENU_Y + self.cursor as f32 * MENU_H;
            if let Some((dir, _, _)) = self.moving {
                let v = self.move_value();
                let off = (MENU_H as f64 / 2.0 * ((PI * v / 100.0).cos() + 1.0)) as f32;
                // 上へ動いたなら下から、下へ動いたなら上から来る
                y += if dir < 0 { off } else { -off };
            }
            // カーソルの光（上流 ctカーソルフラッシュ用：0〜700 を 5 ミリ秒ずつ回し、100 から始まる）
            let flash = (100 + self.flash.elapsed().as_millis() / 5) % 701;
            if flash <= 100 {
                let k = flash as f32 / 100.0;
                let mag = 1.0 + k * 0.5;
                c.draw_opt(
                    &t,
                    MENU_X + MENU_W * (1.0 - mag) / 2.0,
                    y + MENU_H * (1.0 - mag) / 2.0,
                    Opt {
                        src: Some(src(0.0, MENU_H * 5.0, MENU_W, MENU_H)),
                        sx: mag,
                        sy: mag,
                        alpha: 1.0 - k,
                        ..Default::default()
                    },
                );
            }
            c.part(&t, MENU_X, y, src(0.0, MENU_H * 4.0, MENU_W, MENU_H));
            // 上流と同じく OPTION の行は描かない
            c.part(&t, MENU_X, MENU_Y, src(0.0, 0.0, MENU_W, MENU_H));
            c.part(
                &t,
                MENU_X,
                MENU_Y + MENU_H,
                src(0.0, MENU_H * 2.0, MENU_W, MENU_H * 2.0),
            );
        } else {
            draw_plain_menu(c, g, self.cursor);
        }
        if scanning {
            draw_scanning(c, g, self.entered);
        }
        // 白いフェード
        let a = match self.chosen {
            Some((_, at)) => (at.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0),
            None => 1.0 - (self.entered.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0),
        };
        if a > 0.0 {
            c.cover(Color32::from_white_alpha((a * 255.0) as u8));
        }
    }
}

/// 曲を探している印（上流 `CActEnumSongs`：0〜100 を 17 ミリ秒ずつ回し、濃さを sin で揺らす）
pub fn draw_scanning(c: &Canvas, g: &mut Gfx, since: Instant) {
    let Some(t) = g
        .skin
        .tex_optional("Graphics/ScreenTitle NowEnumeratingSongs.png")
    else {
        return;
    };
    let v = (since.elapsed().as_millis() / 17 % 101) as f64;
    let alpha = (176.0 + 80.0 * (2.0 * PI * v * 2.0 / 100.0).sin()).min(255.0) / 255.0;
    c.draw_opt(
        &t,
        18.0,
        7.0,
        Opt {
            alpha: alpha as f32,
            ..Default::default()
        },
    );
}

/// スキンが無いときのメニュー
fn draw_plain_menu(c: &Canvas, g: &mut Gfx, cursor: usize) {
    for (i, name) in ["GAME START", "CONFIGURATION", "EXIT"].iter().enumerate() {
        let color = if i == cursor {
            Color32::from_rgb(255, 200, 0)
        } else {
            Color32::WHITE
        };
        let style = Style::edged(24.0, color, Color32::BLACK, 2.0);
        if let Some(t) = g.texts.get(name, &style) {
            t.draw(
                c,
                MENU_X + (MENU_W - t.w) / 2.0,
                MENU_Y + i as f32 * MENU_H + 4.0,
                1.0,
                1.0,
            );
        }
    }
}

/// 終わるときの画面（上流 `CStageEnd`：9_background.jpg を出し、終わりの音が鳴り終わるまで待つ）
pub struct Ending {
    /// 初めて描いた時刻（画像を読むのに時間がかかっても、出してから待つ）
    shown: Option<Instant>,
    wait: Duration,
}

impl Ending {
    /// `sound` は終わりの音の長さ
    pub fn new(sound: Duration) -> Self {
        Self {
            shown: None,
            wait: sound.max(END_MIN),
        }
    }

    pub fn done(&self) -> bool {
        self.shown.is_some_and(|t| t.elapsed() >= self.wait)
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx) {
        match g.skin.tex("Graphics/9_background.jpg") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::BLACK),
        }
        self.shown.get_or_insert_with(Instant::now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_stops_at_the_ends() {
        let mut v = TitleView::new();
        v.entered -= FADE;
        assert!(!v.move_by(-1));
        assert!(v.move_by(1));
        assert!(v.move_by(1));
        assert_eq!(v.selected(), TitleItem::Exit);
        assert!(!v.move_by(1));
    }

    #[test]
    fn nothing_is_decided_during_the_fade_in() {
        let mut v = TitleView::new();
        assert_eq!(v.decide(), None);
        v.entered -= FADE;
        assert_eq!(v.decide(), Some(TitleItem::GameStart));
        // 決めたあとは動かない
        assert!(!v.move_by(1));
    }
}
