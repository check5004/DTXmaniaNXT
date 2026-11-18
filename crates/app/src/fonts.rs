//! 日本語の字形。egui の標準のフォントには日本語が無いので、OS のフォントを探して足す。
//! フォントのファイルはアプリに同梱しない（ライセンスを持ち込まないため）。

use std::sync::Arc;

const CANDIDATES: &[&str] = &[
    // Windows
    "C:\\Windows\\Fonts\\YuGothM.ttc",
    "C:\\Windows\\Fonts\\meiryo.ttc",
    "C:\\Windows\\Fonts\\msgothic.ttc",
    // macOS
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    // Linux
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
];

pub fn install_japanese(ctx: &egui::Context) {
    let Some((path, bytes)) = CANDIDATES
        .iter()
        .find_map(|p| std::fs::read(p).ok().map(|b| (*p, b)))
    else {
        log::warn!("no Japanese font found; Japanese text will not show");
        return;
    };
    log::info!("font: {path}");
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("ja".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("ja".to_owned());
    }
    ctx.set_fonts(fonts);
}
