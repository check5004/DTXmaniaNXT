//! 基礎練習（ドリル）。曲を使わず、クリックに合わせて決まった型を叩く。
//!
//! 1 回は、1 小節のカウントのあと 4 段でテンポを上げ、1 段 4 小節ずつ叩いて終わる（約 45 秒）。
//! ミスで段をくり返さず、どの段で崩れたかは結果で見せる（2026-11-25 の見本）。
//! 型はレーンごとの文字列で書き、譜面（DTX の文字）にしてふつうの演奏と同じ判定に流す。
//! 型の小節 0 はカウントで、音符は 1 小節目から置く。
//!
//! 電子ドラムはどちらの手で叩いたかを送ってこないので、左右のずれは「型の手順どおりに叩いた」として数える。

use dtx::{DrumLane, NotePos};

use crate::analysis::mean_and_sd;
use crate::judge::{Judgement, Tally};

/// 段の数
pub const STEPS: usize = 4;
/// 1 段の小節の数
pub const STEP_MEASURES: u32 = 4;
/// カウントの小節の数（譜面の 0 小節目）
pub const COUNT_IN_MEASURES: u32 = 1;
/// 選べるテンポの範囲と刻み
pub const BPM_MIN: u32 = 40;
pub const BPM_MAX: u32 = 240;
pub const BPM_STEP: u32 = 5;
/// 1 段ごとの上げ幅の選択肢と既定
pub const RISES: [u32; 4] = [0, 5, 10, 15];
pub const DEFAULT_RISE: u32 = 10;
/// 平均のずれがこれ以上なら「早め」「遅め」と見せる（叩き方のくせと同じ）
pub const LEAN_MS: f64 = 8.0;

/// 叩く手足
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hand {
    Right,
    Left,
    Foot,
}

impl Hand {
    pub const ALL: [Hand; 3] = [Self::Right, Self::Left, Self::Foot];

    fn from_char(c: char) -> Option<Self> {
        match c {
            'R' => Some(Self::Right),
            'L' => Some(Self::Left),
            'F' => Some(Self::Foot),
            _ => None,
        }
    }

    /// 型の文字（R・L・F）
    pub fn letter(self) -> char {
        match self {
            Self::Right => 'R',
            Self::Left => 'L',
            Self::Foot => 'F',
        }
    }
}

/// ドリルの分け方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Timing,
    Rudiments,
    Grooves,
    InnerClock,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Self::Timing,
        Self::Rudiments,
        Self::Grooves,
        Self::InnerClock,
    ];
}

/// ドリル 1 つ
#[derive(Debug, PartialEq, Eq)]
pub struct Drill {
    /// 記録と設定に書く名前（変えない）
    pub id: &'static str,
    pub category: Category,
    /// レーンごとの型。1 マス 1 文字で R・L は手、F は足、- は休み。小節は `|` で分け、
    /// 小節ごとのマスの数がその小節の分け方（16 なら 16 分、12 なら 3 連の 8 分）。どのレーンも同じ小節の数
    pub lanes: &'static [(DrumLane, &'static str)],
    /// クリックを鳴らす小節の数と、消す小節の数（体内時計）。あるときはテンポを上げない
    pub gap: Option<(u32, u32)>,
    /// はじめに選んでおくテンポ
    pub default_bpm: u32,
}

const SD: DrumLane = DrumLane::SD;
const HH: DrumLane = DrumLane::HH;
const BD: DrumLane = DrumLane::BD;

/// すべてのドリル（画面に出す順）
pub const DRILLS: &[Drill] = &[
    // ---- タイミング ----
    Drill {
        id: "quarter",
        category: Category::Timing,
        lanes: &[(SD, "R---L---R---L---")],
        gap: None,
        default_bpm: 90,
    },
    Drill {
        id: "eighth",
        category: Category::Timing,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        gap: None,
        default_bpm: 90,
    },
    Drill {
        id: "sixteenth",
        category: Category::Timing,
        lanes: &[(SD, "RLRLRLRLRLRLRLRL")],
        gap: None,
        default_bpm: 80,
    },
    Drill {
        id: "triplet",
        category: Category::Timing,
        lanes: &[(SD, "RLRLRLRLRLRL")],
        gap: None,
        default_bpm: 80,
    },
    Drill {
        id: "upbeat",
        category: Category::Timing,
        lanes: &[(SD, "--R---R---R---R-")],
        gap: None,
        default_bpm: 90,
    },
    Drill {
        id: "subdivisions",
        category: Category::Timing,
        lanes: &[(
            SD,
            "R---L---R---L---|R-L-R-L-R-L-R-L-|RLRLRLRLRLRL|RLRLRLRLRLRLRLRL",
        )],
        gap: None,
        default_bpm: 80,
    },
    // ---- ルーディメンツ ----
    Drill {
        id: "single_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "RLRLRLRLRLRLRLRL")],
        gap: None,
        default_bpm: 80,
    },
    Drill {
        id: "double_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "RRLLRRLLRRLLRRLL")],
        gap: None,
        default_bpm: 70,
    },
    Drill {
        id: "paradiddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RLRRLRLLRLRRLRLL")],
        gap: None,
        default_bpm: 80,
    },
    Drill {
        id: "double_paradiddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RLRLRRLRLRLL")],
        gap: None,
        default_bpm: 80,
    },
    Drill {
        id: "paradiddle_diddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RLRRLLRLRRLL")],
        gap: None,
        default_bpm: 80,
    },
    Drill {
        id: "six_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "RLLRRLRLLRRL")],
        gap: None,
        default_bpm: 70,
    },
    // ---- 手と足 ----
    Drill {
        id: "eight_beat",
        category: Category::Grooves,
        lanes: &[
            (HH, "R-R-R-R-R-R-R-R-"),
            (SD, "----L-------L---"),
            (BD, "F-------F-F-----"),
        ],
        gap: None,
        default_bpm: 100,
    },
    Drill {
        id: "sixteen_beat",
        category: Category::Grooves,
        lanes: &[
            (HH, "RLRL-LRLRLRL-LRL"),
            (SD, "----R-------R---"),
            (BD, "F-------F-F-----"),
        ],
        gap: None,
        default_bpm: 80,
    },
    Drill {
        id: "four_on_the_floor",
        category: Category::Grooves,
        lanes: &[
            (HH, "--R---R---R---R-"),
            (SD, "----L-------L---"),
            (BD, "F---F---F---F---"),
        ],
        gap: None,
        default_bpm: 110,
    },
    Drill {
        id: "shuffle",
        category: Category::Grooves,
        lanes: &[
            (HH, "R-RR-RR-RR-R"),
            (SD, "---L-----L--"),
            (BD, "F-----F-----"),
        ],
        gap: None,
        default_bpm: 90,
    },
    Drill {
        id: "hand_foot_triplet",
        category: Category::Grooves,
        lanes: &[(SD, "RL-RL-RL-RL-"), (BD, "--F--F--F--F")],
        gap: None,
        default_bpm: 70,
    },
    // ---- 体内時計 ----
    Drill {
        id: "gap_2",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        gap: Some((2, 2)),
        default_bpm: 90,
    },
    Drill {
        id: "gap_4",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        gap: Some((4, 4)),
        default_bpm: 90,
    },
];

/// 型の音符 1 つ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatternNote {
    /// 型の中の小節（0 から）
    pub measure: u32,
    /// 小節の中のマスと、その小節のマスの数
    pub cell: u32,
    pub cells: u32,
    pub lane: DrumLane,
    pub hand: Hand,
}

impl PatternNote {
    /// 小節の中の位置（0 以上 1 未満）
    pub fn pos(&self) -> f64 {
        f64::from(self.cell) / f64::from(self.cells)
    }
}

impl Drill {
    /// `id` のドリル
    pub fn find(id: &str) -> Option<&'static Drill> {
        DRILLS.iter().find(|d| d.id == id)
    }

    /// 型の小節の数
    pub fn measures(&self) -> u32 {
        self.lanes
            .iter()
            .map(|(_, s)| s.split('|').count() as u32)
            .max()
            .unwrap_or(1)
    }

    /// 型の音符（小節、位置の順）
    pub fn notes(&self) -> Vec<PatternNote> {
        let mut v = Vec::new();
        for &(lane, text) in self.lanes {
            for (m, bar) in text.split('|').enumerate() {
                let cells: Vec<char> = bar.chars().filter(|c| !c.is_whitespace()).collect();
                for (i, &c) in cells.iter().enumerate() {
                    if let Some(hand) = Hand::from_char(c) {
                        v.push(PatternNote {
                            measure: m as u32,
                            cell: i as u32,
                            cells: cells.len() as u32,
                            lane,
                            hand,
                        });
                    }
                }
            }
        }
        v.sort_by(|a, b| {
            (a.measure, a.pos())
                .partial_cmp(&(b.measure, b.pos()))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.lane.cmp(&b.lane))
        });
        v
    }

    /// 使うレーン（型に書いた順）
    pub fn lanes(&self) -> Vec<DrumLane> {
        self.lanes.iter().map(|&(l, _)| l).collect()
    }

    /// テンポを上げないドリルか（クリックが消える小節があると、新しいテンポが分からないので）
    pub fn fixed_tempo(&self) -> bool {
        self.gap.is_some()
    }
}

/// 段ごとのテンポ。`target` を最後の段にして、`rise` ずつ下げて並べる（`BPM_MIN` より下げない）。
/// テンポを上げないドリルと `rise` が 0 のときは、どの段も `target`
pub fn step_bpms(drill: &Drill, target: u32, rise: u32) -> [u32; STEPS] {
    let target = target.clamp(BPM_MIN, BPM_MAX);
    let rise = if drill.fixed_tempo() { 0 } else { rise };
    std::array::from_fn(|i| {
        let down = rise * (STEPS - 1 - i) as u32;
        target.saturating_sub(down).max(BPM_MIN).min(target)
    })
}

/// テンポを `BPM_STEP` 刻みで `steps` だけ動かす（範囲に収める）
pub fn nudge_bpm(bpm: u32, steps: i32) -> u32 {
    let v = i64::from(bpm) + i64::from(steps) * i64::from(BPM_STEP);
    v.clamp(i64::from(BPM_MIN), i64::from(BPM_MAX)) as u32
}

/// 1 回の基礎練習（ドリルと段ごとのテンポ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub drill: &'static Drill,
    pub bpms: [u32; STEPS],
}

impl Run {
    pub fn new(drill: &'static Drill, target: u32, rise: u32) -> Self {
        Self {
            drill,
            bpms: step_bpms(drill, target, rise),
        }
    }

    /// 目標のテンポ（最後の段）
    pub fn target(&self) -> u32 {
        self.bpms[STEPS - 1]
    }

    /// 譜面の小節の数（カウントを入れて）
    pub fn measure_count(&self) -> u32 {
        COUNT_IN_MEASURES + STEPS as u32 * STEP_MEASURES
    }

    /// 譜面の小節 `measure` が何段目か（カウントは None）
    pub fn step_of(&self, measure: u32) -> Option<usize> {
        let m = measure.checked_sub(COUNT_IN_MEASURES)?;
        let s = (m / STEP_MEASURES) as usize;
        (s < STEPS).then_some(s)
    }

    /// 段の中の何小節目か（0 から。カウントは None）
    pub fn measure_in_step(&self, measure: u32) -> Option<u32> {
        self.step_of(measure)?;
        Some((measure - COUNT_IN_MEASURES) % STEP_MEASURES)
    }

    /// 譜面の小節 `measure` でクリックを鳴らすか（カウントはいつも鳴らす）
    pub fn clicks(&self, measure: u32) -> bool {
        let Some((on, off)) = self.drill.gap else {
            return true;
        };
        let Some(m) = measure.checked_sub(COUNT_IN_MEASURES) else {
            return true;
        };
        m % (on + off) < on
    }

    /// 譜面の音符（小節と位置、レーン）を叩く手足。型に無い音符は None
    pub fn hand_at(&self, at: NotePos, lane: DrumLane) -> Option<Hand> {
        let m = at.measure.checked_sub(COUNT_IN_MEASURES)?;
        let pm = m % self.drill.measures();
        self.drill.notes().into_iter().find_map(|n| {
            (n.measure == pm
                && n.lane == lane
                && u64::from(n.cell) * u64::from(at.den) == u64::from(at.num) * u64::from(n.cells))
            .then_some(n.hand)
        })
    }

    /// 譜面の文字（DTX）。0 小節目はカウントで音符を置かず、段の頭でテンポを変える
    pub fn to_dtx(&self, title: &str) -> String {
        let mut s = String::new();
        s += &format!(
            "#TITLE: {title}\n#ARTIST: DTXmaniaNXT\n#BPM: {}\n",
            self.bpms[0]
        );
        for (i, b) in self.bpms.iter().enumerate() {
            s += &format!("#BPM{:02}: {b}\n", i + 1);
        }
        s += "#00008: 01\n";
        let pattern_measures = self.drill.measures();
        for step in 0..STEPS {
            let first = COUNT_IN_MEASURES + step as u32 * STEP_MEASURES;
            s += &format!("#{first:03}08: {:02}\n", step + 1);
        }
        for m in COUNT_IN_MEASURES..self.measure_count() {
            let pm = ((m - COUNT_IN_MEASURES) % pattern_measures) as usize;
            for &(lane, text) in self.drill.lanes {
                let Some(bar) = text.split('|').nth(pm) else {
                    continue;
                };
                let cells: String = bar
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .map(|c| {
                        if Hand::from_char(c).is_some() {
                            "01"
                        } else {
                            "00"
                        }
                    })
                    .collect();
                if cells.contains("01") {
                    s += &format!("#{m:03}{:02X}: {cells}\n", lane.channel());
                }
            }
        }
        s
    }
}

// ---- 結果 ----

/// 段の出来
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    /// ミスなし
    Clean,
    /// ミス 1〜2
    Few,
    /// ミス 3 つ以上
    Many,
}

impl Grade {
    /// POOR と MISS の数から
    pub fn of(misses: u32) -> Self {
        match misses {
            0 => Self::Clean,
            1..=2 => Self::Few,
            _ => Self::Many,
        }
    }
}

/// 音符 1 つの結果（判定したものだけ）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteOutcome {
    pub at: NotePos,
    pub lane: DrumLane,
    pub judgement: Judgement,
    /// 叩いた時刻 − 音符の時刻（ミリ秒、マイナスは早い）。見逃しは None
    pub offset_ms: Option<f64>,
}

/// 段 1 つの結果
#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub bpm: u32,
    pub tally: Tally,
    /// 段の音符の数（判定したもの）
    pub notes: usize,
    /// 段を最後まで叩いた
    pub completed: bool,
    pub offset_mean_ms: Option<f64>,
    pub offset_sd_ms: Option<f64>,
}

impl StepResult {
    pub fn misses(&self) -> u32 {
        self.tally.poor + self.tally.miss
    }

    /// 最後まで叩いた段の出来（途中の段は None）
    pub fn grade(&self) -> Option<Grade> {
        (self.completed && self.notes > 0).then(|| Grade::of(self.misses()))
    }
}

/// 手足ごとのずれ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandLag {
    pub hand: Hand,
    /// 叩けた音符の数
    pub hits: usize,
    pub mean_ms: Option<f64>,
    pub sd_ms: Option<f64>,
}

/// 平均のずれの向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lean {
    Early,
    Late,
}

impl HandLag {
    /// 目立つ向き（平均が `LEAN_MS` 以上ずれていて、10 打以上あるとき）
    pub fn lean(&self) -> Option<Lean> {
        let m = self.mean_ms.filter(|_| self.hits >= 10)?;
        if m <= -LEAN_MS {
            Some(Lean::Early)
        } else if m >= LEAN_MS {
            Some(Lean::Late)
        } else {
            None
        }
    }
}

/// 叩いた順の点 1 つ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dot {
    /// 譜面の小節（カウントを入れて）と、小節の中の位置（0〜1）
    pub measure: u32,
    pub pos: f64,
    pub hand: Option<Hand>,
    /// ずれ（ミリ秒）。POOR と MISS は None
    pub offset_ms: Option<f64>,
}

/// 1 回の結果
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub steps: Vec<StepResult>,
    /// 型に出てくる手足だけ（右・左・足の順）
    pub hands: Vec<HandLag>,
    pub dots: Vec<Dot>,
}

impl Summary {
    /// ミスなしで叩けた一番速いテンポ
    pub fn best_clean_bpm(&self) -> Option<u32> {
        self.steps
            .iter()
            .filter(|s| s.grade() == Some(Grade::Clean))
            .map(|s| s.bpm)
            .max()
    }

    /// 全部の段を最後まで叩いた
    pub fn finished(&self) -> bool {
        self.steps.len() == STEPS && self.steps.iter().all(|s| s.completed)
    }
}

/// 判定した音符の結果をまとめる。`now_measure` は今の小節（その小節より前の段は最後まで叩いた）
pub fn summarize(run: &Run, results: &[NoteOutcome], now_measure: u32) -> Summary {
    let mut steps: Vec<StepResult> = run
        .bpms
        .iter()
        .enumerate()
        .map(|(i, &bpm)| {
            let last = COUNT_IN_MEASURES + (i as u32 + 1) * STEP_MEASURES - 1;
            StepResult {
                bpm,
                tally: Tally::default(),
                notes: 0,
                completed: now_measure > last,
                offset_mean_ms: None,
                offset_sd_ms: None,
            }
        })
        .collect();
    let mut step_offsets: Vec<Vec<f64>> = vec![Vec::new(); STEPS];
    let mut hand_offsets: Vec<(Hand, Vec<f64>)> = Vec::new();
    let pattern_hands: Vec<Hand> = Hand::ALL
        .into_iter()
        .filter(|h| run.drill.notes().iter().any(|n| n.hand == *h))
        .collect();
    for &h in &pattern_hands {
        hand_offsets.push((h, Vec::new()));
    }
    let mut dots = Vec::new();
    for r in results {
        let Some(s) = run.step_of(r.at.measure) else {
            continue;
        };
        let step = &mut steps[s];
        step.tally.add(r.judgement);
        step.notes += 1;
        let hit = matches!(
            r.judgement,
            Judgement::Perfect | Judgement::Great | Judgement::Good
        );
        let offset = r.offset_ms.filter(|_| hit);
        let hand = run.hand_at(r.at, r.lane);
        if let Some(o) = offset {
            step_offsets[s].push(o);
            if let Some(h) = hand
                && let Some((_, v)) = hand_offsets.iter_mut().find(|(x, _)| *x == h)
            {
                v.push(o);
            }
        }
        dots.push(Dot {
            measure: r.at.measure,
            pos: f64::from(r.at.num) / f64::from(r.at.den.max(1)),
            hand,
            offset_ms: offset,
        });
    }
    for (s, offs) in steps.iter_mut().zip(&step_offsets) {
        (s.offset_mean_ms, s.offset_sd_ms) = mean_and_sd(offs);
    }
    dots.sort_by(|a, b| {
        (a.measure, a.pos)
            .partial_cmp(&(b.measure, b.pos))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let hands = hand_offsets
        .into_iter()
        .map(|(hand, v)| {
            let (mean_ms, sd_ms) = mean_and_sd(&v);
            HandLag {
                hand,
                hits: v.len(),
                mean_ms,
                sd_ms,
            }
        })
        .collect();
    Summary { steps, hands, dots }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drill(id: &str) -> &'static Drill {
        Drill::find(id).unwrap()
    }

    #[test]
    fn every_pattern_has_the_same_measures_on_each_lane_and_known_letters() {
        let mut ids = std::collections::HashSet::new();
        for d in DRILLS {
            assert!(ids.insert(d.id), "{} twice", d.id);
            let m = d.measures();
            for (lane, text) in d.lanes {
                assert_eq!(text.split('|').count() as u32, m, "{} {lane:?}", d.id);
                for bar in text.split('|') {
                    assert!(
                        bar.chars().all(|c| "RLF-".contains(c)),
                        "{} {lane:?} {bar}",
                        d.id
                    );
                    assert!([12, 16].contains(&bar.len()), "{} {lane:?} {bar}", d.id);
                }
            }
            // 同じ小節のレーンは同じ分け方（同じ時刻の音符が同じマスに並ぶ）
            for pm in 0..m as usize {
                let lens: Vec<usize> = d
                    .lanes
                    .iter()
                    .map(|(_, t)| t.split('|').nth(pm).unwrap().len())
                    .collect();
                assert!(lens.windows(2).all(|w| w[0] == w[1]), "{}", d.id);
            }
            assert!(!d.notes().is_empty());
            assert!((BPM_MIN..=BPM_MAX).contains(&d.default_bpm));
            assert_eq!(d.default_bpm % BPM_STEP, 0);
        }
        assert_eq!(DRILLS.len(), 19);
    }

    #[test]
    fn paradiddle_sticking_is_rlrr_lrll() {
        let s: String = drill("paradiddle")
            .notes()
            .iter()
            .map(|n| n.hand.letter())
            .collect();
        assert_eq!(s, "RLRRLRLLRLRRLRLL");
    }

    #[test]
    fn steps_climb_to_the_target() {
        let d = drill("paradiddle");
        assert_eq!(step_bpms(d, 100, 10), [70, 80, 90, 100]);
        assert_eq!(step_bpms(d, 100, 0), [100; 4]);
        // 下げすぎない
        assert_eq!(step_bpms(d, 50, 10), [40, 40, 40, 50]);
        // 範囲の外は収める
        assert_eq!(step_bpms(d, 500, 5), [225, 230, 235, 240]);
        // クリックが消えるドリルはテンポを上げない
        assert_eq!(step_bpms(drill("gap_2"), 90, 10), [90; 4]);
        assert_eq!(nudge_bpm(100, 1), 105);
        assert_eq!(nudge_bpm(40, -1), 40);
        assert_eq!(nudge_bpm(240, 3), 240);
    }

    #[test]
    fn measures_map_to_steps_and_clicks() {
        let run = Run::new(drill("gap_2"), 90, 0);
        assert_eq!(run.measure_count(), 17);
        assert_eq!(run.step_of(0), None);
        assert_eq!(run.step_of(1), Some(0));
        assert_eq!(run.step_of(4), Some(0));
        assert_eq!(run.step_of(5), Some(1));
        assert_eq!(run.step_of(16), Some(3));
        assert_eq!(run.step_of(17), None);
        assert_eq!(run.measure_in_step(6), Some(1));
        // カウントと、2 小節鳴って 2 小節消える
        let clicks: Vec<bool> = (0..9).map(|m| run.clicks(m)).collect();
        assert_eq!(
            clicks,
            [true, true, true, false, false, true, true, false, false]
        );
        let plain = Run::new(drill("eighth"), 90, 10);
        assert!((0..17).all(|m| plain.clicks(m)));
    }

    #[test]
    fn dtx_has_count_in_tempo_changes_and_notes() {
        let run = Run::new(drill("paradiddle"), 100, 10);
        let text = run.to_dtx("paradiddle");
        let chart = dtx::parse(&text);
        let tempo = dtx::TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        assert_eq!(notes.len(), 16 * 16);
        // カウントの 1 小節（70 BPM の 4 拍）のあとに最初の音符
        let beat_ms = 60_000.0 / 70.0;
        assert!(
            (notes[0].ms - 4.0 * beat_ms).abs() < 1e-6,
            "{}",
            notes[0].ms
        );
        // 段の頭でテンポが変わる（2 段目は 80 BPM）
        let step2 = tempo.measure_start_ms(5);
        let expected = 4.0 * 5.0 * beat_ms;
        assert!((step2 - expected).abs() < 1e-6, "{step2} {expected}");
        let m6 = tempo.measure_start_ms(6);
        assert!((m6 - step2 - 4.0 * 60_000.0 / 80.0).abs() < 1e-6);
        // 手順は譜面の位置から引ける
        let hands: String = notes[..8]
            .iter()
            .map(|n| run.hand_at(n.at, n.lane).unwrap().letter())
            .collect();
        assert_eq!(hands, "RLRRLRLL");
    }

    #[test]
    fn grooves_and_subdivisions_place_every_lane() {
        let run = Run::new(drill("subdivisions"), 80, 0);
        let chart = dtx::parse(&run.to_dtx("x"));
        let tempo = dtx::TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        // 4 小節の型（4 + 8 + 12 + 16 音）を 16 小節で 4 回
        assert_eq!(notes.len(), (4 + 8 + 12 + 16) * 4);
        assert!(notes.iter().all(|n| run.hand_at(n.at, n.lane).is_some()));
        let run = Run::new(drill("eight_beat"), 100, 10);
        let chart = dtx::parse(&run.to_dtx("x"));
        let notes = dtx::TempoMap::build(&chart).drum_notes(&chart);
        assert_eq!(notes.len(), (8 + 2 + 3) * 16);
        let bd = notes.iter().find(|n| n.lane == BD).unwrap();
        assert_eq!(run.hand_at(bd.at, BD), Some(Hand::Foot));
        let sd = notes.iter().find(|n| n.lane == SD).unwrap();
        assert_eq!(run.hand_at(sd.at, SD), Some(Hand::Left));
    }

    #[test]
    fn summary_grades_steps_and_splits_hands() {
        let run = Run::new(drill("single_stroke"), 100, 10);
        let mut results = Vec::new();
        for m in 1..=16u32 {
            for c in 0..16u32 {
                let at = NotePos::new(m, c, 16);
                let right = c % 2 == 0;
                // 3 段目に MISS を 1 つ、4 段目に 3 つ。左手は 10ms 早い
                let miss = (m == 9 && c == 3) || (m == 13 && c < 3);
                results.push(NoteOutcome {
                    at,
                    lane: SD,
                    judgement: if miss {
                        Judgement::Miss
                    } else {
                        Judgement::Perfect
                    },
                    offset_ms: (!miss).then_some(if right { 2.0 } else { -10.0 }),
                });
            }
        }
        let s = summarize(&run, &results, 17);
        let grades: Vec<_> = s.steps.iter().map(StepResult::grade).collect();
        assert_eq!(
            grades,
            [
                Some(Grade::Clean),
                Some(Grade::Clean),
                Some(Grade::Few),
                Some(Grade::Many)
            ]
        );
        assert_eq!(s.best_clean_bpm(), Some(80));
        assert!(s.finished());
        assert_eq!(s.hands.len(), 2);
        let left = s.hands.iter().find(|h| h.hand == Hand::Left).unwrap();
        assert!((left.mean_ms.unwrap() + 10.0).abs() < 1e-9);
        assert_eq!(left.lean(), Some(Lean::Early));
        let right = s.hands.iter().find(|h| h.hand == Hand::Right).unwrap();
        assert_eq!(right.lean(), None);
        assert_eq!(s.dots.len(), 256);
        assert_eq!(s.dots.iter().filter(|d| d.offset_ms.is_none()).count(), 4);
        // 途中でやめたら、叩き終えていない段は出来を出さない
        let partial = summarize(&run, &results[..100], 7);
        assert!(partial.steps[0].completed && !partial.steps[1].completed);
        assert_eq!(partial.steps[1].grade(), None);
        assert!(!partial.finished());
    }
}
