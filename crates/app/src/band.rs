//! 小節の帯。曲の小節を左から並べ、苦手度で色を付ける（苦手なほど赤く、記録の無い小節は暗い灰色）。
//! 練習のポップアップ（区間を選ぶ）と、結果の画面（苦手な小節）で使う。長い曲は段を折り返す。

use egui::{Color32, Pos2, Stroke, StrokeKind};
use engine::practice::Section;

use crate::gfx::{Canvas, Gfx};
use crate::text::Style;
use crate::ui::weakness_color;

/// 選んだ区間の色
pub const SELECTION_COLOR: Color32 = Color32::from_rgb(0x40, 0xC8, 0xFF);
/// 練習でクリアした区間の印の色
pub const CLEARED_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);

/// 1 段に並べる小節の数の目安
const PER_ROW: usize = 64;

/// 帯に描くもの
pub struct BandData<'a> {
    /// 小節ごとの苦手度（記録が無ければ None）
    pub weakness: &'a [Option<f64>],
    /// 白い枠で囲む区間（苦手なフレーズ）
    pub phrases: Vec<Section>,
    /// 青く塗る区間
    pub selected: Option<Section>,
    /// 下に緑の印を付ける区間（練習でクリアした）
    pub cleared: Vec<Section>,
}

/// 帯の場所と形
#[derive(Debug, Clone, Copy)]
pub struct Band {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    /// 1 段の高さ
    pub cell_h: f32,
    /// 段の送り（下に小節の番号を書く）
    pub row_pitch: f32,
    /// 段の数の上限
    pub max_rows: usize,
    /// NX の進み具合のバーと同じく、黒地に 5 ドットごとの縞を敷く
    pub striped: bool,
}

impl Band {
    /// 段の数と、1 段の小節の数、1 小節の幅
    pub fn layout(&self, n: usize) -> (usize, usize, f32) {
        let rows = n.div_ceil(PER_ROW).clamp(1, self.max_rows.max(1));
        let per_row = n.div_ceil(rows).max(1);
        (rows, per_row, self.w / per_row as f32)
    }

    /// 小節 `m` の左上
    pub fn cell_pos(&self, n: usize, m: usize) -> (f32, f32) {
        let (_, per_row, cw) = self.layout(n);
        let (r, col) = (m / per_row, m % per_row);
        (self.x + cw * col as f32, self.y + self.row_pitch * r as f32)
    }

    /// 帯の上の点にある小節
    pub fn measure_at(&self, n: usize, p: Pos2) -> Option<u32> {
        if n == 0 {
            return None;
        }
        let (rows, per_row, cw) = self.layout(n);
        let (dx, dy) = (p.x - self.x, p.y - self.y);
        if !(0.0..self.w).contains(&dx) || dy < 0.0 {
            return None;
        }
        let r = (dy / self.row_pitch) as usize;
        if r >= rows || dy - r as f32 * self.row_pitch > self.cell_h {
            return None;
        }
        let m = r * per_row + (dx / cw) as usize;
        (m < n).then_some(m as u32)
    }

    /// 帯を描く。下の端（小節の番号の下）の y を返す
    pub fn draw(&self, c: &Canvas, g: &mut Gfx, d: &BandData<'_>) -> f32 {
        let n = d.weakness.len();
        if n == 0 {
            return self.y;
        }
        let h = self.cell_h;
        let (rows, per_row, cw) = self.layout(n);
        if self.striped {
            for r in 0..rows {
                let y = self.y + self.row_pitch * r as f32;
                let count = (n - r * per_row).min(per_row);
                let w = cw * count as f32;
                c.fill(self.x, y, w, h, Color32::from_rgb(10, 10, 10));
                let mut sx = 5.0;
                while sx < w {
                    c.fill(
                        self.x + sx,
                        y,
                        5.0_f32.min(w - sx),
                        h,
                        Color32::from_rgb(14, 14, 14),
                    );
                    sx += 10.0;
                }
            }
        }
        let gap = if cw >= 4.0 { 1.0 } else { 0.0 };
        for (m, v) in d.weakness.iter().enumerate() {
            let (x, y) = self.cell_pos(n, m);
            match v {
                Some(_) => c.fill(x, y, cw - gap, h, weakness_color(*v)),
                // 縞の上では記録の無い小節は塗らない（進み具合のバーのたどり着いていない区間と同じ）
                None if self.striped => {}
                None => c.fill(x, y, cw - gap, h, weakness_color(None)),
            }
        }
        let stroke = |x: f32, y: f32, w: f32, h: f32, color: Color32, width: f32| {
            c.painter().rect_stroke(
                c.rect(x, y, w, h),
                0.0,
                Stroke::new(width * c.scale(), color),
                StrokeKind::Outside,
            );
        };
        // 区間を段ごとに分けて (x, y, 幅) にする
        let spans = |s: Section| -> Vec<(f32, f32, f32)> {
            let (first, last) = (s.first as usize, (s.last as usize).min(n - 1));
            let mut out = Vec::new();
            let mut m = first;
            while m <= last {
                let row_end = (m / per_row + 1) * per_row - 1;
                let end = last.min(row_end);
                let (x, y) = self.cell_pos(n, m);
                out.push((x, y, cw * (end - m + 1) as f32 - gap));
                m = end + 1;
            }
            out
        };
        for &p in &d.phrases {
            for (x, y, w) in spans(p) {
                stroke(x, y, w, h, Color32::WHITE, 2.0);
            }
        }
        for &s in &d.cleared {
            for (x, y, w) in spans(s) {
                c.fill(x, y + h + 2.0, w, 3.0, CLEARED_COLOR);
            }
        }
        if let Some(s) = d.selected {
            for (x, y, w) in spans(s) {
                c.fill(x, y - 3.0, w, h + 6.0, SELECTION_COLOR.gamma_multiply(0.3));
                stroke(x, y - 3.0, w, h + 6.0, SELECTION_COLOR, 2.5);
            }
        }
        // 小節の番号（DTX の小節番号）。重ならない間隔で
        let step = [1, 2, 4, 8, 16, 32, 64]
            .into_iter()
            .find(|&s| cw * s as f32 >= 32.0)
            .unwrap_or(128);
        let style = Style::plain(11.0, Color32::from_gray(170));
        for m in (0..n).step_by(step) {
            let (x, y) = self.cell_pos(n, m);
            if let Some(t) = g.texts.get(&m.to_string(), &style) {
                t.draw(c, x, y + h + 6.0, 1.0, 1.0);
            }
        }
        self.y + self.row_pitch * (rows - 1) as f32 + h + 20.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAND: Band = Band {
        x: 100.0,
        y: 50.0,
        w: 640.0,
        cell_h: 24.0,
        row_pitch: 40.0,
        max_rows: 4,
        striped: false,
    };

    #[test]
    fn long_songs_wrap() {
        assert_eq!(BAND.layout(40), (1, 40, 16.0));
        assert_eq!(BAND.layout(150).0, 3);
        assert_eq!(BAND.layout(1000).0, 4);
        assert_eq!(
            Band {
                max_rows: 2,
                ..BAND
            }
            .layout(1000)
            .0,
            2
        );
        // 2 段目の頭の小節
        let (_, per_row, _) = BAND.layout(150);
        let (x, y) = BAND.cell_pos(150, per_row);
        assert_eq!(
            BAND.measure_at(150, Pos2::new(x + 1.0, y + 1.0)),
            Some(per_row as u32)
        );
        assert_eq!(BAND.measure_at(150, Pos2::new(99.0, 51.0)), None);
        // 段のあいだ（番号のところ）は小節ではない
        assert_eq!(BAND.measure_at(150, Pos2::new(101.0, 50.0 + 30.0)), None);
    }
}
