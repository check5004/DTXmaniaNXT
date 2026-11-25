//! ドロップで曲を追加するときに画面へ重ねるもの（見本の案 A、2026-11-25）。
//! - ファイルをウィンドウに重ねているあいだの案内（暗くした画面に点線の枠、追加先）
//! - 写しているあいだの、右下の小さな札（回る輪と %）

use std::path::{Path, PathBuf};

use egui::{Color32, Pos2, Stroke};
use i18n::{t, tf};
use library::import::{self, ImportError};

use crate::gfx::{Canvas, Gfx};
use crate::settings_ui as ui;
use crate::song_adder::{self, Working};
use crate::update_view::{self, ACCENT, Corner};

const SUB: Color32 = Color32::from_rgb(201, 195, 220);
const HINT: Color32 = Color32::from_rgb(170, 166, 188);

/// 重ねているファイルが何か
#[derive(Debug, Clone, PartialEq, Eq)]
enum Hovering {
    /// 譜面だけ（ドロップするとすぐ遊ぶ）
    Charts,
    /// 曲を追加する。読めない種類が混じっていれば、そのわけ
    Songs { warning: Option<String> },
}

fn hovering(files: &[PathBuf]) -> Hovering {
    if !files.is_empty() && files.iter().all(|f| crate::ui::is_chart_file(f)) {
        return Hovering::Charts;
    }
    let warning = files.iter().find_map(|f| match import::source_of(f) {
        Err(e @ ImportError::Unsupported(_)) if !crate::ui::is_chart_file(f) => {
            Some(song_adder::error_message(f, &e))
        }
        _ => None,
    });
    Hovering::Songs { warning }
}

/// 重ねているファイルの名前（いくつもあれば「ほか n 件」）
fn files_label(files: &[PathBuf]) -> String {
    let first = files
        .first()
        .map(|f| song_adder::folder_name(f))
        .unwrap_or_default();
    match files.len() {
        0 | 1 => first,
        n => tf!("{first} and {} more", "{first} ほか {} 件", n - 1),
    }
}

/// ファイルをウィンドウに重ねているあいだの案内。`dest` は追加先
pub fn hover(c: &Canvas, g: &mut Gfx, files: &[PathBuf], dest: &Path) {
    c.cover(Color32::from_rgba_unmultiplied(3, 3, 9, 200));
    dashed_round(c, 90.0, 70.0, 1100.0, 580.0, 26.0);
    tray(c, 640.0, 250.0, 56.0);
    let kind = hovering(files);
    let title = match kind {
        Hovering::Charts => t("Drop to Play", "ドロップして遊ぶ"),
        Hovering::Songs { .. } => t("Drop to Add Songs", "ドロップして曲を追加"),
    };
    let w = 1000.0;
    ui::text_center(
        c,
        g,
        title,
        640.0,
        330.0,
        &ui::bold(46.0, Color32::WHITE),
        w,
    );
    ui::text_center(
        c,
        g,
        &files_label(files),
        640.0,
        405.0,
        &ui::plain(24.0, SUB),
        w,
    );
    if let Hovering::Songs { warning } = kind {
        let to = tf!("Add to  {}", "追加先  {}", song_adder::folder_name(dest));
        ui::text_center(c, g, &to, 640.0, 462.0, &ui::bold(22.0, ACCENT), w);
        let (hint, color) = match &warning {
            Some(w) => (w.as_str(), ui::DANGER),
            None => (
                t(
                    "Zip files or song folders (as many as you like)",
                    "zip ファイル・曲のフォルダ（いくつでも）",
                ),
                HINT,
            ),
        };
        ui::text_center(c, g, hint, 640.0, 506.0, &ui::plain(18.0, color), w);
    }
}

/// 角の丸い点線の枠
fn dashed_round(c: &Canvas, x: f32, y: f32, w: f32, h: f32, r: f32) {
    let corner = |cx: f32, cy: f32, from: f32| {
        (0..=8).map(move |i| {
            let a = (from + i as f32 * 90.0 / 8.0).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
    };
    let pts: Vec<Pos2> = corner(x + w - r, y + r, 270.0)
        .chain(corner(x + w - r, y + h - r, 0.0))
        .chain(corner(x + r, y + h - r, 90.0))
        .chain(corner(x + r, y + r, 180.0))
        .chain([(x + w - r, y)])
        .map(|(px, py)| c.pos(px, py))
        .collect();
    let s = c.scale();
    let shapes = egui::Shape::dashed_line(&pts, Stroke::new(4.0 * s, ACCENT), 18.0 * s, 12.0 * s);
    c.painter().extend(shapes);
}

/// 下向きの矢印と受け皿
fn tray(c: &Canvas, cx: f32, cy: f32, size: f32) {
    let s = c.scale();
    let stroke = Stroke::new(6.0 * s, ACCENT);
    let line = |pts: &[(f32, f32)]| {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| c.pos(x, y)).collect();
        c.painter().add(egui::Shape::line(pts, stroke));
    };
    line(&[(cx, cy - size), (cx, cy + size * 0.35)]);
    line(&[
        (cx - size * 0.45, cy - size * 0.1),
        (cx, cy + size * 0.38),
        (cx + size * 0.45, cy - size * 0.1),
    ]);
    line(&[
        (cx - size * 0.9, cy + size * 0.25),
        (cx - size * 0.9, cy + size * 0.8),
        (cx + size * 0.9, cy + size * 0.8),
        (cx + size * 0.9, cy + size * 0.25),
    ]);
}

/// 写しているあいだの札（右下。ほかに待っていれば「あと n 件」）
pub fn adding_chip(c: &Canvas, g: &mut Gfx, w: &Working, corner: Corner) {
    let percent = (w.fraction() * 100.0).floor() as u32;
    let mut text = tf!(
        "Adding songs: {}  {percent}%",
        "曲を追加しています　{}　{percent}%",
        w.name
    );
    if w.waiting > 0 {
        text += &tf!("  (+{} more)", "（あと {} 件）", w.waiting);
    }
    update_view::chip(c, g, &text, Some(w.fraction()), corner, w.since);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hovering_charts_plays_and_archives_warn() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        assert_eq!(
            hovering(&[PathBuf::from("/a/master.dtx")]),
            Hovering::Charts
        );
        assert_eq!(
            hovering(&[PathBuf::from("/a/song.zip")]),
            Hovering::Songs { warning: None }
        );
        assert_eq!(
            hovering(&[PathBuf::from("/a/song.zip"), PathBuf::from("/a/b.7z")]),
            Hovering::Songs {
                warning: Some("7z は読めません。展開してからフォルダをドロップしてください".into())
            }
        );
        assert_eq!(
            files_label(&[PathBuf::from("/a/x.zip"), PathBuf::from("/a/y.zip")]),
            "x.zip ほか 1 件"
        );
    }
}
