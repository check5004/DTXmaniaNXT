//! 曲の練習（区間練習）のやり方。今までの「段をのぼってクリア」に加えて、目的ごとのやり方を選べる。
//!
//! どのやり方も、選んだ区間を周回する [`crate::practice`] の上に乗る。ここではやり方ごとの決まり
//! （どのレーンを自動にするか、どの周がお手本か、積み上げの長さ、ブレイクで消す小節、消える譜面の割合）だけを決める。

use dtx::{DrumLane, NotePos};

use crate::practice::{Section, Span};

/// 曲の練習のやり方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Way {
    /// 段をのぼる（今まで）。始めの速度から目標の速度まで 10% ずつ
    #[default]
    Climb,
    /// パート練習。選んだパート（[`Part`]）だけを叩き、ほかは自動で鳴らす
    Part,
    /// お手本と交互。自動で叩く 1 周（見て聞くだけ）と、自分で叩く 1 周を交互に
    Demo,
    /// 積み上げ。1 小節がクリアできたら 2 小節、4 小節と区間ぜんぶまで伸ばす（向きは [`Build`]）
    Build,
    /// 曲のブレイク。曲の音とクリックが 2 小節ずつ消える（判定はそのまま）
    Break,
    /// 消える譜面。段の中で周を重ねるごとに音符が減り、最後の周は見えない。落とした音は次の周で戻る
    Fade,
}

impl Way {
    /// 選ぶ順（SD・FT で順に回す）
    pub const ALL: [Way; 6] = [
        Self::Climb,
        Self::Part,
        Self::Demo,
        Self::Build,
        Self::Break,
        Self::Fade,
    ];

    /// 設定と記録に書く名前
    pub fn key(self) -> &'static str {
        match self {
            Self::Climb => "climb",
            Self::Part => "part",
            Self::Demo => "demo",
            Self::Build => "build",
            Self::Break => "break",
            Self::Fade => "fade",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.key() == s)
    }

    /// 前後のやり方（端は反対の端へ回る）
    pub fn step(self, d: i64) -> Self {
        let n = Self::ALL.len() as i64;
        let i = Self::ALL.iter().position(|&w| w == self).unwrap_or(0) as i64;
        Self::ALL[(i + d).rem_euclid(n) as usize]
    }

    /// 1 段の周の数（`step_laps` は選んだ数）。お手本と交互は、お手本と自分を 1 組にするので偶数に切り上げる
    pub fn laps_per_step(self, step_laps: u32) -> u32 {
        let n = step_laps.max(1);
        match self {
            Self::Demo => n.div_ceil(2) * 2,
            _ => n,
        }
    }

    /// 段の `k` 周目（1 から）がお手本の周か。お手本と交互は奇数の周（段の頭はいつもお手本）
    pub fn demo_lap(self, k: usize) -> bool {
        self == Self::Demo && k % 2 == 1
    }
}

/// パート練習で叩くパート
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Part {
    /// 手（足のパッドのほか全部）
    #[default]
    Hands,
    /// 足（バスドラム・左ペダル・左バスドラム）
    Feet,
    HiHat,
    Snare,
    Toms,
    Cymbals,
}

impl Part {
    pub const ALL: [Part; 6] = [
        Self::Hands,
        Self::Feet,
        Self::HiHat,
        Self::Snare,
        Self::Toms,
        Self::Cymbals,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Hands => "hands",
            Self::Feet => "feet",
            Self::HiHat => "hihat",
            Self::Snare => "snare",
            Self::Toms => "toms",
            Self::Cymbals => "cymbals",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == s)
    }

    /// 自分で叩くレーン
    pub fn lanes(self) -> &'static [DrumLane] {
        use DrumLane::*;
        match self {
            Self::Hands => &[HH, SD, HT, LT, CY, FT, HHO, RD, LC],
            Self::Feet => &[BD, LP, LBD],
            Self::HiHat => &[HH, HHO],
            Self::Snare => &[SD],
            Self::Toms => &[HT, LT, FT],
            Self::Cymbals => &[CY, RD, LC],
        }
    }

    /// 自動で叩くレーン（パートの外）
    pub fn auto_lanes(self) -> Vec<DrumLane> {
        DrumLane::ALL
            .into_iter()
            .filter(|l| !self.lanes().contains(l))
            .collect()
    }

    /// レーンの集まり（`DrumLane as usize` のビット）
    pub fn mask(self) -> u16 {
        lane_mask(self.lanes().iter().copied())
    }
}

/// レーンの集まりをビットにする
pub fn lane_mask(lanes: impl IntoIterator<Item = DrumLane>) -> u16 {
    lanes.into_iter().fold(0, |m, l| m | 1 << l as u16)
}

/// 積み上げの向き
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Build {
    /// 区間の頭から後ろへ伸ばす
    #[default]
    Forward,
    /// 区間の終わりから前へ伸ばす（終わりの方が難しいとき）
    Backward,
}

impl Build {
    pub const ALL: [Build; 2] = [Self::Forward, Self::Backward];

    pub fn key(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.key() == s)
    }
}

/// 積み上げの長さ（小節）。1・2・4・8 … と倍にし、最後は区間ぜんぶ
pub fn build_lengths(measures: u32) -> Vec<u32> {
    let n = measures.max(1);
    let mut v = Vec::new();
    let mut len = 1;
    while len < n {
        v.push(len);
        len *= 2;
    }
    v.push(n);
    v
}

/// 区間 `s` のうち、積み上げの長さ `len` 小節の所
pub fn build_section(s: Section, len: u32, build: Build) -> Section {
    let len = len.clamp(1, s.measures());
    match build {
        Build::Forward => Section::new(s.first, s.first + len - 1),
        Build::Backward => Section::new(s.last + 1 - len, s.last),
    }
}

/// 曲のブレイクで、続けて鳴らす・消す小節の数
pub const BREAK_BARS: u32 = 2;

/// 曲のブレイクで、練習を始めてから `bar` 小節目（0 から。周をまたいで数える）を消すか。
/// 2 小節鳴らして 2 小節消す、をくり返す
pub fn break_muted(bar: u32) -> bool {
    (bar / BREAK_BARS) % 2 == 1
}

/// 鳴らす範囲 `span` から、曲の時刻の範囲 `cuts`（時刻順、重ならない）を除いた残り。
/// `speed` は周の速度（残りの鳴らし始めの実時間を出すのに使う）
pub fn cut_span(span: Span, cuts: &[(f64, f64)], speed: f64) -> Vec<Span> {
    let mut out = Vec::new();
    let mut from = span.from_ms;
    let at = |ms: f64| span.at_ms + (ms - span.from_ms) / speed.max(0.01);
    for &(a, b) in cuts {
        if b <= from || a >= span.to_ms {
            continue;
        }
        if a > from {
            out.push(Span {
                from_ms: from,
                to_ms: a,
                at_ms: at(from),
            });
        }
        from = from.max(b);
    }
    if from < span.to_ms {
        out.push(Span {
            from_ms: from,
            to_ms: span.to_ms,
            at_ms: at(from),
        });
    }
    out
}

/// 消える譜面で、段の `k` 周目（1 から、`laps` 周のうち）に見せる音符の割合（0〜1）。
/// 1 周目はぜんぶ、最後の周は 0
pub fn fade_share(k: usize, laps: usize) -> f64 {
    if laps <= 1 {
        return 0.0;
    }
    let k = k.clamp(1, laps);
    1.0 - (k - 1) as f64 / (laps - 1) as f64
}

/// 消える譜面で、音符を隠す順番（0〜1）。小節と位置とレーンから決め、周が変わっても同じ音符は同じ値
/// （割合が減るほど隠れる音符が増える。低い値の音符ほど長く見える）
pub fn fade_rank(at: NotePos, lane: DrumLane) -> f64 {
    let mut h: u32 = 0x811c_9dc5;
    for b in at
        .measure
        .to_le_bytes()
        .into_iter()
        .chain(at.num.to_le_bytes())
        .chain(at.den.to_le_bytes())
        .chain([lane as u8])
    {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    // 下の桁は偏るので、混ぜてから上の桁を使う
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    f64::from(h >> 8) / f64::from(1u32 << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_step_wraps() {
        for w in Way::ALL {
            assert_eq!(Way::from_key(w.key()), Some(w));
        }
        for p in Part::ALL {
            assert_eq!(Part::from_key(p.key()), Some(p));
        }
        for b in Build::ALL {
            assert_eq!(Build::from_key(b.key()), Some(b));
        }
        assert_eq!(Way::Climb.step(-1), Way::Fade);
        assert_eq!(Way::Fade.step(1), Way::Climb);
        assert_eq!(Way::Climb.step(1), Way::Part);
    }

    #[test]
    fn parts_split_hands_and_feet() {
        let hands = Part::Hands.mask();
        let feet = Part::Feet.mask();
        assert_eq!(hands & feet, 0);
        assert_eq!(hands | feet, lane_mask(DrumLane::ALL));
        assert!(Part::Toms.auto_lanes().contains(&DrumLane::SD));
        assert!(!Part::Toms.auto_lanes().contains(&DrumLane::FT));
    }

    #[test]
    fn demo_laps_pair_up() {
        assert_eq!(Way::Demo.laps_per_step(3), 4);
        assert_eq!(Way::Demo.laps_per_step(4), 4);
        assert_eq!(Way::Climb.laps_per_step(3), 3);
        assert!(Way::Demo.demo_lap(1) && !Way::Demo.demo_lap(2) && Way::Demo.demo_lap(3));
        assert!(!Way::Climb.demo_lap(1));
    }

    #[test]
    fn build_doubles_up_to_the_whole_section() {
        assert_eq!(build_lengths(1), vec![1]);
        assert_eq!(build_lengths(4), vec![1, 2, 4]);
        assert_eq!(build_lengths(6), vec![1, 2, 4, 6]);
        let s = Section::new(10, 15);
        assert_eq!(build_section(s, 2, Build::Forward), Section::new(10, 11));
        assert_eq!(build_section(s, 2, Build::Backward), Section::new(14, 15));
        assert_eq!(build_section(s, 9, Build::Backward), s);
    }

    #[test]
    fn cutting_a_span_keeps_the_rest_in_place() {
        let span = Span {
            from_ms: 0.0,
            to_ms: 1000.0,
            at_ms: 500.0,
        };
        let v = cut_span(span, &[(200.0, 400.0), (800.0, 1200.0)], 0.5);
        assert_eq!(
            v,
            vec![
                Span {
                    from_ms: 0.0,
                    to_ms: 200.0,
                    at_ms: 500.0
                },
                Span {
                    from_ms: 400.0,
                    to_ms: 800.0,
                    at_ms: 1300.0
                },
            ]
        );
        assert_eq!(cut_span(span, &[], 1.0), vec![span]);
        assert!(cut_span(span, &[(-10.0, 2000.0)], 1.0).is_empty());
    }

    #[test]
    fn breaks_go_two_on_two_off() {
        let v: Vec<bool> = (0..8).map(break_muted).collect();
        assert_eq!(v, [false, false, true, true, false, false, true, true]);
    }

    fn pos(measure: u32, num: u32, den: u32) -> NotePos {
        NotePos { measure, num, den }
    }

    #[test]
    fn fading_goes_from_all_to_none() {
        assert_eq!(fade_share(1, 4), 1.0);
        assert!((fade_share(2, 4) - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(fade_share(4, 4), 0.0);
        assert_eq!(fade_share(1, 1), 0.0);
        // 隠す順番はばらけていて、同じ音符は同じ値
        let ranks: Vec<f64> = (0..200)
            .map(|i| fade_rank(pos(i / 16, i % 16, 16), DrumLane::SD))
            .collect();
        assert!(ranks.iter().all(|r| (0.0..1.0).contains(r)));
        let low = ranks.iter().filter(|&&r| r < 0.5).count();
        assert!((70..130).contains(&low), "{low}");
        assert_eq!(
            fade_rank(pos(3, 1, 4), DrumLane::HH),
            fade_rank(pos(3, 1, 4), DrumLane::HH)
        );
    }
}
