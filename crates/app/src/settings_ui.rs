//! 設定の画面などで使う、コードで描く部品（板、選んでいる行の枠、分類の絵、パッドの案内、ドラムの配置図）。
//! 背景や上下の帯はスキンの画像を使い、行や板は絵を足さずにここで描く（作り直しの案 2、2026-11-22）。
//! 色は v2 の上の帯（紫から橙）と、選んでいる項目の黄から橙に合わせる。

use dtx::DrumLane;
use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, Pos2, Shape, Stroke, pos2};
use i18n::t;

use crate::gfx::{Canvas, Gfx};
use crate::text::Style;

/// 板の地（濃い紫を少し透かす）
pub const PANEL_BG: Color32 = Color32::from_rgba_premultiplied(8, 6, 15, 219);
/// 前に出す板（選択肢・確かめ）の地
pub const PICK_BG: Color32 = Color32::from_rgba_premultiplied(12, 9, 23, 247);
pub const PANEL_LINE: Color32 = Color32::from_rgba_premultiplied(33, 33, 33, 33);
pub const MAGENTA: Color32 = Color32::from_rgb(200, 31, 201);
pub const ORANGE: Color32 = Color32::from_rgb(255, 154, 31);
pub const YELLOW: Color32 = Color32::from_rgb(255, 225, 74);
pub const TEXT: Color32 = Color32::from_rgb(242, 239, 249);
pub const DIM: Color32 = Color32::from_rgb(189, 182, 210);
pub const MUTED: Color32 = Color32::from_rgb(150, 143, 172);
pub const STRONG: Color32 = Color32::from_rgb(125, 240, 181);
pub const DANGER: Color32 = Color32::from_rgb(255, 154, 138);

/// ふつうの文字
pub fn plain(px: f32, color: Color32) -> Style {
    Style::plain(px, color)
}

/// 太い文字
pub fn bold(px: f32, color: Color32) -> Style {
    Style {
        bold: true,
        ..Style::plain(px, color)
    }
}

/// 黄から橙のグラデーションの太い文字（選んでいる値）
pub fn glow(px: f32) -> Style {
    Style {
        bold: true,
        gradient: Some(Color32::from_rgb(255, 140, 0)),
        ..Style::plain(px, YELLOW)
    }
}

/// 縁取りした見出し（背景の絵の上に置く文字）
pub fn edged(px: f32) -> Style {
    Style {
        bold: true,
        ..Style::edged(px, Color32::WHITE, Color32::BLACK, 2.0)
    }
}

/// 文字を (x, y) を左上にして描き、幅を返す。`max_w` を超えるなら横に縮める
pub fn text(c: &Canvas, g: &mut Gfx, s: &str, x: f32, y: f32, style: &Style, max_w: f32) -> f32 {
    match g.texts.get(s, style) {
        Some(t) => {
            t.draw_fit(c, x, y, max_w);
            t.w.min(max_w)
        }
        None => 0.0,
    }
}

/// 右の端を `right` にそろえて描く
pub fn text_right(c: &Canvas, g: &mut Gfx, s: &str, right: f32, y: f32, style: &Style, max_w: f32) {
    if let Some(t) = g.texts.get(s, style) {
        let w = t.w.min(max_w);
        t.draw_fit(c, right - w, y, max_w);
    }
}

/// 真ん中を `cx` にそろえて描く
pub fn text_center(c: &Canvas, g: &mut Gfx, s: &str, cx: f32, y: f32, style: &Style, max_w: f32) {
    if let Some(t) = g.texts.get(s, style) {
        let w = t.w.min(max_w);
        t.draw_fit(c, cx - w / 2.0, y, max_w);
    }
}

/// 文字の幅（縮めない）
pub fn text_width(g: &mut Gfx, s: &str, style: &Style) -> f32 {
    g.texts.get(s, style).map_or(0.0, |t| t.w)
}

/// 幅 `max_w` で折り返して描き、描いた下の端を返す。`bottom` より下には描かない
#[allow(clippy::too_many_arguments)]
pub fn paragraph(
    c: &Canvas,
    g: &mut Gfx,
    s: &str,
    x: f32,
    y: f32,
    max_w: f32,
    line_h: f32,
    bottom: f32,
    style: &Style,
) -> f32 {
    let mut y = y;
    for line in g.texts.wrap(s, style.px, max_w) {
        if y + line_h > bottom {
            break;
        }
        if !line.is_empty()
            && let Some(t) = g.texts.get(&line, style)
        {
            t.draw_fit(c, x, y, max_w);
        }
        y += line_h;
    }
    y
}

/// 角の丸い四角を塗る
pub fn round_fill(c: &Canvas, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color32) {
    c.painter()
        .rect_filled(c.rect(x, y, w, h), r * c.scale(), color);
}

/// 角の丸い枠
#[allow(clippy::too_many_arguments)]
pub fn round_frame(c: &Canvas, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, color: Color32) {
    c.painter().rect_stroke(
        c.rect(x, y, w, h),
        r * c.scale(),
        Stroke::new(width * c.scale(), color),
        egui::StrokeKind::Inside,
    );
}

/// 左から右へのグラデーションで塗る
pub fn hgrad(c: &Canvas, x: f32, y: f32, w: f32, h: f32, left: Color32, right: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c.pos(x, y), left);
    mesh.colored_vertex(c.pos(x + w, y), right);
    mesh.colored_vertex(c.pos(x + w, y + h), right);
    mesh.colored_vertex(c.pos(x, y + h), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    c.painter().add(mesh);
}

/// 上から下へのグラデーションで塗る
pub fn vgrad(c: &Canvas, x: f32, y: f32, w: f32, h: f32, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c.pos(x, y), top);
    mesh.colored_vertex(c.pos(x + w, y), top);
    mesh.colored_vertex(c.pos(x + w, y + h), bottom);
    mesh.colored_vertex(c.pos(x, y + h), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    c.painter().add(mesh);
}

/// 板（上の縁に v2 の帯と同じ紫と橙の線）
pub fn panel(c: &Canvas, x: f32, y: f32, w: f32, h: f32) {
    panel_with(c, x, y, w, h, PANEL_BG);
}

/// 前に出す板（選択肢、確かめ、割り当ての窓）
pub fn pick_panel(c: &Canvas, x: f32, y: f32, w: f32, h: f32) {
    panel_with(c, x, y, w, h, PICK_BG);
}

fn panel_with(c: &Canvas, x: f32, y: f32, w: f32, h: f32, bg: Color32) {
    c.fill(x, y, w, h, bg);
    c.frame(x, y, w, h, PANEL_LINE);
    c.fill(x, y, w, 3.0, MAGENTA);
    c.fill(x, y + 3.0, w, 1.0, Color32::from_rgb(255, 138, 0));
}

/// 選んでいる行の枠（橙の縁と、橙から紫へ薄く塗った地）。`alpha` は薄くするとき（ほかの列を操作しているとき）
pub fn selection(c: &Canvas, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let a = |v: f32| (v * alpha * 255.0).round() as u8;
    // ぼんやり光らせる（外へ広がる薄い枠）
    for i in 1..=3 {
        let d = i as f32 * 1.5;
        round_frame(
            c,
            x - d,
            y - d,
            w + d * 2.0,
            h + d * 2.0,
            3.0 + d,
            1.5,
            Color32::from_rgba_unmultiplied(255, 140, 0, a(0.16 / i as f32)),
        );
    }
    hgrad(
        c,
        x,
        y,
        w,
        h,
        Color32::from_rgba_unmultiplied(255, 160, 30, a(0.30)),
        Color32::from_rgba_unmultiplied(200, 31, 201, a(0.16)),
    );
    round_frame(
        c,
        x,
        y,
        w,
        h,
        3.0,
        2.0,
        Color32::from_rgba_unmultiplied(255, 160, 30, a(1.0)),
    );
}

/// 三角（`dir` が -1 なら左向き、1 なら右向き、-2 なら上向き、2 なら下向き）。(cx, cy) が真ん中
pub fn triangle(c: &Canvas, cx: f32, cy: f32, size: f32, dir: i32, color: Color32) {
    let s = size / 2.0;
    let pts = match dir {
        -1 => [(cx - s, cy), (cx + s, cy - s), (cx + s, cy + s)],
        1 => [(cx + s, cy), (cx - s, cy - s), (cx - s, cy + s)],
        -2 => [(cx, cy - s), (cx + s, cy + s), (cx - s, cy + s)],
        _ => [(cx, cy + s), (cx - s, cy - s), (cx + s, cy - s)],
    };
    c.painter().add(Shape::convex_polygon(
        pts.iter().map(|&(x, y)| c.pos(x, y)).collect(),
        color,
        Stroke::NONE,
    ));
}

/// 右向きの山形（開くもの、パンくずの区切り）
pub fn chevron(c: &Canvas, cx: f32, cy: f32, size: f32, width: f32, color: Color32) {
    let s = size / 2.0;
    let pts = vec![
        c.pos(cx - s / 2.0, cy - s),
        c.pos(cx + s / 2.0, cy),
        c.pos(cx - s / 2.0, cy + s),
    ];
    c.painter().add(PathShape::line(
        pts,
        PathStroke::new(width * c.scale(), color),
    ));
}

/// するかしないかの印（左上ではなく真ん中の左端を `x`, `cy` で。`sel` は選んでいる行）
pub fn checkbox(c: &Canvas, x: f32, cy: f32, size: f32, on: bool, sel: bool) {
    let y = cy - size / 2.0;
    if on {
        round_fill(c, x, y, size, size, 3.0, ORANGE);
        let pts = vec![
            c.pos(x + size * 0.24, cy),
            c.pos(x + size * 0.43, cy + size * 0.22),
            c.pos(x + size * 0.78, cy - size * 0.24),
        ];
        c.painter().add(PathShape::line(
            pts,
            PathStroke::new(2.0 * c.scale(), Color32::from_rgb(30, 18, 6)),
        ));
    } else {
        let line = if sel {
            ORANGE
        } else {
            Color32::from_rgb(110, 106, 124)
        };
        round_frame(c, x, y, size, size, 3.0, 2.0, line);
    }
}

/// 点
pub fn dot(c: &Canvas, cx: f32, cy: f32, r: f32, color: Color32) {
    c.painter()
        .circle_filled(c.pos(cx, cy), r * c.scale(), color);
}

/// 分類の絵
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Star,
    Stick,
    Lanes,
    Metronome,
    Speaker,
    Midi,
    Folder,
    Loop,
    Gear,
    /// 画面（モニター）
    Monitor,
    /// ゲームプレイ（旗）
    Flag,
}

/// 分類の絵を (x, y) を左上に、`size` の四角に描く（線で描く。24 の升目で考えた形）
pub fn icon(c: &Canvas, kind: Icon, x: f32, y: f32, size: f32, color: Color32) {
    let k = size / 24.0;
    let p = |px: f32, py: f32| c.pos(x + px * k, y + py * k);
    let w = 1.8 * k * c.scale();
    let stroke = Stroke::new(w, color);
    let painter = c.painter();
    let poly = |pts: &[(f32, f32)]| {
        painter.add(PathShape::closed_line(
            pts.iter().map(|&(a, b)| p(a, b)).collect(),
            PathStroke::new(w, color),
        ));
    };
    let line = |a: (f32, f32), b: (f32, f32), width: f32| {
        painter.line_segment([p(a.0, a.1), p(b.0, b.1)], Stroke::new(width, color));
    };
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| {
        let n = 10;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let t = (from + (to - from) * i as f32 / n as f32).to_radians();
                p(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect();
        painter.add(PathShape::line(pts, PathStroke::new(w, color)));
    };
    match kind {
        Icon::Star => poly(&[
            (12.0, 3.0),
            (14.6, 8.6),
            (20.6, 9.3),
            (16.1, 13.4),
            (17.3, 19.4),
            (12.0, 16.5),
            (6.7, 19.4),
            (7.9, 13.4),
            (3.4, 9.3),
            (9.4, 8.6),
        ]),
        Icon::Stick => {
            painter.circle_stroke(p(12.0, 14.0), 6.0 * k * c.scale(), stroke);
            line((5.0, 3.0), (11.0, 11.0), w);
            line((19.0, 3.0), (13.0, 11.0), w);
        }
        Icon::Lanes => {
            for lx in [5.0, 10.0, 15.0, 20.0] {
                line((lx, 3.0), (lx, 21.0), w);
            }
            let thick = 3.0 * k * c.scale();
            line((6.5, 9.0), (8.5, 9.0), thick);
            line((11.5, 14.0), (13.5, 14.0), thick);
            line((16.5, 6.0), (18.5, 6.0), thick);
        }
        Icon::Metronome => {
            poly(&[(8.0, 21.0), (16.0, 21.0), (13.5, 4.0), (10.5, 4.0)]);
            line((12.0, 16.0), (17.0, 7.0), w);
        }
        Icon::Speaker => {
            poly(&[
                (4.0, 9.0),
                (8.0, 9.0),
                (13.0, 5.0),
                (13.0, 19.0),
                (8.0, 15.0),
                (4.0, 15.0),
            ]);
            arc(13.5, 12.0, 3.4, -60.0, 60.0);
            arc(13.5, 12.0, 6.6, -55.0, 55.0);
        }
        Icon::Midi => {
            painter.circle_stroke(p(12.0, 12.0), 9.0 * k * c.scale(), stroke);
            for (dx, dy) in [
                (7.5, 11.0),
                (16.5, 11.0),
                (9.0, 15.0),
                (15.0, 15.0),
                (12.0, 16.5),
            ] {
                painter.circle_filled(p(dx, dy), 1.1 * k * c.scale(), color);
            }
            line((10.5, 4.5), (13.5, 4.5), w);
        }
        Icon::Folder => poly(&[
            (3.0, 6.0),
            (9.0, 6.0),
            (11.0, 8.0),
            (21.0, 8.0),
            (21.0, 19.0),
            (3.0, 19.0),
        ]),
        Icon::Loop => {
            arc(12.0, 12.0, 7.0, 150.0, 330.0);
            arc(12.0, 12.0, 7.0, -30.0, 150.0);
            painter.add(PathShape::line(
                vec![p(18.0, 4.5), p(18.6, 8.6), p(14.6, 8.9)],
                PathStroke::new(w, color),
            ));
            painter.add(PathShape::line(
                vec![p(6.0, 19.5), p(5.4, 15.4), p(9.4, 15.1)],
                PathStroke::new(w, color),
            ));
        }
        Icon::Monitor => {
            poly(&[(3.0, 4.5), (21.0, 4.5), (21.0, 16.5), (3.0, 16.5)]);
            line((12.0, 16.5), (12.0, 20.5), w);
            line((8.0, 20.5), (16.0, 20.5), w);
        }
        Icon::Flag => {
            line((5.0, 3.0), (5.0, 21.0), w);
            painter.add(PathShape::line(
                vec![
                    p(5.0, 4.0),
                    p(19.0, 4.0),
                    p(16.0, 8.0),
                    p(19.0, 12.0),
                    p(5.0, 12.0),
                ],
                PathStroke::new(w, color),
            ));
        }
        Icon::Gear => {
            painter.circle_stroke(p(12.0, 12.0), 3.0 * k * c.scale(), stroke);
            for (a, b) in [
                ((12.0, 2.0), (12.0, 5.0)),
                ((12.0, 19.0), (12.0, 22.0)),
                ((2.0, 12.0), (5.0, 12.0)),
                ((19.0, 12.0), (22.0, 12.0)),
                ((4.9, 4.9), (7.0, 7.0)),
                ((17.0, 17.0), (19.1, 19.1)),
                ((4.9, 19.1), (7.0, 17.0)),
                ((17.0, 7.0), (19.1, 4.9)),
            ] {
                line(a, b, w);
            }
        }
    }
}

/// メニューの操作に使うパッド
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pad {
    LC,
    HH,
    SD,
    HT,
    LT,
    FT,
    CY,
    RD,
    BD,
}

impl Pad {
    pub fn name(self) -> &'static str {
        match self {
            Pad::LC => "LC",
            Pad::HH => "HH",
            Pad::SD => "SD",
            Pad::HT => "HT",
            Pad::LT => "LT",
            Pad::FT => "FT",
            Pad::CY => "CY",
            Pad::RD => "RD",
            Pad::BD => "BD",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Pad::LC => Color32::from_rgb(232, 64, 159),
            Pad::HH => Color32::from_rgb(56, 166, 255),
            Pad::SD => Color32::from_rgb(255, 210, 58),
            Pad::HT => Color32::from_rgb(79, 212, 111),
            Pad::LT => Color32::from_rgb(255, 81, 81),
            Pad::FT => Color32::from_rgb(255, 156, 46),
            Pad::CY => Color32::from_rgb(64, 212, 255),
            Pad::RD => Color32::from_rgb(122, 140, 255),
            Pad::BD => Color32::from_rgb(160, 112, 255),
        }
    }
}

/// レーンの色（ドラムの配置図）
pub fn lane_color(lane: DrumLane) -> Color32 {
    match lane {
        DrumLane::LC => Pad::LC.color(),
        DrumLane::HH | DrumLane::HHO => Pad::HH.color(),
        DrumLane::SD => Pad::SD.color(),
        DrumLane::BD => Pad::BD.color(),
        DrumLane::LBD => crate::play_view::LEFT_BASS_COLOR,
        DrumLane::HT => Pad::HT.color(),
        DrumLane::LT => Pad::LT.color(),
        DrumLane::FT => Pad::FT.color(),
        DrumLane::CY => Pad::CY.color(),
        DrumLane::RD => Pad::RD.color(),
        DrumLane::LP => Color32::from_rgb(255, 122, 217),
    }
}

/// 画面の下の案内 1 つ（叩くパッドと、することの名前）
pub type Hint<'a> = (&'a [Pad], &'a str);

/// 画面の下の案内の帯の上端と高さ
pub const LEGEND_Y: f32 = 690.0;
pub const LEGEND_H: f32 = 30.0;

/// 画面の下に、パッドの役目を並べる（[`LEGEND_Y`] から [`LEGEND_H`] の高さ）。
/// パッドの無い案内（キーボードだけの操作）は、名前だけを出す
pub fn legend(c: &Canvas, g: &mut Gfx, hints: &[Hint<'_>]) {
    const Y: f32 = LEGEND_Y;
    hgrad(
        c,
        0.0,
        Y,
        1280.0,
        LEGEND_H,
        Color32::from_black_alpha(217),
        Color32::from_black_alpha(140),
    );
    let chip = bold(13.0, Color32::BLACK);
    let label = plain(15.0, TEXT);
    let mut x = 28.0;
    for (pads, what) in hints {
        for pad in pads.iter() {
            let w = text_width(g, pad.name(), &chip) + 12.0;
            round_fill(c, x, Y + 6.0, w, 18.0, 9.0, pad.color());
            text(c, g, pad.name(), x + 6.0, Y + 7.0, &chip, w);
            x += w + 2.0;
        }
        x += 4.0;
        x += text(c, g, what, x, Y + 5.0, &label, 1280.0 - x - 10.0);
        x += 18.0;
        if x > 1270.0 {
            break;
        }
    }
}

/// ドラムだけで操作するときの、パッドの役目の図（説明の列の下に出す）
pub fn pad_roles(c: &Canvas, g: &mut Gfx, x: f32, y: f32, w: f32, h: f32) {
    let pads: [(Pad, f32, f32, &str); 9] = [
        (Pad::LC, 0.08, 0.26, t("Back", "戻る")),
        (Pad::HH, 0.2, 0.52, ""),
        (Pad::HT, 0.4, 0.2, t("Up", "上")),
        (Pad::LT, 0.6, 0.2, t("Down", "下")),
        (Pad::CY, 0.88, 0.2, t("Select", "決定")),
        (Pad::RD, 0.9, 0.55, t("Select", "決定")),
        (Pad::SD, 0.32, 0.6, t("Left", "左")),
        (Pad::FT, 0.72, 0.62, t("Right", "右")),
        (Pad::BD, 0.52, 0.8, ""),
    ];
    for (pad, fx, fy, role) in pads {
        let r = if pad == Pad::BD { 26.0 } else { 22.0 };
        let cx = x + fx * w;
        let cy = y + 10.0 + fy * (h - 30.0);
        pad_circle(c, g, cx, cy, r, pad.color(), pad.name(), role, false);
    }
    text(
        c,
        g,
        t(
            "Pad controls for drums-only navigation",
            "ドラムだけで操作するときのパッドの役目",
        ),
        x + 12.0,
        y + h - 22.0,
        &plain(13.0, MUTED),
        w - 24.0,
    );
}

/// パッドの丸（名前と、下に小さく添える文字）
#[allow(clippy::too_many_arguments)]
pub fn pad_circle(
    c: &Canvas,
    g: &mut Gfx,
    cx: f32,
    cy: f32,
    r: f32,
    color: Color32,
    name: &str,
    sub: &str,
    lit: bool,
) {
    let painter = c.painter();
    let center = c.pos(cx, cy);
    let fill = if lit {
        color.gamma_multiply(0.55)
    } else {
        Color32::from_rgba_premultiplied(10, 8, 16, 230)
    };
    painter.circle_filled(center, r * c.scale(), fill);
    painter.circle_stroke(
        center,
        r * c.scale(),
        Stroke::new((if lit { 3.0 } else { 2.0 }) * c.scale(), color),
    );
    let name_style = bold(13.0, Color32::WHITE);
    let sub_style = plain(11.0, if lit { Color32::WHITE } else { DIM });
    if sub.is_empty() {
        text_center(c, g, name, cx, cy - 8.0, &name_style, r * 2.0 - 4.0);
    } else {
        text_center(c, g, name, cx, cy - 14.0, &name_style, r * 2.0 - 4.0);
        text_center(c, g, sub, cx, cy + 2.0, &sub_style, r * 2.0 + 10.0);
    }
}

/// レーンの配置図の位置（1280×720 の画面全体に出すときの真ん中。上流の割り当ての並びとドラムの形から）
pub fn lane_spot(lane: DrumLane) -> (f32, f32) {
    match lane {
        DrumLane::LC => (215.0, 330.0),
        DrumLane::HHO => (365.0, 330.0),
        DrumLane::HH => (365.0, 450.0),
        DrumLane::LP => (400.0, 590.0),
        DrumLane::SD => (500.0, 470.0),
        DrumLane::HT => (560.0, 330.0),
        DrumLane::BD => (700.0, 560.0),
        DrumLane::LBD => (555.0, 595.0),
        DrumLane::LT => (720.0, 330.0),
        DrumLane::FT => (800.0, 470.0),
        DrumLane::CY => (920.0, 300.0),
        DrumLane::RD => (1010.0, 430.0),
    }
}

/// レーンの配置図を (x, y, w, h) に縮めたときの、レーンの真ん中
pub fn lane_spot_in(lane: DrumLane, x: f32, y: f32, w: f32, h: f32) -> Pos2 {
    let (lx, ly) = lane_spot(lane);
    // 配置図の範囲（左 200〜右 1060、上 260〜下 620）
    pos2(x + (lx - 200.0) / 860.0 * w, y + (ly - 260.0) / 360.0 * h)
}
