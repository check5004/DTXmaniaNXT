//! スネアのリムの絵。NX のスキン（7_chips_drums.png のスネアのチップ、7_pads.png のスネアのパッド）から、
//! 起動してから作る（スキンの画像は変えない）。
//!
//! - リムの音符のチップ（設定の「リムの音符の出し方」で選ぶ 3 通り）
//!   - 両はしに印: いつもの黄色のチップの両はしに、縦の短い棒（ドラムを横から見たときのリムの形）
//!   - 縁だけ: 中を抜いた黄色の枠
//!   - 白: チップの色を抜いて白く
//! - リムで叩いたときに光らせる、パッドの黄色い輪

use image::{Rgba, RgbaImage};

/// 7_chips_drums.png のスネアのチップの切り出し（左・幅・高さ。上流 `tUpdateAndDraw_Chip_Drums`）
pub const SD_CHIP_SX: u32 = 126;
pub const CHIP_W: u32 = 64;
pub const CHIP_H: u32 = 64;

/// 7_pads.png のスネアのパッドの左上と大きさ
const SD_PAD: (u32, u32) = (0, 96);
pub const PAD_SIZE: u32 = 96;

/// リムの音符の出し方（設定の並びと同じ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RimLook {
    /// 両はしに印
    Ears,
    /// 縁だけ
    Hollow,
    /// 白
    White,
    /// 出さない（普通の音符と同じ）
    Off,
}

impl RimLook {
    /// [`rim_chips`] の絵の中の左（出さないなら `None`）
    pub fn sx(self) -> Option<f32> {
        let i = match self {
            Self::Ears => 0,
            Self::Hollow => 1,
            Self::White => 2,
            Self::Off => return None,
        };
        Some((i * CHIP_W) as f32)
    }
}

/// チップの絵のうち、見えているドット（不透明さがこれより上）
const SOLID: u8 = 60;

/// スネアのチップの見えている所の枠（左・上・右・下、右と下は含まない）
fn chip_bounds(chip: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for (x, y, p) in chip.enumerate_pixels() {
        if p[3] > SOLID {
            b = Some(match b {
                None => (x, y, x + 1, y + 1),
                Some((l, t, r, bt)) => (l.min(x), t.min(y), r.max(x + 1), bt.max(y + 1)),
            });
        }
    }
    b
}

/// 7_chips_drums.png から、リムの音符のチップを 3 通り横に並べた絵（64 × 3 ドット × 64 ドット）。
/// 並びは [`RimLook::sx`]
pub fn rim_chips(src: RgbaImage) -> RgbaImage {
    let chip = crop(&src, SD_CHIP_SX, 0, CHIP_W, CHIP_H);
    let mut out = RgbaImage::new(CHIP_W * 3, CHIP_H);
    paste(&mut out, &ears(&chip), 0);
    paste(&mut out, &hollow(&chip), CHIP_W);
    paste(&mut out, &white(&chip), CHIP_W * 2);
    out
}

/// 両はしに印: 元のチップの両はしに、上下に 11 ドットずつ伸びる幅 4 ドットの棒を足し、まわりをぼかして光らせる
fn ears(chip: &RgbaImage) -> RgbaImage {
    let Some((l, t, r, b)) = chip_bounds(chip) else {
        return chip.clone();
    };
    let cy = (t + b) as i32 / 2;
    let mut bars = RgbaImage::new(chip.width(), chip.height());
    for x0 in [l as i32 - 1, r as i32 - 3] {
        fill(&mut bars, x0, cy - 11, 4, 23, Rgba([255, 236, 150, 255]));
    }
    let mut out = glow(&bars, 2, [255, 214, 80]);
    over(&mut out, chip);
    over(&mut out, &bars);
    out
}

/// 縁だけ: 元のチップの枠に沿った太さ 3 ドットの黄色の線（中は抜く）。内側に明るい細い線、外にぼかした光
fn hollow(chip: &RgbaImage) -> RgbaImage {
    let Some((l, t, r, b)) = chip_bounds(chip) else {
        return chip.clone();
    };
    let (l, t, r, b) = (l as i32, t as i32 - 1, r as i32, b as i32 + 1);
    let mut frame = RgbaImage::new(chip.width(), chip.height());
    let c = Rgba([255, 226, 90, 255]);
    fill(&mut frame, l, t, r - l, 3, c);
    fill(&mut frame, l, b - 3, r - l, 3, c);
    fill(&mut frame, l, t, 3, b - t, c);
    fill(&mut frame, r - 3, t, 3, b - t, c);
    let mut out = glow(&frame, 2, [255, 200, 40]);
    over(&mut out, &frame);
    let hi = Rgba([255, 250, 210, 255]);
    fill(&mut out, l + 1, t + 1, r - l - 2, 1, hi);
    fill(&mut out, l + 1, b - 2, r - l - 2, 1, hi);
    out
}

/// 白: 元のチップの色を抜き（彩度を 0.08 倍）、少し明るくする
fn white(chip: &RgbaImage) -> RgbaImage {
    let mut out = chip.clone();
    for p in out.pixels_mut() {
        if p[3] == 0 {
            continue;
        }
        let [r, g, b, a] = p.0;
        let max = r.max(g).max(b) as f32;
        let mix = |c: u8| {
            let c = c as f32;
            ((max - (max - c) * 0.08) * 1.15).min(255.0) as u8
        };
        *p = Rgba([mix(r), mix(g), mix(b), a]);
    }
    out
}

/// 7_pads.png から、スネアのパッドの黄色い輪だけを光らせた絵（96 × 96 ドット）。リムで叩いたとき、パッドの上に足して描く
pub fn rim_ring(src: RgbaImage) -> RgbaImage {
    let pad = crop(&src, SD_PAD.0, SD_PAD.1, PAD_SIZE, PAD_SIZE);
    let mut ring = RgbaImage::new(PAD_SIZE, PAD_SIZE);
    for (x, y, p) in pad.enumerate_pixels() {
        let [r, g, b, a] = p.0;
        if a > 100 && r > 180 && g > 140 && b < 90 {
            ring.put_pixel(x, y, Rgba([255, 235, 90, 255]));
        }
    }
    let wide = dilate(&ring, 2);
    let mut out = glow(&wide, 4, [255, 190, 0]);
    // ぼかしは薄いので、2 回重ねて強くする
    let again = out.clone();
    over(&mut out, &again);
    over(&mut out, &ring);
    out
}

fn crop(src: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |i, j| {
        src.get_pixel_checked(x + i, y + j)
            .copied()
            .unwrap_or(Rgba([0, 0, 0, 0]))
    })
}

fn paste(dst: &mut RgbaImage, src: &RgbaImage, x: u32) {
    for (i, j, p) in src.enumerate_pixels() {
        if let Some(d) = dst.get_pixel_mut_checked(x + i, j) {
            *d = *p;
        }
    }
}

/// 長方形を塗る（はみ出した所は描かない）
fn fill(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: Rgba<u8>) {
    for j in y.max(0)..(y + h).max(0) {
        for i in x.max(0)..(x + w).max(0) {
            if let Some(p) = img.get_pixel_mut_checked(i as u32, j as u32) {
                *p = c;
            }
        }
    }
}

/// `src` を `dst` の上に重ねる（ふつうの半透明の重ね方）
fn over(dst: &mut RgbaImage, src: &RgbaImage) {
    for (d, s) in dst.pixels_mut().zip(src.pixels()) {
        let sa = f32::from(s[3]) / 255.0;
        if sa <= 0.0 {
            continue;
        }
        let da = f32::from(d[3]) / 255.0;
        let oa = sa + da * (1.0 - sa);
        let mix = |sc: u8, dc: u8| {
            ((f32::from(sc) * sa + f32::from(dc) * da * (1.0 - sa)) / oa).round() as u8
        };
        *d = Rgba([
            mix(s[0], d[0]),
            mix(s[1], d[1]),
            mix(s[2], d[2]),
            (oa * 255.0).round() as u8,
        ]);
    }
}

/// 見えている所を `r` ドット太らせる
fn dilate(img: &RgbaImage, r: i32) -> RgbaImage {
    let (w, h) = (img.width() as i32, img.height() as i32);
    RgbaImage::from_fn(img.width(), img.height(), |x, y| {
        let (x, y) = (x as i32, y as i32);
        let mut best = Rgba([0, 0, 0, 0]);
        for j in (y - r).max(0)..=(y + r).min(h - 1) {
            for i in (x - r).max(0)..=(x + r).min(w - 1) {
                let p = *img.get_pixel(i as u32, j as u32);
                if p[3] > best[3] {
                    best = p;
                }
            }
        }
        best
    })
}

/// 見えている所の形をぼかした、色 `color` の光（半径 `r` の箱ぼかしを縦と横に 2 回ずつ）
fn glow(img: &RgbaImage, r: i32, color: [u8; 3]) -> RgbaImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut a: Vec<f32> = img.pixels().map(|p| f32::from(p[3])).collect();
    for _ in 0..2 {
        a = box_blur(&a, w, h, r, true);
        a = box_blur(&a, w, h, r, false);
    }
    RgbaImage::from_fn(w as u32, h as u32, |x, y| {
        let v = a[y as usize * w + x as usize];
        Rgba([color[0], color[1], color[2], (v * 1.6).min(255.0) as u8])
    })
}

fn box_blur(a: &[f32], w: usize, h: usize, r: i32, horizontal: bool) -> Vec<f32> {
    let n = (2 * r + 1) as f32;
    let mut out = vec![0.0; a.len()];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for k in -r..=r {
                let (i, j) = if horizontal {
                    (x as i32 + k, y as i32)
                } else {
                    (x as i32, y as i32 + k)
                };
                if i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h {
                    sum += a[j as usize * w + i as usize];
                }
            }
            out[y * w + x] = sum / n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 7_chips_drums.png のスネアの所に、横 54 × 縦 12 ドットの黄色いチップがあるとした絵
    fn sheet() -> RgbaImage {
        let mut img = RgbaImage::new(718, 776);
        fill(
            &mut img,
            SD_CHIP_SX as i32 + 5,
            26,
            54,
            12,
            Rgba([240, 200, 60, 255]),
        );
        img
    }

    #[test]
    fn three_rim_chips_side_by_side() {
        let out = rim_chips(sheet());
        assert_eq!(out.dimensions(), (CHIP_W * 3, CHIP_H));
        // 両はしに印: チップより上の、左の棒の所が見える
        assert!(out.get_pixel(5, 22)[3] > 200);
        assert_eq!(out.get_pixel(30, 22)[3], 0);
        // 縁だけ: 枠は見え、真ん中は抜ける（枠の光がにじむだけ）
        assert!(out.get_pixel(CHIP_W + 30, 26)[3] > 200);
        assert!(out.get_pixel(CHIP_W + 30, 32)[3] < 100);
        // 白: 色が抜けて白っぽい
        let p = out.get_pixel(CHIP_W * 2 + 30, 32);
        assert!(p[2] > 220 && p[0] > 220, "{p:?}");
        assert_eq!(RimLook::Off.sx(), None);
        assert_eq!(RimLook::White.sx(), Some(128.0));
    }

    #[test]
    fn ring_glows_only_around_the_yellow_ring() {
        let mut pads = RgbaImage::new(384, 288);
        // パッドの白い面と、下の黄色い輪
        fill(&mut pads, 10, 96 + 30, 70, 30, Rgba([230, 230, 225, 255]));
        fill(&mut pads, 10, 96 + 60, 70, 4, Rgba([240, 200, 20, 255]));
        let ring = rim_ring(pads);
        assert_eq!(ring.dimensions(), (PAD_SIZE, PAD_SIZE));
        assert!(ring.get_pixel(40, 61)[3] > 200);
        // 面の真ん中は光らせない
        assert_eq!(ring.get_pixel(40, 40)[3], 0);
    }
}
