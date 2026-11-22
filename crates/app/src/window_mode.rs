//! 窓の出し方（設定の「画面の出し方」）を窓に当てる。
//!
//! - 全画面は winit の `Fullscreen::Borderless`（解像度は変えない）。OS は全画面の窓として扱い、
//!   ドライバー（NVIDIA の Vulkan など）は DWM を通さず直に出すので、ほかの窓へ切り替えると一瞬暗くなることがある。
//! - ボーダーレスは、枠のない普通の窓をモニターいっぱいに広げる。モニターとぴったり同じ大きさだと
//!   ドライバーと OS が全画面と見なすので、隣にモニターの無い辺へ 1 ドットはみ出させる
//!   （SDL や GLFW で知られたやり方）。macOS は簡易フルスクリーン（別の操作スペースを作らない）。
//!   ほかの OS は全画面と同じ（Wayland では窓の位置を決められない）。
//! - 全画面とボーダーレスは、設定の「出すモニター」で選んだモニターに出す（選んでいなければ、
//!   見つからなければ窓のあるモニター）。ウィンドウのときは、起動したときにそのモニターの真ん中に開く。

use serde::{Deserialize, Serialize};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window};

use crate::settings::DisplayMode;

/// ボーダーレスの窓をモニターからはみ出させる幅（ドット）
#[cfg_attr(not(windows), allow(dead_code))]
const BLEED: i32 = 1;

/// ウィンドウに戻すとき、前の位置と大きさが分からなければこの大きさで画面の真ん中に
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 720.0);

/// 画面上の四角（物理ドット）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// はみ出させるのは Windows だけ
    #[cfg_attr(not(windows), allow(dead_code))]
    fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    fn of(m: &MonitorHandle) -> Self {
        let (p, s) = (m.position(), m.size());
        Rect {
            x: p.x,
            y: p.y,
            w: s.width as i32,
            h: s.height as i32,
        }
    }
}

/// 全画面とボーダーレスを出すモニター（config.toml の `[monitor]`）。OS の名前と、選んだときの左上の位置で覚える。
/// 探すときは名前と位置 → 名前 → 位置の順（Windows の名前は `\\.\DISPLAY2` のような画面の番号。
/// 同じ機種を 2 枚つなぐと名前が同じになる OS もあるので、位置も見る）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorPick {
    pub name: String,
    pub x: i32,
    pub y: i32,
}

/// つながっているモニター 1 枚（設定の画面の選択肢と、出す先を探すのに使う）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// OS の名前（Windows は `\\.\DISPLAY2` など）
    pub name: String,
    pub rect: Rect,
    /// リフレッシュレート（Hz に丸めたもの）
    pub hz: Option<u32>,
    /// メインのモニター
    pub primary: bool,
}

impl MonitorInfo {
    fn of(m: &MonitorHandle, primary: Option<&MonitorHandle>) -> Self {
        Self {
            name: m.name().unwrap_or_default(),
            rect: Rect::of(m),
            hz: m
                .refresh_rate_millihertz()
                .map(|mhz| (mhz + 500) / 1000)
                .filter(|&hz| hz > 0),
            primary: primary == Some(m),
        }
    }

    /// 設定に残す形
    pub fn pick(&self) -> MonitorPick {
        MonitorPick {
            name: self.name.clone(),
            x: self.rect.x,
            y: self.rect.y,
        }
    }

    /// 選択肢の名前（「2：1920×1080 60Hz（メインの右）」）。頭は Windows なら画面の番号（名前の DISPLAY のあとの数）、
    /// ほかの OS はモニターの名前、名前が無ければ並びの何番目か（`index` は 0 から）
    pub fn label(&self, index: usize, all: &[MonitorInfo]) -> String {
        let id = match display_number(&self.name) {
            Some(n) => n.to_string(),
            None if self.name.is_empty() => (index + 1).to_string(),
            None => self.name.clone(),
        };
        let mut s = format!("{id}：{}×{}", self.rect.w, self.rect.h);
        if let Some(hz) = self.hz {
            s.push_str(&format!(" {hz}Hz"));
        }
        if let Some(place) = self.place(all) {
            s.push_str(&format!("（{place}）"));
        }
        s
    }

    /// メインのモニターから見た向き（メインが分からなければ `None`）
    fn place(&self, all: &[MonitorInfo]) -> Option<&'static str> {
        if self.primary {
            return Some("メイン");
        }
        let main = all.iter().find(|m| m.primary)?;
        let ((x, y), (mx, my)) = (self.rect.center(), main.rect.center());
        let (dx, dy) = (x - mx, y - my);
        Some(if dx.abs() >= dy.abs() {
            if dx >= 0 {
                "メインの右"
            } else {
                "メインの左"
            }
        } else if dy >= 0 {
            "メインの下"
        } else {
            "メインの上"
        })
    }
}

/// Windows のモニターの名前（`\\.\DISPLAY2`）の番号
fn display_number(name: &str) -> Option<u32> {
    name.rsplit_once("DISPLAY")?.1.parse().ok()
}

/// つながっているモニター（左から、同じなら上から）
pub fn monitors(window: &Window) -> Vec<MonitorInfo> {
    let primary = window.primary_monitor();
    let mut all: Vec<MonitorInfo> = window
        .available_monitors()
        .map(|m| MonitorInfo::of(&m, primary.as_ref()))
        .collect();
    all.sort_by_key(|m| (m.rect.x, m.rect.y));
    all
}

/// `pick` に当たるモニターが `all` の何番目か（名前と位置 → 名前 → 位置）
pub fn find(pick: &MonitorPick, all: &[MonitorInfo]) -> Option<usize> {
    let at = |m: &MonitorInfo| m.rect.x == pick.x && m.rect.y == pick.y;
    let named = |m: &MonitorInfo| !pick.name.is_empty() && m.name == pick.name;
    all.iter()
        .position(|m| named(m) && at(m))
        .or_else(|| all.iter().position(named))
        .or_else(|| all.iter().position(at))
}

/// 出す先のモニター。選んでいない、または見つからなければ `None`（窓のあるモニター）。
/// 窓を開く前は `ActiveEventLoop` の、開いたあとは窓のモニターの一覧を渡す
pub fn target_in(
    available: impl Iterator<Item = MonitorHandle>,
    primary: Option<MonitorHandle>,
    pick: Option<&MonitorPick>,
) -> Option<MonitorHandle> {
    let pick = pick?;
    let handles: Vec<MonitorHandle> = available.collect();
    let infos: Vec<MonitorInfo> = handles
        .iter()
        .map(|m| MonitorInfo::of(m, primary.as_ref()))
        .collect();
    let found = find(pick, &infos);
    if found.is_none() {
        log::info!("monitor {:?} is not connected", pick.name);
    }
    found.map(|i| handles[i].clone())
}

/// [`target_in`] の、開いた窓から
pub fn target(window: &Window, pick: Option<&MonitorPick>) -> Option<MonitorHandle> {
    target_in(window.available_monitors(), window.primary_monitor(), pick)
}

/// [`WINDOW_SIZE`] の窓を `m` の真ん中に置くときの左上
pub fn centered(m: &MonitorHandle) -> PhysicalPosition<i32> {
    let size: PhysicalSize<u32> = WINDOW_SIZE.to_physical(m.scale_factor());
    center_of(m, size)
}

/// 大きさ `size` の窓を `m` の真ん中に置くときの左上
fn center_of(m: &MonitorHandle, size: PhysicalSize<u32>) -> PhysicalPosition<i32> {
    let (mp, ms) = (m.position(), m.size());
    PhysicalPosition::new(
        mp.x + (ms.width.saturating_sub(size.width) / 2) as i32,
        mp.y + (ms.height.saturating_sub(size.height) / 2) as i32,
    )
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
/// `saved` はウィンドウのときの位置と大きさで、ウィンドウから離れるときに残し、ウィンドウに戻るときに使う。
/// 全画面とボーダーレスは `target` に出す（`None` なら窓のあるモニター）
pub fn apply(
    window: &Window,
    from: Option<DisplayMode>,
    to: DisplayMode,
    saved: &mut Option<Placement>,
    target: Option<&MonitorHandle>,
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
            enter_borderless(window, target);
        }
        DisplayMode::Fullscreen => {
            leave_borderless(window);
            window.set_fullscreen(Some(Fullscreen::Borderless(target.cloned())));
            // 全画面のあいだは枠を出さない（OS が外す）。戻るときのために元に戻しておく
            window.set_decorations(true);
            window.set_resizable(true);
        }
    }
}

/// 出し方はそのままで、出すモニターが変わった（DPI が変わって合わせ直すときも）。
/// 全画面とボーダーレスは `target` へ移し、ウィンドウは大きさを保って `target` の真ん中へ。
/// `target` が `None`（窓のあるモニター）なら、ボーダーレスは今のモニターに合わせ直し、ほかは動かさない
pub fn move_to(window: &Window, mode: DisplayMode, target: Option<&MonitorHandle>) {
    match mode {
        DisplayMode::Window => {
            if let Some(m) = target {
                let maximized = window.is_maximized();
                if maximized {
                    window.set_maximized(false);
                }
                window.set_outer_position(center_of(m, window.outer_size()));
                if maximized {
                    window.set_maximized(true);
                }
            }
        }
        DisplayMode::Borderless => enter_borderless(window, target),
        DisplayMode::Fullscreen => {
            if target.is_some() {
                window.set_fullscreen(Some(Fullscreen::Borderless(target.cloned())));
            }
        }
    }
}

/// 窓を [`WINDOW_SIZE`] にして、今のモニターの真ん中に
fn center(window: &Window) {
    let size: PhysicalSize<u32> = WINDOW_SIZE.to_physical(window.scale_factor());
    let _ = window.request_inner_size(size);
    if let Some(m) = window.current_monitor() {
        window.set_outer_position(center_of(&m, size));
    }
}

#[cfg(windows)]
fn enter_borderless(window: &Window, target: Option<&MonitorHandle>) {
    let Some(monitor) = target.cloned().or_else(|| window.current_monitor()) else {
        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        return;
    };
    let all: Vec<Rect> = window.available_monitors().map(|m| Rect::of(&m)).collect();
    let r = borderless_rect(Rect::of(&monitor), &all);
    window.set_maximized(false);
    window.set_decorations(false);
    window.set_resizable(false);
    window.set_outer_position(PhysicalPosition::new(r.x, r.y));
    let _ = window.request_inner_size(PhysicalSize::new(r.w as u32, r.h as u32));
}

#[cfg(windows)]
fn leave_borderless(_window: &Window) {}

#[cfg(target_os = "macos")]
fn enter_borderless(window: &Window, target: Option<&MonitorHandle>) {
    use winit::platform::macos::WindowExtMacOS;
    // 簡易フルスクリーンは窓のある画面に広がるので、先に窓を選んだモニターへ移す
    if let Some(m) = target
        && window.current_monitor().as_ref() != Some(m)
    {
        window.set_simple_fullscreen(false);
        window.set_outer_position(m.position());
    }
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
fn enter_borderless(window: &Window, target: Option<&MonitorHandle>) {
    window.set_fullscreen(Some(Fullscreen::Borderless(target.cloned())));
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

    fn info(name: &str, x: i32, y: i32, w: i32, h: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            name: name.to_owned(),
            rect: Rect { x, y, w, h },
            hz: Some(60),
            primary,
        }
    }

    #[test]
    fn monitor_labels_show_number_size_and_place() {
        let all = [
            info(r"\\.\DISPLAY3", -1920, 180, 1920, 1080, false),
            MonitorInfo {
                hz: Some(165),
                ..info(r"\\.\DISPLAY1", 0, 0, 2560, 1440, true)
            },
            info(r"\\.\DISPLAY2", 2560, 0, 1920, 1080, false),
            info(r"\\.\DISPLAY4", 0, 1440, 2560, 1440, false),
        ];
        let labels: Vec<String> = all
            .iter()
            .enumerate()
            .map(|(i, m)| m.label(i, &all))
            .collect();
        assert_eq!(
            labels,
            [
                "3：1920×1080 60Hz（メインの左）",
                "1：2560×1440 165Hz（メイン）",
                "2：1920×1080 60Hz（メインの右）",
                "4：2560×1440 60Hz（メインの下）",
            ]
        );
        // Windows 以外は名前のまま。名前もメインも分からなければ番号だけ
        let mac = [info("DELL U2720Q", 0, 0, 3840, 2160, false)];
        assert_eq!(mac[0].label(0, &mac), "DELL U2720Q：3840×2160 60Hz");
        let bare = MonitorInfo {
            hz: None,
            ..info("", 0, 0, 1280, 720, false)
        };
        assert_eq!(bare.label(1, &[]), "2：1280×720");
    }

    #[test]
    fn picked_monitor_is_found_by_name_then_position() {
        let all = [
            info(r"\\.\DISPLAY1", 0, 0, 1920, 1080, true),
            info(r"\\.\DISPLAY2", 1920, 0, 1920, 1080, false),
        ];
        let pick = all[1].pick();
        assert_eq!(find(&pick, &all), Some(1));
        // 並べ替えて位置が変わっても、名前で
        let moved = [
            info(r"\\.\DISPLAY2", -1920, 0, 1920, 1080, false),
            info(r"\\.\DISPLAY1", 0, 0, 1920, 1080, true),
        ];
        assert_eq!(find(&pick, &moved), Some(0));
        // 同じ名前が 2 枚（同じ機種の macOS など）なら位置で見分ける
        let twins = [
            info("LG HDR 4K", 0, 0, 3840, 2160, true),
            info("LG HDR 4K", 3840, 0, 3840, 2160, false),
        ];
        assert_eq!(find(&twins[1].pick(), &twins), Some(1));
        // 名前が変わっても、同じ位置にあればそれ
        let renamed = [info(r"\\.\DISPLAY5", 1920, 0, 1920, 1080, false)];
        assert_eq!(find(&pick, &renamed), Some(0));
        // 外れていれば無し（窓のあるモニターに出す）
        assert_eq!(find(&pick, &all[..1]), None);
    }

    #[test]
    fn display_numbers_come_from_windows_names() {
        assert_eq!(display_number(r"\\.\DISPLAY12"), Some(12));
        assert_eq!(display_number("DELL U2720Q"), None);
        assert_eq!(display_number(""), None);
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
