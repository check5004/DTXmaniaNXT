//! 結果の画面に足すトレーニングの表示（NX の部品で）。
//!
//! - 練習の結果：ランクとジャケットの場所に、のぼった段と挑戦の印、時間と周の数、ミスした所の地図、
//!   気づいたこと（空打ちの偏り、ずれ）を、文章でなく絵と数で出す。周ごとの成績の表は 2 ページ目
//! - ふだんのプレイ：右の空いた場所に、苦手な小節を進み具合のバーの形（黒地に縞）で出す

use egui::{Color32, Stroke, StrokeKind};
use engine::practice::{Section, SpeedPlan, SpotTally, speed_percent};
use engine::stroke::StrokeTally;
use i18n::{Tr, count, t, tf, tr};

use crate::band::{Band, BandData, CLEARED_COLOR};
use crate::chart_picture::Look;
use crate::game::{Outcome, PracticeOutcome};
use crate::gfx::{Canvas, Gfx};
use crate::inputs::lane_label;
use crate::practice::{
    LapSummary, StrayHint, WEAK_VELOCITY, mark_line, section_label, style_label, wait_progress,
    way_label, wrong_pads,
};
use crate::practice_hud::mark_color;
use crate::text::Style;
use engine::practice_way::{Mark, Way};

const PASS_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);
/// ミスした周・ミス 1 回の拍
const SLIP_COLOR: Color32 = Color32::from_rgb(0xF0, 0xD0, 0x50);
/// 何度もミスした拍・もう一度
const WEAK_COLOR: Color32 = Color32::from_rgb(0xFF, 0x70, 0x50);
/// 気づいたこと（空打ちの偏り、ずれ）
const NOTE_COLOR: Color32 = Color32::from_rgb(0xFF, 0xD0, 0x80);
/// ずれの平均をこれ以上なら知らせる（ミリ秒）と、そのために要る打った数
const OFFSET_NOTE_MS: f64 = 10.0;
const OFFSET_NOTE_HITS: usize = 30;

/// 練習の結果の欄（ランクとジャケットの場所）
const PRACTICE: (f32, f32, f32, f32) = (480.0, 24.0, 780.0, 590.0);
/// 表の 1 行の高さ
const ROW_H: f32 = 24.0;

/// 苦手な小節の欄（右の空いた場所）
const WEAK: (f32, f32, f32, f32) = (836.0, 296.0, 434.0, 330.0);
const WEAK_BAND: Band = Band {
    x: WEAK.0 + 14.0,
    y: WEAK.1 + 46.0,
    w: WEAK.2 - 28.0,
    cell_h: 18.0,
    row_pitch: 32.0,
    max_rows: 3,
    striped: true,
};

fn panel(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32)) {
    c.fill(x, y, w, h, Color32::from_rgba_unmultiplied(8, 8, 16, 225));
    c.frame(x, y, w, h, Color32::from_gray(150));
}

fn text(c: &Canvas, g: &mut Gfx, s: &str, x: f32, y: f32, px: f32, color: Color32) -> f32 {
    let style = Style::edged(px, color, Color32::BLACK, (px / 12.0).max(1.0));
    match g.texts.get(s, &style) {
        Some(t) => {
            t.draw(c, x, y, 1.0, 1.0);
            t.w
        }
        None => 0.0,
    }
}

fn section_text(s: Section) -> String {
    section_label(s)
}

fn plan_text(p: SpeedPlan) -> String {
    match p {
        SpeedPlan::Fixed(s) => tf!("Fixed {s}%", "固定 {s}%"),
        SpeedPlan::Ladder { start, target } => {
            tf!(
                "Step Up {start}% → {target}%",
                "段階アップ {start}% → {target}%"
            )
        }
    }
}

/// 長い文を幅 `max_w` で折り返して描き、次の行の y を返す
#[allow(clippy::too_many_arguments)]
fn paragraph(
    c: &Canvas,
    g: &mut Gfx,
    s: &str,
    x: f32,
    mut y: f32,
    px: f32,
    color: Color32,
    max_w: f32,
) -> f32 {
    for line in g.texts.wrap(s, px, max_w) {
        text(c, g, &line, x, y, px, color);
        y += px + 6.0;
    }
    y
}

/// 空打ちが一つのパッドの弱い打に偏っているときの知らせ
fn stray_note(h: &StrayHint) -> String {
    let (lane, n, weak) = (lane_label(h.lane), h.count, h.weak);
    tf!(
        "{n} stray hits on the {lane} ({weak} of them soft, velocity under {WEAK_VELOCITY}). The pad may be picking up too much. Raising Minimum Velocity under MIDI Devices in Settings cuts these down.",
        "{lane}で空打ちが {n} 回ありました（うち {weak} 回はベロシティ {WEAK_VELOCITY} 未満の弱い入力）。パッドが振動を拾いすぎているかもしれません。設定の「MIDI 機器」で最小ベロシティを上げると減ります。"
    )
}

/// ずれが一つの向きに偏っているときの知らせ
fn offset_note((mean, hits): (f64, usize)) -> Option<String> {
    (hits >= OFFSET_NOTE_HITS && mean.abs() >= OFFSET_NOTE_MS).then(|| {
        tf!(
            "You hit {:.0} ms {} on average. Setting Input Offset in Quick Settings (RD) at song select makes it easier.",
            "平均 {:.0}ms {}に叩いています。選曲の簡易設定（RD）の「入力の補正」を合わせると、判定が合いやすくなります。",
            mean.abs(),
            if mean < 0.0 {
                t("early", "早め")
            } else {
                t("late", "遅め")
            }
        )
    })
}

/// 段の挑戦の印の色（越えた・もう一度・途中でやめた）
fn try_color(passed: Option<bool>) -> Color32 {
    match passed {
        Some(true) => PASS_COLOR,
        Some(false) => WEAK_COLOR,
        None => Color32::from_gray(150),
    }
}

fn round_fill(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32), r: f32, color: Color32) {
    c.painter()
        .rect_filled(c.rect(x, y, w, h), r * c.scale(), color);
}

fn round_frame(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32), r: f32, color: Color32) {
    c.painter().rect_stroke(
        c.rect(x, y, w, h),
        r * c.scale(),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
}

/// 文字を真ん中ぞろえで描く（`cy` は縦の真ん中）
#[allow(clippy::too_many_arguments)]
fn centered(
    c: &Canvas,
    g: &mut Gfx,
    s: &str,
    cx: f32,
    cy: f32,
    px: f32,
    color: Color32,
    max_w: f32,
) {
    let style = Style::edged(px, color, Color32::BLACK, (px / 12.0).max(1.0));
    if let Some(t) = g.texts.get(s, &style) {
        let sx = if t.w > max_w { max_w / t.w } else { 1.0 };
        t.draw(c, cx - t.w * sx / 2.0, cy - t.h / 2.0, sx, 1.0);
    }
}

/// 時間（「8:40」）
fn duration_text(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// 段の絵。段ごとに、挑戦の印（緑は越えた、赤はもう一度）を段の下に
fn draw_stairs(c: &Canvas, g: &mut Gfx, p: &PracticeOutcome, (x, y, w, h): (f32, f32, f32, f32)) {
    let lowest = p
        .attempts
        .iter()
        .map(|a| a.speed)
        .min()
        .unwrap_or(p.options.speed.first_speed());
    let mut steps = p.options.speed.stairs(lowest);
    for a in &p.attempts {
        if !steps.contains(&a.speed) {
            steps.push(a.speed);
        }
    }
    steps.sort_unstable();
    let passed: Vec<u32> = p
        .attempts
        .iter()
        .filter(|a| a.passed == Some(true))
        .map(|a| a.speed)
        .collect();
    let n = steps.len().max(1) as f32;
    let gap = 8.0;
    let sw = (w - gap * (n - 1.0)) / n;
    let rise = (h - 20.0) / n;
    let px = (sw * 0.16).clamp(12.0, 18.0);
    for (i, &s) in steps.iter().enumerate() {
        let bx = x + i as f32 * (sw + gap);
        let top = y + h - (i as f32 + 1.0) * rise;
        let cell = (bx, top, sw, y + h - top);
        let ok = passed.contains(&s);
        if ok {
            round_fill(c, cell, 5.0, PASS_COLOR.gamma_multiply(0.65));
        } else {
            round_fill(c, cell, 5.0, Color32::from_white_alpha(18));
            round_frame(c, cell, 5.0, Color32::from_white_alpha(70));
        }
        let color = if ok {
            Color32::WHITE
        } else {
            Color32::from_gray(190)
        };
        centered(
            c,
            g,
            &format!("{s}%"),
            bx + sw / 2.0,
            top + 15.0,
            px,
            color,
            sw - 4.0,
        );
        // 挑戦の印（この段で挑戦した順に）
        let tries: Vec<Option<bool>> = p
            .attempts
            .iter()
            .filter(|a| a.speed == s)
            .map(|a| a.passed)
            .collect();
        let k = tries.len() as f32;
        let pitch = (sw / k.max(1.0)).min(16.0);
        for (j, t) in tries.iter().enumerate() {
            let cx = bx + sw / 2.0 + (j as f32 - (k - 1.0) / 2.0) * pitch;
            c.painter()
                .circle_filled(c.pos(cx, y + h + 14.0), 5.5 * c.scale(), try_color(*t));
        }
    }
    // いちばん上の段（目標）に旗
    let goal = p.options.speed.goal();
    if let Some(i) = steps.iter().position(|&s| s == goal) {
        let bx = x + i as f32 * (sw + gap);
        let top = y + h - (i as f32 + 1.0) * rise;
        let fx = bx + sw - 14.0;
        let pnt = c.painter();
        pnt.line_segment(
            [c.pos(fx, top - 30.0), c.pos(fx, top)],
            Stroke::new(2.0 * c.scale(), Color32::WHITE),
        );
        pnt.add(egui::Shape::convex_polygon(
            vec![
                c.pos(fx, top - 30.0),
                c.pos(fx - 18.0, top - 24.0),
                c.pos(fx, top - 18.0),
            ],
            PASS_COLOR,
            Stroke::NONE,
        ));
    }
}

/// めぐった所ごとの出来（めぐる順。まだの所と、最後まで叩いた周の無い所は None）
pub fn stop_marks(p: &PracticeOutcome) -> Vec<Option<Mark>> {
    (0..p.options.stops.len())
        .map(|i| p.attempts.iter().find(|a| a.index == i).and_then(Mark::of))
        .collect()
}

/// めぐった所の並び。所ごとに小節と出来の印、ミスの数
fn draw_route(c: &Canvas, g: &mut Gfx, p: &PracticeOutcome, (x, y, w, h): (f32, f32, f32, f32)) {
    let stops = p.options.stops;
    let n = stops.len().max(1) as f32;
    let marks = stop_marks(p);
    let gap = 10.0;
    let cw = (w - gap * (n - 1.0)) / n;
    for (i, s) in stops.as_slice().iter().enumerate() {
        let bx = x + i as f32 * (cw + gap);
        let cell = (bx, y, cw, h);
        let mark = marks.get(i).copied().flatten();
        match mark {
            Some(m) => {
                round_fill(c, cell, 6.0, mark_color(m).gamma_multiply(0.18));
                round_frame(c, cell, 6.0, mark_color(m));
            }
            None => {
                round_fill(c, cell, 6.0, Color32::from_white_alpha(14));
                round_frame(c, cell, 6.0, Color32::from_white_alpha(60));
            }
        }
        let px = (cw * 0.14).clamp(12.0, 16.0);
        centered(
            c,
            g,
            &tf!("Spot {}", "{} か所目", i + 1),
            bx + cw / 2.0,
            y + 16.0,
            px - 2.0,
            Color32::from_gray(190),
            cw - 6.0,
        );
        centered(
            c,
            g,
            &section_text(*s),
            bx + cw / 2.0,
            y + 40.0,
            px,
            Color32::WHITE,
            cw - 6.0,
        );
        let (sym, color) = match mark {
            Some(m) => (m.symbol(), mark_color(m)),
            None => ("-", Color32::from_gray(150)),
        };
        centered(c, g, sym, bx + cw / 2.0, y + h * 0.58, 52.0, color, cw);
        let misses = p
            .attempts
            .iter()
            .find(|a| a.index == i)
            .map(|a| a.missed)
            .filter(|_| mark.is_some());
        if let Some(m) = misses {
            let line = match m {
                0 => t("No misses", "ノーミス").to_owned(),
                1 => t("1 miss", "ミス 1 回").to_owned(),
                m => tf!("{m} misses", "ミス {m} 回"),
            };
            centered(
                c,
                g,
                &line,
                bx + cw / 2.0,
                y + h - 18.0,
                px - 2.0,
                Color32::from_gray(210),
                cw - 6.0,
            );
        }
    }
}

/// 待ちモードの周ごとの行。周ごとにかかった時間と、違うパッドを叩いた回数。下に 1 周目からの変わり方
fn draw_wait_laps(
    c: &Canvas,
    g: &mut Gfx,
    laps: &[&LapSummary],
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let n = laps.len().max(1);
    let rh = ((h - 40.0) / n as f32 - 8.0).min(46.0);
    for (i, l) in laps.iter().enumerate() {
        let top = y + i as f32 * (rh + 8.0);
        let color = if l.wrong == 0 { PASS_COLOR } else { SLIP_COLOR };
        round_fill(c, (x, top, w, rh), 6.0, color.gamma_multiply(0.14));
        round_frame(c, (x, top, w, rh), 6.0, color.gamma_multiply(0.8));
        let mid = top + rh / 2.0 - 11.0;
        text(
            c,
            g,
            &tf!("Loop {}", "{} 周目", i + 1),
            x + 14.0,
            mid,
            18.0,
            Color32::WHITE,
        );
        let took = l
            .took_ms
            .map_or_else(|| "-".to_owned(), crate::drill_hud::clock);
        text(c, g, &took, x + 130.0, mid - 3.0, 24.0, Color32::WHITE);
        let tail = wrong_pads(l.wrong);
        let tw = g
            .texts
            .get(&tail, &Style::edged(17.0, color, Color32::BLACK, 1.5))
            .map_or(0.0, |t| t.w);
        text(c, g, &tail, x + w - 14.0 - tw, mid, 17.0, color);
    }
    if let (Some(a), Some(b)) = (laps.first(), laps.last())
        && laps.len() >= 2
    {
        let line = wait_progress((a.took_ms, a.wrong), (b.took_ms, b.wrong));
        text(
            c,
            g,
            &line,
            x,
            y + n as f32 * (rh + 8.0) + 6.0,
            16.0,
            NOTE_COLOR,
        );
    }
}

/// めぐるやり方で、ミスした所の絵に出す所：いちばんミスした所（無ければ最初の所）
fn worst_stop(p: &PracticeOutcome) -> Section {
    let stops = p.options.stops;
    let worst = (0..stops.len())
        .filter_map(|i| {
            let a = p.attempts.iter().find(|a| a.index == i)?;
            (a.missed > 0).then_some((i, a.missed))
        })
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
    stops
        .as_slice()
        .get(worst.map_or(0, |w| w.0))
        .copied()
        .unwrap_or(p.options.section)
}

/// 練習の結果。段と挑戦の印、時間と周の数、ミスした所の譜面の絵、いちばんミスした所、気づいたこと。
/// めぐるやり方は、段の代わりにめぐった所ごとの出来
pub fn draw_practice(c: &Canvas, g: &mut Gfx, p: &PracticeOutcome) {
    let (x, y, w, _) = PRACTICE;
    panel(c, PRACTICE);
    let left = x + 20.0;
    let o = p.options;
    let tours = o.way.tours();
    let waits = o.way == Way::Wait;
    let head = if waits {
        let speed = p
            .attempts
            .first()
            .map_or(o.speed.first_speed(), |a| a.speed);
        tf!(
            "{}   {}   {speed}%",
            "{}　{}　{speed}%",
            way_label(o.way),
            section_text(o.section)
        )
    } else if tours {
        let n = count(o.stops.len(), "spot", "spots", "か所");
        let speed = p
            .attempts
            .first()
            .map_or(o.speed.first_speed(), |a| a.speed);
        tf!("{}   {n}   {speed}%", "{}　{n}　{speed}%", way_label(o.way))
    } else {
        tf!(
            "{}   Target {}%   {}",
            "{}　目標 {}%　{}",
            section_text(o.section),
            o.speed.goal(),
            style_label(o.style)
        )
    };
    text(c, g, &head, left, y + 14.0, 20.0, Color32::WHITE);
    // 段（左）と数（右）。めぐるやり方は、めぐった所。待ちモードは周ごとの時間
    let done: Vec<&LapSummary> = p.laps.iter().filter(|l| l.completed).collect();
    if waits {
        draw_wait_laps(c, g, &done, (left, y + 60.0, 520.0, 200.0));
    } else if tours {
        draw_route(c, g, p, (left, y + 70.0, 520.0, 190.0));
    } else {
        draw_stairs(c, g, p, (left + 6.0, y + 58.0, 500.0, 190.0));
    }
    let legend_y = y + 278.0;
    let dot = |c: &Canvas, cx: f32, color: Color32| {
        c.painter()
            .circle_filled(c.pos(cx, legend_y + 8.0), 5.0 * c.scale(), color);
    };
    if waits {
        text(
            c,
            g,
            t(
                "Wait Mode has no scoring. The picture shows where you hit a wrong pad.",
                "待ちモードは点を付けません。違うパッドを叩いた所を下の絵に出します。",
            ),
            left,
            legend_y,
            13.0,
            Color32::from_gray(200),
        );
    } else if tours {
        text(
            c,
            g,
            &mark_line(),
            left,
            legend_y,
            13.0,
            Color32::from_gray(200),
        );
    } else {
        dot(c, left + 10.0, PASS_COLOR);
        let w1 = text(
            c,
            g,
            "OK",
            left + 20.0,
            legend_y,
            13.0,
            Color32::from_gray(200),
        );
        dot(c, left + 34.0 + w1, WEAK_COLOR);
        text(
            c,
            g,
            t("Again", "もう一度"),
            left + 44.0 + w1,
            legend_y,
            13.0,
            Color32::from_gray(200),
        );
    }
    let (clean, clean_label) = if waits {
        (
            done.iter().filter(|l| l.wrong == 0).count(),
            t("Loops with no wrong pads", "迷いなしの周"),
        )
    } else {
        (
            done.iter().filter(|l| l.passed == Some(true)).count(),
            t("Clean loops", "ノーミスの周"),
        )
    };
    let nums = [
        (
            duration_text(p.elapsed_ms),
            t("Practice time", "練習した時間"),
        ),
        (done.len().to_string(), t("Loops played", "叩いた周")),
        (clean.to_string(), clean_label),
    ];
    for (i, (v, k)) in nums.iter().enumerate() {
        let ny = y + 56.0 + i as f32 * 74.0;
        text(c, g, v, x + w - 200.0, ny, 40.0, Color32::WHITE);
        text(
            c,
            g,
            k,
            x + w - 200.0,
            ny + 46.0,
            13.0,
            Color32::from_gray(200),
        );
    }
    // ミスした所（全部の周）：区間の譜面の絵に、落とした音の印。いちばん落とした拍は赤く塗る
    let mut spots = SpotTally::default();
    for l in &done {
        spots.add_lap(&l.misses);
    }
    let worst = spots.weak(1);
    let marks = crate::practice::marks_of(done.iter().map(|l| &l.missed_notes));
    let map_y = y + 306.0;
    // めぐるやり方は、いちばんミスした拍のある所（無ければミスの多い所）の絵
    let (map_title, map_section) = if tours {
        let s = worst
            .first()
            .and_then(|(b, _)| {
                o.stops
                    .as_slice()
                    .iter()
                    .copied()
                    .find(|s| s.contains(b.measure))
            })
            .unwrap_or_else(|| worst_stop(p));
        (tf!("Misses ({})", "ミスした所（{}）", section_text(s)), s)
    } else if waits {
        (
            t(
                "Wrong pads (where you hesitated)",
                "迷った所（違うパッドを叩いた音）",
            )
            .to_owned(),
            o.section,
        )
    } else {
        (
            t("Misses (all loops)", "ミスした所（全部の周）").to_owned(),
            o.section,
        )
    };
    text(c, g, &map_title, left, map_y, 15.0, Color32::WHITE);
    let picture = (left - 4.0, map_y + 26.0, w - 32.0, 150.0);
    p.picture.draw(
        c,
        g,
        picture,
        map_section,
        &marks,
        Look {
            beat: worst.first().map(|(b, _)| (b.measure, b.beat)),
            weak_times: o.rule.weak_times(),
            ..Look::default()
        },
    );
    let line_y = picture.1 + picture.3 + 10.0;
    match worst.first() {
        Some((b, spot)) => {
            let lanes: Vec<&str> = spot.lanes.iter().map(|l| l.name()).collect();
            let line = if waits {
                let loops = count(spot.laps, "loop", "loops", "周");
                tf!(
                    "Hesitated most: measure {}, beat {} ({})  {loops}",
                    "いちばん迷った所: {} 小節目の {} 拍目（{}）　{loops}",
                    b.measure,
                    b.beat + 1,
                    lanes.join(t("/", "・"))
                )
            } else {
                let misses = count(spot.laps, "miss", "misses", "回");
                tf!(
                    "Most missed: measure {}, beat {} ({})  {misses}",
                    "いちばんミスした所: {} 小節目の {} 拍目（{}）　{misses}",
                    b.measure,
                    b.beat + 1,
                    lanes.join(t("/", "・"))
                )
            };
            text(c, g, &line, left, line_y, 16.0, WEAK_COLOR);
        }
        None if !done.is_empty() => {
            let ok = if waits {
                t("No wrong pads", "迷いなし")
            } else {
                t("No misses", "ミスなし")
            };
            text(c, g, ok, left, line_y, 16.0, PASS_COLOR);
        }
        None => {
            text(
                c,
                g,
                t(
                    "No loops were played to the end",
                    "最後まで叩いた周はありません",
                ),
                left,
                line_y,
                16.0,
                Color32::from_gray(200),
            );
        }
    }
    // 気づいたこと（短く）
    let mut notes: Vec<String> = Vec::new();
    if let Some(hint) = &p.strays {
        notes.push(stray_note(hint));
    }
    // 待ちモードは止まった所で叩くので、ずれは見ない
    if let Some(note) = p.offset.filter(|_| !waits).and_then(offset_note) {
        notes.push(note);
    }
    let mut ny = line_y + 28.0;
    for n in notes {
        ny = paragraph(c, g, &n, left, ny, 13.0, NOTE_COLOR, w - 40.0);
    }
    // 操作の案内は画面の下の帯（`ResultView::hints`）
}

/// 練習の結果の 2 ページ目：周ごとの成績の表
pub fn draw_practice_laps(c: &Canvas, g: &mut Gfx, p: &PracticeOutcome) {
    let (x, y, _, h) = PRACTICE;
    panel(c, PRACTICE);
    let left = x + 20.0;
    let heading_w = g.heading(c, left, y + 10.0, t("Loop Results", "周ごとの成績"));
    let o = p.options;
    let head = if o.way == Way::Wait {
        tf!(
            "{}   {}",
            "{}　{}",
            way_label(o.way),
            section_text(o.section)
        )
    } else if o.way.tours() {
        let n = count(o.stops.len(), "spot", "spots", "か所");
        tf!("{}   {n}", "{}　{n}", way_label(o.way))
    } else {
        tf!(
            "{}   {}",
            "{}　{}",
            section_text(o.section),
            plan_text(o.speed)
        )
    };
    text(
        c,
        g,
        &head,
        left + heading_w + 24.0,
        y + 14.0,
        18.0,
        Color32::WHITE,
    );
    let done: Vec<&LapSummary> = p.laps.iter().filter(|l| l.completed).collect();
    let mut ty = y + 50.0;
    if p.laps.len() > done.len() {
        text(
            c,
            g,
            t(
                "Loops you stopped partway are left out (and not recorded).",
                "途中でやめた周は含めていません（記録にも残りません）。",
            ),
            left,
            ty,
            14.0,
            Color32::from_gray(160),
        );
        ty += 22.0;
    }
    if done.is_empty() {
        text(
            c,
            g,
            t(
                "No loops were played to the end.",
                "最後まで叩いた周はありません。",
            ),
            left,
            ty,
            18.0,
            Color32::WHITE,
        );
    } else if o.way == Way::Wait {
        // 待ちモードは判定の数を付けないので、周ごとの時間と違うパッドの回数
        let w = PRACTICE.2 - 40.0;
        draw_wait_laps(c, g, &done, (left, ty, w, y + h - 44.0 - ty));
    } else {
        lap_table(c, g, &done, left, ty, y + h - 44.0);
    }
}

/// 表の列（見出し、左の端からの位置）
const COLUMNS: [(Tr, f32); 10] = [
    (tr("Loop", "周"), 0.0),
    (tr("Speed", "速度"), 44.0),
    (tr("PERFECT", "PERFECT"), 116.0),
    (tr("GREAT", "GREAT"), 240.0),
    (tr("GOOD", "GOOD"), 312.0),
    (tr("POOR", "POOR"), 378.0),
    (tr("MISS", "MISS"), 440.0),
    (tr("Stray", "空打ち"), 500.0),
    (tr("Timing", "ずれ"), 566.0),
    (i18n::EMPTY, 672.0),
];

/// 周ごとの成績の表。入りきらないときは新しい周を出す
fn lap_table(c: &Canvas, g: &mut Gfx, laps: &[&LapSummary], x: f32, y: f32, bottom: f32) {
    // 見出し（DFP では幅が足りないので、太字の小さい文字で）
    let head = Style {
        bold: true,
        ..Style::edged(15.0, Color32::from_rgb(190, 225, 255), Color32::BLACK, 1.5)
    };
    for (name, dx) in COLUMNS {
        let name = name.get();
        if let Some(t) = g.texts.get(name, &head).filter(|_| !name.is_empty()) {
            t.draw(c, x + dx, y + 4.0, 1.0, 1.0);
        }
    }
    c.fill(x - 6.0, y + 26.0, 740.0, 1.0, Color32::from_white_alpha(80));
    let mut ty = y + 32.0;
    let rows = (((bottom - ty) / ROW_H).floor() as usize).max(1);
    let (start, omitted) = shown_laps(laps.len(), rows);
    if let Some(n) = omitted {
        text(
            c,
            g,
            &tf!("({n} older loops not shown)", "（古い {n} 周は省略）"),
            x,
            ty,
            14.0,
            Color32::from_gray(160),
        );
        ty += ROW_H;
    }
    rows_of(c, g, &laps[start..], x, ty);
}

/// `n` 周を `rows` 行に出すときの、出す最初の周と、省いた周の数。
/// 入りきらないときは 1 行を「省きました」に使い、新しい周を出す
fn shown_laps(n: usize, rows: usize) -> (usize, Option<usize>) {
    if n <= rows {
        return (0, None);
    }
    let start = n - rows.saturating_sub(1);
    (start, Some(start))
}

fn rows_of(c: &Canvas, g: &mut Gfx, laps: &[&LapSummary], x: f32, mut y: f32) {
    for (i, l) in laps.iter().enumerate() {
        // 段の挑戦の変わり目に線を引く
        if i > 0 && laps[i - 1].attempt != l.attempt {
            c.fill(x - 6.0, y - 3.0, 740.0, 1.0, Color32::from_white_alpha(70));
        }
        if i % 2 == 0 {
            c.fill(
                x - 6.0,
                y - 2.0,
                740.0,
                ROW_H - 2.0,
                Color32::from_white_alpha(10),
            );
        }
        let t = &l.tally;
        let n = l.notes.max(1) as f64;
        let cells = [
            format!("{}", l.index + 1),
            format!("{}%", speed_percent(l.speed)),
            format!("{} ({:.0}%)", t.perfect, f64::from(t.perfect) / n * 100.0),
            t.great.to_string(),
            t.good.to_string(),
            t.poor.to_string(),
            t.miss.to_string(),
            t.stray.to_string(),
            match (l.offset_mean_ms, l.offset_sd_ms) {
                (Some(m), Some(sd)) => format!("{m:+.1} ±{sd:.1}"),
                (Some(m), None) => format!("{m:+.1}"),
                _ => "-".to_owned(),
            },
        ];
        for (s, (_, dx)) in cells.iter().zip(COLUMNS) {
            text(c, g, s, x + dx, y, 17.0, Color32::WHITE);
        }
        let (verdict, color) = match l.passed {
            Some(true) => (i18n::t("Clean", "ノーミス"), PASS_COLOR),
            Some(false) => (i18n::t("Missed", "ミス"), SLIP_COLOR),
            None => (i18n::t("Stopped", "途中"), Color32::from_gray(150)),
        };
        text(c, g, verdict, x + COLUMNS[9].1, y, 17.0, color);
        y += ROW_H;
    }
}

/// ふだんのプレイの苦手な小節（黒地に縞の帯）と、苦手なフレーズ、練習した区間の克服
pub fn draw_weak(c: &Canvas, g: &mut Gfx, o: &Outcome) {
    let Some(w) = &o.weak else {
        return;
    };
    let (x, y, pw, ph) = WEAK;
    panel(c, WEAK);
    g.heading(c, x + 12.0, y + 8.0, t("Weak Measures", "苦手な小節"));
    let source = match (w.plays, w.practice_laps) {
        (0, 0) => t("No records", "記録なし").to_owned(),
        (p, 0) => {
            let p = count(p, "play", "plays", "回");
            tf!("Based on {p}", "直近のプレイ {p}から")
        }
        (0, l) => {
            let l = count(l, "practice loop", "practice loops", "周");
            tf!("Based on {l}", "直近の練習 {l}から")
        }
        (p, l) => {
            let p = count(p, "play", "plays", "回");
            let l = count(l, "practice loop", "practice loops", "周");
            tf!("Based on {p} and {l}", "直近のプレイ {p}と練習 {l}から")
        }
    };
    let small = Style::edged(13.0, Color32::from_gray(200), Color32::BLACK, 1.0);
    if let Some(t) = g.texts.get(&source, &small) {
        t.draw(c, x + pw - 12.0 - t.w, y + 16.0, 1.0, 1.0);
    }
    let cleared: Vec<Section> = o.overcome.iter().map(|c| c.section).collect();
    let bottom = WEAK_BAND.draw(
        c,
        g,
        &BandData {
            weakness: &w.weakness,
            phrases: w
                .phrases
                .iter()
                .take(3)
                .map(|p| Section::new(p.first, p.last))
                .collect(),
            selected: None,
            cleared,
        },
    );
    let mut ty = bottom + 4.0;
    let limit = y + ph - 26.0;
    for (i, p) in w.phrases.iter().take(3).enumerate() {
        if ty + 18.0 > limit {
            break;
        }
        let line = tf!(
            "{}. {}   weakness {:.0}%",
            "{}. {}　苦手度 {:.0}%",
            i + 1,
            section_text(Section::new(p.first, p.last)),
            p.weakness * 100.0
        );
        text(c, g, &line, x + 14.0, ty, 15.0, Color32::from_gray(230));
        ty += 20.0;
    }
    if w.phrases.is_empty() && ty + 18.0 <= limit {
        text(
            c,
            g,
            t("No weak phrases", "苦手なフレーズはありません"),
            x + 14.0,
            ty,
            15.0,
            Color32::from_gray(200),
        );
        ty += 20.0;
    }
    for ch in &o.overcome {
        if ty + 18.0 > limit {
            break;
        }
        let (line, color) = if ch.overcome {
            (
                tf!(
                    "Conquered! {} (practiced at {}%)",
                    "克服！ {}（練習 {}%）",
                    section_text(ch.section),
                    ch.speed
                ),
                CLEARED_COLOR,
            )
        } else {
            (
                tf!(
                    "{} still needs work (practiced at {}%)",
                    "{} はまだミスが出ました（練習 {}%）",
                    section_text(ch.section),
                    ch.speed
                ),
                Color32::from_gray(200),
            )
        };
        text(c, g, &line, x + 14.0, ty, 15.0, color);
        ty += 20.0;
    }
    if let Some(t) = &o.strokes {
        draw_strokes(c, g, t, x, ty, pw, limit);
    }
    // キーボードの P（ずれの板の「O」と同じ形。パッドでは結果の画面から練習へ飛ばない。
    // ドラムだけなら選曲の RD の簡易設定から）
    let hint = if w.phrases.is_empty() {
        t("P: Practice a Section", "P：区間を練習")
    } else {
        t("P: Practice Weak Phrases", "P：苦手なフレーズを練習")
    };
    let style = Style::edged(14.0, Color32::from_gray(210), Color32::BLACK, 1.0);
    if let Some(t) = g.texts.get(hint, &style) {
        t.draw(c, x + pw - 12.0 - t.w, y + ph - 22.0, 1.0, 1.0);
    }
}

/// スネアの打ち分けの欄の高さ（線・見出し・棒 2 本・叩けなかった小節）
const STROKES_H: f32 = 98.0;
/// リムの棒の色（スネアのチップの黄色）
const RIM_COLOR: Color32 = Color32::from_rgb(0xFF, 0xC8, 0x28);

/// 苦手な小節の欄の下に、スネアの打ち分け（リムの音符をリムで、普通の音符をヘッドかリムショットで叩けた数）。
/// 欄の下の端（`limit`）に寄せる。苦手なフレーズなどで場所が無ければ 1 行にまとめる
fn draw_strokes(c: &Canvas, g: &mut Gfx, t: &StrokeTally, x: f32, ty: f32, pw: f32, limit: f32) {
    let top = limit - STROKES_H;
    if ty > top {
        if ty + 18.0 <= limit {
            let line = tf!(
                "Snare strokes   Rim {} / {}   Head {} / {}",
                "スネアの打ち分け　リム {} / {}　ヘッド {} / {}",
                t.rim_ok,
                t.rim_hit,
                t.head_ok,
                t.head_hit
            );
            text(c, g, &line, x + 14.0, ty, 15.0, Color32::from_gray(230));
        }
        return;
    }
    c.fill(x + 12.0, top, pw - 24.0, 1.0, Color32::from_white_alpha(60));
    text(
        c,
        g,
        i18n::t("Snare Strokes", "スネアの打ち分け"),
        x + 14.0,
        top + 8.0,
        17.0,
        Color32::WHITE,
    );
    let rows = [
        (i18n::t("Rim", "リム"), t.rim_ok, t.rim_hit, RIM_COLOR),
        (
            i18n::t("Head", "ヘッド"),
            t.head_ok,
            t.head_hit,
            Color32::from_gray(235),
        ),
    ];
    let (bx, bw) = (x + 84.0, pw - 190.0);
    for (i, (label, ok, n, color)) in rows.into_iter().enumerate() {
        let y = top + 36.0 + i as f32 * 22.0;
        text(c, g, label, x + 14.0, y, 15.0, Color32::from_gray(230));
        c.fill(bx, y + 4.0, bw, 10.0, Color32::from_white_alpha(30));
        if n > 0 {
            c.fill(bx, y + 4.0, bw * ok as f32 / n as f32, 10.0, color);
        }
        let count = if n > 0 {
            format!("{ok} / {n}")
        } else {
            "-".to_owned()
        };
        text(c, g, &count, bx + bw + 12.0, y, 15.0, Color32::WHITE);
    }
    if !t.rim_missed_measures.is_empty() {
        let list = measures_text(&t.rim_missed_measures, 6);
        let line = if t.rim_missed_measures.len() == 1 {
            tf!(
                "Rim missed in measure {list}",
                "リムで叩けなかった所: {list} 小節"
            )
        } else {
            tf!(
                "Rim missed in measures {list}",
                "リムで叩けなかった所: {list} 小節"
            )
        };
        text(c, g, &line, x + 14.0, top + 80.0, 14.0, WEAK_COLOR);
    }
}

/// 小節の番号を「12・28・29」のように。`max` を超えたら「ほか n」を足す
fn measures_text(measures: &[u32], max: usize) -> String {
    let shown: Vec<String> = measures.iter().take(max).map(u32::to_string).collect();
    let mut s = shown.join(t(", ", "・"));
    if measures.len() > max {
        s += &tf!(" +{} more", " ほか {}", measures.len() - max);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_are_listed_with_the_rest_counted() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        assert_eq!(measures_text(&[12, 28, 29], 6), "12・28・29");
        assert_eq!(measures_text(&[1, 2, 3, 4], 2), "1・2 ほか 2");
    }

    #[test]
    fn old_laps_are_omitted_when_the_table_is_full() {
        assert_eq!(shown_laps(3, 10), (0, None));
        assert_eq!(shown_laps(10, 10), (0, None));
        // 11 周を 10 行に：1 行は知らせ、新しい 9 周を出す
        assert_eq!(shown_laps(11, 10), (2, Some(2)));
        assert_eq!(shown_laps(30, 10), (21, Some(21)));
    }
}
