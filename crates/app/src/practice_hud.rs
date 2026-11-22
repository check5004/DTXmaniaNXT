//! 練習中の表示（演奏画面に重ねる）。NX の部品で出す。
//!
//! 練習は、始めの速度から目標の速度まで 10% ずつの段をのぼる（2026-11-22 ユーザーが「段をのぼってクリア」を選択）。
//! 今どこにいて、あと何をすれば終わりかが、いつも見えるようにする。文章は短く、数と絵で見せる。
//! 叩いているあいだは、レーンの上に何も重ねない（2026-11-22 のデザインの決まり）。
//!
//! - スコアの場所（左上）に、この段の何周目かと速度を、スコアの数字（`7_score numbersGD.png`）で
//! - その下の欄に、この段で落とした所の地図（1 マスが 1 拍。1 回は黄、重なったら赤）
//! - 右のジャケットの場所に、目標と「あと何段」、段の絵、この段の周の丸、段の越え方
//! - 進み具合のバー（`CActPerfProgressBar` の場所）に、曲の中の区間と今叩いている位置
//! - 段の変わり目の助走（判定の音符が落ちてくる前）だけ、レーンの真ん中で「OK! 次は 90%」などを知らせる
//! - 目標の段を越えたら、レーンの真ん中に「クリア！」を出し、少しして結果の画面へ
//! - 助走のカウントを、レーンの真ん中にコンボの数字（`ScreenPlayDrums combo.png`）で大きく
//! - 前の段で重なった拍は、落ちてくる譜面の下に赤い帯で光らせる（`play_view` が描く）

use egui::{Color32, Stroke, StrokeKind};
use engine::practice::{BeatSlot, Next, SPEED_MIN, STEP_SPEED, SpotTally, speed_percent};

use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::play_view::JUDGE_LINE_Y;
use crate::practice::{CLEAR_HOLD_MS, Cue, LapState, Practice, pass_line};
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

/// 右の欄（ジャケットの場所）
const CARD: (f32, f32, f32, f32) = (918.0, 296.0, 352.0, 330.0);
/// クリアのバッジ（`7_Excellent.png` の中の場所と大きさ）と縮め方
const BADGE: (f32, f32, f32, f32) = (402.0, 274.0, 480.0, 177.0);
const BADGE_SCALE: f32 = 0.42;

/// 進み具合のバー（play_view の `draw_progress` と同じ）
const BAR_X: f32 = 855.0;
const BAR_Y: f32 = 15.0;
const BAR_W: f32 = 20.0;
const BAR_H: f32 = 540.0;

/// レーンの場所（知らせの帯の幅）
const LANES_X: f32 = 296.0;
const LANES_W: f32 = 558.0;
/// 知らせの帯の上の端（Reverse では下の端から同じだけ離す）と高さ
const BANNER_Y: f32 = 96.0;
const BANNER_H: f32 = 230.0;
/// 知らせが出る・消える速さ（ミリ秒）
const BANNER_FADE_IN_MS: f64 = 150.0;
const BANNER_FADE_OUT_MS: f64 = 200.0;

/// カウントの数字を出す場所（知らせの帯より判定ラインの側）と、1 打ごとに縮む時間
const COUNT_Y: f32 = 440.0;
const COUNT_POP_MS: f64 = 180.0;

const SECTION_COLOR: Color32 = Color32::from_rgb(0x40, 0xC8, 0xFF);
/// ノーミス・ミス 1 回・同じ拍のミスが重なった
const OK_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);
const SLIP_COLOR: Color32 = Color32::from_rgb(0xF0, 0xD0, 0x50);
const WEAK_COLOR: Color32 = Color32::from_rgb(0xFF, 0x60, 0x50);
/// 今の段
const CURRENT_COLOR: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
const DOWN_COLOR: Color32 = SECTION_COLOR;
const GREY: Color32 = Color32::from_gray(200);
const DIM: Color32 = Color32::from_gray(150);
/// 今の段の文字（橙の上）
const INK: Color32 = Color32::from_rgb(50, 30, 0);

/// 練習中の表示をすべて描く。`now` は表示の補正を足した実時間、`last_chip_ms` は曲の最後の音の時刻、
/// `px_per_ms` は音符の落ちる速さ（判定の音符が知らせの帯に届く前に帯を消す）、`reverse` は下から上へ流すか
/// （知らせとカウントを上下逆の側に出す）、`wide` は左バスドラムのレーンを足して広げた幅（レーンの上の知らせを広げ、
/// 進み具合のバーをずらす）
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &Canvas,
    g: &mut Gfx,
    p: &Practice,
    now: f64,
    last_chip_ms: f64,
    px_per_ms: f32,
    reverse: bool,
    wide: f32,
) {
    let ln = Lanes::new(wide);
    draw_progress(c, p, now, last_chip_ms, wide);
    draw_lap_and_speed(c, g, p, now);
    draw_info(c, g, p);
    draw_card(c, g, p, now);
    draw_banner(c, g, ln, p, now, px_per_ms, reverse);
    draw_clear(c, g, ln, p, now, reverse);
    draw_count_in(c, g, ln, p, now, reverse);
}

/// レーンの横の広がり（左バスドラムのレーンを足すと右へ広がる）
#[derive(Clone, Copy)]
struct Lanes {
    w: f32,
}

impl Lanes {
    fn new(wide: f32) -> Self {
        Self { w: LANES_W + wide }
    }

    /// 真ん中の x
    fn cx(self) -> f32 {
        LANES_X + self.w / 2.0
    }
}

/// 高さ `h` の知らせの帯の上の端（チップが出てくる側から `BANNER_Y` 離す）
fn banner_top(h: f32, reverse: bool) -> f32 {
    if reverse {
        720.0 - BANNER_Y - h
    } else {
        BANNER_Y
    }
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

/// 丸（周の印）
fn dot(c: &Canvas, cx: f32, cy: f32, r: f32, fill: Color32, edge: Color32) {
    let p = c.painter();
    p.circle_filled(c.pos(cx, cy), r * c.scale(), fill);
    p.circle_stroke(c.pos(cx, cy), r * c.scale(), Stroke::new(1.5, edge));
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

/// 今の段の何周目か（1 から）。段の変わり目の助走のあいだは 1
fn lap_in_attempt(p: &Practice, now: f64) -> usize {
    let started = p
        .attempt_laps()
        .iter()
        .filter(|l| l.plan.audio.at_ms <= now)
        .count();
    started.clamp(1, p.step_laps())
}

/// 見せる速度。叩いている周の速度。周を叩き終えて次の周を待っているときは次の周の速度
fn current_speed(p: &Practice, now: f64) -> u32 {
    let Some(i) = p.current_lap(now) else {
        return p
            .laps
            .first()
            .map_or(p.speed(), |l| speed_percent(l.plan.speed));
    };
    let lap = &p.laps[i];
    if lap.complete() {
        return p
            .laps
            .get(i + 1)
            .map_or(p.speed(), |l| speed_percent(l.plan.speed));
    }
    speed_percent(lap.plan.speed)
}

/// 周（この段の何周目か / 1 段の周の数）と速度（スコアの場所）。見出しは日本語で
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
    let lap = lap_in_attempt(p, now).min(99).to_string();
    let right = score_digits(c, g, &lap, LAP_X - 10.0, DIGIT_Y, 1.0, 1.0);
    put(
        c,
        g,
        &format!("/{}", p.step_laps()),
        &bold(22.0, Color32::WHITE),
        right + 2.0,
        DIGIT_Y + 24.0,
        Align::Left,
        SPEED_X - right - 8.0,
        1.0,
    );
    let speed = format!("{}%", current_speed(p, now));
    score_digits(c, g, &speed, SPEED_X, DIGIT_Y, 1.0, 1.0);
}

// ---- 左の欄：この段で落とした所 ----

/// 左の欄。この段で落とした所の地図。段の変わり目（次の段をまだ叩いていない）は、さっきの段の地図
fn draw_info(c: &Canvas, g: &mut Gfx, p: &Practice) {
    let (x, y, w, h) = INFO;
    c.fill(x, y, w, h, Color32::from_black_alpha(190));
    c.frame(x, y, w, h, Color32::from_white_alpha(60));
    let current = p.attempt_spots();
    let started = p.attempt_laps().iter().any(|l| l.judged > 0);
    let (title, spots) = match p.done.last() {
        Some(last) if !started => (
            format!("ミスした所（さっきの {} 周）", last.laps),
            last.spots.clone(),
        ),
        _ => (format!("ミスした所（この {} 周）", p.step_laps()), current),
    };
    put(
        c,
        g,
        &title,
        &edged(14.0, Color32::WHITE),
        x + 10.0,
        y + 6.0,
        Align::Left,
        w - 20.0,
        1.0,
    );
    draw_map(
        c,
        g,
        p,
        &spots,
        p.options.rule.weak_times(),
        (x + 8.0, y + 30.0, w - 16.0, h - 36.0),
    );
}

/// 落とした所の地図。1 マスが 1 拍で、落とした周の数（1 回なら黄、重なったら赤で ×2）
fn draw_map(
    c: &Canvas,
    g: &mut Gfx,
    p: &Practice,
    spots: &SpotTally,
    weak_times: u32,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let section = p.options.section;
    let n = section.measures().max(1) as f32;
    let gap = if n > 6.0 { 2.0 } else { 4.0 };
    let row = (h / n - gap).clamp(3.0, 24.0);
    let labels = row >= 11.0;
    let label_w = if labels { 26.0 } else { 0.0 };
    let px = (row * 0.55).clamp(9.0, 13.0);
    for (i, m) in (section.first..=section.last).enumerate() {
        let ry = y + i as f32 * (row + gap);
        if labels {
            put_mid(
                c,
                g,
                &m.to_string(),
                &edged(px, GREY),
                x + label_w - 6.0,
                ry + row / 2.0,
                Align::Right,
                label_w,
            );
        }
        let beats = p.grid.beats_in(m).max(1);
        let cw = (w - label_w) / beats as f32;
        for b in 0..beats {
            let cell = (x + label_w + b as f32 * cw + 2.0, ry, cw - 4.0, row);
            let k = spots.count(BeatSlot {
                measure: m,
                beat: b,
            });
            if k >= weak_times {
                round_fill(c, cell, 4.0, WEAK_COLOR.gamma_multiply(0.92));
                if labels {
                    put_mid(
                        c,
                        g,
                        &format!("×{k}"),
                        &edged(px + 1.0, Color32::WHITE),
                        cell.0 + cell.2 / 2.0,
                        ry + row / 2.0,
                        Align::Center,
                        cell.2,
                    );
                }
            } else if k > 0 {
                round_fill(c, cell, 4.0, SLIP_COLOR.gamma_multiply(0.45));
            } else {
                round_fill(c, cell, 4.0, Color32::from_white_alpha(20));
                round_frame(c, cell, 4.0, Color32::from_white_alpha(45));
            }
        }
    }
}

// ---- 右の欄：段 ----

fn card_frame(c: &Canvas) {
    round_fill(
        c,
        CARD,
        8.0,
        Color32::from_rgba_unmultiplied(8, 10, 22, 242),
    );
    round_frame(c, CARD, 8.0, Color32::from_white_alpha(70));
}

/// 右の欄。目標と「あと何段」、段の絵、この段の周の丸、段の越え方
fn draw_card(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    card_frame(c);
    let (x, y, w, _) = CARD;
    let goal = p.plan().goal();
    put(
        c,
        g,
        &format!("目標 {goal}%"),
        &bold(19.0, Color32::WHITE),
        x + 14.0,
        y + 10.0,
        Align::Left,
        w / 2.0,
        1.0,
    );
    let (right, color) = if p.cleared() {
        ("クリア".to_owned(), OK_COLOR)
    } else {
        (format!("あと {} 段", p.steps_left()), CURRENT_COLOR)
    };
    put(
        c,
        g,
        &right,
        &bold(22.0, color),
        x + w - 14.0,
        y + 7.0,
        Align::Right,
        w / 2.0,
        1.0,
    );
    let current = (!p.cleared()).then(|| p.speed());
    draw_stairs(c, g, p, current, (x + 16.0, y + 50.0, w - 32.0, 150.0));
    if p.cleared() {
        return;
    }
    // この段の周
    put(
        c,
        g,
        &format!("この段の {} 周", p.step_laps()),
        &edged(13.0, GREY),
        x + 14.0,
        y + 218.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    let n = p.step_laps();
    let laps = p.attempt_laps();
    let playing = lap_in_attempt(p, now) - 1;
    let r = 10.0;
    let pitch = ((w - 120.0) / n as f32).min(30.0);
    for i in 0..n {
        let cx = x + 16.0 + r + i as f32 * pitch;
        let cy = y + 256.0;
        match laps.get(i) {
            Some(l) if l.complete() => {
                let fill = if l.clean() { OK_COLOR } else { SLIP_COLOR };
                dot(c, cx, cy, r, fill, fill);
            }
            Some(_) if i == playing => {
                dot(c, cx, cy, r, Color32::TRANSPARENT, CURRENT_COLOR);
                dot(c, cx, cy, r - 4.0, CURRENT_COLOR, CURRENT_COLOR);
            }
            _ => dot(
                c,
                cx,
                cy,
                r,
                Color32::TRANSPARENT,
                Color32::from_white_alpha(90),
            ),
        }
    }
    put_mid(
        c,
        g,
        &format!("{} / {n}", lap_in_attempt(p, now)),
        &bold(18.0, Color32::WHITE),
        x + w - 14.0,
        y + 256.0,
        Align::Right,
        80.0,
    );
    put(
        c,
        g,
        &pass_line(p.options.rule),
        &edged(14.0, Color32::WHITE),
        x + 14.0,
        y + 290.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
}

/// 段の絵。越えた段は緑に ✓、今の段（`current`）は橙に「いまここ」、いちばん上の段に旗
fn draw_stairs(
    c: &Canvas,
    g: &mut Gfx,
    p: &Practice,
    current: Option<u32>,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let steps = p.stairs();
    let done = p.passed_speeds();
    let n = steps.len().max(1) as f32;
    let gap = 6.0;
    let sw = (w - gap * (n - 1.0)) / n;
    let rise = (h - 30.0) / n;
    let px = (sw * 0.2).clamp(11.0, 15.0);
    for (i, &s) in steps.iter().enumerate() {
        let bx = x + i as f32 * (sw + gap);
        let top = y + h - (i as f32 + 1.0) * rise;
        let cell = (bx, top, sw, y + h - top);
        let ink = if Some(s) == current {
            round_fill(
                c,
                (bx - 3.0, top - 3.0, sw + 6.0, y + h - top + 6.0),
                7.0,
                CURRENT_COLOR.gamma_multiply(0.35),
            );
            round_fill(c, cell, 5.0, CURRENT_COLOR.gamma_multiply(0.95));
            put_mid(
                c,
                g,
                "いまここ",
                &edged(12.0, CURRENT_COLOR),
                bx + sw / 2.0,
                top - 13.0,
                Align::Center,
                sw + gap,
            );
            INK
        } else if done.contains(&s) {
            round_fill(c, cell, 5.0, OK_COLOR.gamma_multiply(0.7));
            Color32::WHITE
        } else {
            round_fill(c, cell, 5.0, Color32::from_white_alpha(18));
            round_frame(c, cell, 5.0, Color32::from_white_alpha(70));
            GREY
        };
        let style = if ink == INK {
            Style::plain(px, ink)
        } else {
            edged(px, ink)
        };
        // 越えた段の ✓ は、段が低くて下に入らないときは数字の横に
        let tall = y + h - top >= 44.0;
        let passed = done.contains(&s) && Some(s) != current;
        let label = if passed && !tall {
            format!("{s}% ✓")
        } else {
            format!("{s}%")
        };
        put_mid(
            c,
            g,
            &label,
            &style,
            bx + sw / 2.0,
            top + 14.0,
            Align::Center,
            sw - 4.0,
        );
        if passed && tall {
            put_mid(
                c,
                g,
                "✓",
                &edged(px + 4.0, Color32::WHITE),
                bx + sw / 2.0,
                top + 36.0,
                Align::Center,
                sw,
            );
        }
    }
    // いちばん上の段（目標）に旗
    let goal = p.plan().goal();
    if let Some(i) = steps.iter().position(|&s| s == goal) {
        let bx = x + i as f32 * (sw + gap);
        let top = y + h - (i as f32 + 1.0) * rise;
        let fx = bx + sw - 12.0;
        let pnt = c.painter();
        pnt.line_segment(
            [c.pos(fx, top - 28.0), c.pos(fx, top)],
            Stroke::new(2.0 * c.scale(), Color32::WHITE),
        );
        pnt.add(egui::Shape::convex_polygon(
            vec![
                c.pos(fx, top - 28.0),
                c.pos(fx - 16.0, top - 22.0),
                c.pos(fx, top - 16.0),
            ],
            OK_COLOR,
            Stroke::NONE,
        ));
    }
}

// ---- レーンの真ん中の知らせ ----

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

/// 知らせの帯（黒い地に、上下の色の線）
fn banner_frame(c: &Canvas, ln: Lanes, top: f32, h: f32, color: Color32, alpha: f32) {
    c.fill(
        LANES_X,
        top,
        ln.w,
        h,
        Color32::from_black_alpha(215).gamma_multiply(alpha),
    );
    c.fill(LANES_X, top, ln.w, 2.0, color.gamma_multiply(alpha));
    c.fill(
        LANES_X,
        top + h - 2.0,
        ln.w,
        2.0,
        color.gamma_multiply(alpha),
    );
}

/// 帯の中に 1 行（真ん中ぞろえ、上の端 `y`）
#[allow(clippy::too_many_arguments)]
fn line(c: &Canvas, g: &mut Gfx, ln: Lanes, text: &str, style: &Style, y: f32, alpha: f32) {
    put(
        c,
        g,
        text,
        style,
        ln.cx(),
        y,
        Align::Center,
        ln.w - 24.0,
        alpha,
    );
}

/// 拍の名前（「62 小節目の 3 拍目（HT）」）
fn beat_name(b: BeatSlot, lanes: &[dtx::DrumLane]) -> String {
    let lanes: Vec<&str> = lanes.iter().map(|l| l.name()).collect();
    if lanes.is_empty() {
        format!("{} 小節目の {} 拍目", b.measure, b.beat + 1)
    } else {
        format!(
            "{} 小節目の {} 拍目（{}）",
            b.measure,
            b.beat + 1,
            lanes.join("・")
        )
    }
}

/// 段の始めと、手で速度を変えた周の助走で、レーンの真ん中に知らせる
fn draw_banner(
    c: &Canvas,
    g: &mut Gfx,
    ln: Lanes,
    p: &Practice,
    now: f64,
    px_per_ms: f32,
    reverse: bool,
) {
    let Some((lap, cue, alpha)) = banner(p, now, px_per_ms) else {
        return;
    };
    let top = banner_top(BANNER_H, reverse);
    let speed = speed_percent(lap.plan.speed);
    let last = p.done.last();
    match cue {
        Cue::Start => {
            banner_frame(c, ln, top, BANNER_H, CURRENT_COLOR, alpha);
            line(
                c,
                g,
                ln,
                "スタート",
                &bold(34.0, CURRENT_COLOR),
                top + 30.0,
                alpha,
            );
            let s = format!("{speed}%");
            let w = score_digits_width(g, &s, 0.9);
            score_digits(c, g, &s, ln.cx() - w / 2.0, top + 84.0, 0.9, alpha);
            let rule = format!("1 段 {} 周。{}", p.step_laps(), pass_line(p.options.rule));
            line(
                c,
                g,
                ln,
                &rule,
                &edged(16.0, Color32::WHITE),
                top + 160.0,
                alpha,
            );
        }
        Cue::Step { next, from } => match next {
            Next::Up(to) => {
                banner_frame(c, ln, top, BANNER_H, OK_COLOR, alpha);
                line(c, g, ln, "OK!", &bold(64.0, OK_COLOR), top + 22.0, alpha);
                line(
                    c,
                    g,
                    ln,
                    &format!("次は {to}%"),
                    &bold(34.0, Color32::WHITE),
                    top + 112.0,
                    alpha,
                );
                let sub = match last {
                    Some(r) if r.missed == 0 => "ノーミス".to_owned(),
                    Some(r) => format!("ミス {} 回（同じ拍ではない）", r.missed),
                    None => String::new(),
                };
                line(c, g, ln, &sub, &edged(16.0, GREY), top + 172.0, alpha);
            }
            Next::Again | Next::Down(_) => {
                let (head, color) = match next {
                    Next::Down(to) => (format!("{to}% に下げます"), DOWN_COLOR),
                    _ => (format!("もう一度 {from}%"), SLIP_COLOR),
                };
                banner_frame(c, ln, top, BANNER_H, color, alpha);
                line(c, g, ln, &head, &bold(38.0, color), top + 16.0, alpha);
                let weak = last.map(|r| r.weak()).unwrap_or_default();
                if let Some(&(b, spot)) = weak.first() {
                    let more = if weak.len() > 1 {
                        format!("ほか {} か所", weak.len() - 1)
                    } else {
                        String::new()
                    };
                    let text =
                        format!("{}{more}で {} 回ミス", beat_name(b, &spot.lanes), spot.laps);
                    line(
                        c,
                        g,
                        ln,
                        &text,
                        &edged(18.0, Color32::WHITE),
                        top + 70.0,
                        alpha,
                    );
                    draw_measure_cells(c, g, ln, p, b, spot.laps, top + 104.0, alpha);
                }
                let note = match next {
                    Next::Down(_) => format!("{from}% で、もう一度が 2 回続いたので"),
                    _ => format!("次の {} 周は、この拍が赤く光ります", p.step_laps()),
                };
                line(c, g, ln, &note, &edged(15.0, GREY), top + 152.0, alpha);
                // あと 1 回で下げるときは、先に言っておく
                let down = from.checked_sub(STEP_SPEED).filter(|&d| d >= SPEED_MIN);
                if next == Next::Again
                    && p.again_in_row() + 1 >= engine::practice::DOWN_AFTER
                    && matches!(p.plan(), engine::practice::SpeedPlan::Ladder { .. })
                    && let Some(d) = down
                {
                    line(
                        c,
                        g,
                        ln,
                        &format!("もう一度が 2 回続いたら、{d}% に下げます"),
                        &edged(13.0, DIM),
                        top + 182.0,
                        alpha,
                    );
                }
            }
            Next::Clear => {}
        },
        Cue::Manual { from } => {
            banner_frame(c, ln, top, BANNER_H, Color32::WHITE, alpha);
            line(
                c,
                g,
                ln,
                "速度を変えました",
                &bold(30.0, Color32::WHITE),
                top + 26.0,
                alpha,
            );
            let text = format!("{from}% → {speed}%");
            line(
                c,
                g,
                ln,
                &text,
                &bold(34.0, CURRENT_COLOR),
                top + 84.0,
                alpha,
            );
            line(
                c,
                g,
                ln,
                "この段は 1 周目から数え直します",
                &edged(16.0, GREY),
                top + 150.0,
                alpha,
            );
        }
    }
}

/// 小節 `b.measure` の拍を 1 列に並べ、重なった拍を赤く（×回数）
#[allow(clippy::too_many_arguments)]
fn draw_measure_cells(
    c: &Canvas,
    g: &mut Gfx,
    ln: Lanes,
    p: &Practice,
    b: BeatSlot,
    laps: u32,
    y: f32,
    alpha: f32,
) {
    let beats = p.grid.beats_in(b.measure).max(1);
    let cw = (300.0 / beats as f32).min(76.0);
    let left = ln.cx() - cw * beats as f32 / 2.0;
    for i in 0..beats {
        let cell = (left + i as f32 * cw + 3.0, y, cw - 6.0, 32.0);
        let cx = cell.0 + cell.2 / 2.0;
        if i == b.beat {
            round_fill(c, cell, 5.0, WEAK_COLOR.gamma_multiply(0.95 * alpha));
            put_mid(
                c,
                g,
                &format!("×{laps}"),
                &edged(17.0, Color32::WHITE),
                cx,
                y + 16.0,
                Align::Center,
                cell.2,
            );
        } else {
            round_fill(
                c,
                cell,
                5.0,
                Color32::from_white_alpha(25).gamma_multiply(alpha),
            );
            round_frame(
                c,
                cell,
                5.0,
                Color32::from_white_alpha(60).gamma_multiply(alpha),
            );
            put_mid(
                c,
                g,
                &(i + 1).to_string(),
                &edged(14.0, GREY),
                cx,
                y + 16.0,
                Align::Center,
                cell.2,
            );
        }
    }
}

/// 目標の段を越えたら、結果の画面へ進むまでレーンの真ん中に「クリア！」
fn draw_clear(c: &Canvas, g: &mut Gfx, ln: Lanes, p: &Practice, now: f64, reverse: bool) {
    let Some(at) = p.cleared_at_ms else {
        return;
    };
    let since = now - at;
    if !(0.0..CLEAR_HOLD_MS + 500.0).contains(&since) {
        return;
    }
    let alpha = (since / BANNER_FADE_IN_MS).clamp(0.0, 1.0) as f32;
    let h = 260.0;
    let top = banner_top(h, reverse);
    banner_frame(c, ln, top, h, OK_COLOR, alpha);
    let (sx, sy, bw, bh) = BADGE;
    let mut y = top + 14.0;
    if let Some(t) = g.skin.tex("Graphics/7_Excellent.png") {
        c.draw_opt(
            &t,
            ln.cx() - bw * BADGE_SCALE / 2.0,
            y,
            Opt {
                src: Some(src(sx, sy, bw, bh)),
                sx: BADGE_SCALE,
                sy: BADGE_SCALE,
                alpha,
                ..Default::default()
            },
        );
        y += bh * BADGE_SCALE;
    }
    line(c, g, ln, "クリア！", &bold(48.0, OK_COLOR), y + 4.0, alpha);
    let goal = p.done.last().map_or(p.plan().goal(), |r| r.speed);
    line(
        c,
        g,
        ln,
        &format!("{goal}% の {} 周を、同じ拍のミスなしで", p.step_laps()),
        &edged(16.0, Color32::WHITE),
        y + 70.0,
        alpha,
    );
    line(
        c,
        g,
        ln,
        "まもなく結果へ",
        &edged(13.0, GREY),
        y + 98.0,
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
fn draw_progress(c: &Canvas, p: &Practice, now: f64, last_chip_ms: f64, wide: f32) {
    let bar_x = BAR_X + wide;
    if last_chip_ms <= 0.0 {
        return;
    }
    let t = p.times();
    let top = bar_y(t.end_ms, last_chip_ms);
    let bottom = bar_y(t.start_ms, last_chip_ms);
    let h = (bottom - top).max(3.0);
    c.fill(bar_x, top, BAR_W, h, SECTION_COLOR.gamma_multiply(0.35));
    c.frame(bar_x, top, BAR_W, h, SECTION_COLOR);
    // 今の周の、区間の頭から叩いたところまで
    let Some(i) = p.current_lap(now) else {
        return;
    };
    let song = p.laps[i].plan.song_ms(now).clamp(t.start_ms, t.end_ms);
    let at = bar_y(song, last_chip_ms);
    if bottom > at {
        c.fill(bar_x, at, BAR_W, bottom - at, Color32::from_white_alpha(70));
    }
    c.fill(bar_x - 3.0, at - 1.0, BAR_W + 6.0, 2.0, Color32::WHITE);
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
fn draw_count_in(c: &Canvas, g: &mut Gfx, ln: Lanes, p: &Practice, now: f64, reverse: bool) {
    let Some((n, since)) = count_in(p, now) else {
        return;
    };
    let cy = if reverse { 720.0 - COUNT_Y } else { COUNT_Y };
    let k = (since / COUNT_POP_MS).clamp(0.0, 1.0) as f32;
    let scale = 1.0 + 0.4 * (1.0 - k);
    let d = n.min(9) as u32;
    match g.skin.tex("Graphics/ScreenPlayDrums combo.png") {
        Some(t) => {
            // 1 文字 120×160、5 文字ずつ 2 段（上流のコンボの数字と同じ切り出し）
            let (w, h) = (120.0, 160.0);
            c.draw_opt(
                &t,
                ln.cx() - w * scale / 2.0,
                cy - h * scale / 2.0,
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
                t.draw(c, ln.cx() - t.w / 2.0, cy - t.h / 2.0, 1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_puts_the_banner_on_the_other_side() {
        assert_eq!(banner_top(BANNER_H, false), BANNER_Y);
        // 下から上へ流すときは、下の端から同じだけ離す
        assert_eq!(banner_top(BANNER_H, true) + BANNER_H, 720.0 - BANNER_Y);
    }

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
