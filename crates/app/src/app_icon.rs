//! アプリのアイコン（`assets/icon`）を窓に付ける。
//!
//! - Windows は exe に埋め込んだアイコン（`build.rs`）を、画面の拡大率に合った大きさで読む。
//!   窓の左上は小さいアイコン（16 の倍率）、タスクバーと Alt+Tab は大きいアイコン（32 の倍率）
//! - ほかの OS は同じ絵の PNG を渡す（macOS は窓のアイコンが無いので変わらない）
//! - 付けられなくても遊ぶのには困らないので、ログに残して続ける

use winit::window::Window;

/// exe に埋め込んだアイコンの番号（`build.rs` の `set_icon`）
#[cfg(windows)]
const RESOURCE_ID: u16 = 1;

#[cfg(windows)]
pub fn apply(window: &Window) {
    use winit::dpi::PhysicalSize;
    use winit::platform::windows::{IconExtWindows, WindowExtWindows};
    use winit::window::Icon;

    let scale = window.scale_factor();
    let load = |base: f64| {
        let side = (base * scale).round().max(1.0) as u32;
        Icon::from_resource(RESOURCE_ID, Some(PhysicalSize::new(side, side)))
    };
    match load(16.0) {
        Ok(icon) => window.set_window_icon(Some(icon)),
        Err(e) => log::warn!("窓のアイコンを読めません: {e}"),
    }
    match load(32.0) {
        Ok(icon) => window.set_taskbar_icon(Some(icon)),
        Err(e) => log::warn!("タスクバーのアイコンを読めません: {e}"),
    }
}

#[cfg(not(windows))]
pub fn apply(window: &Window) {
    match png_icon() {
        Ok(icon) => window.set_window_icon(Some(icon)),
        Err(e) => log::warn!("窓のアイコンを読めません: {e:#}"),
    }
}

/// アイコンの PNG（256×256）。.ico の一番大きい絵と同じ
#[cfg(any(not(windows), test))]
const PNG: &[u8] = include_bytes!("../../../assets/icon/dtxmanianxt.png");

#[cfg(not(windows))]
fn png_icon() -> anyhow::Result<winit::window::Icon> {
    let (rgba, width, height) = png_rgba()?;
    Ok(winit::window::Icon::from_rgba(rgba, width, height)?)
}

#[cfg(any(not(windows), test))]
fn png_rgba() -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(PNG, image::ImageFormat::Png)?.into_rgba8();
    let (width, height) = img.dimensions();
    Ok((img.into_raw(), width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_a_square_icon() {
        let (rgba, width, height) = png_rgba().unwrap();
        assert_eq!((width, height), (256, 256));
        assert_eq!(rgba.len(), 256 * 256 * 4);
    }

    /// .ico の中身: 小さい所から 256 まで、そろった大きさが入っていること（exe とインストーラーに入れる絵）
    #[test]
    fn ico_has_every_size() {
        let ico = include_bytes!("../../../assets/icon/dtxmanianxt.ico");
        assert_eq!(&ico[..4], &[0, 0, 1, 0], "ICO の見出し");
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        let sides: Vec<u32> = (0..count)
            .map(|i| match ico[6 + 16 * i] {
                0 => 256,
                w => u32::from(w),
            })
            .collect();
        for side in [16, 24, 32, 48, 256] {
            assert!(sides.contains(&side), "{side}×{side} が無い: {sides:?}");
        }
    }
}
