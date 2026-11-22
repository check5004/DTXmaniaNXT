//! タイトルの画面（上流 `CStageTitle`）と、終わるときの画面（上流 `CStageEnd`）。
//! スキンの 2_ の画像で、背景とロゴとメニュー（GAME START / CONFIGURATION / EXIT）を出す。
//! 入るときは白からフェードし、決めたら App がシャッター（`shutter`）を閉じて次の画面へ移る（NX v2.0）。
//! 曲を探しているあいだは左上に「曲を探しています」の画像を明滅させる（上流 `CActEnumSongs`）。
//! 新しい版があるときだけ、メニューの下に「更新」の行を足す（このアプリだけ。見本の案 A、2026-11-22）。
//! 落としている間は左上の版の下に進み具合を出し、更新して起動し直したら左下に知らせる。

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use egui::Color32;

use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::settings_ui as ui;
use crate::text::Style;
use crate::update::Status;
use crate::update_view::{self, Corner};

/// メニューの 1 行の高さと幅、左上（上流 `MENU_H` など）
const MENU_H: f32 = 39.0;
const MENU_W: f32 = 227.0;
const MENU_X: f32 = 506.0;
const MENU_Y: f32 = 513.0;

/// 白いフェードの長さ（上流 `CActFIFOWhite`：0〜100 を 5 ミリ秒ずつ）
const FADE: Duration = Duration::from_millis(500);

/// 「更新」の行（メニューの板の下に足す。見本の案 A）の左上と大きさ
const UPDATE_X: f32 = 471.0;
const UPDATE_Y: f32 = 646.0;
const UPDATE_W: f32 = 300.0;
const UPDATE_H: f32 = 44.0;

/// 更新のあとの知らせを出しておく時間（秒）と、消えていく時間
const UPDATED_FOR: f32 = 10.0;
const UPDATED_FADE: f32 = 0.6;

/// 終わるときの画面を出しておく一番短い時間（上流 `CStageEnd`：1 秒、かつ音が鳴り終わるまで）
const END_MIN: Duration = Duration::from_millis(1000);

/// メニューの項目
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleItem {
    GameStart,
    Config,
    Exit,
    /// 新しい版があるときだけ出る行（落とす、入れ替える）
    Update,
}

impl TitleItem {
    const ALL: [TitleItem; 4] = [
        TitleItem::GameStart,
        TitleItem::Config,
        TitleItem::Exit,
        TitleItem::Update,
    ];
}

/// メニューの行の上の端（カーソルを置く所）。「更新」の行は板の下に離して置く
fn row_y(i: usize) -> f32 {
    if i < 3 {
        MENU_Y + i as f32 * MENU_H
    } else {
        UPDATE_Y + (UPDATE_H - MENU_H) / 2.0
    }
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
    /// 決めた項目（決めたあとは操作を受け付けない。画面はシャッターを閉じてから移る）
    chosen: Option<TitleItem>,
    /// 更新のようす（新しい版があれば「更新」の行を出す）
    update: Status,
    /// 更新して起動し直した（今の版、前の版）と、知らせを出し始めた時刻
    updated: Option<(String, String, Option<Instant>)>,
    /// 「更新」の行の NEW の明滅と、回る輪の起点
    since: Instant,
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
            update: Status::Idle,
            updated: None,
            since: now,
        }
    }

    /// 出している行の数（新しい版があれば「更新」の行で 4）
    fn len(&self) -> usize {
        if self.update == Status::Idle { 3 } else { 4 }
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
        if !self.ready() || next < 0 || next >= self.len() as i32 {
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

    /// 決める（v2 と同じく、EXIT のほかは App がシャッターを閉じてから移る）。
    /// 「更新」はこの画面にとどまるので、続けて操作できる
    pub fn decide(&mut self) -> Option<TitleItem> {
        if !self.ready() {
            return None;
        }
        let item = self.selected();
        if item != TitleItem::Update {
            self.chosen = Some(item);
        }
        Some(item)
    }

    /// カーソルの移動のアニメの値（0〜100。1 ミリ秒に 1 進む）
    fn move_value(&self) -> f64 {
        self.moving.map_or(100.0, |(_, at, start)| {
            (start + at.elapsed().as_secs_f64() * 1000.0).min(100.0)
        })
    }

    /// 更新のようすを渡す（毎フレーム）。行が消えたらカーソルを戻す
    pub fn set_update(&mut self, status: &Status) {
        if &self.update != status {
            self.update = status.clone();
            if self.cursor >= self.len() {
                self.cursor = self.len() - 1;
                self.moving = None;
            }
        }
    }

    /// 更新して起動し直したことを知らせる（`previous` は前の版。分からなければ空）
    pub fn set_updated(&mut self, current: &str, previous: String) {
        self.updated = Some((current.to_owned(), previous, None));
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, scanning: bool) {
        match g.skin.tex("Graphics/2_background.jpg") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(20, 24, 40)),
        }
        // ロゴ（NX v2.0 は背景の動画の上に透明な 2_background.png を重ねる。動画の代わりは上の静止画）
        if let Some(t) = g.skin.tex_optional("Graphics/2_background.png") {
            c.draw(&t, 0.0, 0.0);
        }
        // 版（v2 は左上 (0, 0) に 12 ピクセルの縁取りした白い文字で出す）
        let version = concat!("DTXmaniaNXT ", env!("CARGO_PKG_VERSION"));
        let style = Style::edged(12.0, Color32::WHITE, Color32::BLACK, 1.0);
        if let Some(t) = g.texts.get(version, &style) {
            t.draw(c, 0.0, 0.0, 1.0, 1.0);
        }
        if self.len() > 3 {
            self.draw_update_row(c);
        }
        if let Some(t) = g.skin.tex("Graphics/2_menu.png") {
            let mut y = row_y(self.cursor);
            if let Some((dir, _, _)) = self.moving {
                let v = self.move_value();
                // 上へ動いたなら下から、下へ動いたなら上から来る（前の行の位置から今の行へ）
                let from = row_y((self.cursor as i32 - dir).clamp(0, 3) as usize);
                y += ((from - y) as f64 * ((PI * v / 100.0).cos() + 1.0) / 2.0) as f32;
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
        if self.len() > 3 {
            self.draw_update_label(c, g);
        }
        if scanning {
            draw_scanning(c, g, self.entered);
        }
        // 落としている間は左上の版の下に（曲を探している印と重なるときは、その下に）
        if let Status::Downloading { version, percent } = &self.update {
            let y = if scanning { 52.0 } else { 22.0 };
            update_view::chip(
                c,
                g,
                &format!("v{version} を落としています {percent}%"),
                Some(f32::from(*percent) / 100.0),
                Corner::TopLeft(6.0, y),
                self.since,
            );
        }
        if let Some((current, previous, shown)) = &mut self.updated {
            let t = shown
                .get_or_insert_with(Instant::now)
                .elapsed()
                .as_secs_f32();
            let alpha = ((UPDATED_FOR - t) / UPDATED_FADE).clamp(0.0, 1.0);
            update_view::updated(c, g, current, previous, alpha);
        }
        // 白からのフェード（出ていくときは v2 と同じくシャッターなので、白へは消さない）
        let a = 1.0 - (self.entered.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        if a > 0.0 {
            c.cover(Color32::from_white_alpha((a * 255.0) as u8));
        }
    }
}

impl TitleView {
    /// 「更新」の行の地（カーソルより下に描く）
    fn draw_update_row(&self, c: &Canvas) {
        let (x, y, w, h) = (UPDATE_X, UPDATE_Y, UPDATE_W, UPDATE_H);
        let top = Color32::from_rgba_unmultiplied(8, 40, 12, 235);
        let bottom = Color32::from_rgba_unmultiplied(4, 22, 6, 235);
        ui::vgrad(c, x, y, w, h, top, bottom);
        if let Status::Downloading { percent, .. } = &self.update {
            c.fill(
                x,
                y,
                w * f32::from(*percent) / 100.0,
                h,
                Color32::from_rgba_unmultiplied(255, 156, 46, 71),
            );
        }
        c.frame(
            x,
            y,
            w,
            h,
            Color32::from_rgba_unmultiplied(110, 220, 90, 140),
        );
    }

    /// 「更新」の行の文字（ようすと、選んでいるかで変わる）
    fn draw_update_label(&self, c: &Canvas, g: &mut Gfx) {
        let selected = self.cursor == 3;
        let (label, new) = match &self.update {
            Status::Idle => return,
            Status::Available(v) if selected => (format!("v{v} に更新する"), false),
            Status::Available(v) => (format!("新しい版 v{v}"), true),
            Status::Downloading { percent, .. } => (format!("落としています {percent}%"), false),
            Status::Ready(_) => ("再起動して入れ替える".to_owned(), false),
            Status::Failed(_) if selected => ("もう一度落とす".to_owned(), false),
            Status::Failed(_) => ("落とせませんでした".to_owned(), false),
        };
        let style = Style {
            bold: true,
            ..Style::edged(22.0, Color32::WHITE, Color32::BLACK, 2.0)
        };
        let badge = ui::bold(12.0, Color32::from_rgb(26, 15, 0));
        let lw = ui::text_width(g, &label, &style).min(UPDATE_W - 24.0);
        let bw = if new {
            ui::text_width(g, "NEW", &badge) + 12.0
        } else {
            0.0
        };
        let gap = if new { 10.0 } else { 0.0 };
        let x = UPDATE_X + (UPDATE_W - lw - gap - bw) / 2.0;
        let cy = UPDATE_Y + UPDATE_H / 2.0;
        if let Some(t) = g.texts.get(&label, &style) {
            t.draw_fit(c, x, cy - t.h / 2.0, lw);
        }
        if new {
            // 見本と同じく 1.2 秒かけて濃くなり、また薄くなる
            let p = self.since.elapsed().as_secs_f32() / 1.2;
            let k = 1.0 - (p % 2.0 - 1.0).abs();
            let alpha = 0.55 + 0.45 * k;
            let bx = x + lw + gap;
            ui::round_fill(
                c,
                bx,
                cy - 9.0,
                bw,
                18.0,
                3.0,
                update_view::ACCENT.gamma_multiply(alpha),
            );
            if let Some(t) = g.texts.get("NEW", &badge) {
                t.draw(c, bx + 6.0, cy - t.h / 2.0, 1.0, alpha);
            }
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

/// スキンが無いときのメニュー（「更新」の行の文字は別に描く）
fn draw_plain_menu(c: &Canvas, g: &mut Gfx, cursor: usize) {
    for (i, name) in ["はじめる", "設定", "終わる"].iter().enumerate() {
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
    fn the_update_row_appears_only_with_a_new_version() {
        let mut v = TitleView::new();
        v.entered -= FADE;
        assert!(v.move_by(1));
        assert!(v.move_by(1));
        assert!(!v.move_by(1));
        v.set_update(&Status::Available("9.9.9".to_owned()));
        assert!(v.move_by(1));
        assert_eq!(v.selected(), TitleItem::Update);
        // 「更新」を決めてもこの画面にとどまる
        assert_eq!(v.decide(), Some(TitleItem::Update));
        assert!(v.ready());
        // 行が消えたら一つ上へ
        v.set_update(&Status::Idle);
        assert_eq!(v.selected(), TitleItem::Exit);
    }

    #[test]
    fn the_cursor_slides_from_the_previous_row() {
        assert_eq!(row_y(1) - row_y(0), MENU_H);
        // 「更新」の行は板の下に離れている
        assert!(row_y(3) > row_y(2) + MENU_H);
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
