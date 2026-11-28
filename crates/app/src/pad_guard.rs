//! 1 打では戻しにくい操作（結果の画面のもう一度など）を、同じパッドを続けて 2 回叩いてから確かめる門。
//!
//! 電子ドラムで遊ぶ人は、結果の画面などで何気なく叩き続ける。1 打で別の画面へ移ると面倒なので、
//! 同じパッドを [`TWICE_US`] 以内に 2 回叩いたら「〜しますか？」と確かめ、CY で「はい」、LC で「いいえ」にする。
//! 1 回目では、画面の下にもう一度叩くと何が起きるかを出すだけ。キーボードの近道（R・P・V・O など）はすぐ動く。

use std::time::{Duration, Instant};

use dtx::DrumLane;
use egui::Color32;
use i18n::{t, tf};

use crate::gfx::{Canvas, Gfx};
use crate::settings_ui::{self as ui, Pad};

/// 同じパッドを続けて叩いたとみなす間（1 回目からの時間）
pub const TWICE_US: i64 = 1_000_000;
/// 1 回目のあと、画面の下に「もう一度叩くと」を出しておく時間
pub const ARMED_SHOW: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, Copy)]
struct Armed<A> {
    lane: DrumLane,
    action: A,
    at_us: i64,
    shown: Instant,
}

#[derive(Debug)]
pub struct PadGuard<A> {
    armed: Option<Armed<A>>,
}

impl<A: Copy + PartialEq> Default for PadGuard<A> {
    fn default() -> Self {
        Self { armed: None }
    }
}

impl<A: Copy + PartialEq> PadGuard<A> {
    /// パッド `lane` を叩いた（その操作は `action`）。同じパッドで同じ操作を [`TWICE_US`] 以内に 2 回目なら true（確かめる）。
    /// 1 回目や、間があいたとき・別のパッドのときは、これを 1 回目として false
    pub fn hit(&mut self, lane: DrumLane, action: A, at_us: i64, now: Instant) -> bool {
        let lane = same_pad(lane);
        let twice = self.armed.is_some_and(|a| {
            a.lane == lane && a.action == action && (0..=TWICE_US).contains(&(at_us - a.at_us))
        });
        self.armed = (!twice).then_some(Armed {
            lane,
            action,
            at_us,
            shown: now,
        });
        twice
    }

    /// ほかのパッドを叩いた・画面が変わったなど。1 回目を忘れる
    pub fn clear(&mut self) {
        self.armed = None;
    }

    /// 画面の下に出す「もう一度叩くと」（1 回目から [`ARMED_SHOW`] のあいだ）
    pub fn armed(&self, now: Instant) -> Option<(DrumLane, A)> {
        self.armed
            .filter(|a| now.saturating_duration_since(a.shown) < ARMED_SHOW)
            .map(|a| (a.lane, a.action))
    }
}

/// 画面の下の帯のすぐ上に「続けて SD を叩くと『もう一度』」を出す（1 回目を叩いたあと）
pub fn draw_armed(c: &Canvas, g: &mut Gfx, lane: DrumLane, what: &str) {
    let pad = pad_of(lane);
    let text = tf!(
        "Hit {pad} again: {what}",
        "続けて {pad} を叩くと「{what}」",
        pad = pad.name()
    );
    let style = ui::bold(17.0, ui::TEXT);
    let w = ui::text_width(g, &text, &style) + 40.0;
    let (x, y, h) = (640.0 - w / 2.0, ui::LEGEND_Y - 50.0, 38.0);
    ui::round_fill(
        c,
        x,
        y,
        w,
        h,
        h / 2.0,
        Color32::from_rgba_unmultiplied(12, 12, 24, 235),
    );
    ui::round_frame(c, x, y, w, h, h / 2.0, 2.0, pad.color());
    ui::text_center(c, g, &text, 640.0, y + 8.0, &style, w);
}

/// 「もう一度遊びますか？」の確かめ。CY ではい、LC でいいえ（キーボードは Enter・Esc）
pub fn draw_confirm(c: &Canvas, g: &mut Gfx, question: &str) {
    c.cover(Color32::from_black_alpha(150));
    let (w, h) = (600.0, 196.0);
    let (x, y) = (640.0 - w / 2.0, 360.0 - h / 2.0);
    ui::panel(c, x, y, w, h);
    ui::text_center(
        c,
        g,
        question,
        640.0,
        y + 40.0,
        &ui::bold(26.0, ui::TEXT),
        w - 40.0,
    );
    let buttons = [
        (Pad::CY, t("Yes (Enter)", "はい（Enter）")),
        (Pad::LC, t("No (Esc)", "いいえ（Esc）")),
    ];
    let (bw, bh, gap) = (230.0, 52.0, 24.0);
    let mut bx = 640.0 - bw - gap / 2.0;
    let by = y + h - bh - 28.0;
    for (pad, label) in buttons {
        ui::round_fill(
            c,
            bx,
            by,
            bw,
            bh,
            8.0,
            Color32::from_rgba_unmultiplied(30, 30, 46, 240),
        );
        ui::round_frame(c, bx, by, bw, bh, 8.0, 2.0, pad.color());
        let chip = ui::bold(14.0, Color32::BLACK);
        ui::round_fill(c, bx + 16.0, by + 15.0, 36.0, 22.0, 11.0, pad.color());
        ui::text_center(c, g, pad.name(), bx + 34.0, by + 17.0, &chip, 36.0);
        ui::text(
            c,
            g,
            label,
            bx + 66.0,
            by + 13.0,
            &ui::bold(20.0, ui::TEXT),
            bw - 76.0,
        );
        bx += bw + gap;
    }
}

/// 帯に出すパッド
fn pad_of(lane: DrumLane) -> Pad {
    match lane {
        DrumLane::LC => Pad::LC,
        DrumLane::HH | DrumLane::HHO => Pad::HH,
        DrumLane::SD => Pad::SD,
        DrumLane::HT => Pad::HT,
        DrumLane::LT => Pad::LT,
        DrumLane::FT => Pad::FT,
        DrumLane::CY => Pad::CY,
        DrumLane::RD => Pad::RD,
        DrumLane::BD | DrumLane::LBD | DrumLane::LP => Pad::BD,
    }
}

/// 開いたハイハットも閉じたハイハットと同じパッド
fn same_pad(lane: DrumLane) -> DrumLane {
    match lane {
        DrumLane::HHO => DrumLane::HH,
        l => l,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Act {
        Retry,
        Replay,
    }

    #[test]
    fn the_same_pad_twice_within_a_second_asks() {
        let now = Instant::now();
        let mut g = PadGuard::default();
        assert!(!g.hit(DrumLane::SD, Act::Retry, 0, now));
        assert_eq!(g.armed(now), Some((DrumLane::SD, Act::Retry)));
        assert!(g.hit(DrumLane::SD, Act::Retry, 600_000, now));
        // 確かめたら 1 回目は忘れる（3 回目は新しい 1 回目）
        assert_eq!(g.armed(now), None);
        assert!(!g.hit(DrumLane::SD, Act::Retry, 700_000, now));
    }

    #[test]
    fn a_slow_second_hit_or_another_pad_starts_over() {
        let now = Instant::now();
        let mut g = PadGuard::default();
        assert!(!g.hit(DrumLane::SD, Act::Retry, 0, now));
        // 1 秒より後は 1 回目からやり直し
        assert!(!g.hit(DrumLane::SD, Act::Retry, 1_200_000, now));
        assert!(g.hit(DrumLane::SD, Act::Retry, 1_900_000, now));
        // 別のパッドをはさむと数え直す
        assert!(!g.hit(DrumLane::SD, Act::Retry, 3_000_000, now));
        assert!(!g.hit(DrumLane::HH, Act::Replay, 3_100_000, now));
        assert!(!g.hit(DrumLane::SD, Act::Retry, 3_200_000, now));
        // 開いたハイハットも同じハイハット
        assert!(!g.hit(DrumLane::HHO, Act::Replay, 5_000_000, now));
        assert!(g.hit(DrumLane::HH, Act::Replay, 5_300_000, now));
    }

    #[test]
    fn the_hint_fades_after_a_second() {
        let now = Instant::now();
        let mut g = PadGuard::default();
        g.hit(DrumLane::FT, Act::Retry, 0, now);
        assert!(g.armed(now + Duration::from_millis(900)).is_some());
        assert!(g.armed(now + Duration::from_millis(1100)).is_none());
        g.hit(DrumLane::FT, Act::Retry, 0, now);
        g.clear();
        assert!(g.armed(now).is_none());
    }
}
