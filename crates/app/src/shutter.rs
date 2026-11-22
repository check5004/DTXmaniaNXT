//! 画面の切り替えのシャッター（NX v2.0 の `GitaDoraTransition`）。
//! 上下の黒い板が回りながら閉じ、閉じきるあいだに右下へロゴが滑り込む。開くときは板が回りながら離れていく。
//!
//! 動きは v2 と同じ式で、進み具合 `progress` が -1〜2 を 1 秒に 3.4 ずつ動く。
//! 閉じるのは 2 → -1（0 で板が閉じきり、0〜-1 のあいだはロゴが滑る）、開くのは 0 → 2。
//! どの画面にも重ねて描き、閉じ終えたかを App が見て次の画面へ移る。

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use egui::Color32;

use crate::gfx::{Canvas, Gfx, Opt};

/// 進み具合が 1 秒に動く量（v2 `animationSpeed`）
const SPEED: f32 = 3.4;
/// 1 回に進める時間の上限（v2 と同じ。重い描画のあとで一気に進まない）
const MAX_STEP: f32 = 0.1;
/// 板の大きさ（v2：3000×1000 の黒い板を、画面の中心から上と下へ）
const PANEL_W: f32 = 3000.0;
const PANEL_H: f32 = 1000.0;
/// 画面の中心から角までの距離。板の間がこれより広ければ板は画面に掛からない
const HALF_DIAGONAL: f32 = 734.4;
/// ロゴ（`logo_small.png` を 412×71 に縮めて、y = 572 に。x は 635 から 815 へ滑る）
const LOGO_W: f32 = 412.0;
const LOGO_H: f32 = 71.0;
const LOGO_Y: f32 = 572.0;
const LOGO_START_X: f32 = 635.0;
const LOGO_FINAL_X: f32 = 815.0;

/// v2 の `Open` の既定の待ち（5 フレーム）
pub const OPEN_DELAY: Duration = Duration::from_millis(83);

pub struct Shutter {
    /// 進み具合（-1 で閉じきり、2 で開ききり）
    progress: f32,
    /// 開く向き（1）か閉じる向き（-1）
    dir: f32,
    animating: bool,
    /// 動き出す時刻（v2 の `delayFrameCounter` を時間にしたもの）
    start_at: Instant,
    /// 前に進めた時刻
    last: Instant,
}

impl Default for Shutter {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutter {
    /// 開いた状態で作る（v2 と同じく進み具合 1.5。板は画面の外）
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            progress: 1.5,
            dir: 1.0,
            animating: false,
            start_at: now,
            last: now,
        }
    }

    /// `delay` 待ってから閉じ始める（v2 `Close`）。開いている途中なら、そこから閉じる
    pub fn close(&mut self, delay: Duration) {
        let from = if self.animating && self.dir > 0.0 {
            self.progress.min(2.0)
        } else if self.progress <= -1.0 {
            -1.0
        } else {
            2.0
        };
        self.start(from, -1.0, delay);
    }

    /// `delay` 待ってから開き始める（v2 `Open`）。開いていても v2 と同じく閉じた形から開く。
    /// 板が閉じきる前に閉じるのをやめたときは、そこから開く
    pub fn open(&mut self, delay: Duration) {
        let from = if self.animating && self.dir < 0.0 {
            self.progress.max(0.0)
        } else {
            0.0
        };
        self.start(from, 1.0, delay);
    }

    fn start(&mut self, progress: f32, dir: f32, delay: Duration) {
        let now = Instant::now();
        self.progress = progress;
        self.dir = dir;
        self.animating = true;
        self.start_at = now + delay;
        self.last = now;
    }

    /// 閉じきって止まっている
    pub fn is_closed(&self) -> bool {
        !self.animating && self.progress <= -1.0
    }

    /// 動いている（待っているあいだも）
    pub fn animating(&self) -> bool {
        self.animating
    }

    /// 時間を進める（描くたびに呼ぶ）
    pub fn update(&mut self, now: Instant) {
        let dt = now
            .saturating_duration_since(self.last)
            .as_secs_f32()
            .min(MAX_STEP);
        self.last = now;
        if !self.animating || now < self.start_at {
            return;
        }
        self.progress += dt * SPEED * self.dir;
        if self.dir > 0.0 && self.progress >= 2.0 {
            self.progress = 2.0;
            self.animating = false;
        } else if self.dir < 0.0 && self.progress <= -1.0 {
            self.progress = -1.0;
            self.animating = false;
        }
    }

    /// 板の回り（ラジアン、時計回り）と、上下の板の間の広さ
    fn panels(&self) -> (f32, f32) {
        let t = self.progress.clamp(0.0, 2.0);
        let degrees = 52.08 * t + 87.88 * t * t - 2.72;
        let distance = 142.74 * t + 1717.83 * t * t - 614.51 * t * t * t - 0.64;
        (TAU * degrees / 360.0 + 0.047, distance)
    }

    /// ロゴの x と濃さ
    fn logo(&self) -> (f32, f32) {
        let r = ((self.progress - 0.63) / (-1.0 - 0.63)).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - r).powi(5);
        let x = LOGO_START_X + eased * (LOGO_FINAL_X - LOGO_START_X);
        let t = self.progress.clamp(0.0, 2.0);
        (x, (1.0 - t / 0.5).clamp(0.0, 1.0))
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        let (angle, distance) = self.panels();
        if self.progress <= 0.0 {
            // 閉じきっている（板の継ぎ目を出さない）
            c.cover(Color32::BLACK);
        } else if distance / 2.0 < HALF_DIAGONAL {
            let (sin, cos) = angle.sin_cos();
            let turn = |x: f32, y: f32| {
                let (dx, dy) = (x - 640.0, y - 360.0);
                (640.0 + dx * cos - dy * sin, 360.0 + dx * sin + dy * cos)
            };
            let (left, right) = (640.0 - PANEL_W / 2.0, 640.0 + PANEL_W / 2.0);
            let gap = distance / 2.0;
            for (top, bottom) in [
                (360.0 - gap - PANEL_H, 360.0 - gap),
                (360.0 + gap, 360.0 + gap + PANEL_H),
            ] {
                c.quad(
                    [
                        turn(left, top),
                        turn(right, top),
                        turn(right, bottom),
                        turn(left, bottom),
                    ],
                    Color32::BLACK,
                );
            }
        }
        let (x, alpha) = self.logo();
        if alpha > 0.0
            && let Some(t) = g.skin.tex_optional("Graphics/logo_small.png")
        {
            c.draw_opt(
                &t,
                x,
                LOGO_Y,
                Opt {
                    sx: LOGO_W / t.w,
                    sy: LOGO_H / t.h,
                    alpha,
                    ..Default::default()
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1/60 秒ずつ `secs` 秒進める
    fn run(s: &mut Shutter, secs: f32) {
        let mut now = s.last;
        for _ in 0..(secs * 60.0).round() as usize {
            now += Duration::from_secs_f32(1.0 / 60.0);
            s.update(now);
        }
    }

    #[test]
    fn starts_open_with_the_panels_off_screen() {
        let s = Shutter::new();
        assert!(!s.is_closed() && !s.animating());
        assert!(s.panels().1 / 2.0 >= HALF_DIAGONAL);
        assert_eq!(s.logo().1, 0.0);
    }

    #[test]
    fn closes_in_under_a_second_and_opens_again() {
        let mut s = Shutter::new();
        s.close(Duration::ZERO);
        // 2 → -1 を 1 秒に 3.4：約 0.88 秒
        run(&mut s, 0.8);
        assert!(!s.is_closed());
        // 板は閉じきり、ロゴが出ている
        assert!(s.progress <= 0.0);
        assert_eq!(s.logo().1, 1.0);
        run(&mut s, 0.1);
        assert!(s.is_closed());
        assert_eq!(s.logo().0, LOGO_FINAL_X);
        s.open(Duration::ZERO);
        assert!(!s.is_closed() && s.animating());
        // 0 → 2：約 0.59 秒
        run(&mut s, 0.6);
        assert!(!s.animating() && !s.is_closed());
        assert!(s.panels().1 / 2.0 >= HALF_DIAGONAL);
    }

    #[test]
    fn waits_before_moving() {
        let mut s = Shutter::new();
        s.close(Duration::from_millis(500));
        run(&mut s, 0.4);
        assert_eq!(s.progress, 2.0);
        run(&mut s, 0.2);
        assert!(s.progress < 2.0);
    }

    #[test]
    fn turning_back_starts_where_the_panels_are() {
        let mut s = Shutter::new();
        s.close(Duration::ZERO);
        run(&mut s, 0.2);
        let at = s.progress;
        assert!(at > 0.0 && at < 2.0);
        s.open(Duration::ZERO);
        assert_eq!(s.progress, at);
        run(&mut s, 0.1);
        let at = s.progress;
        s.close(Duration::ZERO);
        assert_eq!(s.progress, at);
        // 閉じきっていれば閉じたまま
        run(&mut s, 1.0);
        assert!(s.is_closed());
        s.close(Duration::ZERO);
        run(&mut s, 0.1);
        assert!(s.is_closed());
    }

    #[test]
    fn a_long_frame_moves_only_a_little() {
        let mut s = Shutter::new();
        s.close(Duration::ZERO);
        let later = s.last + Duration::from_secs(5);
        s.update(later);
        assert!((s.progress - (2.0 - MAX_STEP * SPEED)).abs() < 1e-4);
    }
}
