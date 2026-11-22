//! 練習中の表示（演奏画面に重ねる）。NX の部品で出す。
//!
//! 叩いているあいだは、レーンの上に何も重ねない（2026-11-22 のデザインの決まり）。
//!
//! - スコアの場所（左上）に周と速度を、スコアの数字（`7_score numbersGD.png`）で
//! - その下の欄に、区間・つなぎ方・合格の基準と、直近の周の合否とわけ
//! - 右のジャケットの場所に、段階アップのはしご（今の速度、あと何回の合格で上がるか、下がるまでの不合格）。
//!   周の合否は、その欄の見出しを少しのあいだ塗り替えて知らせる
//! - 進み具合のバー（`CActPerfProgressBar` の場所）に、曲の中の区間と今叩いている位置
//! - 速度が変わる周とクリアのあとの助走（判定の音符が落ちてくる前）だけ、レーンの真ん中で知らせる
//! - 助走のカウントを、レーンの真ん中にコンボの数字（`ScreenPlayDrums combo.png`）で大きく

use egui::{Color32, Stroke, StrokeKind};
use engine::practice::{FAILS_TO_DOWN, LADDER_UP, PASSES_TO_UP, SpeedPlan, speed_percent};

use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::play_view::JUDGE_LINE_Y;
use crate::practice::{
    Cue, LapState, LapSummary, Practice, lap_reason, rule_pass_label, section_label, style_label,
};
use crate::text::Style;

/// 周と速度の数字の場所（上流のスコアと同じ高さ）
const LAP_X: f32 = 40.0;
const SPEED_X: f32 = 150.0;
const LABEL_Y: f32 = 10.0;
const DIGIT_Y: f32 = 41.0;
/// スコアの数字の 1 文字の幅と送り
const DIGIT_W: f32 = 36.0;
const DIGIT_H: f32 = 50.0;
const DIGIT_ADVANCE: f32 = 34.0;

/// 左の欄（スコアの下、左のパネルの上）
const INFO: (f32, f32, f32, f32) = (6.0, 98.0, 282.0, 148.0);
/// 左の欄に出す直近の周の数
const RECENT_LAPS: usize = 4;

/// 右の欄（ジャケットの場所）
const CARD: (f32, f32, f32, f32) = (918.0, 296.0, 352.0, 330.0);
/// 周の合否を欄の見出しに出しておく時間
const VERDICT_MS: f64 = 1500.0;
/// はしごの、今より遅い済んだ段を出す数の上限（段が多すぎると読めないので）
const DONE_STEPS: usize = 2;

/// 進み具合のバー（play_view の `draw_progress` と同じ）
const BAR_X: f32 = 855.0;
const BAR_Y: f32 = 15.0;
const BAR_W: f32 = 20.0;
const BAR_H: f32 = 540.0;

/// レーンの場所（知らせの帯の幅）
const LANES_X: f32 = 296.0;
const LANES_W: f32 = 558.0;
const LANES_CX: f32 = LANES_X + LANES_W / 2.0;
/// 知らせの帯の上の端と高さ（クリアは大きく）
const BANNER_Y: f32 = 96.0;
const BANNER_H: f32 = 150.0;
const CLEAR_H: f32 = 200.0;
/// 知らせが出る・消える速さ（ミリ秒）
const BANNER_FADE_IN_MS: f64 = 150.0;
const BANNER_FADE_OUT_MS: f64 = 200.0;

/// カウントの数字を出す場所（レーンの真ん中より下。上に知らせが出るので）と、1 打ごとに縮む時間
const COUNT_Y: f32 = 390.0;
const COUNT_POP_MS: f64 = 180.0;

const SECTION_COLOR: Color32 = Color32::from_rgb(0x40, 0xC8, 0xFF);
const PASS_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);
const FAIL_COLOR: Color32 = Color32::from_rgb(0xF0, 0x80, 0x60);
/// 今の速度（はしごの段と、上げる知らせ）
const CURRENT_COLOR: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
const DOWN_COLOR: Color32 = SECTION_COLOR;
const GREY: Color32 = Color32::from_gray(200);

/// 練習中の表示をすべて描く。`now` は表示の補正を足した実時間、`last_chip_ms` は曲の最後の音の時刻、
/// `px_per_ms` は音符の落ちる速さ（判定の音符が知らせの帯に届く前に帯を消す）
pub fn draw(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64, last_chip_ms: f64, px_per_ms: f32) {
    draw_progress(c, p, now, last_chip_ms);
    draw_lap_and_speed(c, g, p, now);
    draw_recent(c, g, p, now);
    draw_card(c, g, p, now);
    draw_banner(c, g, p, now, px_per_ms);
    draw_count_in(c, g, p, now);
}

// ---- 文字と形 ----

#[derive(Clone, Copy)]
enum Align {
    Left,
    Right,
    Center,
}

/// 黒い縁の文字
fn edged(px: f32, color: Color32) -> Style {
    Style::edged(px, color, Color32::BLACK, (px / 12.0).max(1.0))
}

fn bold(px: f32, color: Color32) -> Style {
    Style {
        bold: true,
        ..Style::edged(px, color, Color32::BLACK, (px / 10.0).max(1.5))
    }
}

/// 文字を描いて幅を返す。`y` は上の端。`max_w` を超えるときは横に縮める
#[allow(clippy::too_many_arguments)]
fn put(
    c: &Canvas,
    g: &mut Gfx,
    text: &str,
    style: &Style,
    x: f32,
    y: f32,
    align: Align,
    max_w: f32,
    alpha: f32,
) -> f32 {
    let Some(t) = g.texts.get(text, style) else {
        return 0.0;
    };
    let sx = if t.w > max_w { max_w / t.w } else { 1.0 };
    let w = t.w * sx;
    let left = match align {
        Align::Left => x,
        Align::Right => x - w,
        Align::Center => x - w / 2.0,
    };
    t.draw(c, left, y, sx, alpha);
    w
}

/// 文字を、縦の真ん中を `cy` に合わせて描く
#[allow(clippy::too_many_arguments)]
fn put_mid(
    c: &Canvas,
    g: &mut Gfx,
    text: &str,
    style: &Style,
    x: f32,
    cy: f32,
    align: Align,
    max_w: f32,
) -> f32 {
    let h = g.texts.get(text, style).map_or(0.0, |t| t.h);
    put(c, g, text, style, x, cy - h / 2.0, align, max_w, 1.0)
}

/// 角の丸い四角を塗る
fn round_fill(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32), r: f32, color: Color32) {
    c.painter()
        .rect_filled(c.rect(x, y, w, h), r * c.scale(), color);
}

/// 角の丸い枠
fn round_frame(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32), r: f32, color: Color32) {
    c.painter().rect_stroke(
        c.rect(x, y, w, h),
        r * c.scale(),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
}

/// 丸（合格の印）
fn dot(c: &Canvas, cx: f32, cy: f32, r: f32, fill: Color32, edge: Color32) {
    let p = c.painter();
    p.circle_filled(c.pos(cx, cy), r * c.scale(), fill);
    p.circle_stroke(c.pos(cx, cy), r * c.scale(), Stroke::new(1.5, edge));
}

/// チェックの印（字の形に頼らず線で）
fn check(c: &Canvas, x: f32, cy: f32, size: f32, color: Color32) {
    let stroke = Stroke::new(2.0 * c.scale(), color);
    let a = c.pos(x, cy);
    let b = c.pos(x + size * 0.35, cy + size * 0.35);
    let d = c.pos(x + size, cy - size * 0.45);
    c.painter().line_segment([a, b], stroke);
    c.painter().line_segment([b, d], stroke);
}

/// スコアの数字で `text`（数字と %）を描き、右の端を返す。スキンに無ければふつうの文字で
fn score_digits(
    c: &Canvas,
    g: &mut Gfx,
    text: &str,
    x: f32,
    y: f32,
    scale: f32,
    alpha: f32,
) -> f32 {
    let Some(t) = g.skin.tex("Graphics/7_score numbersGD.png") else {
        let style = Style::edged(
            40.0 * scale,
            Color32::from_rgb(220, 240, 120),
            Color32::BLACK,
            3.0,
        );
        return x + put(c, g, text, &style, x, y, Align::Left, f32::MAX, alpha);
    };
    let pct = g.skin.tex("Graphics/7_RatePercent_l.png");
    let mut at = x;
    for ch in text.chars() {
        if let Some(d) = ch.to_digit(10) {
            c.draw_opt(
                &t,
                at,
                y,
                Opt {
                    src: Some(src(d as f32 * DIGIT_W, 0.0, DIGIT_W, DIGIT_H)),
                    sx: scale,
                    sy: scale,
                    alpha,
                    ..Default::default()
                },
            );
            at += DIGIT_ADVANCE * scale;
        } else if ch == '%' {
            // % は達成率の大きい数字のもの（上流 7_RatePercent_l.png）
            match &pct {
                Some(p) => {
                    c.draw_opt(
                        p,
                        at + 2.0,
                        y + (DIGIT_H - 4.0) * scale - p.h * scale,
                        Opt {
                            sx: scale,
                            sy: scale,
                            alpha,
                            ..Default::default()
                        },
                    );
                    at += 2.0 + p.w * scale;
                }
                None => g.dfp(c, at + 2.0, y + 20.0 * scale, "%", false),
            }
        }
    }
    at
}

/// スコアの数字で描いたときの幅
fn score_digits_width(g: &Gfx, text: &str, scale: f32) -> f32 {
    let pct_w = g
        .skin
        .tex("Graphics/7_RatePercent_l.png")
        .map_or(20.0, |p| p.w);
    text.chars()
        .map(|ch| match ch {
            '%' => 2.0 + pct_w,
            _ => DIGIT_ADVANCE,
        })
        .sum::<f32>()
        * scale
}

// ---- 左上：周と速度 ----

/// 今の周の番号（1 から）。助走のあいだは 1 周目
fn lap_number(p: &Practice, now: f64) -> usize {
    p.current_lap(now).map_or(1, |i| i + 1)
}

/// 今鳴っている周の速度（助走中は 1 周目の速度）
fn current_speed(p: &Practice, now: f64) -> u32 {
    p.current_lap(now)
        .or((!p.laps.is_empty()).then_some(0))
        .map_or(p.ladder().speed(), |i| speed_percent(p.laps[i].plan.speed))
}

/// 周と速度（スコアの場所）。見出しは日本語で
fn draw_lap_and_speed(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let head = bold(22.0, Color32::WHITE);
    put(c, g, "周", &head, LAP_X, LABEL_Y, Align::Left, 100.0, 1.0);
    put(
        c,
        g,
        "速度",
        &head,
        SPEED_X,
        LABEL_Y,
        Align::Left,
        100.0,
        1.0,
    );
    let lap = lap_number(p, now).min(999).to_string();
    score_digits(c, g, &lap, LAP_X, DIGIT_Y, 1.0, 1.0);
    let speed = format!("{}%", current_speed(p, now));
    score_digits(c, g, &speed, SPEED_X, DIGIT_Y, 1.0, 1.0);
}

// ---- 左の欄：区間と直近の周 ----

/// 区間、つなぎ方と合格の基準、直近の周の合否とわけ
fn draw_recent(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let (x, y, w, h) = INFO;
    c.fill(x, y, w, h, Color32::from_black_alpha(190));
    c.frame(x, y, w, h, Color32::from_white_alpha(60));
    let o = p.options;
    let left = x + 8.0;
    let right = x + w - 8.0;
    let title_w = put(
        c,
        g,
        &section_label(o.section),
        &edged(16.0, Color32::WHITE),
        left,
        y + 5.0,
        Align::Left,
        140.0,
        1.0,
    );
    put(
        c,
        g,
        &format!("{} / {}", style_label(o.style), rule_pass_label(o.rule)),
        &edged(12.0, GREY),
        right,
        y + 9.0,
        Align::Right,
        right - left - title_w - 10.0,
        1.0,
    );
    put(
        c,
        g,
        "直近の周",
        &edged(12.0, Color32::from_gray(150)),
        left,
        y + 30.0,
        Align::Left,
        200.0,
        1.0,
    );
    let done: Vec<LapSummary> = p
        .summaries(now)
        .into_iter()
        .filter(|l| l.completed)
        .collect();
    if done.is_empty() {
        put(
            c,
            g,
            "まだ終わった周はありません",
            &edged(13.0, Color32::from_gray(170)),
            left,
            y + 50.0,
            Align::Left,
            w - 16.0,
            1.0,
        );
        return;
    }
    let style = |color| edged(13.0, color);
    for (k, l) in done.iter().rev().take(RECENT_LAPS).enumerate() {
        let ry = y + 50.0 + k as f32 * 22.0;
        let plain = Color32::from_gray(210);
        put(
            c,
            g,
            &format!("{} 周目", l.index + 1),
            &style(plain),
            left,
            ry,
            Align::Left,
            60.0,
            1.0,
        );
        put(
            c,
            g,
            &format!("{}%", speed_percent(l.speed)),
            &style(plain),
            left + 64.0,
            ry,
            Align::Left,
            44.0,
            1.0,
        );
        let ok = l.passed == Some(true);
        let color = if ok { PASS_COLOR } else { FAIL_COLOR };
        put(
            c,
            g,
            if ok { "合格" } else { "不合格" },
            &style(color),
            left + 108.0,
            ry,
            Align::Left,
            50.0,
            1.0,
        );
        put(
            c,
            g,
            &lap_reason(l, o.rule),
            &style(if ok { Color32::WHITE } else { FAIL_COLOR }),
            left + 162.0,
            ry,
            Align::Left,
            right - left - 162.0,
            1.0,
        );
    }
}

// ---- 右の欄：段階アップのはしご ----

/// はしごの段（下から）。済んだ段は今に近いものだけ。固定なら今の速度だけ
fn shown_steps(p: &Practice) -> Vec<u32> {
    let ladder = p.ladder();
    let steps = ladder.steps();
    if steps.is_empty() {
        return vec![ladder.speed()];
    }
    let below = steps.iter().filter(|&&s| s < ladder.speed()).count();
    steps[below.saturating_sub(DONE_STEPS)..].to_vec()
}

/// 固定の速度で、終わりから続けて合格した周の数と、合格の数・終わった周の数
fn fixed_tally(p: &Practice, now: f64) -> (usize, usize, usize) {
    let done: Vec<LapSummary> = p
        .summaries(now)
        .into_iter()
        .filter(|l| l.completed)
        .collect();
    let streak = done
        .iter()
        .rev()
        .take_while(|l| l.passed == Some(true))
        .count();
    let passed = done.iter().filter(|l| l.passed == Some(true)).count();
    (streak, passed, done.len())
}

/// 右の欄。見出し（周の合否が出たら少しのあいだ塗り替える）、はしご、あと何をすればよいか
fn draw_card(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let (x, y, w, _) = CARD;
    round_fill(
        c,
        CARD,
        8.0,
        Color32::from_rgba_unmultiplied(8, 10, 22, 242),
    );
    round_frame(c, CARD, 8.0, Color32::from_white_alpha(70));
    match p.just_settled(now, VERDICT_MS) {
        Some(l) => draw_verdict(c, g, p, &l),
        None => {
            let title_w = put(
                c,
                g,
                "区間練習",
                &bold(19.0, Color32::WHITE),
                x + 14.0,
                y + 10.0,
                Align::Left,
                120.0,
                1.0,
            );
            let o = p.options;
            put(
                c,
                g,
                &format!(
                    "{}　{} / {}",
                    section_label(o.section),
                    style_label(o.style),
                    rule_pass_label(o.rule)
                ),
                &edged(12.0, GREY),
                x + w - 14.0,
                y + 14.0,
                Align::Right,
                w - 40.0 - title_w,
                1.0,
            );
        }
    }
    draw_ladder(c, g, p);
    draw_next_step(c, g, p, now);
}

/// 周の合否（欄の見出しを塗り替える）
fn draw_verdict(c: &Canvas, g: &mut Gfx, p: &Practice, l: &LapSummary) {
    let (x, y, w, _) = CARD;
    let ok = l.passed == Some(true);
    let color = if ok { PASS_COLOR } else { FAIL_COLOR };
    let [r, gr, b, _] = color.to_array();
    round_fill(
        c,
        (x + 1.0, y + 1.0, w - 2.0, 40.0),
        8.0,
        Color32::from_rgba_unmultiplied(r / 4, gr / 4, b / 4, 250),
    );
    c.fill(x + 1.0, y + 39.0, w - 2.0, 2.0, color);
    let head = format!(
        "{} 周目　{}",
        l.index + 1,
        if ok { "合格！" } else { "不合格" }
    );
    let head_w = put(
        c,
        g,
        &head,
        &bold(20.0, color),
        x + 14.0,
        y + 8.0,
        Align::Left,
        200.0,
        1.0,
    );
    let full = l.tally.miss == 0 && l.tally.poor == 0;
    if ok
        && full
        && let Some(t) = g.skin.tex("Graphics/7_FullCombo.png")
    {
        // バッジの部分だけ（1280×720 の絵の中の 490×167）
        let s = 0.27;
        let (bw, bh) = (490.0, 167.0);
        c.draw_opt(
            &t,
            x + w - 12.0 - bw * s,
            y + 21.0 - bh * s / 2.0,
            Opt {
                src: Some(src(397.0, 282.0, bw, bh)),
                sx: s,
                sy: s,
                ..Default::default()
            },
        );
    } else {
        put(
            c,
            g,
            &lap_reason(l, p.options.rule),
            &bold(18.0, if ok { Color32::WHITE } else { color }),
            x + w - 14.0,
            y + 10.0,
            Align::Right,
            w - 42.0 - head_w,
            1.0,
        );
    }
}

/// はしご。上が目標、今の段は橙に合格の印、済んだ段は緑
fn draw_ladder(c: &Canvas, g: &mut Gfx, p: &Practice) {
    let (x, y, w, h) = CARD;
    let ladder = p.ladder();
    let current = ladder.speed();
    let cleared = ladder.cleared();
    let fixed = matches!(ladder.plan(), SpeedPlan::Fixed(_));
    let target = match ladder.plan() {
        SpeedPlan::Ladder { target, .. } => target,
        SpeedPlan::Fixed(s) => s,
    };
    let (passes, _) = ladder.streaks();
    let steps = shown_steps(p);
    let top = y + 48.0;
    let bottom = y + h - 70.0;
    let rh = ((bottom - top) / steps.len().max(1) as f32).min(34.0);
    let bh = rh - 6.0;
    let px = (bh * 0.55).clamp(10.0, 14.0);
    let bx = x + 74.0;
    let bw = w - 88.0;
    for (i, &s) in steps.iter().rev().enumerate() {
        let ry = top + i as f32 * rh;
        let cy = ry + bh / 2.0;
        let done = cleared || s < current;
        let now_here = s == current && !cleared;
        // 速度
        let label_color = if now_here {
            CURRENT_COLOR
        } else if done {
            Color32::from_rgb(220, 255, 225)
        } else {
            Color32::from_gray(170)
        };
        put_mid(
            c,
            g,
            &format!("{s}%"),
            &bold(px + 3.0, label_color),
            x + 64.0,
            cy,
            Align::Right,
            60.0,
        );
        // 段の板
        let bar = (bx, ry, bw, bh);
        if now_here {
            round_fill(
                c,
                (bx - 3.0, ry - 3.0, bw + 6.0, bh + 6.0),
                7.0,
                Color32::from_rgba_unmultiplied(0xFF, 0xB8, 0x40, 70),
            );
            round_fill(c, bar, 5.0, CURRENT_COLOR.gamma_multiply(0.92));
        } else if done {
            round_fill(c, bar, 5.0, PASS_COLOR.gamma_multiply(0.6));
        } else {
            round_fill(c, bar, 5.0, Color32::from_white_alpha(22));
            round_frame(c, bar, 5.0, Color32::from_white_alpha(50));
        }
        // 段の中身
        let dark = |color| Style::plain(px, color);
        if now_here {
            let name = if fixed {
                "固定の速度"
            } else if s == target {
                "いまの速度（目標）"
            } else {
                "いまの速度"
            };
            put_mid(
                c,
                g,
                name,
                &dark(Color32::from_rgb(50, 30, 0)),
                bx + 10.0,
                cy,
                Align::Left,
                bw - 70.0,
            );
            if !fixed {
                let r = (bh / 2.0 - 3.0).clamp(3.0, 7.0);
                for k in 0..PASSES_TO_UP {
                    let fill = if k < passes {
                        Color32::from_rgb(40, 120, 60)
                    } else {
                        Color32::from_white_alpha(90)
                    };
                    let cx = bx + bw - 16.0 - (PASSES_TO_UP - 1 - k) as f32 * 22.0;
                    dot(c, cx, cy, r, fill, Color32::from_rgb(40, 25, 0));
                }
            }
        } else if done {
            let ink = Color32::from_rgb(10, 50, 20);
            check(c, bx + 10.0, cy, px * 0.8, ink);
            let label = if cleared && s == current {
                "クリア"
            } else {
                "合格 2 回"
            };
            put_mid(
                c,
                g,
                label,
                &dark(ink),
                bx + 14.0 + px,
                cy,
                Align::Left,
                bw - 30.0,
            );
        } else if s == target {
            put_mid(
                c,
                g,
                &format!("目標　合格 {PASSES_TO_UP} 回でクリア"),
                &edged(px, Color32::from_gray(230)),
                bx + 10.0,
                cy,
                Align::Left,
                bw - 20.0,
            );
        }
    }
}

/// 欄の下の 2 行。あと何回の合格で何 % か、下がるまでの不合格、行き詰まったときの案内
fn draw_next_step(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let (x, y, w, h) = CARD;
    let ladder = p.ladder();
    let (passes, fails) = ladder.streaks();
    let need = PASSES_TO_UP.saturating_sub(passes);
    let fixed = matches!(ladder.plan(), SpeedPlan::Fixed(_));
    let (line, color) = if fixed {
        let (streak, passed, done) = fixed_tally(p, now);
        (
            format!("合格 {passed} / {done} 周（続けて {streak} 回）"),
            Color32::WHITE,
        )
    } else if ladder.cleared() {
        (
            format!("クリア！ {}% で合格 {PASSES_TO_UP} 回", ladder.speed()),
            PASS_COLOR,
        )
    } else if let Some(s) = p.pending_speed(now) {
        (format!("次の周のあと {s}% に変えます"), CURRENT_COLOR)
    } else if let Some(up) = ladder.next_up() {
        (format!("あと {need} 回の合格で {up}%"), Color32::WHITE)
    } else {
        (format!("あと {need} 回の合格でクリア"), Color32::WHITE)
    };
    put(
        c,
        g,
        &line,
        &bold(18.0, color),
        x + 14.0,
        y + h - 62.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    let (sub, color) = if fixed {
        (", と . で 5% ずつ変えられます".to_owned(), GREY)
    } else if ladder.cleared() {
        (
            "Esc で結果へ。続けて叩いても記録に残ります".to_owned(),
            GREY,
        )
    } else if ladder.stuck() {
        (
            format!(
                "この速度で不合格 {} 回。, で 5% 下げられます",
                ladder.fails_here()
            ),
            CURRENT_COLOR,
        )
    } else if let Some(d) = ladder.next_down() {
        (
            format!("不合格 {fails}/{FAILS_TO_DOWN}（{FAILS_TO_DOWN} 回続くと {d}% に下げます）"),
            GREY,
        )
    } else {
        (
            format!("不合格 {fails}/{FAILS_TO_DOWN}（始めの速度より下げません）"),
            GREY,
        )
    };
    put(
        c,
        g,
        &sub,
        &edged(12.0, color),
        x + 14.0,
        y + h - 30.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
}

// ---- レーンの真ん中：助走のあいだの知らせ ----

/// 知らせを出す周と、その濃さ（0〜1）。まだ始まっていない最初の周に知らせがあり、それより前の周を
/// 叩き終えていて、その周の判定の音符が帯に届く前のあいだだけ
fn banner(p: &Practice, now: f64, px_per_ms: f32) -> Option<(&LapState, Cue, f32)> {
    let i = p.laps.iter().position(|l| l.plan.start_ms > now)?;
    let lap = &p.laps[i];
    let cue = lap.cue?;
    if !lap.plan.has_run_up() || p.laps[..i].iter().any(|l| !l.complete()) {
        return None;
    }
    let reach_ms = f64::from((JUDGE_LINE_Y - BANNER_Y) / px_per_ms.max(0.01));
    let hide_at = lap.plan.start_ms - reach_ms;
    let appear_at = match i.checked_sub(1) {
        Some(prev) => p.laps[prev]
            .settled_at_ms
            .unwrap_or(lap.plan.audio.at_ms)
            .min(lap.plan.audio.at_ms),
        None => lap.plan.audio.at_ms - 300.0,
    };
    if now >= hide_at || now < appear_at {
        return None;
    }
    let fade_in = ((now - appear_at) / BANNER_FADE_IN_MS).clamp(0.0, 1.0);
    let fade_out = ((hide_at - now) / BANNER_FADE_OUT_MS).clamp(0.0, 1.0);
    Some((lap, cue, fade_in.min(fade_out) as f32))
}

/// 速度が変わる周とクリアのあとの助走で、レーンの真ん中に知らせる
fn draw_banner(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64, px_per_ms: f32) {
    let Some((lap, cue, alpha)) = banner(p, now, px_per_ms) else {
        return;
    };
    let speed = speed_percent(lap.plan.speed);
    let ladder = p.ladder();
    if cue == Cue::Clear {
        draw_clear(c, g, speed, alpha);
        return;
    }
    let (head, color, from) = match cue {
        Cue::Start => ("練習スタート", CURRENT_COLOR, None),
        Cue::Up { from } => ("速度アップ", CURRENT_COLOR, Some(from)),
        Cue::Down { from } => ("速度ダウン", DOWN_COLOR, Some(from)),
        Cue::Manual { from } => ("速度を変えました", Color32::WHITE, Some(from)),
        Cue::Clear => unreachable!(),
    };
    let sub = match cue {
        Cue::Start => match ladder.plan() {
            SpeedPlan::Ladder { target, .. } => {
                format!("合格 {PASSES_TO_UP} 回ごとに {LADDER_UP}% 上げます（目標 {target}%）")
            }
            SpeedPlan::Fixed(_) => "速度は固定（, と . で変えられます）".to_owned(),
        },
        Cue::Up { .. } => format!("合格 {PASSES_TO_UP} 回。助走のあと {speed}% で叩きます"),
        Cue::Down { .. } => {
            format!("不合格が {FAILS_TO_DOWN} 回続いたので、助走のあと {speed}% で叩きます")
        }
        _ => format!("助走のあと {speed}% で叩きます"),
    };
    c.fill(
        LANES_X,
        BANNER_Y,
        LANES_W,
        BANNER_H,
        Color32::from_black_alpha(175).gamma_multiply(alpha),
    );
    c.fill(LANES_X, BANNER_Y, LANES_W, 3.0, color.gamma_multiply(alpha));
    c.fill(
        LANES_X,
        BANNER_Y + BANNER_H - 3.0,
        LANES_W,
        3.0,
        color.gamma_multiply(alpha),
    );
    put(
        c,
        g,
        head,
        &bold(30.0, color),
        LANES_CX,
        BANNER_Y + 8.0,
        Align::Center,
        LANES_W - 20.0,
        alpha,
    );
    // 80% → 85%（始めは速度だけ）
    let scale = 0.85;
    let to = format!("{speed}%");
    let digits_y = BANNER_Y + 50.0;
    match from {
        Some(from) => {
            let from = format!("{from}%");
            let arrow_w = 50.0;
            let total =
                score_digits_width(g, &from, scale) + arrow_w + score_digits_width(g, &to, scale);
            let mut at = LANES_CX - total / 2.0;
            at = score_digits(c, g, &from, at, digits_y, scale, alpha);
            put(
                c,
                g,
                "→",
                &bold(34.0, Color32::WHITE),
                at + arrow_w / 2.0,
                digits_y + 2.0,
                Align::Center,
                arrow_w,
                alpha,
            );
            score_digits(c, g, &to, at + arrow_w, digits_y, scale, alpha);
        }
        None => {
            let w = score_digits_width(g, &to, scale);
            score_digits(c, g, &to, LANES_CX - w / 2.0, digits_y, scale, alpha);
        }
    }
    put(
        c,
        g,
        &sub,
        &edged(16.0, Color32::WHITE),
        LANES_CX,
        BANNER_Y + 110.0,
        Align::Center,
        LANES_W - 24.0,
        alpha,
    );
}

/// クリアの知らせ（NX の Excellent の絵で）
fn draw_clear(c: &Canvas, g: &mut Gfx, speed: u32, alpha: f32) {
    c.fill(
        LANES_X,
        BANNER_Y,
        LANES_W,
        CLEAR_H,
        Color32::from_black_alpha(180).gamma_multiply(alpha),
    );
    c.fill(
        LANES_X,
        BANNER_Y,
        LANES_W,
        3.0,
        PASS_COLOR.gamma_multiply(alpha),
    );
    c.fill(
        LANES_X,
        BANNER_Y + CLEAR_H - 3.0,
        LANES_W,
        3.0,
        PASS_COLOR.gamma_multiply(alpha),
    );
    let mut ty = BANNER_Y + 10.0;
    if let Some(t) = g.skin.tex("Graphics/7_Excellent.png") {
        // バッジの部分だけ（1280×720 の絵の中の 480×177）
        let s = 0.55;
        let (bw, bh) = (480.0, 177.0);
        c.draw_opt(
            &t,
            LANES_CX - bw * s / 2.0,
            ty,
            Opt {
                src: Some(src(402.0, 274.0, bw, bh)),
                sx: s,
                sy: s,
                alpha,
                ..Default::default()
            },
        );
        ty += bh * s + 2.0;
    }
    put(
        c,
        g,
        "区間クリア",
        &bold(28.0, PASS_COLOR),
        LANES_CX,
        ty,
        Align::Center,
        LANES_W - 20.0,
        alpha,
    );
    ty += 36.0;
    put(
        c,
        g,
        &format!("目標の {speed}% で {PASSES_TO_UP} 回合格しました"),
        &edged(16.0, Color32::WHITE),
        LANES_CX,
        ty,
        Align::Center,
        LANES_W - 24.0,
        alpha,
    );
    ty += 22.0;
    put(
        c,
        g,
        &format!("Esc で結果へ。続けて叩けば {speed}% のまま回ります"),
        &edged(14.0, GREY),
        LANES_CX,
        ty,
        Align::Center,
        LANES_W - 24.0,
        alpha,
    );
}

// ---- 進み具合のバー ----

/// 進み具合のバーの上の y（曲の時刻 → 下が曲の頭）
fn bar_y(song_ms: f64, last_chip_ms: f64) -> f32 {
    let r = (song_ms / last_chip_ms).clamp(0.0, 1.0) as f32;
    BAR_Y + BAR_H * (1.0 - r)
}

/// 進み具合のバーに、曲の中の区間と、今叩いている位置
fn draw_progress(c: &Canvas, p: &Practice, now: f64, last_chip_ms: f64) {
    if last_chip_ms <= 0.0 {
        return;
    }
    let t = p.times();
    let top = bar_y(t.end_ms, last_chip_ms);
    let bottom = bar_y(t.start_ms, last_chip_ms);
    let h = (bottom - top).max(3.0);
    c.fill(BAR_X, top, BAR_W, h, SECTION_COLOR.gamma_multiply(0.35));
    c.frame(BAR_X, top, BAR_W, h, SECTION_COLOR);
    // 今の周の、区間の頭から叩いたところまで
    let Some(i) = p.current_lap(now) else {
        return;
    };
    let song = p.laps[i].plan.song_ms(now).clamp(t.start_ms, t.end_ms);
    let at = bar_y(song, last_chip_ms);
    if bottom > at {
        c.fill(BAR_X, at, BAR_W, bottom - at, Color32::from_white_alpha(70));
    }
    c.fill(BAR_X - 3.0, at - 1.0, BAR_W + 6.0, 2.0, Color32::WHITE);
}

// ---- 助走のカウント ----

/// 助走のカウント（鳴ったクリックの数。周の中は None）
fn count_in(p: &Practice, now: f64) -> Option<(usize, f64)> {
    let lap = p.laps.iter().find(|l| l.in_run_up(now))?;
    let n = lap.plan.clicks.iter().filter(|&&c| c <= now).count();
    let since = lap.plan.clicks.get(n.checked_sub(1)?).map(|&c| now - c)?;
    Some((n, since))
}

/// 助走のカウントを大きく。クリックが鳴るたびに大きく出て縮む
fn draw_count_in(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let Some((n, since)) = count_in(p, now) else {
        return;
    };
    let k = (since / COUNT_POP_MS).clamp(0.0, 1.0) as f32;
    let scale = 1.0 + 0.4 * (1.0 - k);
    let d = n.min(9) as u32;
    match g.skin.tex("Graphics/ScreenPlayDrums combo.png") {
        Some(t) => {
            // 1 文字 120×160、5 文字ずつ 2 段（上流のコンボの数字と同じ切り出し）
            let (w, h) = (120.0, 160.0);
            c.draw_opt(
                &t,
                LANES_CX - w * scale / 2.0,
                COUNT_Y - h * scale / 2.0,
                Opt {
                    src: Some(src((d % 5) as f32 * w, (d / 5) as f32 * h, w, h)),
                    sx: scale,
                    sy: scale,
                    alpha: 0.6 + 0.4 * (1.0 - k),
                    ..Default::default()
                },
            );
        }
        None => {
            let style = Style::edged(96.0, Color32::WHITE, Color32::BLACK, 4.0);
            if let Some(t) = g.texts.get(&d.to_string(), &style) {
                t.draw(c, LANES_CX - t.w / 2.0, COUNT_Y - t.h / 2.0, 1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_runs_from_the_bottom() {
        assert_eq!(bar_y(0.0, 1000.0), BAR_Y + BAR_H);
        assert_eq!(bar_y(1000.0, 1000.0), BAR_Y);
        assert_eq!(bar_y(500.0, 1000.0), BAR_Y + BAR_H / 2.0);
        // 曲の外ははみ出さない
        assert_eq!(bar_y(-10.0, 1000.0), BAR_Y + BAR_H);
        assert_eq!(bar_y(2000.0, 1000.0), BAR_Y);
    }
}
