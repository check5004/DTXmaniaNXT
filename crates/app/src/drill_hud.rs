//! 基礎練習中の表示（演奏画面に重ねる）。区間練習と同じ場所に、同じ部品で出す。
//!
//! - 左上（スコアの場所）に、この段の何小節目か（持久は全体の何小節目か）とテンポ
//! - 右（ジャケットの場所）に、見出し（目標・最高・残りの時間）、やり方の絵、この段の小節の丸。
//!   テンポを上げる・見ないで・聞いてまねるは段の絵、限界チャレンジは今の段のまわりの段と落とした数、
//!   持久は 30 秒ほどの区切りごとのばらつき。聞いてまねるは「聞いて」「叩いて」を大きく。
//!   クリックの消えるドリルなら、鳴る小節と消える小節の並び
//! - カウントの小節は、レーンの真ん中に拍の数字を大きく
//! - 叩いているあいだは、レーンの上に何も重ねない（叩く手の文字は音符の横に `play_view` が描く）

use egui::{Color32, Stroke};
use engine::drill::{COUNT_IN_MEASURES, Grade, MANY_MISSES, Summary, Way};
use engine::practice_way::{PEEK_AFTER, PEEK_BARS};
use i18n::{t, tf};

use crate::drill::DrillPlay;
use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::practice::PEEK_NOTE_MS;
use crate::practice_hud::{
    Align, CARD, COUNT_POP_MS, COUNT_Y, CURRENT_COLOR, DIGIT_Y, GREY, INK, LABEL_Y, LANES_W,
    LANES_X, LAP_X, OK_COLOR, SLIP_COLOR, SPEED_X, bold, card_frame, dot, edged, put, put_mid,
    round_fill, round_frame, score_digits,
};
use crate::text::Style;

/// 3 つ以上落とした段
pub const MANY_COLOR: Color32 = Color32::from_rgb(0xFF, 0x78, 0x68);
/// 聞いてまねるのお手本の小節
pub const DEMO_COLOR: Color32 = Color32::from_rgb(0xB4, 0x8C, 0xFF);

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
    draw_card(c, g, d, summary, m, now);
    draw_count_in(c, g, d, now, reverse, wide);
}

/// 今の段（カウントの間は最初の段、終わったら最後の段。限界チャレンジで終えたらその段）
fn step_at(d: &DrillPlay, m: u32) -> usize {
    if let Some((s, _)) = d.stopped {
        return s;
    }
    match d.run.step_of(m) {
        Some(s) => s,
        None if m == 0 => 0,
        None => d.run.steps() - 1,
    }
}

/// 終わったか（最後の小節を過ぎたか、限界チャレンジで終えたか）
fn done_at(d: &DrillPlay, m: u32) -> bool {
    m >= d.run.measure_count() || d.stopped.is_some()
}

/// 左上：この段の何小節目か（持久は全体の何小節目か）とテンポ
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
    let run = &d.run;
    let whole = matches!(run.way, Way::Endurance(_));
    let total = if whole {
        run.measure_count() - COUNT_IN_MEASURES
    } else {
        run.step_measures
    };
    let bar = match run.measure_in_step(m) {
        _ if done_at(d, m) && d.stopped.is_none() => total,
        Some(_) if whole => m + 1 - COUNT_IN_MEASURES,
        Some(i) => i + 1,
        None if m == 0 => 0,
        None => total,
    };
    let right = score_digits(c, g, &bar.to_string(), LAP_X - 10.0, DIGIT_Y, 1.0, 1.0);
    put(
        c,
        g,
        &format!("/{total}"),
        &bold(22.0, Color32::WHITE),
        right + 2.0,
        DIGIT_Y + 24.0,
        Align::Left,
        SPEED_X - right - 8.0,
        1.0,
    );
    let bpm = run.bpms[step_at(d, m)].to_string();
    score_digits(c, g, &bpm, SPEED_X, DIGIT_Y, 1.0, 1.0);
}

/// 右の欄。上にやり方ごとの見出しと絵、下に今の段の小節の丸とひとこと
fn draw_card(c: &Canvas, g: &mut Gfx, d: &DrillPlay, summary: Option<&Summary>, m: u32, now: f64) {
    card_frame(c);
    let (x, y, w, _) = CARD;
    let run = &d.run;
    let done = done_at(d, m);
    let step = step_at(d, m);
    let grades: Vec<Option<Grade>> = (0..run.steps())
        .map(|i| summary.and_then(|s| s.steps.get(i)).and_then(|s| s.grade()))
        .collect();
    let current = (!done).then_some(step);
    // 見出し（左）
    let head = match run.way {
        Way::Climb => tf!("Target {}", "目標 {}", run.target()),
        Way::Limit => match summary.and_then(Summary::best_clean_bpm) {
            Some(b) => tf!("Best {b}", "ベスト {b}"),
            None => tf!("From {}", "{} から", run.target()),
        },
        _ => tf!("Tempo {}", "テンポ {}", run.target()),
    };
    put(
        c,
        g,
        &head,
        &bold(19.0, Color32::WHITE),
        x + 14.0,
        y + 10.0,
        Align::Left,
        w / 2.0,
        1.0,
    );
    // 見出し（右）。終わり、あと何段、持久は残りの時間
    let left = run.steps() - 1 - step;
    let (right, color) = if done {
        (t("Done", "終了").to_owned(), OK_COLOR)
    } else if let Way::Endurance(_) = run.way {
        let end = d.measure_start_ms(run.measure_count());
        let rest = (end - now.max(d.measure_start_ms(COUNT_IN_MEASURES))).max(0.0);
        (tf!("{} left", "あと {}", clock(rest)), CURRENT_COLOR)
    } else if run.way == Way::Limit {
        (tf!("Step {}", "{} 段目", step + 1), CURRENT_COLOR)
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
    let pic = (x + 16.0, y + 50.0, w - 32.0, 150.0);
    match run.way {
        Way::Limit => {
            // 今の段のまわりだけ（多いので）
            let from = step.saturating_sub(5);
            let to = (from + 8).min(run.steps());
            stairs(
                c,
                g,
                &Stairs {
                    bpms: &run.bpms[from..to],
                    current: current.map(|s| s - from),
                    grades: &grades[from..to],
                    marks: true,
                    hidden: &[],
                    flag: false,
                },
                pic,
            );
        }
        Way::Endurance(_) => parts(c, g, d, summary, now, pic),
        _ => {
            let hidden: Vec<bool> = (0..run.steps()).map(|i| run.step_hidden(i)).collect();
            stairs(
                c,
                g,
                &Stairs {
                    bpms: &run.bpms,
                    current,
                    grades: &grades,
                    marks: true,
                    hidden: &hidden,
                    flag: run.way == Way::Climb,
                },
                pic,
            );
        }
    }
    if done {
        return;
    }
    if let Way::Endurance(_) = run.way {
        put(
            c,
            g,
            t(
                "Same tempo all the way. Watch your spread grow or not.",
                "テンポは変えずに最後まで。30 秒ごとのばらつきを見ます",
            ),
            &edged(14.0, Color32::WHITE),
            x + 14.0,
            y + 222.0,
            Align::Left,
            w - 28.0,
            1.0,
        );
        return;
    }
    let bars = run.step_measures;
    put(
        c,
        g,
        &tf!("This step: {bars} bars", "この段の {bars} 小節"),
        &edged(13.0, GREY),
        x + 14.0,
        y + 212.0,
        Align::Left,
        w - 28.0,
        1.0,
    );
    let playing = run.measure_in_step(m);
    let r = 10.0;
    let first = run.step_range(step).start;
    for i in 0..bars {
        let cx = x + 16.0 + r + i as f32 * 30.0;
        let cy = y + 244.0;
        // 聞いてまねるのお手本の小節は紫
        let ink = if run.is_demo(first + i) {
            DEMO_COLOR
        } else {
            CURRENT_COLOR
        };
        match playing {
            Some(p) if i < p => dot(c, cx, cy, r, ink, ink),
            Some(p) if i == p => {
                dot(c, cx, cy, r, Color32::TRANSPARENT, ink);
                dot(c, cx, cy, r - 4.0, ink, ink);
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
        &format!("{} / {bars}", playing.map_or(0, |p| p + 1)),
        &bold(18.0, Color32::WHITE),
        x + w - 14.0,
        y + 244.0,
        Align::Right,
        80.0,
    );
    match (run.way, run.drill.gap) {
        (Way::Echo, _) => {
            // いまはお手本を見て聞くか、自分で叩くか
            let (text, color) = if m >= COUNT_IN_MEASURES && run.is_demo(m) {
                (t("Watch & Listen", "見て聞いて"), DEMO_COLOR)
            } else if m >= COUNT_IN_MEASURES {
                (t("Your Turn", "まねて叩いて"), CURRENT_COLOR)
            } else {
                (t("Watch First", "まず見て聞いて"), DEMO_COLOR)
            };
            put(
                c,
                g,
                text,
                &bold(28.0, color),
                x + 14.0,
                y + 270.0,
                Align::Left,
                w - 28.0,
                1.0,
            );
            if d.peeked_at_ms
                .is_some_and(|at| (0.0..PEEK_NOTE_MS).contains(&(now - at)))
            {
                put(
                    c,
                    g,
                    &tf!(
                        "Missed in a row: the next {PEEK_BARS} turns show the notes",
                        "続けてミスしたので、次の {PEEK_BARS} 回は音符を出します"
                    ),
                    &edged(13.0, DEMO_COLOR),
                    x + 14.0,
                    y + 306.0,
                    Align::Left,
                    w - 28.0,
                    1.0,
                );
            }
        }
        (Way::Limit, _) => {
            // この段で落とした数（3 つで終わり）
            put_mid(
                c,
                g,
                t("Misses", "ミス"),
                &edged(14.0, GREY),
                x + 14.0,
                y + 296.0,
                Align::Left,
                80.0,
            );
            let n = d.misses_in(step);
            for i in 0..MANY_MISSES {
                let cx = x + 84.0 + i as f32 * 28.0;
                if i < n {
                    dot(c, cx, y + 296.0, 9.0, MANY_COLOR, MANY_COLOR);
                } else {
                    dot(
                        c,
                        cx,
                        y + 296.0,
                        9.0,
                        Color32::TRANSPARENT,
                        Color32::from_white_alpha(110),
                    );
                }
            }
            put_mid(
                c,
                g,
                t("3 in one step ends it", "同じ段で 3 つで終わり"),
                &edged(13.0, GREY),
                x + w - 14.0,
                y + 296.0,
                Align::Right,
                w - 190.0,
            );
        }
        (_, Some((on, off))) => draw_gap(c, g, d, m, on + off),
        _ => {
            let peeked = d
                .peeked_at_ms
                .is_some_and(|at| (0.0..PEEK_NOTE_MS).contains(&(now - at)));
            let (text, color) = match run.way {
                _ if peeked => (
                    tf!(
                        "Missed in a row: the next {PEEK_BARS} bars show the notes",
                        "続けてミスしたので、次の {PEEK_BARS} 小節は音符を出します"
                    ),
                    DEMO_COLOR,
                ),
                Way::Blind(_) => (
                    tf!(
                        "Hidden from step 2. Miss {PEEK_AFTER} bars in a row and the notes come back for {PEEK_BARS}",
                        "2 段目から隠します。{PEEK_AFTER} 小節続けてミスしたら {PEEK_BARS} 小節だけ出します"
                    ),
                    Color32::WHITE,
                ),
                Way::Echo => (
                    t(
                        "Watch a bar, then play it back with the notes hidden",
                        "お手本のあと、隠した 1 小節をまねて叩きます",
                    )
                    .to_owned(),
                    Color32::WHITE,
                ),
                _ => (
                    t(
                        "Misses don't stop you. Each step moves on.",
                        "ミスが続いても止まらずに次の段へ",
                    )
                    .to_owned(),
                    Color32::WHITE,
                ),
            };
            put(
                c,
                g,
                &text,
                &edged(14.0, color),
                x + 14.0,
                y + 290.0,
                Align::Left,
                w - 28.0,
                1.0,
            );
        }
    }
}

/// 持久の区切り（30 秒ほど）の帯。済んだ区切りは出来の色で、高さはばらつき（低いほどそろっている）
fn parts(
    c: &Canvas,
    g: &mut Gfx,
    d: &DrillPlay,
    summary: Option<&Summary>,
    now: f64,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let run = &d.run;
    let n = run.steps();
    let gap = 6.0;
    let pw = (w - gap * (n as f32 - 1.0)) / n as f32;
    let base = y + h - 24.0;
    let tall = h - 50.0;
    for i in 0..n {
        let r = run.step_range(i);
        let (from, to) = (d.measure_start_ms(r.start), d.measure_start_ms(r.end));
        let px = x + i as f32 * (pw + gap);
        round_fill(
            c,
            (px, base - tall, pw, tall),
            5.0,
            Color32::from_white_alpha(14),
        );
        let step = summary.and_then(|s| s.steps.get(i));
        if let Some(sr) = step.filter(|s| s.completed) {
            // ばらつき 0〜30 ms を高さに
            let sd = sr.offset_sd_ms.unwrap_or(30.0).min(30.0) as f32;
            let bh = (tall * (0.15 + 0.85 * sd / 30.0)).max(8.0);
            let color = sr.grade().map_or(GREY, grade_color);
            round_fill(c, (px, base - bh, pw, bh), 5.0, color.gamma_multiply(0.8));
            if let Some(sd) = sr.offset_sd_ms {
                put_mid(
                    c,
                    g,
                    &format!("±{sd:.0}"),
                    &edged(13.0, Color32::WHITE),
                    px + pw / 2.0,
                    base - bh - 12.0,
                    Align::Center,
                    pw + gap,
                );
            }
        } else if now >= from && now < to {
            let k = ((now - from) / (to - from)).clamp(0.0, 1.0) as f32;
            round_fill(c, (px, base - 8.0, pw * k, 8.0), 4.0, CURRENT_COLOR);
            round_frame(c, (px, base - tall, pw, tall), 5.0, CURRENT_COLOR);
        }
        put_mid(
            c,
            g,
            &up_to(to - d.measure_start_ms(COUNT_IN_MEASURES)),
            &edged(12.0, GREY),
            px + pw / 2.0,
            base + 12.0,
            Align::Center,
            pw + gap,
        );
    }
}

/// 分:秒
pub fn clock(ms: f64) -> String {
    let s = (ms / 1000.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// 区切りの終わりの時刻（「〜0:31」）
pub fn up_to(ms: f64) -> String {
    tf!("to {}", "〜{}", clock(ms))
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
    let first = COUNT_IN_MEASURES;
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
                "今はクリックなし。テンポを保って",
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

/// 段の絵の中身
pub struct Stairs<'a> {
    /// 段ごとのテンポ
    pub bpms: &'a [u32],
    /// 今の段（橙に「いまここ」）
    pub current: Option<usize>,
    /// 終えた段の出来（色）
    pub grades: &'a [Option<Grade>],
    /// 終えた段の中に出来の印を出す
    pub marks: bool,
    /// 段ごとに音符を隠すか（見ないで。空なら出さない）
    pub hidden: &'a [bool],
    /// いちばん上の段に旗（目標）
    pub flag: bool,
}

/// 段の絵。高さで上がっていくことを見せ、中にテンポ。今の段は橙に「いまここ」、
/// 終えた段は出来の色と、`marks` なら中に出来の印。`hidden` があれば段の下の方に、音符を見せるか隠すか
pub fn stairs(c: &Canvas, g: &mut Gfx, s: &Stairs, (x, y, w, h): (f32, f32, f32, f32)) {
    let n = s.bpms.len().max(1) as f32;
    let gap = if n > 6.0 { 4.0 } else { 8.0 };
    let sw = (w - gap * (n - 1.0)) / n;
    let rise = (h - 30.0) / n;
    let px = (sw * 0.22).clamp(12.0, 17.0);
    // テンポが変わらない回は段にせず、同じ高さに並べる
    let flat = s.bpms.iter().all(|&b| b == s.bpms[0]);
    let level = |i: usize| if flat { 2.0 } else { i as f32 + 1.0 };
    for (i, &bpm) in s.bpms.iter().enumerate() {
        let bx = x + i as f32 * (sw + gap);
        let top = y + h - level(i) * rise;
        let cell = (bx, top, sw, y + h - top);
        let grade = s.grades.get(i).copied().flatten();
        let ink = if Some(i) == s.current {
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
                (sw + gap).max(60.0),
            );
            INK
        } else if let Some(gr) = grade {
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
        // 低くて細い段は印を出さない（色で分かる）
        let room = y + h - top >= 44.0 || sw >= 56.0;
        let mark = grade.filter(|_| s.marks && Some(i) != s.current && room);
        // 段が低くて数字の下に印が入らないときは、数字と印を横に並べる
        let side = mark.is_some() && y + h - top < 44.0;
        let (num_x, num_w) = if side {
            (bx + sw * 0.36, sw * 0.6)
        } else {
            (bx + sw / 2.0, sw - 4.0)
        };
        // 低い段は数字を段の真ん中に
        let num_y = top + 15.0_f32.min((y + h - top) / 2.0 + 1.0);
        put_mid(
            c,
            g,
            &bpm.to_string(),
            &style,
            num_x,
            num_y,
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
        // 見ないで: 段の下に音符の見え方（見せる段は白い音符、隠す段は暗い帯に「?」）
        if let Some(&hide) = s.hidden.get(i) {
            let (nw, nh) = ((sw * 0.5).min(34.0), 14.0);
            let nx = bx + (sw - nw) / 2.0;
            let ny = y + h - 21.0;
            if hide {
                round_fill(
                    c,
                    (nx, ny, nw, nh),
                    3.0,
                    Color32::from_rgba_unmultiplied(4, 4, 10, 200),
                );
                put_mid(
                    c,
                    g,
                    "?",
                    &edged(12.0, Color32::from_white_alpha(170)),
                    nx + nw / 2.0,
                    ny + nh / 2.0,
                    Align::Center,
                    nw,
                );
            } else {
                round_fill(c, (nx, ny + 2.0, nw, nh - 4.0), 3.0, Color32::WHITE);
            }
        }
    }
    if !s.flag {
        return;
    }
    // いちばん上の段（目標）に旗
    let last = s.bpms.len().saturating_sub(1);
    let bx = x + (n - 1.0) * (sw + gap);
    let top = y + h - level(last) * rise;
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
    let Some((beat, beat_ms)) = d.count_in_beat(now) else {
        return;
    };
    let since = now - beat_ms;
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
    use engine::drill::{Drill, Run, Way};

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
        assert!(!done_at(&d, 16));
        assert!(done_at(&d, 17));
    }

    #[test]
    fn a_stopped_limit_test_stays_on_its_step() {
        let run = Run::with(
            Drill::find("paradiddle").unwrap(),
            Way::Limit,
            100,
            10,
            false,
        );
        let chart = dtx::parse(&run.to_dtx("limit"));
        let tempo = dtx::TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        let mut d = DrillPlay::new(run, &tempo, &notes);
        use engine::judge::Judgement::{Miss, Perfect};
        let sd = dtx::DrumLane::SD;
        assert!(!d.on_judged((3, sd), Miss, 0.0, 0.0));
        assert!(!d.on_judged((3, sd), Perfect, 0.0, 0.0));
        assert!(!d.on_judged((4, sd), Miss, 0.0, 0.0));
        assert!(d.on_judged((4, sd), Miss, 5000.0, 5000.0));
        assert!(!d.on_judged((5, sd), Miss, 6000.0, 6000.0));
        assert_eq!(d.misses_in(1), 3);
        assert_eq!(step_at(&d, 9), 1);
        assert!(done_at(&d, 9));
        assert!(!d.clicks_at(5100.0));
        assert!(d.clicks_at(4900.0));
    }

    #[test]
    fn clocks_read_minutes_and_seconds() {
        assert_eq!(clock(44_400.0), "0:44");
        assert_eq!(clock(65_000.0), "1:05");
    }

    #[test]
    fn grades_have_their_own_marks_and_colors() {
        let all = [Grade::Clean, Grade::Few, Grade::Many];
        let marks: Vec<_> = all.iter().map(|&g| grade_mark(g)).collect();
        assert_eq!(marks, ["◎", "○", "×"]);
        assert_ne!(grade_color(Grade::Few), grade_color(Grade::Many));
    }
}
