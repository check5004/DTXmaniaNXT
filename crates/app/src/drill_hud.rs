//! 基礎練習中の表示（演奏画面に重ねる）。区間練習と同じ場所に、同じ部品で出す。
//!
//! - 左上（スコアの場所）に、この段の何小節目かとテンポ
//! - 右（ジャケットの場所）に、目標と「あと何段」、段の絵（テンポ）、この段の 4 小節の丸。
//!   クリックの消えるドリルなら、鳴る小節と消える小節の並び
//! - カウントの小節は、レーンの真ん中に拍の数字を大きく
//! - 叩いているあいだは、レーンの上に何も重ねない（叩く手の文字は音符の横に `play_view` が描く）

use egui::{Color32, Stroke};
use engine::drill::{Grade, STEP_MEASURES, STEPS, Summary};
use i18n::{t, tf};

use crate::drill::DrillPlay;
use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::practice_hud::{
    Align, CARD, COUNT_POP_MS, COUNT_Y, CURRENT_COLOR, DIGIT_Y, GREY, INK, LABEL_Y, LANES_W,
    LANES_X, LAP_X, OK_COLOR, SLIP_COLOR, SPEED_X, bold, card_frame, dot, edged, put, put_mid,
    round_fill, round_frame, score_digits,
};
use crate::text::Style;

/// 3 つ以上落とした段
pub const MANY_COLOR: Color32 = Color32::from_rgb(0xFF, 0x78, 0x68);

/// 段の出来の色
pub fn grade_color(g: Grade) -> Color32 {
    match g {
        Grade::Clean => OK_COLOR,
        Grade::Few => SLIP_COLOR,
        Grade::Many => MANY_COLOR,
    }
}

/// 段の出来の印（◎ ミスなし、○ 1〜2、× 3 つ以上）
pub fn grade_mark(g: Grade) -> &'static str {
    match g {
        Grade::Clean => "◎",
        Grade::Few => "○",
        Grade::Many => "×",
    }
}

/// 基礎練習中の表示をすべて描く。`now` は表示の補正を足した曲の時刻、`summary` は今までの出来、
/// `reverse` は下から上へ流すか、`wide` は左バスドラムのレーンを足して広げた幅
pub fn draw(
    c: &Canvas,
    g: &mut Gfx,
    d: &DrillPlay,
    summary: Option<&Summary>,
    now: f64,
    reverse: bool,
    wide: f32,
) {
    let m = d.measure_at(now);
    draw_bar_and_tempo(c, g, d, m);
    draw_card(c, g, d, summary, m);
    draw_count_in(c, g, d, now, reverse, wide);
}

/// 今の段（カウントの間は最初の段、終わったら最後の段）
fn step_at(d: &DrillPlay, m: u32) -> usize {
    match d.run.step_of(m) {
        Some(s) => s,
        None if m == 0 => 0,
        None => STEPS - 1,
    }
}

/// 左上：この段の何小節目か / 4 とテンポ
fn draw_bar_and_tempo(c: &Canvas, g: &mut Gfx, d: &DrillPlay, m: u32) {
    let head = bold(22.0, Color32::WHITE);
    put(
        c,
        g,
        t("BAR", "小節"),
        &head,
        LAP_X,
        LABEL_Y,
        Align::Left,
        100.0,
        1.0,
    );
    put(
        c,
        g,
        t("TEMPO", "テンポ"),
        &head,
        SPEED_X,
        LABEL_Y,
        Align::Left,
        100.0,
        1.0,
    );
    let bar = match d.run.measure_in_step(m) {
        Some(i) => (i + 1).to_string(),
        None if m == 0 => "0".to_owned(),
        None => STEP_MEASURES.to_string(),
    };
    let right = score_digits(c, g, &bar, LAP_X - 10.0, DIGIT_Y, 1.0, 1.0);
    put(
        c,
        g,
        &format!("/{STEP_MEASURES}"),
        &bold(22.0, Color32::WHITE),
        right + 2.0,
        DIGIT_Y + 24.0,
        Align::Left,
        SPEED_X - right - 8.0,
        1.0,
    );
    let bpm = d.run.bpms[step_at(d, m)].to_string();
    score_digits(c, g, &bpm, SPEED_X, DIGIT_Y, 1.0, 1.0);
}

/// 右の欄。目標と「あと何段」、段の絵、この段の小節の丸、クリックの消えるドリルなら鳴る小節の並び
fn draw_card(c: &Canvas, g: &mut Gfx, d: &DrillPlay, summary: Option<&Summary>, m: u32) {
    card_frame(c);
    let (x, y, w, _) = CARD;
    let done = m >= d.run.measure_count();
    let step = step_at(d, m);
    put(
        c,
        g,
        &tf!("Target {}", "目標 {}", d.run.target()),
        &bold(19.0, Color32::WHITE),
        x + 14.0,
        y + 10.0,
        Align::Left,
        w / 2.0,
        1.0,
    );
    let left = STEPS - 1 - step;
    let (right, color) = if done {
        (t("Done", "おわり").to_owned(), OK_COLOR)
    } else if left == 0 {
        (t("Last step", "最後の段").to_owned(), CURRENT_COLOR)
    } else {
        (tf!("{left} to go", "あと {left} 段"), CURRENT_COLOR)
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
    let grades: [Option<Grade>; STEPS] =
        std::array::from_fn(|i| summary.and_then(|s| s.steps.get(i)).and_then(|s| s.grade()));
    let current = (!done).then_some(step);
    stairs(
        c,
        g,
        &d.run.bpms,
        current,
        &grades,
        true,
        (x + 16.0, y + 50.0, w - 32.0, 150.0),
    );
    if done {
        return;
    }
    put(
        c,
        g,
        &tf!(
            "This step: {STEP_MEASURES} bars",
            "この段の {STEP_MEASURES} 小節"
        ),
        &edged(13.0, GREY),
        x + 14.0,
        y + 212.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    let playing = d.run.measure_in_step(m);
    let r = 10.0;
    for i in 0..STEP_MEASURES {
        let cx = x + 16.0 + r + i as f32 * 30.0;
        let cy = y + 244.0;
        match playing {
            Some(p) if i < p => dot(c, cx, cy, r, CURRENT_COLOR, CURRENT_COLOR),
            Some(p) if i == p => {
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
        &format!("{} / {STEP_MEASURES}", playing.map_or(0, |p| p + 1)),
        &bold(18.0, Color32::WHITE),
        x + w - 14.0,
        y + 244.0,
        Align::Right,
        80.0,
    );
    match d.run.drill.gap {
        Some((on, off)) => draw_gap(c, g, d, m, on + off),
        None => {
            put(
                c,
                g,
                t(
                    "Misses don't stop you. Each step moves on.",
                    "ミスが続いても止まらずに次の段へ",
                ),
                &edged(14.0, Color32::WHITE),
                x + 14.0,
                y + 290.0,
                Align::Left,
                w - 28.0,
                1.0,
            );
        }
    }
}

/// クリックの鳴る小節（明るい四角）と消える小節（暗い四角）の並び。今の小節に枠
fn draw_gap(c: &Canvas, g: &mut Gfx, d: &DrillPlay, m: u32, cycle: u32) {
    let (x, y, w, _) = CARD;
    put_mid(
        c,
        g,
        t("Click", "クリック"),
        &edged(13.0, GREY),
        x + 14.0,
        y + 281.0,
        Align::Left,
        80.0,
    );
    let first = engine::drill::COUNT_IN_MEASURES;
    let now = m.checked_sub(first).map(|i| i % cycle);
    for i in 0..cycle {
        let bx = x + 100.0 + i as f32 * 34.0;
        let cell = (bx, y + 270.0, 26.0, 22.0);
        let on = d.run.clicks(first + i);
        let fill = if on {
            Color32::from_gray(225)
        } else {
            Color32::from_rgb(70, 72, 90)
        };
        round_fill(c, cell, 4.0, fill);
        if now == Some(i) {
            round_frame(c, (bx - 3.0, y + 267.0, 32.0, 28.0), 5.0, CURRENT_COLOR);
        }
    }
    let silent = m >= first && !d.run.clicks(m);
    let (text, color) = if silent {
        (
            t(
                "No click now. Keep the beat.",
                "いまはクリックなし。拍を保って",
            ),
            CURRENT_COLOR,
        )
    } else {
        (
            t(
                "Clicks drop out. Keep time on your own.",
                "クリックが消えても同じテンポで",
            ),
            Color32::WHITE,
        )
    };
    put(
        c,
        g,
        text,
        &edged(14.0, color),
        x + 14.0,
        y + 300.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
}

/// 段の絵。高さで上がっていくことを見せ、中にテンポ。今の段（`current`）は橙に「いまここ」、
/// 終えた段は出来の色（`grades`）と、`marks` なら中に出来の印、いちばん上の段に旗
pub fn stairs(
    c: &Canvas,
    g: &mut Gfx,
    bpms: &[u32; STEPS],
    current: Option<usize>,
    grades: &[Option<Grade>; STEPS],
    marks: bool,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let n = STEPS as f32;
    let gap = 8.0;
    let sw = (w - gap * (n - 1.0)) / n;
    let rise = (h - 30.0) / n;
    let px = (sw * 0.22).clamp(12.0, 17.0);
    // テンポが変わらない回は段にせず、同じ高さに並べる
    let flat = bpms.iter().all(|&b| b == bpms[0]);
    let level = |i: usize| if flat { 2.0 } else { i as f32 + 1.0 };
    for (i, &bpm) in bpms.iter().enumerate() {
        let bx = x + i as f32 * (sw + gap);
        let top = y + h - level(i) * rise;
        let cell = (bx, top, sw, y + h - top);
        let ink = if Some(i) == current {
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
                t("NOW", "いまここ"),
                &edged(12.0, CURRENT_COLOR),
                bx + sw / 2.0,
                top - 13.0,
                Align::Center,
                sw + gap,
            );
            INK
        } else if let Some(gr) = grades[i] {
            round_fill(c, cell, 5.0, grade_color(gr).gamma_multiply(0.7));
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
        let mark = grades[i].filter(|_| marks && Some(i) != current);
        // 段が低くて数字の下に印が入らないときは、数字と印を横に並べる
        let side = mark.is_some() && y + h - top < 44.0;
        let (num_x, num_w) = if side {
            (bx + sw * 0.36, sw * 0.6)
        } else {
            (bx + sw / 2.0, sw - 4.0)
        };
        put_mid(
            c,
            g,
            &bpm.to_string(),
            &style,
            num_x,
            top + 15.0,
            Align::Center,
            num_w,
        );
        if let Some(gr) = mark {
            let (mx, my) = if side {
                (bx + sw * 0.78, top + 15.0)
            } else {
                (bx + sw / 2.0, top + 38.0)
            };
            put_mid(
                c,
                g,
                grade_mark(gr),
                &edged(px + 4.0, Color32::WHITE),
                mx,
                my,
                Align::Center,
                sw,
            );
        }
    }
    // いちばん上の段（目標）に旗
    let bx = x + (n - 1.0) * (sw + gap);
    let top = y + h - level(STEPS - 1) * rise;
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

/// カウントの小節の拍の数字を、レーンの真ん中に大きく。拍ごとに大きく出て縮む
fn draw_count_in(c: &Canvas, g: &mut Gfx, d: &DrillPlay, now: f64, reverse: bool, wide: f32) {
    let Some(beat) = d.count_in_beat(now) else {
        return;
    };
    let (from, to) = (d.measure_start_ms(0), d.measure_start_ms(1));
    let len = (to - from) / 4.0;
    let since = now - from - f64::from(beat - 1) * len;
    let cx = LANES_X + (LANES_W + wide) / 2.0;
    let cy = if reverse { 720.0 - COUNT_Y } else { COUNT_Y };
    let k = (since / COUNT_POP_MS).clamp(0.0, 1.0) as f32;
    let scale = 1.0 + 0.4 * (1.0 - k);
    let digit = beat.min(9);
    match g.skin.tex("Graphics/ScreenPlayDrums combo.png") {
        Some(tex) => {
            // 1 文字 120×160、5 文字ずつ 2 段（コンボの数字と同じ切り出し）
            let (w, h) = (120.0, 160.0);
            c.draw_opt(
                &tex,
                cx - w * scale / 2.0,
                cy - h * scale / 2.0,
                Opt {
                    src: Some(src((digit % 5) as f32 * w, (digit / 5) as f32 * h, w, h)),
                    sx: scale,
                    sy: scale,
                    alpha: 0.6 + 0.4 * (1.0 - k),
                    ..Default::default()
                },
            );
        }
        None => {
            let style = Style::edged(96.0, Color32::WHITE, Color32::BLACK, 4.0);
            if let Some(tx) = g.texts.get(&digit.to_string(), &style) {
                tx.draw(c, cx - tx.w / 2.0, cy - tx.h / 2.0, 1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::drill::{Drill, Run};

    fn play(id: &str) -> DrillPlay {
        let run = Run::new(Drill::find(id).unwrap(), 100, 10);
        let chart = dtx::parse(&run.to_dtx(id));
        let tempo = dtx::TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        DrillPlay::new(run, &tempo, &notes)
    }

    #[test]
    fn the_count_in_shows_the_first_step_and_the_end_the_last() {
        let d = play("paradiddle");
        assert_eq!(step_at(&d, 0), 0);
        assert_eq!(step_at(&d, 1), 0);
        assert_eq!(step_at(&d, 5), 1);
        assert_eq!(step_at(&d, 16), 3);
        assert_eq!(step_at(&d, 17), 3);
    }

    #[test]
    fn grades_have_their_own_marks_and_colors() {
        let all = [Grade::Clean, Grade::Few, Grade::Many];
        let marks: Vec<_> = all.iter().map(|&g| grade_mark(g)).collect();
        assert_eq!(marks, ["◎", "○", "×"]);
        assert_ne!(grade_color(Grade::Few), grade_color(Grade::Many));
    }
}
