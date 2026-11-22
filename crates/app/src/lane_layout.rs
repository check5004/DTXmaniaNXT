//! ドラムのレーンの並び（上流 DTXManiaNX の LaneType A〜D と RDPosition）。
//! レーンの枠・レーンの光・チップ・パッド・火花・判定の文字の x を、上流の表と同じ値で持つ。
//! 上流は種類ごとの表（`nチャンネルtoX座標B` など）と、既定の表からずらす決まりで書いているので、
//! ここでは並び 4 つ × RD の位置 2 つの結果を表にした。

use dtx::DrumLane;

/// 上流の `ELane`（ドラムの 10 レーン）。表の添字に使う
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    LC,
    HH,
    SD,
    BD,
    HT,
    LT,
    FT,
    CY,
    LP,
    RD,
}

impl Lane {
    /// 叩いた・判定したレーンを上流のレーンに（上流 `nパッド0Atoレーン07`、`nチャンネル0Atoレーン07`）
    pub fn of(l: DrumLane) -> Self {
        match l {
            DrumLane::HH | DrumLane::HHO => Self::HH,
            DrumLane::SD => Self::SD,
            DrumLane::BD => Self::BD,
            DrumLane::HT => Self::HT,
            DrumLane::LT => Self::LT,
            DrumLane::FT => Self::FT,
            DrumLane::CY => Self::CY,
            DrumLane::RD => Self::RD,
            DrumLane::LC => Self::LC,
            DrumLane::LP | DrumLane::LBD => Self::LP,
        }
    }

    pub fn i(self) -> usize {
        self as usize
    }
}

/// レーンの並び（上流 `eLaneType.Drums`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaneType {
    /// LC HH LP SD HT BD LT FT
    #[default]
    A,
    /// LC HH SD LP BD HT LT FT（ペダル 2 つとタム 3 つをまとめる）
    B,
    /// LC HH LP SD BD HT LT FT（タム 3 つをまとめる）
    C,
    /// LC HH SD HT LP BD LT FT（左右対称）
    D,
}

/// RD のレーンの位置（上流 `eRDPosition`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RdPosition {
    /// … CY RD（右端が RD。上流 RCRD、既定）
    #[default]
    Right,
    /// … RD CY（右端が CY。上流 RDRC）
    LeftOfCy,
}

/// 枠（7_Paret.png）の x（上流 `CActPerfDrumsLaneFlushD` の「動くレーン」）。`Lane` の順
const PANEL_X: [[f32; 10]; 4] = [
    // LC    HH     SD     BD     HT     LT     FT     CY     LP     RD
    [
        295.0, 367.0, 467.0, 573.0, 524.0, 642.0, 691.0, 745.0, 416.0, 815.0,
    ],
    [
        295.0, 367.0, 416.0, 524.0, 593.0, 642.0, 691.0, 745.0, 473.0, 815.0,
    ],
    [
        295.0, 367.0, 467.0, 524.0, 593.0, 642.0, 691.0, 745.0, 416.0, 815.0,
    ],
    [
        295.0, 367.0, 416.0, 573.0, 473.0, 642.0, 691.0, 745.0, 522.0, 815.0,
    ],
];

/// レーンの光の x（上流 `stレーンサイズ` とその並べ替え）
const FLUSH_X: [[f32; 10]; 4] = [
    [
        298.0, 370.0, 470.0, 582.0, 528.0, 645.0, 694.0, 748.0, 419.0, 815.0,
    ],
    [
        298.0, 370.0, 419.0, 534.0, 595.0, 645.0, 694.0, 748.0, 475.0, 815.0,
    ],
    [
        298.0, 370.0, 470.0, 534.0, 595.0, 645.0, 694.0, 748.0, 419.0, 815.0,
    ],
    [
        298.0, 370.0, 419.0, 582.0, 476.0, 645.0, 694.0, 748.0, 526.0, 815.0,
    ],
];

/// チップの x（上流 `nチャンネルtoX座標`、`B`、`C`、`D`）
const CHIP_X: [[f32; 10]; 4] = [
    [
        298.0, 370.0, 470.0, 582.0, 527.0, 645.0, 694.0, 748.0, 419.0, 815.0,
    ],
    [
        298.0, 370.0, 419.0, 533.0, 596.0, 645.0, 694.0, 748.0, 476.0, 815.0,
    ],
    [
        298.0, 370.0, 470.0, 533.0, 596.0, 645.0, 694.0, 748.0, 419.0, 815.0,
    ],
    [
        298.0, 370.0, 419.0, 582.0, 476.0, 645.0, 694.0, 748.0, 525.0, 815.0,
    ],
];

/// パッドの x（上流 `CActPerfDrumsPad.st基本位置` とその並べ替え）
const PAD_X: [[f32; 10]; 4] = [
    [
        263.0, 336.0, 446.0, 565.0, 510.0, 622.0, 672.0, 735.0, 396.0, 791.0,
    ],
    [
        263.0, 336.0, 392.0, 517.0, 580.0, 624.0, 672.0, 735.0, 456.0, 791.0,
    ],
    [
        263.0, 336.0, 446.0, 517.0, 580.0, 622.0, 672.0, 735.0, 396.0, 791.0,
    ],
    [
        263.0, 336.0, 392.0, 565.0, 457.0, 622.0, 672.0, 735.0, 496.0, 791.0,
    ],
];

/// 火花の中心（+320 する前。上流 `nレーンの中央X座標`、`B`、`C`、`D`）
const FIRE_X: [[f32; 10]; 4] = [
    [
        7.0, 71.0, 176.0, 293.0, 230.0, 349.0, 398.0, 464.0, 124.0, 514.0,
    ],
    [
        7.0, 71.0, 124.0, 240.0, 297.0, 349.0, 398.0, 464.0, 180.0, 514.0,
    ],
    [
        7.0, 71.0, 176.0, 242.0, 297.0, 349.0, 398.0, 464.0, 124.0, 508.0,
    ],
    [
        7.0, 71.0, 124.0, 294.0, 182.0, 349.0, 398.0, 464.0, 230.0, 514.0,
    ],
];

/// 判定の文字を出すレーンの x と幅（上流 `sizeXW`、`_B`、`_C`、`_D`）
const JUDGE_XW: [[(i32, i32); 10]; 4] = [
    [
        (290, 80),
        (367, 46),
        (470, 54),
        (582, 60),
        (528, 46),
        (645, 46),
        (694, 46),
        (748, 64),
        (419, 46),
        (815, 80),
    ],
    [
        (290, 80),
        (367, 46),
        (419, 54),
        (534, 60),
        (590, 46),
        (645, 46),
        (694, 46),
        (748, 64),
        (478, 46),
        (815, 64),
    ],
    [
        (290, 80),
        (367, 46),
        (470, 54),
        (534, 60),
        (590, 46),
        (645, 46),
        (694, 46),
        (748, 64),
        (419, 46),
        (815, 64),
    ],
    [
        (290, 80),
        (367, 46),
        (419, 54),
        (582, 60),
        (476, 46),
        (645, 46),
        (694, 46),
        (748, 64),
        (528, 46),
        (815, 64),
    ],
];

/// HH オープンのチップの x（並びによらない。上流 `nチャンネルtoX座標` の HHO）
const HHO_CHIP_X: f32 = 373.0;

/// レーンごとの x（`Lane` の順）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub panel: [f32; 10],
    pub flush: [f32; 10],
    pub chip: [f32; 10],
    pub pad: [f32; 10],
    /// 火花の中心（+320 する前）
    pub fire: [f32; 10],
    /// 判定の文字の x と幅
    pub judge: [(i32, i32); 10],
}

impl Default for Layout {
    fn default() -> Self {
        Self::new(LaneType::A, RdPosition::Right)
    }
}

impl Layout {
    pub fn new(lanes: LaneType, rd: RdPosition) -> Self {
        let t = lanes as usize;
        let mut l = Self {
            panel: PANEL_X[t],
            flush: FLUSH_X[t],
            chip: CHIP_X[t],
            pad: PAD_X[t],
            fire: FIRE_X[t],
            judge: JUDGE_XW[t],
        };
        if rd == RdPosition::LeftOfCy {
            let (cy, rd) = (Lane::CY.i(), Lane::RD.i());
            l.panel[cy] = 783.0;
            l.panel[rd] = 746.0;
            l.flush[cy] = 786.0;
            l.flush[rd] = 748.0;
            l.chip[cy] = 786.0;
            l.chip[rd] = 746.0;
            l.pad[cy] = 776.0;
            l.pad[rd] = 720.0;
            l.fire[cy] = if lanes == LaneType::A { 498.0 } else { 500.0 };
            l.fire[rd] = 448.0;
            l.judge[cy].0 = 791;
            l.judge[rd].0 = 730;
            // 上流の `nチャンネルtoX座標C改` と `D改` は LT と LP が 1〜2 ドットずれている
            match lanes {
                LaneType::C => l.chip[Lane::LT.i()] = 644.0,
                LaneType::D => l.chip[Lane::LP.i()] = 527.0,
                _ => {}
            }
        }
        l
    }

    /// 設定の表の「レーンの並び」と「RD の位置」から
    pub fn from_settings(v: &crate::settings::Values) -> Self {
        use crate::settings as s;
        let lanes = match v.choice(s::LANE_TYPE) {
            1 => LaneType::B,
            2 => LaneType::C,
            3 => LaneType::D,
            _ => LaneType::A,
        };
        let rd = match v.choice(s::RD_POSITION) {
            1 => RdPosition::LeftOfCy,
            _ => RdPosition::Right,
        };
        Self::new(lanes, rd)
    }

    /// チップを置くレーンの x（絵による細かいずらしは描く側で）
    pub fn chip_x(&self, l: DrumLane) -> f32 {
        match l {
            DrumLane::HHO => HHO_CHIP_X,
            _ => self.chip[Lane::of(l).i()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANES: [Lane; 10] = [
        Lane::LC,
        Lane::HH,
        Lane::SD,
        Lane::BD,
        Lane::HT,
        Lane::LT,
        Lane::FT,
        Lane::CY,
        Lane::LP,
        Lane::RD,
    ];

    /// 枠の幅（7_Paret.png の切り出しの幅。`Lane` の順）
    const PANEL_W: [f32; 10] = [72.0, 49.0, 57.0, 69.0, 49.0, 49.0, 54.0, 70.0, 51.0, 38.0];

    fn all() -> Vec<(LaneType, RdPosition, Layout)> {
        let mut v = Vec::new();
        for t in [LaneType::A, LaneType::B, LaneType::C, LaneType::D] {
            for rd in [RdPosition::Right, RdPosition::LeftOfCy] {
                v.push((t, rd, Layout::new(t, rd)));
            }
        }
        v
    }

    fn order(l: &Layout) -> Vec<Lane> {
        let mut lanes = LANES.to_vec();
        lanes.sort_by(|a, b| l.panel[a.i()].total_cmp(&l.panel[b.i()]));
        lanes
    }

    #[test]
    fn lanes_line_up_like_upstream() {
        use Lane::*;
        let ends = |rd| match rd {
            RdPosition::Right => [CY, RD],
            RdPosition::LeftOfCy => [RD, CY],
        };
        for (t, rd, l) in all() {
            let mut want = match t {
                LaneType::A => vec![LC, HH, LP, SD, HT, BD, LT, FT],
                LaneType::B => vec![LC, HH, SD, LP, BD, HT, LT, FT],
                LaneType::C => vec![LC, HH, LP, SD, BD, HT, LT, FT],
                LaneType::D => vec![LC, HH, SD, HT, LP, BD, LT, FT],
            };
            want.extend(ends(rd));
            assert_eq!(order(&l), want, "{t:?} {rd:?}");
        }
    }

    #[test]
    fn panels_sit_side_by_side() {
        for (t, rd, l) in all() {
            let lanes = order(&l);
            for w in lanes.windows(2) {
                let end = l.panel[w[0].i()] + PANEL_W[w[0].i()];
                let gap = l.panel[w[1].i()] - end;
                assert!(gap.abs() <= 1.0, "{t:?} {rd:?} {:?}→{:?} {gap}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn every_part_of_a_lane_sits_on_its_panel() {
        for (t, rd, l) in all() {
            for lane in LANES {
                let i = lane.i();
                let (left, right) = (l.panel[i] - 4.0, l.panel[i] + PANEL_W[i] + 4.0);
                let inside = |x: f32, what: &str| {
                    assert!(
                        (left..=right).contains(&x),
                        "{t:?} {rd:?} {lane:?} {what} {x} は {left}〜{right} の外"
                    );
                };
                inside(l.chip[i], "chip");
                inside(l.flush[i], "flush");
                inside(l.pad[i] + 48.0, "pad");
                inside(l.fire[i] + 320.0, "fire");
                let (x, w) = l.judge[i];
                inside((x + w / 2) as f32, "judge");
            }
        }
    }

    #[test]
    fn layout_follows_the_settings() {
        use crate::settings as s;
        let mut v = s::Values::default();
        assert_eq!(Layout::from_settings(&v), Layout::default());
        v.set_raw(s::LANE_TYPE, 3);
        v.set_raw(s::RD_POSITION, 1);
        let l = Layout::from_settings(&v);
        assert_eq!(l, Layout::new(LaneType::D, RdPosition::LeftOfCy));
        assert_eq!(l.chip_x(DrumLane::LBD), 527.0);
        assert_eq!(l.chip_x(DrumLane::HHO), HHO_CHIP_X);
    }
}
