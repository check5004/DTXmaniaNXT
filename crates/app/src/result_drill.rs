//! 基礎練習の結果（結果の画面の、ランクとジャケットの場所）。文章でなく段と点で見せる。
//!
//! - 左上（スコアの場所）に、ミスなしで叩けた一番速いテンポと、最高を更新したか
//! - 右の欄に、段ごとの出来（◎ ミスなし、○ 1〜2、× 3 つ以上）、右手・左手ごとのずれの平均とばらつき、
//!   叩いた順のずれの点（段の区切りとテンポを下に）
//! - 左右は「手順どおりに叩いた」として数える（電子ドラムはどちらの手かを送ってこない）

use egui::{Color32, Stroke};
use engine::drill::{COUNT_IN_MEASURES, Grade, Hand, Lean, STEP_MEASURES, STEPS};
use i18n::{t, tf};

use crate::drill::{DrillOutcome, LEFT_COLOR, RIGHT_COLOR, hand_name};
use crate::drill_hud::{MANY_COLOR, grade_color, grade_mark, stairs};
use crate::gfx::{Canvas, Gfx};
use crate::settings_ui as ui;
use crate::text::Style;

/// 右の欄
const PANEL: (f32, f32, f32, f32) = (480.0, 24.0, 780.0, 596.0);
/// ずれの棒の端（ミリ秒）と、点の図の上下の端（ミリ秒）
const LAG_RANGE_MS: f64 = 40.0;
const DOT_RANGE_MS: f64 = 60.0;
const FOOT_COLOR: Color32 = Color32::from_gray(200);
const ACCENT: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);

fn edged(px: f32, color: Color32) -> Style {
    Style::edged(px, color, Color32::BLACK, (px / 12.0).max(1.0))
}

fn bold(px: f32, color: Color32) -> Style {
    Style {
        bold: true,
        ..edged(px, color)
    }
}

fn hand_color(h: Option<Hand>) -> Color32 {
    match h {
        Some(Hand::Right) => RIGHT_COLOR,
        Some(Hand::Left) => LEFT_COLOR,
        _ => FOOT_COLOR,
    }
}

/// 分:秒
fn clock(ms: f64) -> String {
    let s = (ms / 1000.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// ずれの数（+3 ms）
fn ms_text(v: f64) -> String {
    format!("{:+.0} ms", v)
}

/// スコアの場所：ミスなしで叩けた一番速いテンポと、最高を更新したか
pub fn draw_headline(c: &Canvas, g: &mut Gfx, o: &DrillOutcome) {
    g.heading(c, 30.0, 28.0, t("Drill Result", "基礎練習の結果"));
    let (text, color) = match o.best_now() {
        Some(b) => (tf!("Clean up to {b} ✓", "{b} まで ✓"), Color32::WHITE),
        None => (
            t("No clean step this time", "ミスなしの段はまだ").to_owned(),
            Color32::from_gray(220),
        ),
    };
    let style = Style {
        bold: true,
        ..Style::edged(36.0, color, Color32::BLACK, 3.0)
    };
    let w = match g.texts.get(&text, &style) {
        Some(tx) => {
            tx.draw_fit(c, 30.0, 56.0, 250.0);
            tx.w.min(250.0)
        }
        None => 0.0,
    };
    if !o.new_best() {
        return;
    }
    let label = match o.best_before {
        Some(b) => tf!("New best (was {b})", "最高を更新（前は {b}）"),
        None => t("First clean record", "はじめての記録").to_owned(),
    };
    let st = bold(14.0, Color32::from_rgb(40, 24, 0));
    let pw = ui::text_width(g, &label, &st) + 20.0;
    let (px, py) = (40.0 + w, 64.0);
    ui::round_fill(c, px, py, pw, 28.0, 14.0, ACCENT);
    ui::text(c, g, &label, px + 10.0, py + 5.0, &st, pw);
}

/// 右の欄。段ごとの出来、手ごとのずれ、叩いた順のずれ
pub fn draw(c: &Canvas, g: &mut Gfx, o: &DrillOutcome, name: &str, perfect_ms: f64) {
    let (x, y, w, h) = PANEL;
    c.fill(x, y, w, h, Color32::from_rgba_unmultiplied(8, 8, 16, 225));
    c.frame(x, y, w, h, Color32::from_gray(150));
    let run = &o.run;
    // 名前とテンポ・長さ
    let nw = ui::text(
        c,
        g,
        name,
        x + 22.0,
        y + 16.0,
        &bold(22.0, Color32::WHITE),
        280.0,
    );
    let tempo = if run.bpms[0] == run.target() {
        run.target().to_string()
    } else {
        format!("{} → {}", run.bpms[0], run.target())
    };
    let bars = STEPS as u32 * STEP_MEASURES;
    let meta = tf!(
        "{tempo} · {bars} bars · {}",
        "{tempo} ・{bars} 小節・{}",
        clock(o.elapsed_ms)
    );
    ui::text(
        c,
        g,
        &meta,
        x + 34.0 + nw,
        y + 22.0,
        &edged(15.0, Color32::from_gray(190)),
        300.0,
    );
    // 段と出来
    let grades: [Option<Grade>; STEPS] =
        std::array::from_fn(|i| o.summary.steps.get(i).and_then(|s| s.grade()));
    let sx = x + 22.0;
    let sw = 300.0;
    stairs(c, g, &run.bpms, None, &grades, (sx, y + 66.0, sw, 120.0));
    let cell = (sw - 8.0 * 3.0) / STEPS as f32;
    for (i, gr) in grades.iter().enumerate() {
        let cx = sx + i as f32 * (cell + 8.0) + cell / 2.0;
        let (mark, color) = match gr {
            Some(gr) => (grade_mark(*gr), grade_color(*gr)),
            None => ("—", Color32::from_gray(140)),
        };
        ui::text_center(c, g, mark, cx, y + 194.0, &bold(20.0, color), cell);
    }
    ui::text(
        c,
        g,
        t(
            "◎ no misses   ○ 1–2 misses   × 3 or more",
            "◎ ミスなし　○ ミス 1〜2　× 3 つ以上",
        ),
        sx,
        y + 226.0,
        &edged(12.0, Color32::from_gray(180)),
        sw + 40.0,
    );
    draw_hands(c, g, o, (x + 370.0, y + 60.0, w - 392.0, 180.0));
    draw_dots(
        c,
        g,
        o,
        perfect_ms,
        (x + 22.0, y + 262.0, w - 44.0, h - 286.0),
    );
}

/// 手ごとのずれ（平均とばらつき）。棒の真ん中が 0、左が早い、右が遅い
fn draw_hands(c: &Canvas, g: &mut Gfx, o: &DrillOutcome, (x, y, w, _h): (f32, f32, f32, f32)) {
    ui::text(
        c,
        g,
        t(
            "Timing by hand (average and spread)",
            "手ごとのずれ（平均とばらつき）",
        ),
        x,
        y,
        &edged(14.0, Color32::from_gray(200)),
        w,
    );
    let label_w = 60.0;
    let value_w = 110.0;
    let bx = x + label_w;
    let bw = w - label_w - value_w;
    let scale = bw / (2.0 * LAG_RANGE_MS as f32);
    let zero = bx + bw / 2.0;
    for (k, lag) in o.summary.hands.iter().enumerate() {
        let ry = y + 36.0 + k as f32 * 52.0;
        let color = hand_color(Some(lag.hand));
        ui::text(
            c,
            g,
            hand_name(lag.hand),
            x,
            ry + 2.0,
            &bold(18.0, color),
            label_w,
        );
        ui::round_fill(
            c,
            bx,
            ry + 4.0,
            bw,
            18.0,
            4.0,
            Color32::from_white_alpha(22),
        );
        c.fill(zero, ry, 1.0, 26.0, Color32::from_white_alpha(160));
        let at = |ms: f64| zero + (ms.clamp(-LAG_RANGE_MS, LAG_RANGE_MS) as f32) * scale;
        if let (Some(mean), Some(sd)) = (lag.mean_ms, lag.sd_ms) {
            let (l, r) = (at(mean - sd), at(mean + sd));
            ui::round_fill(
                c,
                l,
                ry + 6.0,
                (r - l).max(2.0),
                14.0,
                7.0,
                color.gamma_multiply(0.55),
            );
            c.fill(at(mean) - 1.5, ry - 1.0, 3.0, 28.0, color);
            ui::text(
                c,
                g,
                &ms_text(mean),
                bx + bw + 12.0,
                ry + 1.0,
                &bold(18.0, Color32::WHITE),
                70.0,
            );
        } else {
            ui::text(
                c,
                g,
                "—",
                bx + bw + 12.0,
                ry + 1.0,
                &bold(18.0, Color32::from_gray(150)),
                70.0,
            );
        }
        ui::text(
            c,
            g,
            t("early", "早い"),
            bx,
            ry + 24.0,
            &edged(11.0, Color32::from_gray(150)),
            60.0,
        );
        ui::text_right(
            c,
            g,
            t("late", "遅い"),
            bx + bw,
            ry + 24.0,
            &edged(11.0, Color32::from_gray(150)),
            60.0,
        );
        if let Some(lean) = lag.lean() {
            let label = match lean {
                Lean::Early => t("early", "早め"),
                Lean::Late => t("late", "遅め"),
            };
            let st = bold(13.0, Color32::from_rgb(10, 20, 40));
            let pw = ui::text_width(g, label, &st) + 16.0;
            ui::round_fill(c, x + w - pw, ry + 2.0, pw, 22.0, 11.0, color);
            ui::text(c, g, label, x + w - pw + 8.0, ry + 4.0, &st, pw);
        }
    }
}

/// 叩いた順のずれの点。横は譜面の位置（段の区切りとテンポを下に）、縦は上が早い・下が遅い。
/// 見逃した音符は上の端に × で
fn draw_dots(
    c: &Canvas,
    g: &mut Gfx,
    o: &DrillOutcome,
    perfect_ms: f64,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    ui::text(
        c,
        g,
        t("Timing, note by note", "叩いた順のずれ"),
        x,
        y,
        &edged(14.0, Color32::from_gray(200)),
        300.0,
    );
    // 凡例
    let mut lx = x + w;
    let legend: Vec<(&str, Color32)> = {
        let mut v = Vec::new();
        for lag in &o.summary.hands {
            v.push((hand_name(lag.hand), hand_color(Some(lag.hand))));
        }
        v.push((t("miss", "ミス"), MANY_COLOR));
        v
    };
    for (label, color) in legend.iter().rev() {
        let st = edged(13.0, Color32::from_gray(210));
        let tw = ui::text_width(g, label, &st);
        lx -= tw;
        ui::text(c, g, label, lx, y, &st, tw + 2.0);
        c.painter()
            .circle_filled(c.pos(lx - 9.0, y + 9.0), 4.0 * c.scale(), *color);
        lx -= 30.0;
    }
    let (px, py, pw, ph) = (x + 40.0, y + 26.0, w - 40.0, h - 50.0);
    ui::round_fill(
        c,
        px,
        py,
        pw,
        ph,
        6.0,
        Color32::from_rgba_unmultiplied(4, 4, 10, 200),
    );
    let cy = py + ph / 2.0;
    let sy = ph / 2.0 / DOT_RANGE_MS as f32;
    // PERFECT の幅
    let band = (perfect_ms.min(DOT_RANGE_MS) as f32) * sy;
    c.fill(px, cy - band, pw, band * 2.0, Color32::from_white_alpha(14));
    c.fill(px, cy, pw, 1.0, Color32::from_white_alpha(90));
    ui::text(
        c,
        g,
        t("early", "早い"),
        x,
        py + 4.0,
        &edged(12.0, Color32::from_gray(160)),
        40.0,
    );
    ui::text(
        c,
        g,
        "0",
        x + 20.0,
        cy - 8.0,
        &edged(12.0, Color32::from_gray(160)),
        20.0,
    );
    ui::text(
        c,
        g,
        t("late", "遅い"),
        x,
        py + ph - 20.0,
        &edged(12.0, Color32::from_gray(160)),
        40.0,
    );
    let bars = (STEPS as u32 * STEP_MEASURES) as f32;
    let bar_w = pw / bars;
    for s in 0..STEPS {
        let sx = px + s as f32 * STEP_MEASURES as f32 * bar_w;
        if s > 0 {
            c.fill(sx, py, 1.0, ph, Color32::from_white_alpha(60));
        }
        ui::text_center(
            c,
            g,
            &o.run.bpms[s].to_string(),
            sx + STEP_MEASURES as f32 * bar_w / 2.0,
            py + ph + 6.0,
            &edged(13.0, Color32::from_gray(200)),
            80.0,
        );
    }
    let p = c.painter();
    for d in &o.summary.dots {
        let Some(m) = d.measure.checked_sub(COUNT_IN_MEASURES) else {
            continue;
        };
        let dx = px + (m as f32 + d.pos as f32) * bar_w + 1.0;
        match d.offset_ms {
            Some(ms) => {
                let dy = cy + (ms.clamp(-DOT_RANGE_MS, DOT_RANGE_MS) as f32) * sy;
                p.circle_filled(c.pos(dx, dy), 2.6 * c.scale(), hand_color(d.hand));
            }
            None => {
                let (mx, my, r) = (dx, py + 8.0, 4.0);
                let stroke = Stroke::new(2.0 * c.scale(), MANY_COLOR);
                p.line_segment([c.pos(mx - r, my - r), c.pos(mx + r, my + r)], stroke);
                p.line_segment([c.pos(mx - r, my + r), c.pos(mx + r, my - r)], stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_offsets_read_naturally() {
        assert_eq!(clock(44_400.0), "0:44");
        assert_eq!(clock(65_000.0), "1:05");
        assert_eq!(ms_text(3.2), "+3 ms");
        assert_eq!(ms_text(-12.0), "-12 ms");
    }
}
