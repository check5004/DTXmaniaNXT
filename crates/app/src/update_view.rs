//! 更新のようすを画面に出す部品（見本の案 A、2026-11-22）。
//! - 落としている間の、画面の隅の小さな印（回る輪と %）
//! - 「再起動して入れ替える」を選んだあとの「更新しています」の板
//! - 更新して起動し直したときの「v… になりました」の知らせ
//!
//! タイトルのメニューの下に足す「更新」の行は `title_view` が描く。

use std::f32::consts::TAU;
use std::time::Instant;

use egui::Color32;
use egui::epaint::{PathShape, PathStroke};
use i18n::{t, tf};

use crate::gfx::{Canvas, Gfx};
use crate::settings_ui as ui;

/// 落としているときの印などの橙（見本の `--accent`）
pub const ACCENT: Color32 = Color32::from_rgb(255, 156, 46);
/// 更新のあとの知らせの緑（見本の `--ok`）
const OK: Color32 = Color32::from_rgb(79, 212, 111);
/// 小さな板の地と縁
const CHIP_BG: Color32 = Color32::from_rgba_premultiplied(8, 6, 14, 199);
const CHIP_LINE: Color32 = Color32::from_rgba_premultiplied(46, 46, 46, 46);
const SUB: Color32 = Color32::from_rgb(201, 195, 220);

/// 隅の印を置く所
#[derive(Debug, Clone, Copy)]
pub enum Corner {
    /// 左上を (x, y) に
    TopLeft(f32, f32),
    /// 右上を (x, y) に
    TopRight(f32, f32),
    /// 右下を (x, y) に
    BottomRight(f32, f32),
}

/// 回る輪（薄い輪の上を、橙の 4 分の 1 が回る）
pub fn spinner(c: &Canvas, cx: f32, cy: f32, r: f32, since: Instant) {
    let s = c.scale();
    c.painter().circle_stroke(
        c.pos(cx, cy),
        r * s,
        egui::Stroke::new(2.0 * s, Color32::from_white_alpha(64)),
    );
    let start = since.elapsed().as_secs_f32() * TAU;
    let pts = (0..=12)
        .map(|i| {
            let a = start + i as f32 / 12.0 * TAU / 4.0;
            c.pos(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    c.painter()
        .add(PathShape::line(pts, PathStroke::new(2.0 * s, ACCENT)));
}

/// 隅の小さな印。`progress`（0〜1）があれば回る輪と下の細い棒も出す
pub fn chip(
    c: &Canvas,
    g: &mut Gfx,
    text: &str,
    progress: Option<f32>,
    corner: Corner,
    since: Instant,
) {
    let style = ui::bold(14.0, Color32::WHITE);
    let tw = ui::text_width(g, text, &style);
    let ring = if progress.is_some() { 22.0 } else { 0.0 };
    let (w, h) = (20.0 + ring + tw, 28.0);
    let (x, y) = match corner {
        Corner::TopLeft(x, y) => (x, y),
        Corner::TopRight(r, y) => (r - w, y),
        Corner::BottomRight(r, b) => (r - w, b - h),
    };
    ui::round_fill(c, x, y, w, h, 4.0, CHIP_BG);
    ui::round_frame(c, x, y, w, h, 4.0, 1.0, CHIP_LINE);
    if let Some(p) = progress {
        spinner(c, x + 17.0, y + 13.0, 6.0, since);
        let (bx, bw) = (x + 10.0, w - 20.0);
        c.fill(bx, y + h - 5.0, bw, 2.0, Color32::from_white_alpha(38));
        c.fill(bx, y + h - 5.0, bw * p.clamp(0.0, 1.0), 2.0, ACCENT);
    }
    ui::text(c, g, text, x + 10.0 + ring, y + 4.0, &style, tw);
}

/// 「再起動して入れ替える」を選んだあと、閉じるまで出す板
pub fn applying(c: &Canvas, g: &mut Gfx, from: &str, to: &str, since: Instant) {
    c.cover(Color32::from_rgba_premultiplied(3, 3, 9, 219));
    let (x, y, w, h) = (360.0, 275.0, 560.0, 170.0);
    ui::pick_panel(c, x, y, w, h);
    let (tx, tw) = (x + 32.0, w - 64.0);
    ui::text(
        c,
        g,
        t("Updating", "更新しています"),
        tx,
        y + 24.0,
        &ui::bold(30.0, Color32::WHITE),
        tw,
    );
    ui::text(
        c,
        g,
        &format!("v{from} → v{to}"),
        tx,
        y + 68.0,
        &ui::bold(22.0, ACCENT),
        tw,
    );
    ui::paragraph(
        c,
        g,
        t(
            "The game will close to install the update, then start the new version in a few seconds.",
            "入れ替えのためにいったん閉じます。数秒で、新しい版が自動で起動します。",
        ),
        tx,
        y + 106.0,
        tw,
        26.0,
        y + h - 30.0,
        &ui::plain(16.0, ui::DIM),
    );
    // 左から右へ流れる棒（どれだけ進んだかは分からないので、動いていることだけを見せる）
    let (by, bh) = (y + h - 24.0, 6.0);
    ui::round_fill(c, tx, by, tw, bh, 3.0, Color32::from_white_alpha(31));
    let t = (since.elapsed().as_secs_f32() / 1.4).fract();
    let seg = tw * 0.3;
    let left = (tx - seg + t * (tw + seg)).max(tx);
    let right = (tx - seg + t * (tw + seg) + seg).min(tx + tw);
    if right > left {
        ui::round_fill(c, left, by, right - left, bh, 3.0, ACCENT);
    }
}

/// 更新して起動し直したときの知らせ（左下、操作の案内の帯の上）。`previous` は前の版（分からなければ空）
pub fn updated(c: &Canvas, g: &mut Gfx, current: &str, previous: &str, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let a = |col: Color32| col.gamma_multiply(alpha);
    let main = tf!("Updated to v{current}", "v{current} になりました");
    let sub = (!previous.is_empty()).then(|| tf!("from v{previous}", "前は v{previous}"));
    let main_style = ui::bold(17.0, Color32::WHITE);
    let sub_style = ui::plain(14.0, SUB);
    let mw = ui::text_width(g, &main, &main_style);
    let sw = sub
        .as_deref()
        .map_or(0.0, |s| 10.0 + ui::text_width(g, s, &sub_style));
    let (w, h) = (18.0 + mw + sw + 14.0, 38.0);
    let (x, y) = (16.0, ui::LEGEND_Y - 12.0 - h);
    ui::round_fill(c, x, y, w, h, 4.0, a(CHIP_BG));
    ui::round_frame(c, x, y, w, h, 4.0, 1.0, a(CHIP_LINE));
    c.fill(x, y, 4.0, h, a(OK));
    if let Some(t) = g.texts.get(&main, &main_style) {
        t.draw(c, x + 18.0, y + 8.0, 1.0, alpha);
    }
    if let Some(s) = &sub
        && let Some(t) = g.texts.get(s, &sub_style)
    {
        t.draw(c, x + 18.0 + mw + 10.0, y + 11.0, 1.0, alpha);
    }
}
