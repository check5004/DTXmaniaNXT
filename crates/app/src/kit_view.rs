//! 曲ごとの音源のキットを見せる部品。選曲では、キットを決めた曲のジャケットの下の端に札を出す。

use egui::Color32;

use crate::gfx::{Canvas, Gfx};
use crate::settings_ui as ui;

/// 選曲のジャケット（320, 35 から 300 四方）の下の端に重ねる札
const TAG_X: f32 = 328.0;
const TAG_Y: f32 = 292.0;
const TAG_W: f32 = 284.0;
const TAG_H: f32 = 34.0;

/// 選曲の札（機種の名前の印と、キットの番号と名前）
pub fn draw_tag(c: &Canvas, g: &mut Gfx, model: &str, label: &str) {
    ui::round_fill(
        c,
        TAG_X,
        TAG_Y,
        TAG_W,
        TAG_H,
        7.0,
        Color32::from_rgba_unmultiplied(8, 6, 15, 225),
    );
    ui::round_frame(
        c,
        TAG_X,
        TAG_Y,
        TAG_W,
        TAG_H,
        7.0,
        1.0,
        Color32::from_rgba_unmultiplied(255, 255, 255, 46),
    );
    let chip = ui::bold(13.0, Color32::BLACK);
    let mark = model;
    let cw = ui::text_width(g, mark, &chip) + 12.0;
    ui::round_fill(c, TAG_X + 10.0, TAG_Y + 8.0, cw, 18.0, 4.0, ui::STRONG);
    ui::text(c, g, mark, TAG_X + 16.0, TAG_Y + 9.0, &chip, cw);
    let x = TAG_X + 20.0 + cw;
    ui::text(
        c,
        g,
        label,
        x,
        TAG_Y + 6.0,
        &ui::bold(17.0, ui::TEXT),
        TAG_X + TAG_W - x - 10.0,
    );
}
