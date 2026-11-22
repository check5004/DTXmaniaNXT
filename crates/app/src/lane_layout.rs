//! ドラムのレーンの並び（上流 DTXManiaNX の LaneType A〜D と RDPosition）。
//! レーンの枠・レーンの光・チップ・パッド・火花・判定の文字の x を、上流の表と同じ値で持つ。
//! 上流は種類ごとの表（`nチャンネルtoX座標B` など）と、既定の表からずらす決まりで書いているので、
//! ここでは並び 4 つ × RD の位置 2 つの結果を表にした。
//! 左バスドラムのレーン（NXT だけのもの）を足すときは、LP のすぐ右に LP と同じ幅で入れ、右の側をずらす。

use dtx::DrumLane;

/// 上流の `ELane`（ドラムの 10 レーン）と、NXT で足せる左バスドラムのレーン。表の添字に使う
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
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
    /// 左バスドラム（LP の右に足したときだけ使う）
    LBD,
}

/// レーンの数（左バスドラムのレーンも入れて）
pub const LANES: usize = 11;

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

/// レーンを並べる左の端と、上流の 10 レーンの幅（判定の線・小節線・シャッターの幅）
pub const LANES_LEFT: f32 = 295.0;
const LANES_W: f32 = 559.0;
/// 足す左バスドラムのレーンの幅（LP の枠と同じ）
pub const LEFT_BASS_W: f32 = 51.0;

/// 上流の表（10 レーン）を、左バスドラムのぶんも入れた長さにする（左バスドラムは LP と同じ値）
fn with_left_bass<T: Copy>(a: [T; 10]) -> [T; LANES] {
    let mut out = [a[Lane::LP.i()]; LANES];
    out[..10].copy_from_slice(&a);
    out
}

/// レーンごとの x（`Lane` の順）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub panel: [f32; LANES],
    pub flush: [f32; LANES],
    pub chip: [f32; LANES],
    pub pad: [f32; LANES],
    /// 火花の中心（+320 する前）
    pub fire: [f32; LANES],
    /// 判定の文字の x と幅
    pub judge: [(i32, i32); LANES],
    /// LP の右に左バスドラムのレーンを足した
    pub left_bass_lane: bool,
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
            panel: with_left_bass(PANEL_X[t]),
            flush: with_left_bass(FLUSH_X[t]),
            chip: with_left_bass(CHIP_X[t]),
            pad: with_left_bass(PAD_X[t]),
            fire: with_left_bass(FIRE_X[t]),
            judge: with_left_bass(JUDGE_XW[t]),
            left_bass_lane: false,
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
        l.chip[Lane::LBD.i()] = l.chip[Lane::LP.i()];
        l
    }

    /// LP のすぐ右に左バスドラムのレーンを足す（実物のツインペダルと同じく、左ペダルの右に左のビーター）。
    /// LP より右のレーンは [`LEFT_BASS_W`] だけ右へずらす
    pub fn with_left_bass_lane(mut self) -> Self {
        if self.left_bass_lane {
            return self;
        }
        let (lp, lbd) = (Lane::LP.i(), Lane::LBD.i());
        let w = LEFT_BASS_W;
        for i in 0..LANES {
            if i != lbd && self.panel[i] > self.panel[lp] {
                self.panel[i] += w;
                self.flush[i] += w;
                self.chip[i] += w;
                self.pad[i] += w;
                self.fire[i] += w;
                self.judge[i].0 += w as i32;
            }
        }
        self.panel[lbd] = self.panel[lp] + w;
        self.flush[lbd] = self.flush[lp] + w;
        self.chip[lbd] = self.chip[lp] + w;
        self.pad[lbd] = self.pad[lp] + w;
        self.fire[lbd] = self.fire[lp] + w;
        self.judge[lbd] = (self.judge[lp].0 + w as i32, self.judge[lp].1);
        self.left_bass_lane = true;
        self
    }

    /// 足したレーンのぶんの幅（足していなければ 0）
    pub fn extra_w(&self) -> f32 {
        if self.left_bass_lane {
            LEFT_BASS_W
        } else {
            0.0
        }
    }

    /// レーン全体の幅（判定の線・小節線・シャッターの幅）
    pub fn width(&self) -> f32 {
        LANES_W + self.extra_w()
    }

    /// このレーンを描くか（左バスドラムのレーンは足したときだけ）
    pub fn shows(&self, l: Lane) -> bool {
        l != Lane::LBD || self.left_bass_lane
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
        let l = Self::new(lanes, rd);
        if v.left_bass_lane() {
            l.with_left_bass_lane()
        } else {
            l
        }
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

    const ALL_LANES: [Lane; LANES] = [
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
        Lane::LBD,
    ];

    /// 枠の幅（7_Paret.png の切り出しの幅。`Lane` の順。左バスドラムは LP と同じ）
    const PANEL_W: [f32; LANES] = [
        72.0, 49.0, 57.0, 69.0, 49.0, 49.0, 54.0, 70.0, 51.0, 38.0, 51.0,
    ];

    /// 並び 4 つ × RD の位置 2 つ × 左バスドラムのレーンの有無
    fn all() -> Vec<(LaneType, RdPosition, Layout)> {
        let mut v = Vec::new();
        for t in [LaneType::A, LaneType::B, LaneType::C, LaneType::D] {
            for rd in [RdPosition::Right, RdPosition::LeftOfCy] {
                let l = Layout::new(t, rd);
                v.push((t, rd, l));
                v.push((t, rd, l.with_left_bass_lane()));
            }
        }
        v
    }

    fn order(l: &Layout) -> Vec<Lane> {
        let mut lanes: Vec<Lane> = ALL_LANES.into_iter().filter(|&x| l.shows(x)).collect();
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
            // 左バスドラムのレーンは LP のすぐ右
            if l.left_bass_lane {
                let lp = want.iter().position(|&x| x == LP).unwrap();
                want.insert(lp + 1, LBD);
            }
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
            // 右の端は判定の線などの幅と同じ
            let last = lanes.last().unwrap().i();
            let right = l.panel[last] + PANEL_W[last];
            assert!(
                (right - (LANES_LEFT + l.width())).abs() <= 1.0,
                "{t:?} {rd:?} {right}"
            );
        }
    }

    #[test]
    fn every_part_of_a_lane_sits_on_its_panel() {
        for (t, rd, l) in all() {
            for lane in ALL_LANES.into_iter().filter(|&x| l.shows(x)) {
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
        assert!(!l.left_bass_lane && l.extra_w() == 0.0);
        // 左バスドラムのレーンを足す出し方
        v.set_raw(s::LBD_GRAPHICS, 1);
        let l = Layout::from_settings(&v);
        assert!(l.left_bass_lane && l.shows(Lane::LBD));
        assert_eq!(l.width(), 559.0 + LEFT_BASS_W);
        assert_eq!(l.chip[Lane::LBD.i()], 527.0 + LEFT_BASS_W);
    }
}
