//! 練習中の表示（演奏画面に重ねる）。NX の部品で出す。
//!
//! - スコアの場所（左上）に周と速度を、スコアの数字（`7_score numbersGD.png`）と DFP の文字で
//! - その下に、区間・合格の基準・段階アップの進み・直近の周の成績の欄
//! - 進み具合のバー（`CActPerfProgressBar` の場所）に、曲の中の区間と今叩いている位置
//! - 助走のカウントを、レーンの真ん中にコンボの数字（`ScreenPlayDrums combo.png`）で大きく

use egui::Color32;
use engine::practice::{
    FAILS_TO_DOWN, LoopStyle, PASSES_TO_UP, PassRule, SpeedChange, SpeedPlan, speed_percent,
};

use crate::gfx::{Canvas, Gfx, Opt, src};
use crate::practice::{LapSummary, Practice};
use crate::text::Style;

/// 周と速度の数字の場所（上流のスコアと同じ高さ）
const LAP_X: f32 = 40.0;
const SPEED_X: f32 = 150.0;
const LABEL_Y: f32 = 12.0;
const DIGIT_Y: f32 = 41.0;
/// スコアの数字の 1 文字の幅と送り
const DIGIT_W: f32 = 36.0;
const DIGIT_H: f32 = 50.0;
const DIGIT_ADVANCE: f32 = 34.0;

/// 欄（スコアの下、左のパネルの上）
const INFO: (f32, f32, f32, f32) = (6.0, 98.0, 282.0, 148.0);

/// 進み具合のバー（play_view の `draw_progress` と同じ）
const BAR_X: f32 = 855.0;
const BAR_Y: f32 = 15.0;
const BAR_W: f32 = 20.0;
const BAR_H: f32 = 540.0;

/// カウントの数字を出す場所（レーンの真ん中）と、1 打ごとに縮む時間
const COUNT_X: f32 = 574.0;
const COUNT_Y: f32 = 300.0;
const COUNT_POP_MS: f64 = 180.0;

const SECTION_COLOR: Color32 = Color32::from_rgb(0x40, 0xC8, 0xFF);
const PASS_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);
const FAIL_COLOR: Color32 = Color32::from_rgb(0xF0, 0x80, 0x60);

/// 練習中の表示をすべて描く。`now` は表示の補正を足した実時間、`last_chip_ms` は曲の最後の音の時刻
pub fn draw(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64, last_chip_ms: f64) {
    draw_progress(c, p, now, last_chip_ms);
    draw_lap_and_speed(c, g, p, now);
    draw_info(c, g, p, now);
    draw_count_in(c, g, p, now);
}

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

/// 周と速度（スコアの場所）
fn draw_lap_and_speed(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    g.dfp(c, LAP_X, LABEL_Y, "LAP", false);
    g.dfp(c, SPEED_X, LABEL_Y, "SPEED", false);
    let lap = lap_number(p, now).min(999).to_string();
    let speed = current_speed(p, now).to_string();
    match g.skin.tex("Graphics/7_score numbersGD.png") {
        Some(t) => {
            let digits = |text: &str, x: f32| {
                for (i, ch) in text.chars().enumerate() {
                    if let Some(d) = ch.to_digit(10) {
                        c.part(
                            &t,
                            x + i as f32 * DIGIT_ADVANCE,
                            DIGIT_Y,
                            src(d as f32 * DIGIT_W, 0.0, DIGIT_W, DIGIT_H),
                        );
                    }
                }
                x + text.len() as f32 * DIGIT_ADVANCE
            };
            digits(&lap, LAP_X);
            let end = digits(&speed, SPEED_X);
            // % は達成率の大きい数字のもの（上流 7_RatePercent_l.png）
            match g.skin.tex("Graphics/7_RatePercent_l.png") {
                Some(pct) => c.draw(&pct, end + 2.0, DIGIT_Y + DIGIT_H - pct.h - 4.0),
                None => g.dfp(c, end + 2.0, DIGIT_Y + 20.0, "%", false),
            }
        }
        None => {
            let style = Style::edged(40.0, Color32::from_rgb(220, 240, 120), Color32::BLACK, 3.0);
            for (text, x) in [(lap, LAP_X), (format!("{speed}%"), SPEED_X)] {
                if let Some(t) = g.texts.get(&text, &style) {
                    t.draw(c, x, DIGIT_Y, 1.0, 1.0);
                }
            }
        }
    }
}

/// 区間、合格の基準、段階アップの進み、直近の周の成績
fn draw_info(c: &Canvas, g: &mut Gfx, p: &Practice, now: f64) {
    let (x, y, w, h) = INFO;
    c.fill(x, y, w, h, Color32::from_black_alpha(190));
    c.frame(x, y, w, h, Color32::from_white_alpha(60));
    let o = p.options;
    let left = x + 8.0;
    let mut y = y + 6.0;
    let line = |g: &mut Gfx, text: &str, px: f32, color: Color32, at_x: f32, y: f32| {
        let style = Style::edged(px, color, Color32::BLACK, 1.0);
        if let Some(t) = g.texts.get(text, &style) {
            t.draw_fit(c, at_x, y, x + w - 8.0 - at_x);
        }
    };
    let section = if o.section.first == o.section.last {
        format!("小節 {}", o.section.first)
    } else {
        format!("小節 {}〜{}", o.section.first, o.section.last)
    };
    line(g, &section, 16.0, Color32::WHITE, left, y);
    let rule = format!("{} / {}", style_name(o.style), rule_name(o.rule));
    line(
        g,
        &rule,
        13.0,
        Color32::from_gray(200),
        left + 140.0,
        y + 2.0,
    );
    y += 20.0;
    let ladder = p.ladder();
    let (passes, fails) = ladder.streaks();
    let plan = match ladder.plan() {
        SpeedPlan::Ladder { target, .. } => {
            format!("目標 {target}%　合格 {passes}/{PASSES_TO_UP}　不合格 {fails}/{FAILS_TO_DOWN}")
        }
        SpeedPlan::Fixed(_) => format!("速度は固定　連続合格 {}", p.pass_streak()),
    };
    line(g, &plan, 13.0, Color32::from_gray(225), left, y);
    y += 18.0;
    // 速度が動いたこと、決めた速度が次の周からのこと、クリア
    let note = if ladder.cleared() {
        Some(("クリア！".to_owned(), PASS_COLOR))
    } else if let Some(n) = p.speed_note(now) {
        match n.change {
            SpeedChange::Up => Some((
                format!("合格が続いたので {}% に上げます", n.speed),
                SECTION_COLOR,
            )),
            SpeedChange::Down => Some((
                format!("不合格が続いたので {}% に下げます", n.speed),
                FAIL_COLOR,
            )),
            SpeedChange::Cleared => Some((format!("{}% で合格が続きました", n.speed), PASS_COLOR)),
            SpeedChange::Stay => None,
        }
    } else if p.last_speed().is_some_and(|s| s != ladder.speed()) {
        Some((
            format!("{}% は {} 周目から", ladder.speed(), p.laps.len() + 1),
            SECTION_COLOR,
        ))
    } else {
        None
    };
    if let Some((text, color)) = note {
        line(g, &text, 13.0, color, left, y);
    }
    y += 18.0;
    let done: Vec<LapSummary> = p
        .summaries(now)
        .into_iter()
        .filter(|l| l.completed)
        .collect();
    let passed = done.iter().filter(|l| l.passed == Some(true)).count();
    line(
        g,
        &format!("合格 {passed} / {} 周", done.len()),
        13.0,
        Color32::from_gray(225),
        left,
        y,
    );
    y += 19.0;
    // 直近の 3 周（新しいものが上）。列ごとに揃える
    for l in done.iter().rev().take(3) {
        // ずれは結果の画面で見られるので、ここでは合否に関わる数だけ
        let cols: [(String, f32, Color32); 5] = [
            (format!("#{}", l.index + 1), 0.0, Color32::from_gray(200)),
            (
                format!("{}%", speed_percent(l.speed)),
                26.0,
                Color32::from_gray(200),
            ),
            (
                format!("PF {:.0}%", l.perfect_rate() * 100.0),
                64.0,
                Color32::WHITE,
            ),
            (
                format!("MISS {} / POOR {}", l.tally.miss, l.tally.poor),
                122.0,
                Color32::WHITE,
            ),
            verdict(l),
        ];
        for (text, dx, color) in cols {
            line(g, &text, 12.0, color, left + dx, y);
        }
        y += 15.0;
    }
}

/// 周の合否（欄の右の端）
fn verdict(l: &LapSummary) -> (String, f32, Color32) {
    match l.passed {
        Some(true) => ("合格".to_owned(), 236.0, PASS_COLOR),
        Some(false) => ("不合格".to_owned(), 230.0, FAIL_COLOR),
        None => ("途中".to_owned(), 236.0, Color32::from_gray(150)),
    }
}

fn style_name(s: LoopStyle) -> &'static str {
    match s {
        LoopStyle::Connect => "Connect",
        LoopStyle::Restart => "Restart",
    }
}

fn rule_name(r: PassRule) -> &'static str {
    match r {
        PassRule::Easy => "Easy",
        PassRule::Normal => "Normal",
        PassRule::Strict => "Strict",
    }
}

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

/// 助走のカウント（鳴ったクリックの数。周の中は None）
fn count_in(p: &Practice, now: f64) -> Option<(usize, f64)> {
    let lap = p.laps.iter().find(|l| {
        !l.plan.clicks.is_empty() && now >= l.plan.audio.at_ms && now < l.plan.start_ms
    })?;
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
                COUNT_X - w * scale / 2.0,
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
                t.draw(c, COUNT_X - t.w / 2.0, COUNT_Y - t.h / 2.0, 1.0, 1.0);
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
