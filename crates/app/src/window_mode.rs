//! 窓の出し方（設定の「画面の出し方」）を窓に当てる。
//!
//! - 全画面は winit の `Fullscreen::Borderless`（解像度は変えない）。OS は全画面の窓として扱い、
//!   ドライバー（NVIDIA の Vulkan など）は DWM を通さず直に出すので、ほかの窓へ切り替えると一瞬暗くなることがある。
//! - ボーダーレスは、枠のない普通の窓をモニターいっぱいに広げる。モニターとぴったり同じ大きさだと
//!   ドライバーと OS が全画面と見なすので、隣にモニターの無い辺へ 1 ドットはみ出させる
//!   （SDL や GLFW で知られたやり方）。macOS は簡易フルスクリーン（別の操作スペースを作らない）。
//!   ほかの OS は全画面と同じ（Wayland では窓の位置を決められない）。

use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::window::{Fullscreen, Window};

use crate::settings::DisplayMode;

/// ボーダーレスの窓をモニターからはみ出させる幅（ドット）
#[cfg_attr(not(windows), allow(dead_code))]
const BLEED: i32 = 1;

/// ウィンドウに戻すとき、前の位置と大きさが分からなければこの大きさで画面の真ん中に
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 720.0);

/// 画面上の四角（物理ドット）。はみ出させるのは Windows だけ
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Rect {
    fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

/// ボーダーレスの窓の四角。`monitor` を覆い、ほかのモニター（`all`）の無い辺へ [`BLEED`] だけはみ出す。
/// 下・上・右・左の順に探し、どの辺にも隣があれば下（隣のモニターに 1 ドット出るだけ）
#[cfg_attr(not(windows), allow(dead_code))]
pub fn borderless_rect(monitor: Rect, all: &[Rect]) -> Rect {
    let m = monitor;
    let sides = [
        Rect {
            h: m.h + BLEED,
            ..m
        },
        Rect {
            y: m.y - BLEED,
            h: m.h + BLEED,
            ..m
        },
        Rect {
            w: m.w + BLEED,
            ..m
        },
        Rect {
            x: m.x - BLEED,
            w: m.w + BLEED,
            ..m
        },
    ];
    sides
        .into_iter()
        .find(|r| all.iter().all(|o| *o == m || !r.intersects(o)))
        .unwrap_or(sides[0])
}

/// ウィンドウのときの位置と大きさ（ウィンドウから離れるときに残し、戻るときに使う）
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    maximized: bool,
}

impl Placement {
    fn of(window: &Window) -> Option<Self> {
        if window.is_minimized() == Some(true) {
            return None;
        }
        Some(Self {
            pos: window.outer_position().ok()?,
            size: window.inner_size(),
            maximized: window.is_maximized(),
        })
    }
}

/// 窓の出し方を `from` から `to` に変える（`from` が `None` なら開いたばかり）。
/// `saved` はウィンドウのときの位置と大きさで、ウィンドウから離れるときに残し、ウィンドウに戻るときに使う
pub fn apply(
    window: &Window,
    from: Option<DisplayMode>,
    to: DisplayMode,
    saved: &mut Option<Placement>,
) {
    if from == Some(to) {
        return;
    }
    if from == Some(DisplayMode::Window) {
        *saved = Placement::of(window);
    }
    match to {
        DisplayMode::Window => {
            leave_borderless(window);
            window.set_fullscreen(None);
            window.set_decorations(true);
            window.set_resizable(true);
            match saved {
                Some(p) => {
                    window.set_outer_position(p.pos);
                    let _ = window.request_inner_size(p.size);
                    if p.maximized {
                        window.set_maximized(true);
                    }
                }
                // 全画面で開いてから戻ったとき
                None => center(window),
            }
        }
        DisplayMode::Borderless => {
            window.set_fullscreen(None);
            enter_borderless(window);
        }
        DisplayMode::Fullscreen => {
            leave_borderless(window);
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            // 全画面のあいだは枠を出さない（OS が外す）。戻るときのために元に戻しておく
            window.set_decorations(true);
            window.set_resizable(true);
        }
    }
}

/// 窓を [`WINDOW_SIZE`] にして、今のモニターの真ん中に
fn center(window: &Window) {
    let size: PhysicalSize<u32> = WINDOW_SIZE.to_physical(window.scale_factor());
    let _ = window.request_inner_size(size);
    if let Some(m) = window.current_monitor() {
        let (mp, ms) = (m.position(), m.size());
        let x = mp.x + (ms.width.saturating_sub(size.width) / 2) as i32;
        let y = mp.y + (ms.height.saturating_sub(size.height) / 2) as i32;
        window.set_outer_position(PhysicalPosition::new(x, y));
    }
}

#[cfg(windows)]
fn enter_borderless(window: &Window) {
    let Some(monitor) = window.current_monitor() else {
        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        return;
    };
    let rect = |m: &winit::monitor::MonitorHandle| {
        let (p, s) = (m.position(), m.size());
        Rect {
            x: p.x,
            y: p.y,
            w: s.width as i32,
            h: s.height as i32,
        }
    };
    let all: Vec<Rect> = window.available_monitors().map(|m| rect(&m)).collect();
    let r = borderless_rect(rect(&monitor), &all);
    window.set_maximized(false);
    window.set_decorations(false);
    window.set_resizable(false);
    window.set_outer_position(PhysicalPosition::new(r.x, r.y));
    let _ = window.request_inner_size(PhysicalSize::new(r.w as u32, r.h as u32));
}

#[cfg(windows)]
fn leave_borderless(_window: &Window) {}

#[cfg(target_os = "macos")]
fn enter_borderless(window: &Window) {
    use winit::platform::macos::WindowExtMacOS;
    // 全画面（別の操作スペース）から抜けきる前だと断られる
    if !window.set_simple_fullscreen(true) {
        log::warn!("simple fullscreen was refused");
    }
}

#[cfg(target_os = "macos")]
fn leave_borderless(window: &Window) {
    use winit::platform::macos::WindowExtMacOS;
    window.set_simple_fullscreen(false);
}

#[cfg(not(any(windows, target_os = "macos")))]
fn enter_borderless(window: &Window) {
    window.set_fullscreen(Some(Fullscreen::Borderless(None)));
}

#[cfg(not(any(windows, target_os = "macos")))]
fn leave_borderless(_window: &Window) {}

#[cfg(test)]
mod tests {
    use super::*;

    const FHD: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    #[test]
    fn one_monitor_bleeds_below() {
        let r = borderless_rect(FHD, &[FHD]);
        assert_eq!(r, Rect { h: 1081, ..FHD });
    }

    #[test]
    fn avoids_the_neighbour_monitors() {
        // 下にもう 1 枚あれば上へ
        let below = Rect { y: 1080, ..FHD };
        let r = borderless_rect(FHD, &[FHD, below]);
        assert_eq!(
            r,
            Rect {
                y: -1,
                h: 1081,
                ..FHD
            }
        );
        // 上下にあれば右へ
        let above = Rect { y: -1080, ..FHD };
        let r = borderless_rect(FHD, &[above, FHD, below]);
        assert_eq!(r, Rect { w: 1921, ..FHD });
        // 右の 4K（下にずれていても辺が触れていれば隣）
        let right = Rect {
            x: 1920,
            y: -500,
            w: 3840,
            h: 2160,
        };
        let r = borderless_rect(FHD, &[above, FHD, below, right]);
        assert_eq!(
            r,
            Rect {
                x: -1,
                w: 1921,
                ..FHD
            }
        );
    }

    #[test]
    fn neighbours_that_only_touch_a_corner_do_not_count() {
        // 右下の角だけで触れているモニターは、下の辺の隣ではない
        let corner = Rect {
            x: 1920,
            y: 1080,
            ..FHD
        };
        assert_eq!(
            borderless_rect(FHD, &[FHD, corner]),
            Rect { h: 1081, ..FHD }
        );
    }

    #[test]
    fn surrounded_monitor_still_bleeds_below() {
        let around = [
            Rect { y: 1080, ..FHD },
            Rect { y: -1080, ..FHD },
            Rect { x: 1920, ..FHD },
            Rect { x: -1920, ..FHD },
        ];
        let mut all = vec![FHD];
        all.extend(around);
        assert_eq!(borderless_rect(FHD, &all), Rect { h: 1081, ..FHD });
    }
}
