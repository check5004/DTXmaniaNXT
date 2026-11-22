//! 練習中の表示（演奏画面に重ねる）。NX の部品で出す。
//!
//! 叩いているあいだは、レーンの上に何も重ねない（2026-11-22 のデザインの決まり）。
//!
//! - スコアの場所（左上）に、このセットの何周目かと速度を、スコアの数字（`7_score numbersGD.png`）で
//! - その下の欄に、このセットの直近の周の中身（ノーミスか、落とした拍）
//! - 右のジャケットの場所に、セットの周の丸、落とした所の地図（1 マスが 1 拍）、速度の段、セットのあとの決まり
//! - 進み具合のバー（`CActPerfProgressBar` の場所）に、曲の中の区間と今叩いている位置
//! - セットの始めと速度を変えた周の助走（判定の音符が落ちてくる前）だけ、レーンの真ん中で知らせる
//! - セットの合間（音符が落ちてこない）は、レーンの上にまとめと次の一手の選択肢を出す
//! - 助走のカウントを、レーンの真ん中にコンボの数字（`ScreenPlayDrums combo.png`）で大きく

use egui::{Color32, Stroke, StrokeKind};
use engine::practice::{
    BeatSlot, Choice, SET_SPEED_STEP, Section, SetKind, SetReport, speed_percent,
};

use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::play_view::JUDGE_LINE_Y;
use crate::practice::{
    Cue, LapState, Practice, beat_label, beat_short, lanes_label, lap_reason, rule_detail,
    rule_label, section_label, style_label,
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
/// 周の中身を欄の見出しに出しておく時間
const LAP_FLASH_MS: f64 = 1500.0;
/// セットの合間のまとめ（レーンの上）
const BREAK: (f32, f32, f32, f32) = (304.0, 40.0, 542.0, 560.0);
/// 合間の選択肢の行の間隔
const CHOICE_PITCH: f32 = 42.0;
/// 合間に出すクリアのバッジ（`7_Excellent.png` の中の高さ）と縮め方
const BADGE_H: f32 = 177.0;
const BADGE_SCALE: f32 = 0.3;

/// 進み具合のバー（play_view の `draw_progress` と同じ）
const BAR_X: f32 = 855.0;
const BAR_Y: f32 = 15.0;
const BAR_W: f32 = 20.0;
const BAR_H: f32 = 540.0;

/// レーンの場所（知らせの帯の幅）
const LANES_X: f32 = 296.0;
const LANES_W: f32 = 558.0;
/// レーンの高さ（判定の線と打面まで。下のゲージとずれのバーは入れない）
const LANES_H: f32 = 625.0;
/// 知らせの帯の上の端（Reverse では下の端から同じだけ離す）と高さ
const BANNER_Y: f32 = 96.0;
const BANNER_H: f32 = 150.0;
/// 知らせが出る・消える速さ（ミリ秒）
const BANNER_FADE_IN_MS: f64 = 150.0;
const BANNER_FADE_OUT_MS: f64 = 200.0;

/// カウントの数字を出す場所（レーンの真ん中より判定ラインの側。反対の側に知らせが出るので）と、
/// 1 打ごとに縮む時間
const COUNT_Y: f32 = 390.0;
const COUNT_POP_MS: f64 = 180.0;

const SECTION_COLOR: Color32 = Color32::from_rgb(0x40, 0xC8, 0xFF);
/// ノーミス・うっかり（1 回きり）・苦手（同じ拍を何度も）
const OK_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);
const SLIP_COLOR: Color32 = Color32::from_rgb(0xF0, 0xD0, 0x50);
const WEAK_COLOR: Color32 = Color32::from_rgb(0xFF, 0x60, 0x50);
/// 今の速度（速度の段と、選んでいる選択肢）
const CURRENT_COLOR: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
const DOWN_COLOR: Color32 = SECTION_COLOR;
const GREY: Color32 = Color32::from_gray(200);

/// 練習中の表示をすべて描く。`now` は表示の補正を足した実時間、`last_chip_ms` は曲の最後の音の時刻、
/// `px_per_ms` は音符の落ちる速さ（判定の音符が知らせの帯に届く前に帯を消す）、`reverse` は下から上へ流すか
/// （知らせとカウントを上下逆の側に出す）、`wide` は左バスドラムのレーンを足して広げた幅（レーンの上のものを広げ、
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
    draw_progress(c, p, now, last_chip_ms, wide);
    draw_lap_and_speed(c, g, p, now);
    draw_recent(c, g, p);
    draw_card(c, g, p, now);
    draw_banner(c, g, p, now, px_per_ms, reverse, wide);
    draw_count_in(c, g, p, now, reverse, wide);
    draw_break(c, g, p, now, reverse, wide);
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

/// 今のセットの何周目か（1 から）。セットの合間はセットの周の数
fn lap_in_set(p: &Practice, now: f64) -> usize {
    if p.between.is_some() {
        return p.set_laps();
    }
    let started = p
        .set_laps_so_far()
        .iter()
        .filter(|l| l.plan.audio.at_ms <= now)
        .count();
    started.clamp(1, p.set_laps())
}

/// 今鳴っている周の速度（助走中や合間は次の周の速度）
fn current_speed(p: &Practice, now: f64) -> u32 {
    match p.current_lap(now) {
        Some(i) if p.between.is_none() => speed_percent(p.laps[i].plan.speed),
        _ if p.between.is_some() => p.between.as_ref().map_or(p.speed(), |b| b.report.speed),
        _ => p.speed(),
    }
}

/// 周（このセットの何周目か / セットの周の数）と速度（スコアの場所）。見出しは日本語で
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
    let lap = lap_in_set(p, now).min(99).to_string();
    let right = score_digits(c, g, &lap, LAP_X - 10.0, DIGIT_Y, 1.0, 1.0);
    put(
        c,
        g,
        &format!("/{}", p.set_laps()),
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

// ---- 左の欄：直近の周（合間はセットのまとめ） ----

/// 周の色。ノーミスは緑、苦手の拍を落とした周は赤、うっかりだけなら黄
fn lap_color(l: &LapState, weak: &[BeatSlot]) -> Color32 {
    if l.clean() {
        OK_COLOR
    } else if l.misses.iter().any(|(b, _)| weak.contains(b)) {
        WEAK_COLOR
    } else {
        SLIP_COLOR
    }
}

/// 左の欄。叩いているあいだはこのセットの直近の周の中身、合間はセットのまとめ
fn draw_recent(c: &Canvas, g: &mut Gfx, p: &Practice) {
    let (x, y, w, h) = INFO;
    c.fill(x, y, w, h, Color32::from_black_alpha(190));
    c.frame(x, y, w, h, Color32::from_white_alpha(60));
    let left = x + 8.0;
    let right = x + w - 8.0;
    let line = |c: &Canvas, g: &mut Gfx, k: usize, text: &str, color: Color32| {
        put(
            c,
            g,
            text,
            &edged(13.0, color),
            left,
            y + 30.0 + k as f32 * 22.0,
            Align::Left,
            right - left,
            1.0,
        );
    };
    if let Some(b) = &p.between {
        let r = &b.report;
        put(
            c,
            g,
            &format!("セット {} の周", r.index + 1),
            &edged(14.0, Color32::WHITE),
            left,
            y + 6.0,
            Align::Left,
            right - left,
            1.0,
        );
        line(
            c,
            g,
            0,
            &format!(
                "ノーミス {} 周 / ミスあり {} 周",
                r.clean_laps,
                r.laps - r.clean_laps
            ),
            Color32::WHITE,
        );
        let weak = r.weak();
        let mut k = 1;
        for (slot, spot) in weak.iter().take(2) {
            line(
                c,
                g,
                k,
                &format!(
                    "苦手: {}（{}、{} 周で {} 回）",
                    beat_label(*slot),
                    lanes_label(&spot.lanes),
                    r.laps,
                    spot.laps
                ),
                WEAK_COLOR,
            );
            k += 1;
        }
        let slips: Vec<String> = r
            .spots
            .spots()
            .filter(|(_, s)| s.laps < r.weak_times)
            .map(|(b, _)| beat_short(b))
            .collect();
        if k < 4 && !slips.is_empty() {
            let more = if slips.len() > 3 { " ほか" } else { "" };
            line(
                c,
                g,
                k,
                &format!("うっかり: {}{more}", slips[..slips.len().min(3)].join("、")),
                SLIP_COLOR,
            );
        } else if weak.is_empty() && slips.is_empty() {
            line(c, g, 1, "全部の周をノーミスで叩けました", OK_COLOR);
        }
        return;
    }
    let title_w = put(
        c,
        g,
        &section_label(p.set_section()),
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
        &format!(
            "{} / 苦手の見方 {}",
            style_label(p.options.style),
            rule_label(p.options.rule)
        ),
        &edged(12.0, GREY),
        right,
        y + 9.0,
        Align::Right,
        right - left - title_w - 10.0,
        1.0,
    );
    let set = p.set_laps_so_far();
    let done: Vec<(usize, &LapState)> = set
        .iter()
        .enumerate()
        .filter(|(_, l)| l.complete() && !l.notes.is_empty())
        .collect();
    if done.is_empty() {
        line(c, g, 0, "直近の周", Color32::from_gray(150));
        line(
            c,
            g,
            1,
            "このセットはまだ叩き終えた周がありません",
            Color32::from_gray(170),
        );
        return;
    }
    let weak: Vec<BeatSlot> = p.report().weak().iter().map(|(b, _)| *b).collect();
    for (k, (i, l)) in done.iter().rev().take(RECENT_LAPS + 1).enumerate() {
        let color = lap_color(l, &weak);
        line(
            c,
            g,
            k,
            &format!("{} 周目　{}", i + 1, lap_reason(&l.summary())),
            color,
        );
    }
}

// ---- 右の欄：セット（周の丸、落とした所の地図、速度の段） ----

/// 欄の枠
fn card_frame(c: &Canvas) {
    round_fill(
        c,
        CARD,
        8.0,
        Color32::from_rgba_unmultiplied(8, 10, 22, 242),
    );
    round_frame(c, CARD, 8.0, Color32::from_white_alpha(70));
}

/// 右の欄。叩いているあいだはこのセット、合間はこれまでのセット
fn draw_card(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    card_frame(c);
    if p.between.is_some() {
        draw_history(c, g, p, now);
    } else {
        draw_set(c, g, p, now);
    }
}

/// 叩いているセット
fn draw_set(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let (x, y, w, _) = CARD;
    let report = p.report();
    let weak: Vec<BeatSlot> = report.weak().iter().map(|(b, _)| *b).collect();
    let set = p.set_laps_so_far();
    // 見出し。周の音符をすべて判定したら、少しのあいだその周の中身に塗り替える
    let flash = p
        .just_settled(now, LAP_FLASH_MS)
        .filter(|l| l.set == p.set_index());
    match flash {
        Some(l) => draw_lap_flash(c, g, p, l, &weak),
        None => {
            let title = match p.kind() {
                SetKind::Whole => format!("セット {}", p.set_index() + 1),
                SetKind::Focus(_) => "苦手の所だけ".to_owned(),
            };
            let title_w = put(
                c,
                g,
                &title,
                &bold(19.0, Color32::WHITE),
                x + 14.0,
                y + 10.0,
                Align::Left,
                150.0,
                1.0,
            );
            put(
                c,
                g,
                &format!(
                    "{}　{} 周で 1 セット",
                    section_label(p.set_section()),
                    p.set_laps()
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
    // 周の丸
    let n = p.set_laps();
    let playing = set
        .iter()
        .position(|l| !l.complete() && l.plan.audio.at_ms <= now);
    let gap = ((w - 100.0) / n as f32).min(30.0);
    let r = (gap * 0.32).clamp(5.0, 9.0);
    for k in 0..n {
        let cx = x + 16.0 + r + k as f32 * gap;
        let cy = y + 52.0;
        match set.get(k) {
            Some(l) if l.complete() => {
                let col = lap_color(l, &weak);
                dot(c, cx, cy, r, col, col);
            }
            _ if playing == Some(k) || (playing.is_none() && k == set.len().min(n - 1)) => {
                dot(
                    c,
                    cx,
                    cy,
                    r,
                    CURRENT_COLOR.gamma_multiply(0.75),
                    CURRENT_COLOR,
                );
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
    put(
        c,
        g,
        &format!("{}/{n} 周", lap_in_set(p, now)),
        &edged(14.0, Color32::WHITE),
        x + w - 14.0,
        y + 44.0,
        Align::Right,
        80.0,
        1.0,
    );
    put(
        c,
        g,
        "落とした所（このセット）",
        &edged(12.0, GREY),
        x + 14.0,
        y + 70.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    let dim = match p.kind() {
        SetKind::Whole => None,
        SetKind::Focus(f) => Some(f),
    };
    draw_map(c, g, p, &report, (x + 12.0, y + 90.0, w - 24.0, 124.0), dim);
    // 苦手
    let (line, color) = match report.weak().first() {
        Some((slot, spot)) => (
            format!(
                "苦手: {}（{}）",
                beat_label(*slot),
                lanes_label(&spot.lanes)
            ),
            WEAK_COLOR,
        ),
        None if report.laps == 0 => (String::new(), GREY),
        None if report.spots.spots().next().is_none() => ("ここまでノーミス".to_owned(), OK_COLOR),
        None => (
            "苦手なし（1 回きりのミスはうっかり）".to_owned(),
            SLIP_COLOR,
        ),
    };
    put(
        c,
        g,
        &line,
        &bold(15.0, color),
        x + 14.0,
        y + 220.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    draw_steps(c, g, p, y + 248.0);
    let (next, color) = match (p.pending_speed(now), p.kind(), p.plan().goal()) {
        (Some(s), _, _) => (format!("次の周のあと {s}% に変えます"), CURRENT_COLOR),
        (None, SetKind::Focus(_), _) => (
            format!(
                "落とさずに回れたら{}に戻る",
                section_label(p.options.section)
            ),
            Color32::WHITE,
        ),
        (None, SetKind::Whole, Some(goal)) if p.speed() < goal => (
            format!(
                "セットのあと: 苦手が無ければ {}% へ",
                (p.speed() + SET_SPEED_STEP).min(goal)
            ),
            Color32::WHITE,
        ),
        (None, SetKind::Whole, Some(_)) => (
            "セットのあと: 苦手が無ければクリア".to_owned(),
            Color32::WHITE,
        ),
        (None, SetKind::Whole, None) => (
            "セットのあと: 落とした所と次の一手を出します".to_owned(),
            Color32::WHITE,
        ),
    };
    put(
        c,
        g,
        &next,
        &bold(15.0, color),
        x + 14.0,
        y + 282.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    put(
        c,
        g,
        &format!(
            "1 回きりのミスは数えません。{}",
            rule_detail(p.options.rule)
        ),
        &edged(12.0, GREY),
        x + 14.0,
        y + 306.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
}

/// 周の中身を見出しに（ノーミスなら FULL COMBO の絵も）
fn draw_lap_flash(c: &Canvas, g: &mut Gfx, p: &Practice, l: &LapState, weak: &[BeatSlot]) {
    let (x, y, w, _) = CARD;
    let color = lap_color(l, weak);
    let [r, gr, b, _] = color.to_array();
    round_fill(
        c,
        (x + 1.0, y + 1.0, w - 2.0, 34.0),
        8.0,
        Color32::from_rgba_unmultiplied(r / 4, gr / 4, b / 4, 250),
    );
    let k = p
        .set_laps_so_far()
        .iter()
        .position(|s| std::ptr::eq(s, l))
        .map_or(l.plan.index + 1, |i| i + 1);
    let head = format!("{k} 周目　{}", if l.clean() { "ノーミス！" } else { "" });
    let head_w = put(
        c,
        g,
        head.trim_end(),
        &bold(18.0, color),
        x + 14.0,
        y + 8.0,
        Align::Left,
        200.0,
        1.0,
    );
    if l.clean()
        && let Some(t) = g.skin.tex("Graphics/7_FullCombo.png")
    {
        // バッジの部分だけ（1280×720 の絵の中の 490×167）
        let s = 0.22;
        let (bw, bh) = (490.0, 167.0);
        c.draw_opt(
            &t,
            x + w - 12.0 - bw * s,
            y + 18.0 - bh * s / 2.0,
            Opt {
                src: Some(src(397.0, 282.0, bw, bh)),
                sx: s,
                sy: s,
                ..Default::default()
            },
        );
    } else if !l.clean() {
        put(
            c,
            g,
            &lap_reason(&l.summary()),
            &bold(16.0, color),
            x + w - 14.0,
            y + 10.0,
            Align::Right,
            w - 42.0 - head_w,
            1.0,
        );
    }
}

/// 落とした所の地図。1 マスが 1 拍で、落とした周の数（1 なら黄、苦手なら赤）。
/// `dim` があれば、その外の小節は薄く（苦手の所だけのセット）
fn draw_map(
    c: &Canvas,
    g: &mut Gfx,
    p: &Practice,
    report: &SetReport,
    (x, y, w, h): (f32, f32, f32, f32),
    dim: Option<Section>,
) {
    let section = p.options.section;
    let n = section.measures().max(1) as f32;
    let row = (h / n - 4.0).clamp(4.0, 26.0);
    let labels = row >= 11.0;
    let label_w = if labels { 34.0 } else { 0.0 };
    let px = (row * 0.55).clamp(9.0, 14.0);
    for (i, m) in (section.first..=section.last).enumerate() {
        let ry = y + i as f32 * (row + 4.0);
        let faded = dim.is_some_and(|f| !f.contains(m));
        if labels {
            put_mid(
                c,
                g,
                &m.to_string(),
                &edged(px, if faded { Color32::from_gray(110) } else { GREY }),
                x + label_w - 8.0,
                ry + row / 2.0,
                Align::Right,
                label_w,
            );
        }
        let beats = p.grid.beats_in(m).max(1);
        let cw = (w - label_w) / beats as f32;
        for b in 0..beats {
            let cell = (x + label_w + b as f32 * cw + 2.0, ry, cw - 4.0, row);
            if faded {
                round_fill(c, cell, 4.0, Color32::from_white_alpha(8));
                round_frame(c, cell, 4.0, Color32::from_white_alpha(25));
                continue;
            }
            let k = report.spots.count(BeatSlot {
                measure: m,
                beat: b,
            });
            let (fill, text) = if k >= report.weak_times {
                (WEAK_COLOR.gamma_multiply(0.9), Some(format!("×{k}")))
            } else if k > 0 {
                (SLIP_COLOR.gamma_multiply(0.45), Some(k.to_string()))
            } else {
                (Color32::from_white_alpha(22), None)
            };
            round_fill(c, cell, 4.0, fill);
            if k == 0 {
                round_frame(c, cell, 4.0, Color32::from_white_alpha(45));
            }
            if let (Some(t), true) = (text, labels) {
                put_mid(
                    c,
                    g,
                    &t,
                    &edged(px, Color32::WHITE),
                    cell.0 + cell.2 / 2.0,
                    ry + row / 2.0,
                    Align::Center,
                    cell.2,
                );
            }
        }
    }
}

/// 速度の段（段階アップなら始めの速度から目標まで。苦手なしで叩けた速度は緑、今は橙）
fn draw_steps(c: &Canvas, g: &mut Gfx, p: &Practice, y: f32) {
    let (x, _, w, _) = CARD;
    let current = match &p.between {
        Some(b) => b.report.speed,
        None => p.speed(),
    };
    let mut steps = p.plan().steps();
    if steps.is_empty() {
        steps = vec![current];
    }
    let goal = p.plan().goal();
    let done = p.clean_speeds();
    let gap = 8.0;
    let n = steps.len() as f32;
    let cw = ((w - 28.0 - gap * (n - 1.0)) / n).min(120.0);
    for (i, &s) in steps.iter().enumerate() {
        let bx = x + 14.0 + i as f32 * (cw + gap);
        let cell = (bx, y, cw, 24.0);
        let (ink, label) = if s == current {
            round_fill(
                c,
                (bx - 2.0, y - 2.0, cw + 4.0, 28.0),
                6.0,
                CURRENT_COLOR.gamma_multiply(0.3),
            );
            round_fill(c, cell, 5.0, CURRENT_COLOR.gamma_multiply(0.92));
            (Color32::from_rgb(50, 30, 0), true)
        } else if done.contains(&s) {
            round_fill(c, cell, 5.0, OK_COLOR.gamma_multiply(0.6));
            (Color32::WHITE, true)
        } else {
            round_fill(c, cell, 5.0, Color32::from_white_alpha(22));
            round_frame(c, cell, 5.0, Color32::from_white_alpha(60));
            (Color32::from_gray(190), true)
        };
        if label {
            let text = match goal {
                Some(t) if s == t && cw >= 70.0 => format!("{s}%　目標"),
                None => format!("{s}%　固定"),
                _ => format!("{s}%"),
            };
            put_mid(
                c,
                g,
                &text,
                &Style::plain(13.0, ink),
                bx + cw / 2.0,
                y + 12.0,
                Align::Center,
                cw - 4.0,
            );
        }
    }
}

/// これまでのセット（合間）
fn draw_history(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let (x, y, w, _) = CARD;
    put(
        c,
        g,
        "これまでのセット",
        &bold(19.0, Color32::WHITE),
        x + 14.0,
        y + 10.0,
        Align::Left,
        200.0,
        1.0,
    );
    put(
        c,
        g,
        &section_label(p.options.section),
        &edged(12.0, GREY),
        x + w - 14.0,
        y + 14.0,
        Align::Right,
        120.0,
        1.0,
    );
    let shown = 3;
    let skip = p.reports.len().saturating_sub(shown);
    for (k, r) in p.reports.iter().skip(skip).enumerate() {
        let ry = y + 44.0 + k as f32 * 50.0;
        let cell = (x + 12.0, ry, w - 24.0, 44.0);
        round_fill(c, cell, 6.0, Color32::from_white_alpha(16));
        round_frame(c, cell, 6.0, Color32::from_white_alpha(40));
        let what = match r.kind {
            SetKind::Whole => String::new(),
            SetKind::Focus(f) => format!("　{}", section_label(f)),
        };
        put(
            c,
            g,
            &format!("セット {}　{}%{what}", r.index + 1, r.speed),
            &edged(14.0, Color32::WHITE),
            x + 24.0,
            ry + 5.0,
            Align::Left,
            w - 130.0,
            1.0,
        );
        put(
            c,
            g,
            &format!("ノーミス {}/{}", r.clean_laps, r.laps),
            &edged(12.0, GREY),
            x + w - 24.0,
            ry + 7.0,
            Align::Right,
            100.0,
            1.0,
        );
        let weak = r.weak();
        let (line, color) = match weak.first() {
            Some((b, _)) if weak.len() == 1 => {
                (format!("苦手 1（{}）", beat_short(*b)), WEAK_COLOR)
            }
            Some(_) => (format!("苦手 {} か所", weak.len()), WEAK_COLOR),
            None if r.cleared() => ("苦手なし（クリア）".to_owned(), OK_COLOR),
            None => ("苦手なし".to_owned(), OK_COLOR),
        };
        put(
            c,
            g,
            &line,
            &edged(13.0, color),
            x + 24.0,
            ry + 24.0,
            Align::Left,
            w - 48.0,
            1.0,
        );
    }
    if let Some(first) = p.laps.first() {
        let secs = ((now - first.plan.audio.at_ms) / 1000.0).max(0.0) as u64;
        put(
            c,
            g,
            &format!("練習した時間 {} 分 {} 秒", secs / 60, secs % 60),
            &edged(13.0, GREY),
            x + 14.0,
            y + 200.0,
            Align::Left,
            w - 28.0,
            1.0,
        );
    }
    draw_steps(c, g, p, y + 248.0);
    let goal_line = match p.plan().goal() {
        _ if p.cleared() => "クリアしました。続けても記録に残ります".to_owned(),
        Some(t) => format!("クリア: {t}% で苦手の無いセット"),
        None => "固定の速度。上げ下げは合間に選べます".to_owned(),
    };
    put(
        c,
        g,
        &goal_line,
        &bold(
            15.0,
            if p.cleared() {
                OK_COLOR
            } else {
                Color32::WHITE
            },
        ),
        x + 14.0,
        y + 282.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    put(
        c,
        g,
        "セットの合間は、叩くまで止まって待ちます",
        &edged(12.0, GREY),
        x + 14.0,
        y + 306.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
}

// ---- レーンの上：セットの合間のまとめと選択肢 ----

/// 選択肢の名前と、右に出す中身
fn choice_text(p: &Practice, report: &SetReport, choice: Choice) -> (String, String) {
    let n = p.set_laps();
    match choice {
        Choice::Quit => ("練習をやめる".to_owned(), "結果へ".to_owned()),
        Choice::Set { kind, speed } => {
            let section = match kind {
                SetKind::Whole => p.options.section,
                SetKind::Focus(f) => f,
            };
            let detail = format!("{}　{speed}%", section_label(section));
            let name = match (report.kind, kind) {
                (SetKind::Whole, SetKind::Focus(_)) => format!("苦手の所だけ {n} 周"),
                (SetKind::Focus(_), SetKind::Whole) => "区間に戻って 1 セット".to_owned(),
                (SetKind::Focus(_), SetKind::Focus(_)) if speed < report.speed => {
                    "速度を下げて苦手の所".to_owned()
                }
                (SetKind::Focus(_), SetKind::Focus(_)) => "苦手の所をもう 1 セット".to_owned(),
                (SetKind::Whole, SetKind::Whole) if speed > report.speed => {
                    "速度を上げて次のセット".to_owned()
                }
                (SetKind::Whole, SetKind::Whole) if speed < report.speed => {
                    "速度を下げてもう 1 セット".to_owned()
                }
                (SetKind::Whole, SetKind::Whole) => "区間をもう 1 セット".to_owned(),
            };
            (name, detail)
        }
    }
}

/// セットの合間。音符の無いレーンの上に、まとめと次の一手
fn draw_break(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64, reverse: bool, wide: f32) {
    let lanes_w = LANES_W + wide;
    let Some(b) = &p.between else {
        return;
    };
    if now < b.show_from_ms {
        return;
    }
    let alpha = (((now - b.show_from_ms) / BANNER_FADE_IN_MS).clamp(0.0, 1.0)) as f32;
    let r = &b.report;
    let cleared = r.cleared() && p.plan().goal().is_none_or(|t| r.speed >= t);
    // 中身の高さに合わせ、BREAK の中で上下の真ん中に置く
    let badge_h = if cleared {
        BADGE_H * BADGE_SCALE - 6.0
    } else {
        0.0
    };
    let map_h = (p.options.section.measures() as f32 * 28.0).min(116.0);
    let content = 12.0
        + badge_h
        + 36.0
        + 30.0
        + map_h
        + 6.0
        + 30.0
        + b.choices.list.len() as f32 * CHOICE_PITCH
        + 40.0;
    let (x, top, w, full) = BREAK;
    let w = w + wide;
    let h = content.min(full);
    let y = top + (full - h) / 2.0;
    // 上下逆なら、レーンもパネルも上下に映す（ゲージとずれのバーの側は暗くしない）
    let (y, lanes_y) = if reverse {
        (720.0 - y - h, 720.0 - LANES_H)
    } else {
        (y, 0.0)
    };
    c.fill(
        LANES_X,
        lanes_y,
        lanes_w,
        LANES_H,
        Color32::from_black_alpha(200).gamma_multiply(alpha),
    );
    let accent = if cleared { OK_COLOR } else { CURRENT_COLOR };
    round_fill(
        c,
        (x, y, w, h),
        10.0,
        Color32::from_rgba_unmultiplied(8, 10, 22, 245).gamma_multiply(alpha),
    );
    c.painter().rect_stroke(
        c.rect(x, y, w, h),
        10.0 * c.scale(),
        Stroke::new(2.0 * c.scale(), accent.gamma_multiply(alpha)),
        StrokeKind::Inside,
    );
    let cx = x + w / 2.0;
    let mut ty = y + 12.0;
    if cleared && let Some(t) = g.skin.tex("Graphics/7_Excellent.png") {
        // バッジの部分だけ（1280×720 の絵の中の 480×177）
        let s = BADGE_SCALE;
        let (bw, bh) = (480.0, BADGE_H);
        c.draw_opt(
            &t,
            cx - bw * s / 2.0,
            ty - 4.0,
            Opt {
                src: Some(src(402.0, 274.0, bw, bh)),
                sx: s,
                sy: s,
                alpha,
                ..Default::default()
            },
        );
        ty += bh * s - 6.0;
    }
    let title = if cleared {
        "区間クリア！".to_owned()
    } else {
        match r.kind {
            SetKind::Whole => format!("セット {} おわり", r.index + 1),
            SetKind::Focus(_) => "苦手の所だけ　おわり".to_owned(),
        }
    };
    put(
        c,
        g,
        &title,
        &bold(26.0, accent),
        cx,
        ty,
        Align::Center,
        w - 20.0,
        alpha,
    );
    ty += 36.0;
    let what = match r.kind {
        SetKind::Whole => String::new(),
        SetKind::Focus(f) => format!("{}　", section_label(f)),
    };
    put(
        c,
        g,
        &format!(
            "{what}{}%　{} 周　ノーミス {} 周",
            r.speed, r.laps, r.clean_laps
        ),
        &edged(16.0, Color32::WHITE),
        cx,
        ty,
        Align::Center,
        w - 20.0,
        alpha,
    );
    ty += 30.0;
    let dim = match r.kind {
        SetKind::Whole => None,
        SetKind::Focus(f) => Some(f),
    };
    draw_map(c, g, p, r, (x + 90.0, ty, w - 180.0, map_h), dim);
    ty += map_h + 6.0;
    let weak = r.weak();
    let (line, color) = match weak.as_slice() {
        [] if r.spots.spots().next().is_none() => {
            ("全部の周をノーミスで叩けました".to_owned(), OK_COLOR)
        }
        [] => (
            format!("苦手はありません（1 回きりのミスが {} か所）", r.slips()),
            OK_COLOR,
        ),
        [(slot, spot)] => (
            format!(
                "苦手は{}（{}）。{} 周で {} 回落としました",
                beat_label(*slot),
                lanes_label(&spot.lanes),
                r.laps,
                spot.laps
            ),
            WEAK_COLOR,
        ),
        many => (
            format!(
                "苦手が {} か所（{} ほか）",
                many.len(),
                beat_label(many[0].0)
            ),
            WEAK_COLOR,
        ),
    };
    put(
        c,
        g,
        &line,
        &edged(15.0, color),
        cx,
        ty,
        Align::Center,
        w - 24.0,
        alpha,
    );
    ty += 30.0;
    let ready = b.ready(now);
    for (i, &choice) in b.choices.list.iter().enumerate() {
        let ry = ty + i as f32 * CHOICE_PITCH;
        let cell = (x + 28.0, ry, w - 56.0, 36.0);
        let (name, detail) = choice_text(p, r, choice);
        let rec = i == b.choices.recommended;
        let detail = if rec {
            format!("{detail}　おすすめ")
        } else {
            detail
        };
        if i == b.cursor {
            round_fill(
                c,
                (cell.0 - 2.0, cell.1 - 2.0, cell.2 + 4.0, cell.3 + 4.0),
                7.0,
                CURRENT_COLOR.gamma_multiply(0.3 * alpha),
            );
            round_fill(
                c,
                cell,
                6.0,
                CURRENT_COLOR.gamma_multiply(if ready { 0.92 } else { 0.55 } * alpha),
            );
            let ink = Color32::from_rgb(40, 25, 0);
            put_mid(
                c,
                g,
                &format!("▶ {name}"),
                &Style {
                    bold: true,
                    ..Style::plain(17.0, ink)
                },
                cell.0 + 16.0,
                ry + 18.0,
                Align::Left,
                cell.2 * 0.6,
            );
            put_mid(
                c,
                g,
                &detail,
                &Style::plain(14.0, Color32::from_rgb(60, 40, 0)),
                cell.0 + cell.2 - 16.0,
                ry + 18.0,
                Align::Right,
                cell.2 * 0.4,
            );
        } else {
            round_fill(c, cell, 6.0, Color32::from_white_alpha(18));
            round_frame(c, cell, 6.0, Color32::from_white_alpha(50));
            put_mid(
                c,
                g,
                &format!("　 {name}"),
                &edged(17.0, Color32::WHITE),
                cell.0 + 16.0,
                ry + 18.0,
                Align::Left,
                cell.2 * 0.6,
            );
            put_mid(
                c,
                g,
                &detail,
                &edged(14.0, if rec { CURRENT_COLOR } else { GREY }),
                cell.0 + cell.2 - 16.0,
                ry + 18.0,
                Align::Right,
                cell.2 * 0.4,
            );
        }
    }
    let foot = if ready {
        "HT / LT で選ぶ　CY で決める（↑↓ と Enter でも）"
    } else {
        "ひと息ついてから選んでください"
    };
    put(
        c,
        g,
        foot,
        &edged(13.0, GREY),
        cx,
        y + h - 26.0,
        Align::Center,
        w - 24.0,
        alpha,
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

/// セットの始めと速度を変えた周の助走で、レーンの真ん中に知らせる
fn draw_banner(
    c: &Canvas,
    g: &mut Gfx,
    p: &Practice,
    now: f64,
    px_per_ms: f32,
    reverse: bool,
    wide: f32,
) {
    let lanes_w = LANES_W + wide;
    let lanes_cx = LANES_X + lanes_w / 2.0;
    let Some((lap, cue, alpha)) = banner(p, now, px_per_ms) else {
        return;
    };
    let top = banner_top(BANNER_H, reverse);
    let speed = speed_percent(lap.plan.speed);
    let n = p.set_laps();
    let (head, color, from, sub) = match cue {
        Cue::Start => (
            "練習スタート".to_owned(),
            CURRENT_COLOR,
            None,
            format!("{n} 周で 1 セット。セットのあとに落とした所と次の一手を出します"),
        ),
        Cue::Set { kind, from } => {
            let changed = (from != speed).then_some(from);
            match kind {
                SetKind::Focus(f) => (
                    "苦手の所だけ".to_owned(),
                    CURRENT_COLOR,
                    changed,
                    format!(
                        "{}　{n} 周。落とさずに回れたら区間に戻ります",
                        section_label(f)
                    ),
                ),
                SetKind::Whole => {
                    let head = match changed {
                        Some(f) if f < speed => "速度アップ".to_owned(),
                        Some(_) => "速度ダウン".to_owned(),
                        None => format!("セット {}", lap.set + 1),
                    };
                    let color = match changed {
                        Some(f) if f > speed => DOWN_COLOR,
                        _ => CURRENT_COLOR,
                    };
                    (
                        head,
                        color,
                        changed,
                        format!(
                            "セット {}　{}　{n} 周",
                            lap.set + 1,
                            section_label(p.options.section)
                        ),
                    )
                }
            }
        }
        Cue::Manual { from } => (
            "速度を変えました".to_owned(),
            Color32::WHITE,
            Some(from),
            format!("このセットは {speed}% の周から数え直します"),
        ),
    };
    c.fill(
        LANES_X,
        top,
        lanes_w,
        BANNER_H,
        Color32::from_black_alpha(175).gamma_multiply(alpha),
    );
    c.fill(LANES_X, top, lanes_w, 3.0, color.gamma_multiply(alpha));
    c.fill(
        LANES_X,
        top + BANNER_H - 3.0,
        lanes_w,
        3.0,
        color.gamma_multiply(alpha),
    );
    put(
        c,
        g,
        &head,
        &bold(30.0, color),
        lanes_cx,
        top + 8.0,
        Align::Center,
        lanes_w - 20.0,
        alpha,
    );
    // 80% → 90%（変えないときは速度だけ）
    let scale = 0.85;
    let to = format!("{speed}%");
    let digits_y = top + 50.0;
    match from {
        Some(from) => {
            let from = format!("{from}%");
            let arrow_w = 50.0;
            let total =
                score_digits_width(g, &from, scale) + arrow_w + score_digits_width(g, &to, scale);
            let mut at = lanes_cx - total / 2.0;
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
            score_digits(c, g, &to, lanes_cx - w / 2.0, digits_y, scale, alpha);
        }
    }
    put(
        c,
        g,
        &sub,
        &edged(16.0, Color32::WHITE),
        lanes_cx,
        top + 110.0,
        Align::Center,
        lanes_w - 24.0,
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
fn draw_count_in(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64, reverse: bool, wide: f32) {
    let lanes_cx = LANES_X + (LANES_W + wide) / 2.0;
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
                lanes_cx - w * scale / 2.0,
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
                t.draw(c, lanes_cx - t.w / 2.0, cy - t.h / 2.0, 1.0, 1.0);
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
