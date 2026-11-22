//! 結果の画面に足すトレーニングの表示（NX の部品で）。
//!
//! - 練習の結果：ランクとジャケットの場所に、合格の数・クリア・気づいたこと（空打ちの偏り、ずれ）と、
//!   周ごとの成績の表を出す
//! - ふだんのプレイ：右の空いた場所に、苦手な小節を進み具合のバーの形（黒地に縞）で出す

use egui::Color32;
use engine::practice::{PASSES_TO_UP, Section, SpeedPlan, speed_percent};

use crate::band::{Band, BandData, CLEARED_COLOR};
use crate::game::{Outcome, PracticeOutcome};
use crate::gfx::{Canvas, Gfx};
use crate::inputs::lane_label;
use crate::practice::{
    LapSummary, StrayHint, WEAK_VELOCITY, rule_pass_label, section_label, style_label,
};
use crate::text::Style;

const PASS_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);
const FAIL_COLOR: Color32 = Color32::from_rgb(0xF0, 0x80, 0x60);
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
        "空打ち {} 回のうち {} 回が{}で、{} 回は強さ {WEAK_VELOCITY} 未満の弱い打でした。パッドが振動を拾いすぎているかもしれません。設定の「MIDI 機器」で最小ベロシティを上げると減ります。",
        h.total,
        h.count,
        lane_label(h.lane),
        h.weak
    )
}

/// ずれが一つの向きに偏っているときの知らせ
fn offset_note((mean, hits): (f64, usize)) -> Option<String> {
    (hits >= OFFSET_NOTE_HITS && mean.abs() >= OFFSET_NOTE_MS).then(|| {
        format!(
            "叩くタイミングは平均 {mean:+.0}ms（{}）でした。いつもこうなら、入力の補正（演奏中に - / =）で合わせると判定が楽になります。",
            if mean < 0.0 { "早め" } else { "遅め" }
        )
    })
}

/// 練習の結果。合格の数、クリア、気づいたこと、周ごとの成績の表
pub fn draw_practice(c: &Canvas, g: &mut Gfx, p: &PracticeOutcome) {
    let (x, y, w, h) = PRACTICE;
    panel(c, PRACTICE);
    let left = x + 20.0;
    let heading_w = g.heading(c, left, y + 10.0, "練習の結果");
    let o = p.options;
    let head = format!(
        "{}　{} / {} / {}",
        section_text(o.section),
        style_label(o.style),
        rule_pass_label(o.rule),
        plan_text(o.speed)
    );
    text(
        c,
        g,
        &head,
        left + heading_w + 24.0,
        y + 14.0,
        18.0,
        Color32::WHITE,
    );
    let mut ty = y + 50.0;
    let max_w = w - 40.0;
    let done: Vec<&LapSummary> = p.laps.iter().filter(|l| l.completed).collect();
    if done.is_empty() {
        text(
            c,
            g,
            "最後まで叩いた周はありませんでした。",
            left,
            ty,
            20.0,
            Color32::WHITE,
        );
        ty += 30.0;
    } else {
        let passed = done.iter().filter(|l| l.passed == Some(true)).count();
        let line = format!(
            "{} 周のうち合格 {passed} 周（{}）",
            done.len(),
            rule_pass_label(o.rule)
        );
        text(c, g, &line, left, ty, 20.0, Color32::WHITE);
        ty += 28.0;
        let reached = done.iter().map(|l| speed_percent(l.speed)).max();
        let top = done
            .iter()
            .filter(|l| l.passed == Some(true))
            .map(|l| speed_percent(l.speed))
            .max();
        let mut parts = Vec::new();
        if let (SpeedPlan::Ladder { start, target }, Some(r)) = (o.speed, reached) {
            parts.push(format!("{start}% から始めて {r}% まで（目標 {target}%）"));
        }
        if let Some(top) = top {
            parts.push(format!("合格した一番速い速度 {top}%"));
        }
        if !parts.is_empty() {
            text(
                c,
                g,
                &parts.join("　"),
                left,
                ty,
                16.0,
                Color32::from_gray(220),
            );
            ty += 22.0;
        }
    }
    if p.cleared
        && let SpeedPlan::Ladder { target, .. } = o.speed
    {
        let line = format!("クリア！ 目標の {target}% で {PASSES_TO_UP} 回合格しました");
        text(c, g, &line, left, ty, 20.0, PASS_COLOR);
        ty += 28.0;
    }
    // 気づいたこと
    if let Some(hint) = &p.strays {
        ty = paragraph(
            c,
            g,
            &stray_note(hint),
            left,
            ty + 2.0,
            14.0,
            NOTE_COLOR,
            max_w,
        );
    }
    if let Some(note) = p.offset.and_then(offset_note) {
        ty = paragraph(c, g, &note, left, ty + 2.0, 14.0, NOTE_COLOR, max_w);
    }
    if p.laps.len() > done.len() {
        text(
            c,
            g,
            "途中でやめた周は数えず、記録にも残しません。",
            left,
            ty,
            14.0,
            Color32::from_gray(160),
        );
        ty += 20.0;
    }
    if !done.is_empty() {
        ty += 8.0;
        lap_table(c, g, &done, left, ty, y + h - 44.0);
    }
    // 操作の案内
    let keys =
        "R：もう一度　　P：区間を選び直す　　Enter（ドラムは右シンバルかライド）：選曲に戻る";
    let style = Style::edged(15.0, Color32::from_gray(220), Color32::BLACK, 1.5);
    if let Some(t) = g.texts.get(keys, &style) {
        t.draw_fit(c, x + ((w - t.w) / 2.0).max(10.0), y + h - 30.0, w - 20.0);
    }
}

/// 表の列（見出し、左の端からの位置）
const COLUMNS: [(&str, f32); 10] = [
    ("周", 0.0),
    ("速度", 56.0),
    ("PERFECT", 138.0),
    ("GREAT", 268.0),
    ("GOOD", 344.0),
    ("POOR", 414.0),
    ("MISS", 482.0),
    ("空打ち", 546.0),
    ("ずれ", 616.0),
    ("", 718.0),
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
            Some(true) => ("合格", PASS_COLOR),
            Some(false) => ("不合格", FAIL_COLOR),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_laps_are_omitted_when_the_table_is_full() {
        assert_eq!(shown_laps(3, 10), (0, None));
        assert_eq!(shown_laps(10, 10), (0, None));
        // 11 周を 10 行に：1 行は知らせ、新しい 9 周を出す
        assert_eq!(shown_laps(11, 10), (2, Some(2)));
        assert_eq!(shown_laps(30, 10), (21, Some(21)));
    }
}
