//! 基礎練習の結果（結果の画面の、ランクとジャケットの場所）。文章でなく段と点で見せる。
//!
//! - 左上（スコアの場所）に、ミスなしで叩けた一番速いテンポと、最高を更新したか
//! - 右の欄に、段ごとの出来（◎ ミスなし、○ 1〜2、× 3 つ以上）、右手・左手ごとのずれの平均とばらつき、
//!   叩いた順のずれの点（段の区切りとテンポを下に）。限界チャレンジは終えた段までの段、
//!   持久は 30 秒ほどの区切りごとのばらつき、聞いてまねるはお手本の小節に色を敷く
//! - 左右は「手順どおりに叩いた」として数える（電子ドラムはどちらの手かを送ってこない）
//! - アクセントのある型で強さが届いたら、左上にアクセントの出た割合、段の印の下に段ごとの割合、
//!   ずれの点を半分の高さにして、その下に型の場所ごとの強さ（[`engine::accent`] でまわりと比べる）

use egui::{Color32, Stroke};
use engine::drill::{COUNT_IN_MEASURES, Grade, Hand, Lean, Strength, Way};
use i18n::{t, tf};

use crate::drill::{
    ACCENT_COLOR, DrillOutcome, LEFT_COLOR, RIGHT_COLOR, hand_name, lead_name, way_name,
};
use crate::drill_hud::{
    MANY_COLOR, NORMAL_STROKE, Stairs, clock, grade_color, grade_mark, stairs, up_to,
};
use crate::gfx::{Canvas, Gfx};
use crate::settings_ui as ui;
use crate::text::Style;

/// 右の欄
const PANEL: (f32, f32, f32, f32) = (480.0, 24.0, 780.0, 596.0);
/// ずれの棒の端（ミリ秒）と、点の図の上下の端（ミリ秒）
const LAG_RANGE_MS: f64 = 40.0;
const DOT_RANGE_MS: f64 = 60.0;
const FOOT_COLOR: Color32 = Color32::from_gray(200);
const ACCENT: Color32 = ACCENT_COLOR;
/// 強さを見せるとき、ずれの点の図の高さ（残りが強さの図）
const DOTS_H_WITH_STRENGTH: f32 = 150.0;
/// クリックの消えた小節の地の色
const SILENT_COLOR: Color32 = Color32::from_rgba_premultiplied(40, 26, 70, 90);
/// 聞いてまねるのお手本の小節の地の色
const DEMO_SHADE: Color32 = Color32::from_rgba_premultiplied(45, 35, 64, 70);

fn edged(px: f32, color: Color32) -> Style {
    Style::edged(px, color, Color32::BLACK, (px / 12.0).max(1.0))
}

fn bold(px: f32, color: Color32) -> Style {
    Style {
        bold: true,
        ..edged(px, color)
    }
}

/// 色の札の上の文字（縁取りなし。暗い文字に黒い縁を付けるとつぶれる）
fn pill_text(px: f32, color: Color32) -> Style {
    Style {
        bold: true,
        ..Style::plain(px, color)
    }
}

fn hand_color(h: Option<Hand>) -> Color32 {
    match h {
        Some(Hand::Right) => RIGHT_COLOR,
        Some(Hand::Left) => LEFT_COLOR,
        _ => FOOT_COLOR,
    }
}

/// ずれの数（+3 ms）
fn ms_text(v: f64) -> String {
    format!("{:+.0} ms", v)
}

/// スコアの場所：ミスなしで叩けた一番速いテンポと、最高を更新したか
pub fn draw_headline(c: &Canvas, g: &mut Gfx, o: &DrillOutcome) {
    g.heading(c, 30.0, 28.0, t("Drill Result", "基礎練習の結果"));
    let (text, color) = match o.best_now() {
        // テンポを上げない回（クリックの消えるドリル、上げ幅 0）は「まで」と言わない
        Some(b) if o.run.bpms.iter().all(|&v| v == b) => {
            (tf!("Clean at {b} ✓", "{b} でミスなし ✓"), Color32::WHITE)
        }
        Some(b) => (tf!("Clean up to {b} ✓", "{b} まで ✓"), Color32::WHITE),
        None if matches!(o.run.way, Way::Endurance(_)) => (
            t("No clean part this time", "ミスなしの区切りはありません").to_owned(),
            Color32::from_gray(220),
        ),
        None => (
            t("No clean step this time", "ミスなしの段はありません").to_owned(),
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
    if let Some(rate) = o.summary.strength.as_ref().and_then(|s| s.total().rate()) {
        let p = (rate * 100.0).round() as u32;
        ui::text(
            c,
            g,
            &tf!("Accents {p}%", "アクセント {p}%"),
            30.0,
            108.0,
            &bold(26.0, ACCENT),
            400.0,
        );
        ui::text(
            c,
            g,
            t(
                "Accents played clearly louder than the notes around them",
                "まわりの音よりはっきり強く叩けたアクセントの割合",
            ),
            32.0,
            142.0,
            &edged(13.0, Color32::from_gray(210)),
            420.0,
        );
    }
    if !o.new_best() {
        return;
    }
    let label = match o.best_before {
        Some(b) => tf!("New best (was {b})", "最高を更新（前は {b}）"),
        None => t("First clean record", "初めての記録").to_owned(),
    };
    let st = pill_text(15.0, Color32::from_rgb(40, 24, 0));
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
    // 名前とやり方・テンポ・長さ
    let nw = ui::text(
        c,
        g,
        name,
        x + 22.0,
        y + 16.0,
        &bold(22.0, Color32::WHITE),
        280.0,
    );
    let last = *run.bpms.last().unwrap_or(&run.target());
    let tempo = match run.way {
        Way::Limit => {
            let top = played_steps(o)
                .last()
                .map_or(run.target(), |&i| run.bpms[i]);
            format!("{} → {top}", run.target())
        }
        _ if run.bpms[0] == last => last.to_string(),
        _ => format!("{} → {last}", run.bpms[0]),
    };
    let mut way = way_name(run.way);
    if run.lefty {
        way = format!("{way}・{}", lead_name(true));
    }
    let meta = tf!(
        "{way} · {tempo} · {}",
        "{way}・{tempo}・{}",
        clock(o.elapsed_ms)
    );
    ui::text(
        c,
        g,
        &meta,
        x + 34.0 + nw,
        y + 22.0,
        &edged(15.0, Color32::from_gray(190)),
        w - 56.0 - nw,
    );
    // 段と出来（限界チャレンジは叩いた段のうち後ろの 8 段、持久は区切りごとのばらつき）
    let sx = x + 22.0;
    let sw = 300.0;
    let shown: Vec<usize> = match run.way {
        Way::Limit => {
            let p = played_steps(o);
            p[p.len().saturating_sub(8)..].to_vec()
        }
        _ => (0..run.steps()).collect(),
    };
    let grades: Vec<Option<Grade>> = shown
        .iter()
        .map(|&i| o.summary.steps.get(i).and_then(|s| s.grade()))
        .collect();
    if let Way::Endurance(_) = run.way {
        draw_parts(c, g, o, (sx, y + 66.0, sw, 120.0));
    } else {
        let bpms: Vec<u32> = shown.iter().map(|&i| run.bpms[i]).collect();
        let hidden: Vec<bool> = shown.iter().map(|&i| run.step_hidden(i)).collect();
        // 出来の印は段の下に色付きで出す
        stairs(
            c,
            g,
            &Stairs {
                bpms: &bpms,
                current: None,
                grades: &grades,
                marks: false,
                hidden: &hidden,
                flag: run.way == Way::Climb,
            },
            (sx, y + 66.0, sw, 120.0),
        );
    }
    let n = shown.len().max(1) as f32;
    let gap = if n > 6.0 { 4.0 } else { 8.0 };
    let cell = (sw - gap * (n - 1.0)) / n;
    for (i, gr) in grades.iter().enumerate() {
        let cx = sx + i as f32 * (cell + gap) + cell / 2.0;
        let (mark, color) = match gr {
            Some(gr) => (grade_mark(*gr), grade_color(*gr)),
            None => ("—", Color32::from_gray(140)),
        };
        let px = if n > 6.0 { 15.0 } else { 20.0 };
        ui::text_center(c, g, mark, cx, y + 194.0, &bold(px, color), cell + gap);
    }
    let strength = o.summary.strength.as_ref();
    // 段ごとのアクセントの出た割合（印の下）
    if let Some(st) = strength {
        let px = if n > 6.0 { 11.0 } else { 13.0 };
        for (i, &step) in shown.iter().enumerate() {
            let Some(rate) = st.steps.get(step).and_then(|a| a.rate()) else {
                continue;
            };
            let cx = sx + i as f32 * (cell + gap) + cell / 2.0;
            ui::text_center(
                c,
                g,
                &format!(">{:.0}%", rate * 100.0),
                cx,
                y + 218.0,
                &bold(px, ACCENT),
                cell + gap,
            );
        }
    }
    let legend = match run.way {
        Way::Limit => t(
            "◎ no misses   ○ 1–2 misses   × 3 (the end)",
            "◎ ミスなし　○ ミス 1〜2　× 3 つで終わり",
        ),
        Way::Endurance(_) => t(
            "Bar height = spread (lower is steadier)",
            "棒の高さはばらつき（低いほどそろっている）",
        ),
        _ => t(
            "◎ no misses   ○ 1–2 misses   × 3 or more",
            "◎ ミスなし　○ ミス 1〜2　× 3 つ以上",
        ),
    };
    let legend = if strength.is_some() {
        tf!("{legend}　> accents", "{legend}　> アクセント")
    } else {
        legend.to_owned()
    };
    ui::text(
        c,
        g,
        &legend,
        sx,
        y + if strength.is_some() { 238.0 } else { 226.0 },
        &edged(12.0, Color32::from_gray(180)),
        sw + 60.0,
    );
    draw_hands(c, g, o, (x + 370.0, y + 60.0, w - 392.0, 180.0));
    let bottom = (x + 22.0, y + 262.0, w - 44.0, h - 286.0);
    match strength {
        Some(st) => {
            let (bx, by, bw, bh) = bottom;
            draw_dots(c, g, o, perfect_ms, (bx, by, bw, DOTS_H_WITH_STRENGTH));
            let top = by + DOTS_H_WITH_STRENGTH + 8.0;
            draw_strength(c, g, o, st, (bx, top, bw, by + bh - top));
        }
        None => draw_dots(c, g, o, perfect_ms, bottom),
    }
}

/// 型の場所ごとの強さ（平均）。横は型の小節（拍の番号と線）、棒の高さは強さ（0〜127）、
/// 橙がアクセント（まわりより強くなかった所は中を抜く）、灰がふつうの音、点線がふつうの強さ。
/// アクセントのあるレーンが 2 つ以上なら、レーンごとに段を分ける
fn draw_strength(
    c: &Canvas,
    g: &mut Gfx,
    o: &DrillOutcome,
    st: &Strength,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let drill = o.run.drill;
    let measures = drill.measures();
    let title = if measures == 1 {
        t("Strength across the bar (average)", "1 小節の強さ（平均）").to_owned()
    } else {
        tf!(
            "Strength across {measures} bars (average)",
            "{measures} 小節の強さ（平均）"
        )
    };
    ui::text(
        c,
        g,
        &title,
        x,
        y,
        &edged(14.0, Color32::from_gray(200)),
        300.0,
    );
    // 凡例（右から）
    let mut lx = x + w;
    let st13 = edged(13.0, Color32::from_gray(210));
    for (label, color, dashed) in [
        (
            t("normal level", "ふつうの強さ"),
            Color32::from_white_alpha(220),
            true,
        ),
        (t("normal", "ふつう"), NORMAL_STROKE, false),
        (t("accent", "アクセント"), ACCENT, false),
    ] {
        let tw = ui::text_width(g, label, &st13);
        lx -= tw;
        ui::text(c, g, label, lx, y, &st13, tw + 2.0);
        if dashed {
            for k in 0..3 {
                c.fill(lx - 18.0 + k as f32 * 5.0, y + 9.0, 3.0, 1.0, color);
            }
        } else {
            ui::round_fill(c, lx - 16.0, y + 3.0, 11.0, 12.0, 2.0, color);
        }
        lx -= 30.0;
    }
    let (px, py, pw, ph) = (x + 40.0, y + 24.0, w - 40.0, h - 24.0);
    ui::round_fill(
        c,
        px,
        py,
        pw,
        ph,
        6.0,
        Color32::from_rgba_unmultiplied(4, 4, 10, 200),
    );
    let mw = pw / measures.max(1) as f32;
    // 拍の線と番号（拍子どおり。7/8 などはクリックのまとまりごと）、小節の区切り
    let beats = drill.count_in_clicks();
    for m in 0..measures {
        let mx = px + m as f32 * mw;
        for (b, &(pos, _)) in beats.iter().enumerate() {
            let bx = mx + pos as f32 * mw;
            if b > 0 || m > 0 {
                let a = if b == 0 { 90 } else { 35 };
                c.fill(bx, py, 1.0, ph, Color32::from_white_alpha(a));
            }
            ui::text(
                c,
                g,
                &(b + 1).to_string(),
                bx + 3.0,
                py + 2.0,
                &edged(11.0, Color32::from_gray(150)),
                20.0,
            );
        }
    }
    let rows = st.shapes.len().max(1);
    let letter_h = 16.0;
    let row_h = (ph - 14.0) / rows as f32;
    for (k, shape) in st.shapes.iter().enumerate() {
        let top = py + 14.0 + k as f32 * row_h;
        let base = top + row_h - letter_h - 2.0;
        let scale = (row_h - letter_h - 6.0) / 127.0;
        if rows > 1 {
            ui::text(
                c,
                g,
                shape.lane.name(),
                x,
                base - 20.0,
                &bold(13.0, ui::lane_color(shape.lane)),
                38.0,
            );
        }
        for cell in &shape.cells {
            let n = cell.note;
            let cw = mw / n.cells as f32;
            let cx = px + n.measure as f32 * mw + (n.cell as f32 + 0.5) * cw;
            let bw = (cw * 0.6).clamp(3.0, 26.0);
            if let Some(mean) = cell.mean {
                let bh = (mean as f32 * scale).max(2.0);
                let r = (cx - bw / 2.0, base - bh, bw, bh);
                let stood = shape
                    .normal
                    .is_none_or(|normal| engine::accent::stands_out(mean.round() as u8, normal));
                match (n.accent, stood) {
                    (true, true) => ui::round_fill(c, r.0, r.1, r.2, r.3, 3.0, ACCENT),
                    (true, false) => ui::round_frame(c, r.0, r.1, r.2, r.3, 3.0, 2.0, ACCENT),
                    (false, _) => ui::round_fill(c, r.0, r.1, r.2, r.3, 3.0, NORMAL_STROKE),
                }
            }
            // 叩く手（アクセントは大文字、ふつうは小文字。足は書かない）
            if cw >= 12.0 && !n.hand.is_foot() {
                let letter = match (n.hand, n.accent) {
                    (Hand::Right, true) => "R",
                    (Hand::Right, false) => "r",
                    (_, true) => "L",
                    _ => "l",
                };
                ui::text_center(
                    c,
                    g,
                    letter,
                    cx,
                    base + 1.0,
                    &bold(13.0, hand_color(Some(n.hand))),
                    cw.max(14.0),
                );
            }
        }
        if let Some(normal) = shape.normal {
            let ly = base - normal as f32 * scale;
            let mut lx = px + 4.0;
            while lx < px + pw - 4.0 {
                c.fill(lx, ly, 4.0, 1.0, Color32::from_white_alpha(220));
                lx += 8.0;
            }
        }
    }
}

/// 音符を判定した段（添字の順）。限界チャレンジで終えた段の後ろは入らない
fn played_steps(o: &DrillOutcome) -> Vec<usize> {
    let last = o.summary.steps.iter().rposition(|s| s.notes > 0);
    (0..last.map_or(0, |l| l + 1)).collect()
}

/// 持久の区切りごとのばらつき。棒の高さがばらつき（0〜30 ms）、色が出来、上に ±ms
fn draw_parts(c: &Canvas, g: &mut Gfx, o: &DrillOutcome, (x, y, w, h): (f32, f32, f32, f32)) {
    let steps = &o.summary.steps;
    let n = steps.len().max(1) as f32;
    let gap = 8.0;
    let pw = (w - gap * (n - 1.0)) / n;
    let base = y + h;
    let tall = h - 24.0;
    for (i, s) in steps.iter().enumerate() {
        let px = x + i as f32 * (pw + gap);
        ui::round_fill(
            c,
            px,
            base - tall,
            pw,
            tall,
            5.0,
            Color32::from_white_alpha(14),
        );
        let Some(sd) = s.offset_sd_ms else {
            continue;
        };
        let bh = (tall * (0.15 + 0.85 * sd.min(30.0) as f32 / 30.0)).max(8.0);
        let color = s.grade().map_or(Color32::from_gray(160), grade_color);
        ui::round_fill(c, px, base - bh, pw, bh, 5.0, color.gamma_multiply(0.75));
        ui::text_center(
            c,
            g,
            &format!("±{sd:.0}"),
            px + pw / 2.0,
            base - bh - 22.0,
            &bold(15.0, Color32::WHITE),
            pw + gap,
        );
    }
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
            // ずれの数字の下に札を置く（数字の横だと「ms」に重なる）
            let st = pill_text(12.0, Color32::from_rgb(10, 20, 40));
            let pw = ui::text_width(g, label, &st) + 16.0;
            let px = bx + bw + 12.0;
            ui::round_fill(c, px, ry + 25.0, pw, 19.0, 9.5, color);
            ui::text(c, g, label, px + 8.0, ry + 26.0, &st, pw);
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
    let run = &o.run;
    let gap = run.drill.gap.is_some();
    let echo = run.way == Way::Echo;
    // （名前、色、四角の見本か）
    let legend: Vec<(&str, Color32, bool)> = {
        let mut v = Vec::new();
        for lag in &o.summary.hands {
            v.push((hand_name(lag.hand), hand_color(Some(lag.hand)), false));
        }
        v.push((t("miss", "ミス"), MANY_COLOR, false));
        if gap {
            v.push((t("no click", "クリックなし"), SILENT_COLOR, true));
        }
        if echo {
            v.push((t("heard", "お手本"), DEMO_SHADE, true));
        }
        v
    };
    for (label, color, square) in legend.iter().rev() {
        let st = edged(13.0, Color32::from_gray(210));
        let tw = ui::text_width(g, label, &st);
        lx -= tw;
        ui::text(c, g, label, lx, y, &st, tw + 2.0);
        if *square {
            c.fill(lx - 15.0, y + 3.0, 11.0, 12.0, *color);
            c.frame(
                lx - 15.0,
                y + 3.0,
                11.0,
                12.0,
                Color32::from_white_alpha(90),
            );
        } else {
            c.painter()
                .circle_filled(c.pos(lx - 9.0, y + 9.0), 4.0 * c.scale(), *color);
        }
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
    // 横は叩いた所まで（限界チャレンジは終えた段まで）
    let steps = match run.way {
        Way::Limit => played_steps(o).last().map_or(1, |&i| i + 1),
        _ => run.steps(),
    };
    let bars_n = steps as u32 * run.step_measures;
    let bar_w = pw / bars_n.max(1) as f32;
    // クリックの消えた小節と、お手本の小節に色を敷く
    for m in 0..bars_n {
        let at = m + COUNT_IN_MEASURES;
        let shade = if gap && !run.clicks(at) {
            Some(SILENT_COLOR)
        } else if run.is_demo(at) {
            Some(DEMO_SHADE)
        } else {
            None
        };
        if let Some(color) = shade {
            c.fill(px + m as f32 * bar_w, py, bar_w, ph, color);
        }
    }
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
    // 段の区切りと、下にテンポ（持久は区切りの終わりの時刻）。狭ければ間引く
    let step_w = run.step_measures as f32 * bar_w;
    let every = (44.0 / step_w).ceil().max(1.0) as usize;
    let bar_ms = |bpm: u32| run.drill.bar_len() * 4.0 * 60_000.0 / f64::from(bpm.max(1));
    for (s, bpm) in run.bpms.iter().take(steps).enumerate() {
        let sx = px + s as f32 * step_w;
        if s > 0 {
            c.fill(sx, py, 1.0, ph, Color32::from_white_alpha(60));
        }
        if s % every != 0 {
            continue;
        }
        let label = match run.way {
            Way::Endurance(_) => {
                up_to(f64::from((s as u32 + 1) * run.step_measures) * bar_ms(*bpm))
            }
            _ => bpm.to_string(),
        };
        ui::text_center(
            c,
            g,
            &label,
            sx + step_w / 2.0,
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
        assert_eq!(ms_text(3.2), "+3 ms");
        assert_eq!(ms_text(-12.0), "-12 ms");
    }
}
