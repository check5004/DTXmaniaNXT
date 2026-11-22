//! 結果の画面に足すトレーニングの表示（NX の部品で）。
//!
//! - 練習の結果：ランクとジャケットの場所に、のぼった段と挑戦の印、時間と周の数、ミスした所の地図、
//!   気づいたこと（空打ちの偏り、ずれ）を、文章でなく絵と数で出す。周ごとの成績の表は 2 ページ目
//! - ふだんのプレイ：右の空いた場所に、苦手な小節を進み具合のバーの形（黒地に縞）で出す

use egui::{Color32, Stroke, StrokeKind};
use engine::practice::{BeatSlot, Section, SpeedPlan, SpotTally, speed_percent};
use engine::stroke::StrokeTally;

use crate::band::{Band, BandData, CLEARED_COLOR};
use crate::game::{Outcome, PracticeOutcome};
use crate::gfx::{Canvas, Gfx};
use crate::inputs::lane_label;
use crate::practice::{LapSummary, StrayHint, WEAK_VELOCITY, section_label, style_label};
use crate::text::Style;

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
        SpeedPlan::Fixed(s) => format!("固定 {s}%"),
        SpeedPlan::Ladder { start, target } => format!("段階アップ {start}% → {target}%"),
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
    format!(
        "空打ちが {}で {} 回（うち弱い打 {} 回、強さ {WEAK_VELOCITY} 未満）。パッドの拾いすぎかも。設定の「MIDI 機器」で最小ベロシティを上げると減ります。",
        lane_label(h.lane),
        h.count,
        h.weak
    )
}

/// ずれが一つの向きに偏っているときの知らせ
fn offset_note((mean, hits): (f64, usize)) -> Option<String> {
    (hits >= OFFSET_NOTE_HITS && mean.abs() >= OFFSET_NOTE_MS).then(|| {
        format!(
            "叩くのが平均 {:.0}ms {}。いつもなら、演奏中に - / = で入力の補正を合わせると楽になります。",
            mean.abs(),
            if mean < 0.0 { "早め" } else { "遅め" }
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

/// 落とした所の地図（全部の周）。1 マスが 1 拍で、落とした周の数（1 回は黄、2 回から赤で ×n）
fn draw_map(
    c: &Canvas,
    g: &mut Gfx,
    p: &PracticeOutcome,
    spots: &SpotTally,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let s = p.options.section;
    let n = s.measures().max(1) as f32;
    let gap = if n > 6.0 { 2.0 } else { 4.0 };
    let row = (h / n - gap).clamp(3.0, 26.0);
    let labels = row >= 11.0;
    let label_w = if labels { 30.0 } else { 0.0 };
    let px = (row * 0.55).clamp(9.0, 14.0);
    for (i, m) in (s.first..=s.last).enumerate() {
        let ry = y + i as f32 * (row + gap);
        if labels {
            let style = Style::edged(px, Color32::from_gray(200), Color32::BLACK, 1.0);
            if let Some(t) = g.texts.get(&m.to_string(), &style) {
                t.draw(c, x + label_w - 8.0 - t.w, ry + (row - t.h) / 2.0, 1.0, 1.0);
            }
        }
        let beats = p.beats_per_measure.get(i).copied().unwrap_or(4).max(1);
        let cw = (w - label_w) / beats as f32;
        for b in 0..beats {
            let cell = (x + label_w + b as f32 * cw + 2.0, ry, cw - 4.0, row);
            let k = spots.count(BeatSlot {
                measure: m,
                beat: b,
            });
            if k >= 2 {
                round_fill(c, cell, 4.0, WEAK_COLOR.gamma_multiply(0.92));
                if labels {
                    centered(
                        c,
                        g,
                        &format!("×{k}"),
                        cell.0 + cell.2 / 2.0,
                        ry + row / 2.0,
                        px + 1.0,
                        Color32::WHITE,
                        cell.2,
                    );
                }
            } else if k == 1 {
                round_fill(c, cell, 4.0, SLIP_COLOR.gamma_multiply(0.45));
            } else {
                round_fill(c, cell, 4.0, Color32::from_white_alpha(20));
                round_frame(c, cell, 4.0, Color32::from_white_alpha(45));
            }
        }
    }
}

/// 練習の結果。段と挑戦の印、時間と周の数、ミスした所の地図、いちばんミスした所、気づいたこと
pub fn draw_practice(c: &Canvas, g: &mut Gfx, p: &PracticeOutcome) {
    let (x, y, w, h) = PRACTICE;
    panel(c, PRACTICE);
    let left = x + 20.0;
    let o = p.options;
    let head = format!(
        "{}　目標 {}%　{}",
        section_text(o.section),
        o.speed.goal(),
        style_label(o.style)
    );
    text(c, g, &head, left, y + 14.0, 20.0, Color32::WHITE);
    // 段（左）と数（右）
    draw_stairs(c, g, p, (left + 6.0, y + 58.0, 500.0, 190.0));
    let legend_y = y + 278.0;
    let dot = |c: &Canvas, cx: f32, color: Color32| {
        c.painter()
            .circle_filled(c.pos(cx, legend_y + 8.0), 5.0 * c.scale(), color);
    };
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
        "もう一度",
        left + 44.0 + w1,
        legend_y,
        13.0,
        Color32::from_gray(200),
    );
    let done: Vec<&LapSummary> = p.laps.iter().filter(|l| l.completed).collect();
    let clean = done.iter().filter(|l| l.passed == Some(true)).count();
    let nums = [
        (duration_text(p.elapsed_ms), "練習した時間"),
        (done.len().to_string(), "叩いた周"),
        (clean.to_string(), "ノーミスの周"),
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
    // ミスした所（全部の周）
    let mut spots = SpotTally::default();
    for l in &done {
        spots.add_lap(&l.misses);
    }
    let map_y = y + 306.0;
    text(
        c,
        g,
        "ミスした所（全部の周）",
        left,
        map_y,
        15.0,
        Color32::WHITE,
    );
    draw_map(c, g, p, &spots, (left - 4.0, map_y + 28.0, 340.0, 150.0));
    let worst = spots.weak(1);
    let wx = left + 370.0;
    match worst.first() {
        Some((b, spot)) => {
            text(
                c,
                g,
                "いちばんミスした所",
                wx,
                map_y + 30.0,
                14.0,
                Color32::from_gray(200),
            );
            let name = format!("{} 小節目の {} 拍目", b.measure, b.beat + 1);
            text(c, g, &name, wx, map_y + 54.0, 22.0, WEAK_COLOR);
            let lanes: Vec<&str> = spot.lanes.iter().map(|l| l.name()).collect();
            let line = format!("{}　{} 回", lanes.join("・"), spot.laps);
            text(c, g, &line, wx, map_y + 86.0, 18.0, Color32::WHITE);
        }
        None if !done.is_empty() => {
            text(c, g, "ミスなし", wx, map_y + 54.0, 22.0, PASS_COLOR);
        }
        None => {
            text(
                c,
                g,
                "最後まで叩いた周はありません",
                wx,
                map_y + 54.0,
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
    if let Some(note) = p.offset.and_then(offset_note) {
        notes.push(note);
    }
    let mut ny = map_y + 126.0;
    for n in notes {
        ny = paragraph(c, g, &n, wx, ny, 13.0, NOTE_COLOR, x + w - 20.0 - wx);
    }
    // 操作の案内
    let keys = "R：もう一度　　P：区間を選び直す　　→：周ごとの表　　Enter：選曲へ";
    let style = Style::edged(15.0, Color32::from_gray(220), Color32::BLACK, 1.5);
    if let Some(t) = g.texts.get(keys, &style) {
        t.draw_fit(c, x + ((w - t.w) / 2.0).max(10.0), y + h - 30.0, w - 20.0);
    }
}

/// 練習の結果の 2 ページ目：周ごとの成績の表
pub fn draw_practice_laps(c: &Canvas, g: &mut Gfx, p: &PracticeOutcome) {
    let (x, y, w, h) = PRACTICE;
    panel(c, PRACTICE);
    let left = x + 20.0;
    let heading_w = g.heading(c, left, y + 10.0, "周ごとの成績");
    let o = p.options;
    let head = format!("{}　{}", section_text(o.section), plan_text(o.speed));
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
            "途中でやめた周は入れていません（記録にも残しません）。",
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
            "最後まで叩いた周はありません。",
            left,
            ty,
            18.0,
            Color32::WHITE,
        );
    } else {
        lap_table(c, g, &done, left, ty, y + h - 44.0);
    }
    let keys = "←：練習の結果　　R：もう一度　　P：区間を選び直す　　Enter：選曲へ";
    let style = Style::edged(15.0, Color32::from_gray(220), Color32::BLACK, 1.5);
    if let Some(t) = g.texts.get(keys, &style) {
        t.draw_fit(c, x + ((w - t.w) / 2.0).max(10.0), y + h - 30.0, w - 20.0);
    }
}

/// 表の列（見出し、左の端からの位置）
const COLUMNS: [(&str, f32); 10] = [
    ("周", 0.0),
    ("速度", 44.0),
    ("PERFECT", 116.0),
    ("GREAT", 240.0),
    ("GOOD", 312.0),
    ("POOR", 378.0),
    ("MISS", 440.0),
    ("空打ち", 500.0),
    ("ずれ", 566.0),
    ("", 672.0),
];

/// 周ごとの成績の表。入りきらないときは新しい周を出す
fn lap_table(c: &Canvas, g: &mut Gfx, laps: &[&LapSummary], x: f32, y: f32, bottom: f32) {
    // 見出し（DFP では幅が足りないので、太字の小さい文字で）
    let head = Style {
        bold: true,
        ..Style::edged(15.0, Color32::from_rgb(190, 225, 255), Color32::BLACK, 1.5)
    };
    for (name, dx) in COLUMNS {
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
            &format!("（古い {n} 周は省きました）"),
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
            Some(true) => ("ノーミス", PASS_COLOR),
            Some(false) => ("ミス", SLIP_COLOR),
            None => ("途中", Color32::from_gray(150)),
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
    g.heading(c, x + 12.0, y + 8.0, "苦手な小節");
    let source = match (w.plays, w.practice_laps) {
        (0, 0) => "記録なし".to_owned(),
        (p, 0) => format!("直近のプレイ {p} 回から"),
        (0, l) => format!("直近の練習 {l} 周から"),
        (p, l) => format!("直近のプレイ {p} 回と練習 {l} 周から"),
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
        let line = format!(
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
            "苦手なフレーズはありません",
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
                format!("克服！ {}（練習 {}%）", section_text(ch.section), ch.speed),
                CLEARED_COLOR,
            )
        } else {
            (
                format!(
                    "{} はまだ崩れました（練習 {}%）",
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
    let hint = if w.phrases.is_empty() {
        "P：区間を練習"
    } else {
        "P：苦手なフレーズを練習"
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
            let line = format!(
                "スネアの打ち分け　リム {} / {}　ヘッド {} / {}",
                t.rim_ok, t.rim_hit, t.head_ok, t.head_hit
            );
            text(c, g, &line, x + 14.0, ty, 15.0, Color32::from_gray(230));
        }
        return;
    }
    c.fill(x + 12.0, top, pw - 24.0, 1.0, Color32::from_white_alpha(60));
    text(
        c,
        g,
        "スネアの打ち分け",
        x + 14.0,
        top + 8.0,
        17.0,
        Color32::WHITE,
    );
    let rows = [
        ("リム", t.rim_ok, t.rim_hit, RIM_COLOR),
        ("ヘッド", t.head_ok, t.head_hit, Color32::from_gray(235)),
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
        let line = format!(
            "リムで叩けなかった所: {} 小節",
            measures_text(&t.rim_missed_measures, 6)
        );
        text(c, g, &line, x + 14.0, top + 80.0, 14.0, WEAK_COLOR);
    }
}

/// 小節の番号を「12・28・29」のように。`max` を超えたら「ほか n」を足す
fn measures_text(measures: &[u32], max: usize) -> String {
    let shown: Vec<String> = measures.iter().take(max).map(u32::to_string).collect();
    let mut s = shown.join("・");
    if measures.len() > max {
        s += &format!(" ほか {}", measures.len() - max);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_are_listed_with_the_rest_counted() {
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
