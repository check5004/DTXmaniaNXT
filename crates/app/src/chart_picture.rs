//! 譜面の絵。区間の音符を、行をレーン（楽譜と同じくシンバルが上、バスドラが下）、横を時間にして小さく描く。
//!
//! 小節の番号だけでは曲のどこのことか分からないので、小節を出す所（練習の入口、演奏中のミスした所、
//! 「もう一度」の板、練習の結果）にこの絵を添える。ミスした音には輪を付け（1 回は黄、2 回からは赤で「×回数」）、
//! 区間の音を流しているあいだは今鳴っている所に白い線を引く。

use std::collections::HashMap;

use dtx::{DrumLane, DrumNote, NotePos, TempoMap};
use egui::{Color32, Stroke, StrokeKind};
use engine::practice::Section;

use crate::gfx::{Canvas, Gfx};
use crate::settings_ui::lane_color;
use crate::text::Style;

/// 行の順（上から）。開いたハイハットは閉じたハイハットの行に描く
const ROWS: [DrumLane; 11] = [
    DrumLane::LC,
    DrumLane::CY,
    DrumLane::RD,
    DrumLane::HH,
    DrumLane::HT,
    DrumLane::SD,
    DrumLane::LT,
    DrumLane::FT,
    DrumLane::BD,
    DrumLane::LBD,
    DrumLane::LP,
];

/// 何度もミスした音（赤）と、1 回だけミスした音（黄）
pub const WEAK_COLOR: Color32 = Color32::from_rgb(0xFF, 0x70, 0x50);
pub const SLIP_COLOR: Color32 = Color32::from_rgb(0xF0, 0xD0, 0x50);
/// 既定で、これだけミスした音を赤く、「×回数」を付ける
const WEAK_TIMES: u32 = 2;

/// 音符ごとのミスした回数（譜面の位置とレーン）
pub type Marks = HashMap<(NotePos, DrumLane), u32>;

fn row_of(lane: DrumLane) -> DrumLane {
    if lane == DrumLane::HHO {
        DrumLane::HH
    } else {
        lane
    }
}

/// 絵に描く音符
#[derive(Debug, Clone, Copy, PartialEq)]
struct Chip {
    pos: NotePos,
    lane: DrumLane,
}

impl Chip {
    /// 小節の中の位置（0 以上 1 未満）
    fn frac(&self) -> f32 {
        self.pos.num as f32 / self.pos.den.max(1) as f32
    }
}

/// 1 曲の音符を、小節ごとに引ける形で持つ
#[derive(Debug, Clone, Default)]
pub struct ChartPicture {
    /// 小節の順
    chips: Vec<Chip>,
    /// 小節ごとの長さ（拍）
    beats: Vec<f64>,
}

/// 描き方
#[derive(Debug, Clone, Copy)]
pub struct Look {
    /// 左にレーンの名前
    pub labels: bool,
    /// 上に小節の番号
    pub numbers: bool,
    /// 何度もミスした音に「×回数」の札
    pub counts: bool,
    /// 今鳴っている所（区間の頭から 0〜1）
    pub playhead: Option<f32>,
    /// 赤く塗る拍（小節、小節の中の拍）
    pub beat: Option<(u32, u32)>,
    /// これだけミスした音を赤くする（練習中は、段の決まりの「同じ拍で何回」に合わせる）
    pub weak_times: u32,
    pub alpha: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            labels: true,
            numbers: true,
            counts: true,
            playhead: None,
            beat: None,
            weak_times: WEAK_TIMES,
            alpha: 1.0,
        }
    }
}

impl Look {
    /// 一覧に並べる小さな絵（名前も番号も札も付けない）
    pub fn thumbnail() -> Self {
        Self {
            labels: false,
            numbers: false,
            counts: false,
            ..Self::default()
        }
    }
}

impl ChartPicture {
    pub fn new(notes: &[DrumNote], tempo: &TempoMap) -> Self {
        let mut chips: Vec<Chip> = notes
            .iter()
            .map(|n| Chip {
                pos: n.at,
                lane: n.lane,
            })
            .collect();
        chips.sort_by_key(|c| (c.pos.measure, c.frac().to_bits()));
        let beats = (0..tempo.measure_count())
            .map(|m| tempo.measure_start_beat(m + 1) - tempo.measure_start_beat(m))
            .collect();
        Self { chips, beats }
    }

    /// 区間の音符
    fn chips_in(&self, s: Section) -> &[Chip] {
        let from = self.chips.partition_point(|c| c.pos.measure < s.first);
        let to = self.chips.partition_point(|c| c.pos.measure <= s.last);
        &self.chips[from..to]
    }

    /// 区間にあるレーン（行の順）
    fn rows(&self, s: Section) -> Vec<DrumLane> {
        let chips = self.chips_in(s);
        ROWS.into_iter()
            .filter(|&r| chips.iter().any(|c| row_of(c.lane) == r))
            .collect()
    }

    /// 小節 `m` の長さ（拍）。分からなければ 4
    fn beats_in(&self, m: u32) -> f64 {
        self.beats
            .get(m as usize)
            .copied()
            .filter(|b| *b > 0.0)
            .unwrap_or(4.0)
    }

    /// 区間 `s` を (x, y, w, h) に描く。`marks` はミスした回数
    pub fn draw(
        &self,
        c: &Canvas,
        g: &mut Gfx,
        (x, y, w, h): (f32, f32, f32, f32),
        s: Section,
        marks: &Marks,
        look: Look,
    ) {
        let a = look.alpha;
        let fade = |color: Color32| color.gamma_multiply(a);
        let p = c.painter();
        p.rect_filled(
            c.rect(x, y, w, h),
            5.0 * c.scale(),
            fade(Color32::from_rgba_unmultiplied(10, 10, 18, 235)),
        );
        let rows = self.rows(s);
        let n_rows = rows.len().max(1) as f32;
        let label_w = if look.labels { 30.0 } else { 4.0 };
        let head_h = if look.numbers { 16.0 } else { 3.0 };
        let (left, top) = (x + label_w, y + head_h);
        let right = x + w - 4.0;
        let bottom = y + h - 3.0;
        let rh = (bottom - top) / n_rows;
        let measures = s.measures().max(1) as f32;
        let mw = (right - left) / measures;
        // 行の縞とレーンの名前
        for (i, &lane) in rows.iter().enumerate() {
            let ry = top + i as f32 * rh;
            if i % 2 == 0 {
                c.fill(
                    left,
                    ry,
                    right - left,
                    rh,
                    fade(Color32::from_white_alpha(7)),
                );
            }
            if look.labels {
                let px = (rh * 0.72).clamp(9.0, 13.0);
                let style = Style {
                    bold: true,
                    ..Style::edged(px, fade(lane_color(lane)), Color32::BLACK, 1.0)
                };
                if let Some(t) = g.texts.get(lane.name(), &style) {
                    t.draw(c, left - 5.0 - t.w, ry + (rh - t.h) / 2.0, 1.0, a);
                }
            }
        }
        // 赤く塗る拍
        if let Some((m, b)) = look.beat
            && s.contains(m)
        {
            let len = self.beats_in(m);
            let mx = left + (m - s.first) as f32 * mw;
            let bx = mx + mw * (f64::from(b) / len) as f32;
            let bw = mw * (1.0_f64.min(len - f64::from(b)) / len) as f32;
            c.fill(
                bx,
                top,
                bw,
                bottom - top,
                fade(WEAK_COLOR.gamma_multiply(0.22)),
            );
        }
        // 拍と小節の線、小節の番号
        for (i, m) in (s.first..=s.last).enumerate() {
            let mx = left + i as f32 * mw;
            let len = self.beats_in(m);
            let mut b = 0.0;
            while b < len - 1e-6 {
                let bx = mx + mw * (b / len) as f32;
                let (color, width) = if b == 0.0 {
                    (Color32::from_rgba_unmultiplied(150, 150, 170, 210), 1.5)
                } else {
                    (Color32::from_rgba_unmultiplied(70, 70, 86, 170), 1.0)
                };
                c.fill(bx, top, width, bottom - top, fade(color));
                b += 1.0;
            }
            if look.numbers {
                let style = Style::edged(11.0, fade(Color32::from_gray(200)), Color32::BLACK, 1.0);
                if let Some(t) = g.texts.get(&m.to_string(), &style) {
                    t.draw(c, mx + 3.0, y + 2.0, 1.0, a);
                }
            }
        }
        // 音符
        let chips = self.chips_in(s);
        let x_of = |chip: &Chip| {
            let i = (chip.pos.measure - s.first) as f32;
            left + mw * (i + chip.frac())
        };
        // 音符の幅は 16 分の間隔の 6 割
        let sixteenth = mw / (4.0 * self.beats_in(s.first) as f32);
        let chip_w = (sixteenth * 0.62).clamp(2.0, 11.0);
        let chip_h = (rh * 0.64).max(2.5);
        let r = (chip_w.min(chip_h) * 0.35).min(3.0) * c.scale();
        for chip in chips {
            let Some(row) = rows.iter().position(|&l| l == row_of(chip.lane)) else {
                continue;
            };
            let cx = x_of(chip) + chip_w / 2.0 + 1.0;
            let cy = top + (row as f32 + 0.5) * rh;
            let k = marks.get(&(chip.pos, chip.lane)).copied().unwrap_or(0);
            if k > 0 {
                let ring = if k >= look.weak_times.max(1) {
                    WEAK_COLOR
                } else {
                    SLIP_COLOR
                };
                let e = if look.counts { 3.0 } else { 2.0 };
                let rect = c.rect(
                    cx - chip_w / 2.0 - e,
                    cy - chip_h / 2.0 - e,
                    chip_w + e * 2.0,
                    chip_h + e * 2.0,
                );
                p.rect_filled(rect, r + e, fade(ring.gamma_multiply(0.35)));
                p.rect_stroke(
                    rect,
                    r + e,
                    Stroke::new(1.6 * c.scale(), fade(ring)),
                    StrokeKind::Inside,
                );
            }
            let rect = c.rect(cx - chip_w / 2.0, cy - chip_h / 2.0, chip_w, chip_h);
            let color = fade(lane_color(chip.lane));
            if chip.lane == DrumLane::HHO {
                // 開いたハイハットは中を抜く
                p.rect_filled(rect, r, fade(Color32::from_rgb(10, 10, 18)));
                p.rect_stroke(
                    rect,
                    r,
                    Stroke::new(1.5 * c.scale(), color),
                    StrokeKind::Inside,
                );
            } else {
                p.rect_filled(rect, r, color);
            }
        }
        // ×回数の札（音符の上。一番上の行なら下）。音符が詰まっていて前の札と重なるなら出さない
        if look.counts {
            let mut last_right: HashMap<usize, f32> = HashMap::new();
            for chip in chips {
                let k = marks.get(&(chip.pos, chip.lane)).copied().unwrap_or(0);
                if k < look.weak_times.max(1) {
                    continue;
                }
                let Some(row) = rows.iter().position(|&l| l == row_of(chip.lane)) else {
                    continue;
                };
                let label = format!("×{k}");
                let style = Style {
                    bold: true,
                    ..Style::plain(11.0, fade(Color32::WHITE))
                };
                let Some(t) = g.texts.get(&label, &style) else {
                    continue;
                };
                let cx = x_of(chip) + chip_w / 2.0 + 1.0;
                let cy = top + (row as f32 + 0.5) * rh;
                let (tw, th) = (t.w + 8.0, t.h + 2.0);
                let lx = (cx - tw / 2.0).clamp(x + 2.0, x + w - tw - 2.0);
                if last_right.get(&row).is_some_and(|&r| lx < r + 2.0) {
                    continue;
                }
                last_right.insert(row, lx + tw);
                let ty = if row == 0 {
                    cy + chip_h / 2.0 + 3.0
                } else {
                    cy - chip_h / 2.0 - th - 3.0
                };
                p.rect_filled(c.rect(lx, ty, tw, th), 3.0 * c.scale(), fade(WEAK_COLOR));
                t.draw(c, lx + 4.0, ty + 1.0, 1.0, a);
            }
        }
        // 今鳴っている所
        if let Some(f) = look.playhead {
            let px = left + (right - left) * f.clamp(0.0, 1.0);
            c.fill(
                px - 4.0,
                top,
                8.0,
                bottom - top,
                fade(Color32::from_white_alpha(36)),
            );
            c.fill(
                px - 1.0,
                top - 2.0,
                2.0,
                bottom - top + 2.0,
                fade(Color32::WHITE),
            );
            p.add(egui::Shape::convex_polygon(
                vec![
                    c.pos(px - 5.0, top - 7.0),
                    c.pos(px + 5.0, top - 7.0),
                    c.pos(px, top - 1.0),
                ],
                fade(Color32::WHITE),
                Stroke::NONE,
            ));
        }
        p.rect_stroke(
            c.rect(x, y, w, h),
            5.0 * c.scale(),
            Stroke::new(1.0, fade(Color32::from_white_alpha(40))),
            StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(measure: u32, num: u32, den: u32, lane: DrumLane) -> DrumNote {
        DrumNote {
            ms: 0.0,
            lane,
            wav: 1,
            at: NotePos::new(measure, num, den),
            beat: 0.0,
        }
    }

    #[test]
    fn rows_follow_the_score_order_and_only_the_lanes_in_the_section() {
        let chart = dtx::load(b"#BPM 120\n#00113: 01\n#00211: 01\n");
        let tempo = TempoMap::build(&chart);
        let p = ChartPicture::new(
            &[
                note(1, 0, 4, DrumLane::BD),
                note(1, 1, 4, DrumLane::HHO),
                note(1, 2, 4, DrumLane::CY),
                note(2, 0, 4, DrumLane::SD),
            ],
            &tempo,
        );
        assert_eq!(
            p.rows(Section::new(1, 1)),
            vec![DrumLane::CY, DrumLane::HH, DrumLane::BD]
        );
        assert_eq!(p.chips_in(Section::new(2, 2)).len(), 1);
        assert_eq!(p.beats_in(1), 4.0);
        // 譜面の外の小節は 4 拍とみなす
        assert_eq!(p.beats_in(99), 4.0);
    }
}
