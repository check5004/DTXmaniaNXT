//! ずれの表示（このアプリで足したもの。絵は足さずにコードで描く）。
//! - 演奏中の「ずれのバー」: 左のスコアの下に、叩いた音符のずれを横の目盛りに印で出す（osu! の hit error bar と同じ形）。
//!   レーンにはかぶせない。練習ではスコアの場所に練習の欄が出るので、左下（成績の板の下）に出す。
//!   真ん中が ±0、左が早い、右が遅い。帯は判定の幅、印の色は判定の色で、少しずつ消える。▼ と数字は直近の平均
//! - 結果の画面の「ずれ」の板: 今回のずれの散らばりと、この曲・全体のずれ、補正のおすすめ（FT で受ける）

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::Color32;
use engine::judge::{HitWindows, Judgement};
use engine::lag::Advice;
use i18n::{count, t, tf};

use crate::game::Outcome;
use crate::gfx::{Canvas, Gfx};
use crate::text::Style;

/// 印が消えるまで
const MARK_LIFE: Duration = Duration::from_secs(4);
/// 出したばかりの印を長く描く間
const MARK_FRESH: Duration = Duration::from_millis(300);
/// 残しておく印の数（速い曲でも 4 秒ぶんは足りる）
const MAX_MARKS: usize = 256;
/// ▼ と数字に使う、直近の打数
pub const MEAN_OF: usize = 20;

/// バーの板の左端と幅（左の列。スコアと成績の板と同じ幅に収める）
const BOX_X: f32 = 10.0;
const BOX_W: f32 = 276.0;
/// バーの真ん中の x と、POOR の幅の端までの長さ（右に平均の数を出す）
const BAR_X: f32 = BOX_X + 106.0;
const BAR_HALF: f32 = 84.0;
/// バーの真ん中の y（スコアの下の空き。練習では成績の板の下）
const BAR_Y: f32 = 126.0;
const PRACTICE_BAR_Y: f32 = 699.0;

/// 判定の色（結果の分析と同じ）
pub fn judgement_color(j: Judgement) -> Color32 {
    match j {
        Judgement::Perfect => Color32::from_rgb(0xFF, 0xE0, 0x60),
        Judgement::Great => Color32::from_rgb(0x70, 0xE0, 0x90),
        Judgement::Good => Color32::from_rgb(0x70, 0xB0, 0xF0),
        Judgement::Poor => Color32::from_rgb(0xB0, 0xB0, 0xB0),
        Judgement::Miss => Color32::from_rgb(0xF0, 0x60, 0x60),
    }
}

/// ずれ `ms` の判定（帯の色に使う。判定の幅の外は POOR の色）
fn zone(w: &HitWindows, ms: f64) -> Judgement {
    let a = ms.abs();
    if a <= w.perfect {
        Judgement::Perfect
    } else if a <= w.great {
        Judgement::Great
    } else if a <= w.good {
        Judgement::Good
    } else {
        Judgement::Poor
    }
}

/// 符号つきのミリ秒（0 は ±0）
pub fn signed_ms(ms: f64) -> String {
    let r = ms.round() as i64;
    if r == 0 {
        "±0 ms".to_owned()
    } else {
        format!("{r:+} ms")
    }
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn draw_text(c: &Canvas, g: &mut Gfx, s: &str, x: f32, y: f32, px: f32, color: Color32) -> f32 {
    let style = Style::edged(px, color, Color32::BLACK, (px / 12.0).max(1.0));
    match g.texts.get(s, &style) {
        Some(t) => {
            t.draw(c, x, y, 1.0, 1.0);
            t.w
        }
        None => 0.0,
    }
}

/// 右をそろえて描く
fn draw_text_right(c: &Canvas, g: &mut Gfx, s: &str, right: f32, y: f32, px: f32, color: Color32) {
    let style = Style::edged(px, color, Color32::BLACK, (px / 12.0).max(1.0));
    if let Some(t) = g.texts.get(s, &style) {
        t.draw(c, right - t.w, y, 1.0, 1.0);
    }
}

/// 判定の幅の帯（横）。`cx` が ±0、`half` が POOR の端まで
fn bands(c: &Canvas, w: &HitWindows, cx: f32, cy: f32, half: f32, h: f32, alpha: f32) {
    let s = half / w.poor as f32;
    for (ms, j) in [
        (w.poor, Judgement::Poor),
        (w.good, Judgement::Good),
        (w.great, Judgement::Great),
        (w.perfect, Judgement::Perfect),
    ] {
        let a = if j == Judgement::Poor {
            alpha * 0.6
        } else {
            alpha
        };
        let r = ms as f32 * s;
        c.fill(
            cx - r,
            cy - h / 2.0,
            r * 2.0,
            h,
            with_alpha(judgement_color(j), a),
        );
    }
}

/// 下向きの三角（平均の印）
fn caret(c: &Canvas, x: f32, tip_y: f32, color: Color32) {
    c.quad(
        [
            (x - 6.0, tip_y - 7.0),
            (x + 6.0, tip_y - 7.0),
            (x, tip_y),
            (x, tip_y),
        ],
        color,
    );
}

/// 演奏中のずれのバー
#[derive(Debug, Clone)]
pub struct HitErrorBar {
    windows: HitWindows,
    /// 叩いた時刻、ずれ（ミリ秒）、判定。古い順
    marks: VecDeque<(Instant, f64, Judgement)>,
    /// 直近のずれ（▼ と数字）
    recent: VecDeque<f64>,
}

impl HitErrorBar {
    pub fn new(windows: HitWindows) -> Self {
        Self {
            windows: windows.ordered(),
            marks: VecDeque::new(),
            recent: VecDeque::new(),
        }
    }

    /// 叩いた音符 1 つ（見逃しは入れない）
    pub fn push(&mut self, at: Instant, offset_ms: f64, judgement: Judgement) {
        if !offset_ms.is_finite() {
            return;
        }
        if self.marks.len() >= MAX_MARKS {
            self.marks.pop_front();
        }
        self.marks.push_back((at, offset_ms, judgement));
        if self.recent.len() >= MEAN_OF {
            self.recent.pop_front();
        }
        self.recent.push_back(offset_ms);
    }

    /// 直近 [`MEAN_OF`] 打のずれの平均
    pub fn mean(&self) -> Option<f64> {
        (!self.recent.is_empty())
            .then(|| self.recent.iter().sum::<f64>() / self.recent.len() as f64)
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, now: Instant, practice: bool) {
        while self
            .marks
            .front()
            .is_some_and(|(at, ..)| now.saturating_duration_since(*at) > MARK_LIFE)
        {
            self.marks.pop_front();
        }
        let cy = if practice { PRACTICE_BAR_Y } else { BAR_Y };
        let w = self.windows;
        let s = BAR_HALF / w.poor as f32;
        c.fill(
            BOX_X,
            cy - 18.0,
            BOX_W,
            36.0,
            Color32::from_black_alpha(165),
        );
        bands(c, &w, BAR_X, cy, BAR_HALF, 4.0, 0.6);
        for &(at, ms, j) in &self.marks {
            let age = now.saturating_duration_since(at);
            let a = 1.0 - age.as_secs_f32() / MARK_LIFE.as_secs_f32();
            let h = if age < MARK_FRESH { 20.0 } else { 15.0 };
            let x = BAR_X + (ms.clamp(-w.poor, w.poor) as f32) * s;
            c.fill(
                x - 1.0,
                cy - h / 2.0,
                2.0,
                h,
                with_alpha(judgement_color(j), a),
            );
        }
        c.fill(BAR_X - 1.0, cy - 12.0, 2.0, 24.0, Color32::WHITE);
        let small = Color32::from_gray(190);
        draw_text(
            c,
            g,
            t("-", "早"),
            BAR_X - BAR_HALF - 15.0,
            cy - 8.0,
            12.0,
            small,
        );
        draw_text_right(
            c,
            g,
            t("+", "遅"),
            BAR_X + BAR_HALF + 15.0,
            cy - 8.0,
            12.0,
            small,
        );
        if let Some(m) = self.mean() {
            let x = BAR_X + (m.clamp(-w.poor, w.poor) as f32) * s;
            caret(c, x, cy - 11.0, Color32::WHITE);
            draw_text_right(
                c,
                g,
                &signed_ms(m),
                BOX_X + BOX_W - 8.0,
                cy - 9.0,
                15.0,
                Color32::WHITE,
            );
        }
    }
}

/// 結果の画面の「ずれ」の板に出すもの
#[derive(Debug, Clone)]
pub struct LagResult {
    /// この曲の譜面（難易度すべて。この曲の補正を書く先）
    pub charts: Vec<PathBuf>,
    pub advice: Advice,
    /// おすすめにしたなら、その前の値（入力の補正（全体）、この曲の補正）。もう一度 FT でもとに戻す
    pub applied: Option<(f64, f64)>,
}

/// 板の場所（苦手な小節の板の上の空き）
const PANEL: (f32, f32, f32, f32) = (836.0, 62.0, 434.0, 222.0);

/// おすすめの中身（「この曲の補正 +0 → +10 ms」など）。すすめるものが無ければ None
pub fn advice_text(a: &Advice) -> Option<String> {
    let global = (a.global_ms != a.global_now).then(|| {
        tf!(
            "Global {:+} → {:+}",
            "全体 {:+} → {:+}",
            a.global_now,
            a.global_ms
        )
    });
    let song = (a.song_ms != a.song_now).then(|| {
        tf!(
            "This song {:+} → {:+}",
            "この曲 {:+} → {:+}",
            a.song_now,
            a.song_ms
        )
    });
    match (global, song) {
        (Some(g), Some(s)) => Some(tf!("{g}, {s} ms", "{g}、{s} ms")),
        (Some(_), None) => Some(tf!(
            "Global offset {:+} → {:+} ms",
            "入力の補正（全体 {:+} → {:+} ms）",
            a.global_now,
            a.global_ms
        )),
        (None, Some(_)) => Some(tf!(
            "Song offset {:+} → {:+} ms",
            "この曲の補正 {:+} → {:+} ms",
            a.song_now,
            a.song_ms
        )),
        (None, None) => None,
    }
}

/// 結果の 1 ページ目の「ずれ」の板（普通のプレイだけ）
pub fn draw_result(c: &Canvas, g: &mut Gfx, o: &Outcome) {
    let Some(lag) = &o.lag else {
        return;
    };
    let (x, y, pw, ph) = PANEL;
    c.fill(x, y, pw, ph, Color32::from_rgba_unmultiplied(8, 8, 16, 225));
    c.frame(x, y, pw, ph, Color32::from_gray(150));
    g.heading(c, x + 12.0, y + 8.0, t("Timing", "ずれ"));
    draw_text_right(
        c,
        g,
        t("Early ←   → Late", "早い ←　→ 遅い"),
        x + pw - 12.0,
        y + 16.0,
        13.0,
        Color32::from_gray(200),
    );
    histogram(
        c,
        x + 14.0,
        y + 44.0,
        pw - 28.0,
        40.0,
        &o.offsets,
        &o.windows,
    );
    let white = Color32::from_gray(235);
    let gray = Color32::from_gray(200);
    let mut ty = y + 94.0;
    let now = match (o.offset_mean_ms, o.offset_sd_ms) {
        (Some(m), Some(sd)) => tf!(
            "This play {}   Spread ±{sd:.1} ms",
            "今回 {}　ばらつき ±{sd:.1} ms",
            signed_ms_1(m)
        ),
        (Some(m), None) => tf!("This play {}", "今回 {}", signed_ms_1(m)),
        _ => t(
            "No notes were hit this time",
            "今回は叩けた音符がありません",
        )
        .to_owned(),
    };
    draw_text(c, g, &now, x + 14.0, ty, 15.0, white);
    ty += 22.0;
    let a = &lag.advice;
    let song = match (a.song, a.song_residual_ms()) {
        (Some(s), Some(r)) if s.enough() => {
            tf!(
                "This song ({}): {}",
                "この曲 直近 {} {}",
                count(s.sessions, "play", "plays", "回"),
                signed_ms(r)
            )
        }
        _ => tf!(
            "Not enough hits on this song yet (needs {})",
            "この曲はまだ記録が足りません（{} 打から）",
            engine::lag::MIN_HITS
        ),
    };
    draw_text(c, g, &song, x + 14.0, ty, 14.0, gray);
    ty += 20.0;
    if let (Some((_, n)), Some(r)) = (a.global, a.global_residual_ms()) {
        draw_text(
            c,
            g,
            &tf!(
                "All songs (median of {n}) {}",
                "全部の曲（{n} 曲の真ん中）{}",
                signed_ms(r)
            ),
            x + 14.0,
            ty,
            14.0,
            gray,
        );
    }
    let (bx, by, bw, bh) = (x + 12.0, y + ph - 52.0, pw - 24.0, 40.0);
    let orange = Color32::from_rgb(0xFF, 0xB8, 0x40);
    let (label, hint) = match (lag.applied, advice_text(a)) {
        (Some(_), Some(text)) => (text, t("Undo", "もとに戻す")),
        (None, Some(text)) => (text, t("Apply", "そうする")),
        (_, None) => {
            if a.song.is_some_and(|s| s.enough()) {
                draw_text(
                    c,
                    g,
                    t(
                        "Your current offset looks right",
                        "今の補正のままでよさそうです",
                    ),
                    bx + 2.0,
                    by + 12.0,
                    14.0,
                    gray,
                );
            }
            return;
        }
    };
    c.fill(bx, by, bw, bh, with_alpha(orange, 0.12));
    c.frame(bx, by, bw, bh, orange);
    let head = if lag.applied.is_some() {
        t("Applied", "変えました")
    } else {
        t("Suggested", "おすすめ")
    };
    let hw = draw_text(c, g, head, bx + 10.0, by + 11.0, 13.0, orange);
    draw_text(
        c,
        g,
        &label,
        bx + 18.0 + hw,
        by + 10.0,
        15.0,
        Color32::WHITE,
    );
    // パッドの印（FT）と、キーボードの O
    let hint = tf!("FT / O: {hint}", "FT / O：{hint}");
    draw_text_right(c, g, &hint, bx + bw - 10.0, by + 11.0, 13.0, white);
}

/// 小数 1 桁の符号つきのミリ秒
fn signed_ms_1(ms: f64) -> String {
    format!("{ms:+.1} ms")
}

/// 今回のずれの散らばり（6 ms ごとの棒）。帯は判定の幅、白い線が ±0、▼ が平均
fn histogram(c: &Canvas, x: f32, y: f32, w: f32, h: f32, offsets: &[f64], windows: &HitWindows) {
    let win = windows.ordered();
    c.fill(x, y, w, h, Color32::from_black_alpha(130));
    let half = w / 2.0;
    let cx = x + half;
    let s = half / win.poor as f32;
    bands(c, &win, cx, y + h - 2.0, half, 4.0, 0.45);
    const BIN: f64 = 6.0;
    let bins = ((win.poor * 2.0) / BIN).ceil() as usize;
    let mut counts = vec![0usize; bins.max(1)];
    for &o in offsets {
        if o.abs() > win.poor {
            continue;
        }
        let i = (((o + win.poor) / BIN) as usize).min(counts.len() - 1);
        counts[i] += 1;
    }
    let top = counts.iter().copied().max().unwrap_or(0);
    if top > 0 {
        for (i, &n) in counts.iter().enumerate() {
            if n == 0 {
                continue;
            }
            let from = i as f64 * BIN - win.poor;
            let bh = (h - 8.0) * n as f32 / top as f32;
            let color = judgement_color(zone(&win, from + BIN / 2.0));
            c.fill(
                cx + from as f32 * s + 1.0,
                y + h - 5.0 - bh,
                (BIN as f32 * s - 2.0).max(1.0),
                bh,
                color,
            );
        }
    }
    c.fill(cx - 1.0, y, 2.0, h, Color32::WHITE);
    if !offsets.is_empty() {
        let m = offsets.iter().sum::<f64>() / offsets.len() as f64;
        caret(
            c,
            cx + m.clamp(-win.poor, win.poor) as f32 * s,
            y,
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advice(global: (i64, i64), song: (i64, i64)) -> Advice {
        Advice {
            global: None,
            song: None,
            global_now: global.0,
            global_ms: global.1,
            song_now: song.0,
            song_ms: song.1,
        }
    }

    #[test]
    fn advice_names_what_changes() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        assert_eq!(advice_text(&advice((3, 3), (0, 0))), None);
        assert_eq!(
            advice_text(&advice((3, 3), (0, 10))).as_deref(),
            Some("この曲の補正 +0 → +10 ms")
        );
        assert_eq!(
            advice_text(&advice((0, 12), (0, 0))).as_deref(),
            Some("入力の補正（全体 +0 → +12 ms）")
        );
        assert_eq!(
            advice_text(&advice((0, 12), (5, -2))).as_deref(),
            Some("全体 +0 → +12、この曲 +5 → -2 ms")
        );
        let _en = i18n::scoped(i18n::Lang::En);
        assert_eq!(
            advice_text(&advice((0, 12), (0, 0))).as_deref(),
            Some("Global offset +0 → +12 ms")
        );
        assert_eq!(
            advice_text(&advice((0, 12), (5, -2))).as_deref(),
            Some("Global +0 → +12, This song +5 → -2 ms")
        );
    }

    #[test]
    fn the_bar_keeps_the_recent_mean() {
        let mut b = HitErrorBar::new(HitWindows::default());
        assert_eq!(b.mean(), None);
        let t = Instant::now();
        for i in 0..30 {
            b.push(t, if i < 10 { 100.0 } else { 10.0 }, Judgement::Great);
        }
        // 古い 10 打は平均から外れる
        assert_eq!(b.mean(), Some(10.0));
        b.push(t, f64::NAN, Judgement::Perfect);
        assert_eq!(b.mean(), Some(10.0));
    }

    #[test]
    fn signed_ms_shows_zero_plainly() {
        assert_eq!(signed_ms(0.4), "±0 ms");
        assert_eq!(signed_ms(-12.6), "-13 ms");
        assert_eq!(signed_ms(7.0), "+7 ms");
    }
}
