//! 基礎練習（ドリル）。曲を使わず、クリックに合わせて決まった型を叩く。
//!
//! 1 回は、1 小節のカウントのあと段に分けて叩いて終わる。段の分け方は「やり方」（[`Way`]）で決まる。
//! 既定の「テンポを上げる」は 4 段でテンポを上げ、1 段 4 小節ずつ叩く（約 45 秒）。
//! ミスで段をくり返さず、どの段で崩れたかは結果で見せる（2026-11-25 の見本）。
//! やり方はほかに、限界チャレンジ・持久・見ないで・聞いてまねる（2026-11-27 の見本）。
//! 「左から」は型の R と L を入れ替える（どのやり方とも合わせられる）。
//! 型はレーンごとの文字列で書き、譜面（DTX の文字）にしてふつうの演奏と同じ判定に流す。
//! 型の小節 0 はカウントで、音符は 1 小節目から置く。
//!
//! 電子ドラムはどちらの手で叩いたかを送ってこないので、左右のずれは「型の手順どおりに叩いた」として数える。
//!
//! 型にはアクセントを書ける（大文字がアクセント、小文字がふつうの音）。アクセントが出たかは叩く強さを
//! まわりのふつうの音と比べて見る（[`crate::accent`]）。強さは段の出来・最高・合否には入れない。

use dtx::{DrumLane, NotePos};

use crate::accent;
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
/// 段の出来が「× 3 つ以上」になるミスの数。限界チャレンジは同じ段でこれだけ落としたら終わる
pub const MANY_MISSES: u32 = 3;
/// 限界チャレンジ: 1 段の小節の数、1 段ごとの上げ幅、段の数の上限
pub const LIMIT_STEP_MEASURES: u32 = 2;
pub const LIMIT_RISE: u32 = 5;
pub const LIMIT_MAX_STEPS: usize = 24;
/// 持久: 選べる長さ（分）と、ずれを見る区切りの長さ（秒）
pub const ENDURANCE_MINUTES: [u32; 3] = [1, 2, 3];
pub const ENDURANCE_SEGMENT_S: f64 = 30.0;
/// 見ないで: 段ごとの音符の濃さ（最後の段は見えない）
pub const BLIND_ALPHA: [f32; STEPS] = [1.0, 0.55, 0.25, 0.0];

/// 叩く手足
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hand {
    Right,
    Left,
    /// 右足（バスドラ）
    Foot,
    /// 左足（ハイハットのペダルと左のバスドラ）
    LeftFoot,
}

impl Hand {
    pub const ALL: [Hand; 4] = [Self::Right, Self::Left, Self::Foot, Self::LeftFoot];

    /// 型の文字とレーンから（大文字と小文字は同じ手）。足（F）は左のペダルと左のバスドラなら左足
    fn of(c: char, lane: DrumLane) -> Option<Self> {
        match c.to_ascii_uppercase() {
            'R' => Some(Self::Right),
            'L' => Some(Self::Left),
            'F' if matches!(lane, DrumLane::LP | DrumLane::LBD) => Some(Self::LeftFoot),
            'F' => Some(Self::Foot),
            _ => None,
        }
    }

    /// 型の文字（R・L・F。左足も F）
    pub fn letter(self) -> char {
        match self {
            Self::Right => 'R',
            Self::Left => 'L',
            Self::Foot | Self::LeftFoot => 'F',
        }
    }

    /// 足か
    pub fn is_foot(self) -> bool {
        matches!(self, Self::Foot | Self::LeftFoot)
    }

    /// 右手と左手を入れ替えた手（「左から」の手順）。足はそのまま
    pub fn mirrored(self) -> Self {
        match self {
            Self::Right => Self::Left,
            Self::Left => Self::Right,
            h => h,
        }
    }
}

/// 1 回のやり方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Way {
    /// 4 段でテンポを上げる（1 段 4 小節。上げ幅は選べる）
    Climb,
    /// 限界チャレンジ。2 小節ごとに +5 し、同じ段で 3 つ落としたら終わる
    Limit,
    /// 持久。テンポを変えずに決めた分数（1〜3 分）叩き続け、30 秒ごとに区切って見る
    Endurance(u32),
    /// 見ないで。段が進むごとに音符が薄くなり、最後の段は見えない（テンポは変えない）
    Blind,
    /// 聞いてまねる。お手本の 1 小節（音だけ、譜面は出さない）と自分の 1 小節を交互に（テンポは変えない）
    Echo,
}

impl Way {
    /// 選ぶ順（HH で順に回す）
    pub const ALL: [Way; 7] = [
        Self::Climb,
        Self::Limit,
        Self::Endurance(1),
        Self::Endurance(2),
        Self::Endurance(3),
        Self::Blind,
        Self::Echo,
    ];

    /// 設定と記録に書く名前
    pub fn key(self) -> String {
        match self {
            Self::Climb => "climb".to_owned(),
            Self::Limit => "limit".to_owned(),
            Self::Endurance(m) => format!("endurance{m}"),
            Self::Blind => "blind".to_owned(),
            Self::Echo => "echo".to_owned(),
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.key() == s)
    }

    /// ドリル `d` で使えるか。クリックの消えるドリルは、テンポを上げ続ける限界チャレンジと、
    /// 小節ごとに交代する聞いてまねるには使わない（消える小節と重なって分からなくなるので）
    pub fn fits(self, d: &Drill) -> bool {
        !(d.gap.is_some() && matches!(self, Self::Limit | Self::Echo))
    }

    /// ドリル `d` で選んだとき実際に使うやり方（使えなければテンポを上げる）
    pub fn for_drill(self, d: &Drill) -> Self {
        if self.fits(d) { self } else { Self::Climb }
    }

    /// 次のやり方（ドリル `d` で使えないものは飛ばす。最後の次は最初）
    pub fn next_for(self, d: &Drill) -> Self {
        let i = Self::ALL.iter().position(|&w| w == self).unwrap_or(0);
        (1..=Self::ALL.len())
            .map(|k| Self::ALL[(i + k) % Self::ALL.len()])
            .find(|w| w.fits(d))
            .unwrap_or(Self::Climb)
    }

    /// 段ごとにテンポが変わるか
    pub fn changes_tempo(self) -> bool {
        matches!(self, Self::Climb | Self::Limit)
    }
}

/// ドリルの分け方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Timing,
    Rudiments,
    /// アクセント（強く叩く音の位置）
    Accents,
    /// 基本のビート
    Grooves,
    /// ジャンルのビート
    Styles,
    /// タム回しとフィル
    Fills,
    /// 足
    Feet,
    /// シンバルとハイハット
    Cymbals,
    /// 拍子とポリリズム
    Meters,
    InnerClock,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Self::Timing,
        Self::Rudiments,
        Self::Accents,
        Self::Grooves,
        Self::Styles,
        Self::Fills,
        Self::Feet,
        Self::Cymbals,
        Self::Meters,
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
    /// 小節ごとのマスの数がその小節の分け方（4/4 で 16 なら 16 分、12 なら 3 連の 8 分）。どのレーンも同じ小節の数。
    /// レーンごとにマスの数が違ってもよい（ポリリズム）。
    /// 小文字（r・l・f）のあるレーンは、大文字がアクセント、小文字がふつうの音（教則本の書き方）。
    /// 全部大文字のレーンにはアクセントが無い
    pub lanes: &'static [(DrumLane, &'static str)],
    /// クリックを鳴らす小節の数と、消す小節の数（体内時計）。あるときはテンポを上げない
    pub gap: Option<(u32, u32)>,
    /// はじめに選んでおくテンポ（4 分音符）
    pub default_bpm: u32,
    /// 拍子（分子と分母）。小節の長さは 分子 / 分母 で、4/4 なら 1
    pub meter: (u32, u32),
    /// クリックの型（1 小節）。X は強いクリック、x はふつうのクリック、- は鳴らさない。マスの数で位置を決める。
    /// None は拍ごと（小節の頭が強い）。分母が 8 の拍子はカウントもこの型で鳴らす
    pub click: Option<&'static str>,
}

impl Drill {
    /// 書かない項目の既定（`..Drill::BASE` で使う）
    const BASE: Drill = Drill {
        id: "",
        category: Category::Timing,
        lanes: &[],
        gap: None,
        default_bpm: 100,
        meter: (4, 4),
        click: None,
    };
}

const SD: DrumLane = DrumLane::SD;
const HH: DrumLane = DrumLane::HH;
const HHO: DrumLane = DrumLane::HHO;
const BD: DrumLane = DrumLane::BD;
const HT: DrumLane = DrumLane::HT;
const LT: DrumLane = DrumLane::LT;
const FT: DrumLane = DrumLane::FT;
const CY: DrumLane = DrumLane::CY;
const RD: DrumLane = DrumLane::RD;
const LC: DrumLane = DrumLane::LC;
const LP: DrumLane = DrumLane::LP;
const LBD: DrumLane = DrumLane::LBD;

/// 8 分のハイハットと 2・4 拍のスネア（ビートの多くで使う）
const HATS_8: &str = "R-R-R-R-R-R-R-R-";
/// 8 分のハイハットで、拍の頭にアクセント（ビートとジャンルで使う）
const HATS_8_ON_BEAT: &str = "R-r-R-r-R-r-R-r-";
const BACKBEAT: &str = "----L-------L---";

/// すべてのドリル（画面に出す順。分け方の中はこの順）
pub const DRILLS: &[Drill] = &[
    // ---- タイミング ----
    Drill {
        id: "quarter",
        lanes: &[(SD, "R---L---R---L---")],
        ..Drill::BASE
    },
    Drill {
        id: "eighth",
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        ..Drill::BASE
    },
    Drill {
        id: "sixteenth",
        lanes: &[(SD, "RLRLRLRLRLRLRLRL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "triplet",
        lanes: &[(SD, "RLRLRLRLRLRL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "upbeat",
        lanes: &[(SD, "--R---R---R---R-")],
        ..Drill::BASE
    },
    Drill {
        id: "sixteenth_e",
        lanes: &[(SD, "-R---L---R---L--")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "sixteenth_a",
        lanes: &[(SD, "---R---L---R---L")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "dotted_sixteenth",
        lanes: &[(SD, "R--LR--LR--LR--L")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "sixteenth_dotted",
        lanes: &[(SD, "RL--RL--RL--RL--")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "dotted_eighths",
        // 3 マスおきなので 3 小節でひと回り
        lanes: &[(SD, "R--L--R--L--R--L|--R--L--R--L--R-|-L--R--L--R--L--")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "quarter_triplet",
        lanes: &[(SD, "R-L-R-L-R-L-")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "swung_eighths",
        lanes: &[(SD, "R-LR-LR-LR-L")],
        ..Drill::BASE
    },
    Drill {
        id: "quintuplet",
        lanes: &[(SD, "RLRLRLRLRLRLRLRLRLRL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "sextuplet",
        lanes: &[(SD, "RLRLRLRLRLRLRLRLRLRLRLRL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "subdivisions",
        lanes: &[(
            SD,
            "R---L---R---L---|R-L-R-L-R-L-R-L-|RLRLRLRLRLRL|RLRLRLRLRLRLRLRL",
        )],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "eighth_sixteenth",
        lanes: &[(SD, "R-L-R-L-R-L-R-L-|RLRLRLRLRLRLRLRL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "triplet_sixteenth",
        lanes: &[(SD, "RLRLRLRLRLRL|RLRLRLRLRLRLRLRL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    // ---- ルーディメンツ ----
    Drill {
        id: "single_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "RLRLRLRLRLRLRLRL")],
        ..Drill::BASE
    },
    Drill {
        id: "double_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "RRLLRRLLRRLLRRLL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "triple_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "RRRLLLRRRLLL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "five_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "rrllR---llrrL---")],
        ..Drill::BASE
    },
    Drill {
        id: "six_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "RllrrLRllrrL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "seven_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "rrllrrL-llrrllR-")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "nine_stroke",
        category: Category::Rudiments,
        lanes: &[(SD, "rrllrrllR---|llrrllrrL---")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "paradiddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RlrrLrllRlrrLrll")],
        ..Drill::BASE
    },
    Drill {
        id: "inverted_paradiddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RllrLrrlRllrLrrl")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "reverse_paradiddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RRLRLLRLRRLRLLRL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "double_paradiddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RlrlrrLrlrll")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "triple_paradiddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RlrlrlrrLrlrlrll")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "paradiddle_diddle",
        category: Category::Rudiments,
        lanes: &[(SD, "RlrrllRlrrll")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "paradiddle_triplets",
        category: Category::Rudiments,
        lanes: &[(SD, "RlrrLrllRlrr|LrllRlrrLrll")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "single_stroke_four",
        category: Category::Rudiments,
        lanes: &[(SD, "rlrL--rlrL--")],
        ..Drill::BASE
    },
    Drill {
        id: "single_stroke_seven",
        category: Category::Rudiments,
        lanes: &[(SD, "rlrlrlR-----lrlrlrL-----")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "double_triplets",
        category: Category::Rudiments,
        lanes: &[(SD, "RRLLRRLLRRLL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    // ---- アクセント ----
    Drill {
        id: "accent_eighths",
        category: Category::Accents,
        lanes: &[(SD, "R-l-R-l-R-l-R-l-")],
        ..Drill::BASE
    },
    Drill {
        id: "accent_upbeats",
        category: Category::Accents,
        lanes: &[(SD, "r-L-r-L-r-L-r-L-")],
        ..Drill::BASE
    },
    Drill {
        id: "accent_sixteenth_1",
        category: Category::Accents,
        lanes: &[(SD, "RlrlRlrlRlrlRlrl")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "accent_sixteenth_2",
        category: Category::Accents,
        lanes: &[(SD, "rLrlrLrlrLrlrLrl")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "accent_sixteenth_3",
        category: Category::Accents,
        lanes: &[(SD, "rlRlrlRlrlRlrlRl")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "accent_sixteenth_4",
        category: Category::Accents,
        lanes: &[(SD, "rlrLrlrLrlrLrlrL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "accent_moving",
        category: Category::Accents,
        // 1 小節ごとに、アクセントが 16 分の 1 つ目 → 2 つ目 → 3 つ目 → 4 つ目へ動く
        lanes: &[(
            SD,
            "RlrlRlrlRlrlRlrl|rLrlrLrlrLrlrLrl|rlRlrlRlrlRlrlRl|rlrLrlrLrlrLrlrL",
        )],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "accent_triplets",
        category: Category::Accents,
        lanes: &[(SD, "RlrLrlRlrLrl")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "accent_threes",
        category: Category::Accents,
        // 16 分に 3 つずつ。拍の頭とずれていく
        lanes: &[(SD, "RlrLrlRlrLrlRlrL")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "accent_doubles",
        category: Category::Accents,
        lanes: &[(SD, "RrLlRrLlRrLlRrLl")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "hats_accent_upbeats",
        category: Category::Accents,
        lanes: &[
            (HH, "r-R-r-R-r-R-r-R-"),
            (SD, BACKBEAT),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    // ---- ビート ----
    Drill {
        id: "eight_beat_basic",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, BACKBEAT),
            (BD, "F-------F-------"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "eight_beat",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, BACKBEAT),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "kick_and_of_2",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, BACKBEAT),
            (BD, "F-----F-F-------"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "kick_and_of_4",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, BACKBEAT),
            (BD, "F-------F-----F-"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "kick_sixteenth",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, BACKBEAT),
            (BD, "F------F--F-----"),
        ],
        ..Drill::BASE
    },
    Drill {
        id: "quarter_hats",
        category: Category::Grooves,
        lanes: &[
            (HH, "R---R---R---R---"),
            (SD, BACKBEAT),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "sixteen_beat",
        category: Category::Grooves,
        lanes: &[
            (HH, "RLRL-LRLRLRL-LRL"),
            (SD, "----R-------R---"),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "sixteen_beat_kick",
        category: Category::Grooves,
        lanes: &[
            (HH, "RLRL-LRLRLRL-LRL"),
            (SD, "----R-------R---"),
            (BD, "F--F--F---F--F--"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "sixteen_one_hand",
        category: Category::Grooves,
        lanes: &[
            (HH, "RrrrRrrrRrrrRrrr"),
            (SD, BACKBEAT),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "half_time",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, "--------L-------"),
            (BD, "F-----F---F-----"),
        ],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "two_beat",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, "--L---L---L---L-"),
            (BD, "F---F---F---F---"),
        ],
        default_bpm: 140,
        ..Drill::BASE
    },
    Drill {
        id: "snare_every_beat",
        category: Category::Grooves,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, "L---L---L---L---"),
            (BD, "F-------F-------"),
        ],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "four_on_the_floor",
        category: Category::Grooves,
        lanes: &[
            (HH, "--R---R---R---R-"),
            (SD, BACKBEAT),
            (BD, "F---F---F---F---"),
        ],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "shuffle",
        category: Category::Grooves,
        lanes: &[
            (HH, "R-rR-rR-rR-r"),
            (SD, "---L-----L--"),
            (BD, "F-----F-----"),
        ],
        ..Drill::BASE
    },
    Drill {
        id: "hand_foot_triplet",
        category: Category::Grooves,
        lanes: &[(SD, "RL-RL-RL-RL-"), (BD, "--F--F--F--F")],
        default_bpm: 90,
        ..Drill::BASE
    },
    // ---- ジャンル ----
    Drill {
        id: "funk_ghost",
        category: Category::Styles,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, "----L--l-l--L--l"),
            (BD, "F-----F---F-----"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "linear",
        category: Category::Styles,
        lanes: &[
            (HH, "-R-R-R-R-R-R-R-R"),
            (SD, "--L-L-----L-L---"),
            (BD, "F-----F-F-----F-"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "disco",
        category: Category::Styles,
        lanes: &[
            (HH, "R---R---R---R---"),
            (HHO, "--R---R---R---R-"),
            (SD, BACKBEAT),
            (BD, "F---F---F---F---"),
        ],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "hip_hop",
        category: Category::Styles,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, BACKBEAT),
            (BD, "F------F-F---F--"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "half_time_shuffle",
        category: Category::Styles,
        lanes: &[
            (HH, "R-rR-rR-rR-r"),
            (SD, "-L--L-LL--L-"),
            (BD, "F--------F--"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "twelve_eight",
        category: Category::Styles,
        lanes: &[
            (HH, "RrrRrrRrrRrr"),
            (SD, "---L-----L--"),
            (BD, "F-----F-----"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "one_drop",
        category: Category::Styles,
        lanes: &[
            (HH, "R-rR-rR-rR-r"),
            (SD, "------L-----"),
            (BD, "------F-----"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "bossa_nova",
        category: Category::Styles,
        lanes: &[
            (HH, "R-R-R-R-R-R-R-R-|R-R-R-R-R-R-R-R-"),
            (SD, "L-----L-----L---|----L-----L-----"),
            (BD, "F-----F-F-----F-|F-----F-F-----F-"),
        ],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "samba",
        category: Category::Styles,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, "---L---L---L---L"),
            (BD, "F--FF--FF--FF--F"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "jazz_ride",
        category: Category::Styles,
        lanes: &[(RD, "R--R-RR--R-R"), (LP, "---F-----F--")],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "jazz_comping",
        category: Category::Styles,
        lanes: &[
            (RD, "R--R-RR--R-R"),
            (SD, "--L-----L---"),
            (LP, "---F-----F--"),
        ],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "drum_and_bass",
        category: Category::Styles,
        lanes: &[
            (HH, HATS_8_ON_BEAT),
            (SD, BACKBEAT),
            (BD, "F---------F-----"),
        ],
        default_bpm: 170,
        ..Drill::BASE
    },
    Drill {
        id: "blast_beat",
        category: Category::Styles,
        lanes: &[
            (CY, "R-R-R-R-R-R-R-R-"),
            (SD, "-L-L-L-L-L-L-L-L"),
            (BD, "F-F-F-F-F-F-F-F-"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    // ---- タム回しとフィル ----
    Drill {
        id: "toms_eighths",
        category: Category::Fills,
        lanes: &[
            (SD, "R-L-------------"),
            (HT, "----R-L---------"),
            (LT, "--------R-L-----"),
            (FT, "------------R-L-"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "toms_sixteenths",
        category: Category::Fills,
        lanes: &[
            (SD, "RLRL------------"),
            (HT, "----RLRL--------"),
            (LT, "--------RLRL----"),
            (FT, "------------RLRL"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "toms_upward",
        category: Category::Fills,
        lanes: &[
            (FT, "RLRL------------"),
            (LT, "----RLRL--------"),
            (HT, "--------RLRL----"),
            (SD, "------------RLRL"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "toms_triplets",
        category: Category::Fills,
        lanes: &[
            (SD, "RLR---------"),
            (HT, "---LRL------"),
            (LT, "------RLR---"),
            (FT, "---------LRL"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "toms_sextuplets",
        category: Category::Fills,
        lanes: &[
            (SD, "RLRLRL------------------"),
            (HT, "------RLRLRL------------"),
            (LT, "------------RLRLRL------"),
            (FT, "------------------RLRLRL"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "snare_and_toms",
        category: Category::Fills,
        lanes: &[
            (SD, "R-R-R-R-R-R-R-R-"),
            (HT, "-L-L-L-L--------"),
            (FT, "---------L-L-L-L"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "rll_fill",
        category: Category::Fills,
        lanes: &[
            (HT, "R--R--------"),
            (FT, "------R--R--"),
            (SD, "-LL-LL-LL-LL"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "hand_hand_foot",
        category: Category::Fills,
        lanes: &[
            (SD, "RL----------"),
            (HT, "---RL-------"),
            (LT, "------RL----"),
            (FT, "---------RL-"),
            (BD, "--F--F--F--F"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "fill_one_beat",
        category: Category::Fills,
        lanes: &[
            (HH, "R-R-R-R-R-R-R-R-|R-R-R-R-R-R-----"),
            (SD, "----L-------L---|----L-------RL--"),
            (FT, "----------------|--------------RL"),
            (BD, "F-------F-F-----|F-------F-F-----"),
        ],
        ..Drill::BASE
    },
    Drill {
        id: "fill_two_beats",
        category: Category::Fills,
        lanes: &[
            (HH, "R-R-R-R-R-R-R-R-|R-R-R-R---------"),
            (SD, "----L-------L---|----L---RLRL----"),
            (HT, "----------------|------------RL--"),
            (FT, "----------------|--------------RL"),
            (BD, "F-------F-F-----|F-------F-------"),
        ],
        ..Drill::BASE
    },
    Drill {
        id: "fill_four_bars",
        category: Category::Fills,
        lanes: &[
            (
                CY,
                "R---------------|----------------|----------------|----------------",
            ),
            (
                HH,
                "--R-R-R-R-R-R-R-|R-R-R-R-R-R-R-R-|R-R-R-R-R-R-R-R-|----------------",
            ),
            (
                SD,
                "----L-------L---|----L-------L---|----L-------L---|RLRL------------",
            ),
            (
                HT,
                "----------------|----------------|----------------|----RLRL--------",
            ),
            (
                LT,
                "----------------|----------------|----------------|--------RLRL----",
            ),
            (
                FT,
                "----------------|----------------|----------------|------------RLRL",
            ),
            (
                BD,
                "F-------F-F-----|F-------F-F-----|F-------F-F-----|----------------",
            ),
        ],
        ..Drill::BASE
    },
    // ---- 足 ----
    Drill {
        id: "kick_eighths",
        category: Category::Feet,
        lanes: &[
            (HH, "R---R---R---R---"),
            (SD, BACKBEAT),
            (BD, "F-F-F-F-F-F-F-F-"),
        ],
        ..Drill::BASE
    },
    Drill {
        id: "kick_upbeats",
        category: Category::Feet,
        lanes: &[(HH, "R---R---R---R---"), (BD, "--F---F---F---F-")],
        ..Drill::BASE
    },
    Drill {
        id: "kick_doubles",
        category: Category::Feet,
        lanes: &[(HH, HATS_8), (SD, BACKBEAT), (BD, "FF------FF------")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "hihat_foot",
        category: Category::Feet,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-"), (LP, "F---F---F---F---")],
        ..Drill::BASE
    },
    Drill {
        id: "hihat_foot_2_4",
        category: Category::Feet,
        lanes: &[
            (RD, HATS_8),
            (BD, "F-------F-------"),
            (LP, "----F-------F---"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "kick_and_hihat_foot",
        category: Category::Feet,
        lanes: &[(BD, "F---F---F---F---"), (LP, "--F---F---F---F-")],
        ..Drill::BASE
    },
    Drill {
        id: "double_bass_eighths",
        category: Category::Feet,
        lanes: &[
            (RD, "R---R---R---R---"),
            (SD, BACKBEAT),
            (BD, "F---F---F---F---"),
            (LBD, "--F---F---F---F-"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "double_bass_triplets",
        category: Category::Feet,
        lanes: &[
            (RD, "R--R--R--R--"),
            (SD, "---L-----L--"),
            (BD, "F-F-F-F-F-F-"),
            (LBD, "-F-F-F-F-F-F"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "double_bass_sixteenths",
        category: Category::Feet,
        lanes: &[
            (RD, "R---R---R---R---"),
            (SD, BACKBEAT),
            (BD, "F-F-F-F-F-F-F-F-"),
            (LBD, "-F-F-F-F-F-F-F-F"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "gallop",
        category: Category::Feet,
        lanes: &[
            (RD, "R---R---R---R---"),
            (SD, BACKBEAT),
            (BD, "F--FF--FF--FF--F"),
            (LBD, "--F---F---F---F-"),
        ],
        default_bpm: 90,
        ..Drill::BASE
    },
    // ---- シンバルとハイハット ----
    Drill {
        id: "open_hihat_four",
        category: Category::Cymbals,
        lanes: &[
            (HH, "R-R-R-R-R-R-R---"),
            (HHO, "--------------R-"),
            (SD, BACKBEAT),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "open_hihat_upbeats",
        category: Category::Cymbals,
        lanes: &[
            (HH, "R---R---R---R---"),
            (HHO, "--R---R---R---R-"),
            (SD, BACKBEAT),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "ride_and_crash",
        category: Category::Cymbals,
        lanes: &[
            (CY, "R---------------"),
            (RD, "--R-R-R-R-R-R-R-"),
            (SD, BACKBEAT),
            (BD, "F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "hats_to_ride",
        category: Category::Cymbals,
        lanes: &[
            (HH, "R-R-R-R-R-R-R-R-|----------------"),
            (RD, "----------------|R-R-R-R-R-R-R-R-"),
            (SD, "----L-------L---|----L-------L---"),
            (BD, "F-------F-F-----|F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "left_right_crash",
        category: Category::Cymbals,
        lanes: &[
            (LC, "L---------------|----------------"),
            (CY, "----------------|R---------------"),
            (HH, "--R-R-R-R-R-R-R-|--R-R-R-R-R-R-R-"),
            (SD, "----L-------L---|----L-------L---"),
            (BD, "F-------F-F-----|F-------F-F-----"),
        ],
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "crash_every_beat",
        category: Category::Cymbals,
        lanes: &[
            (CY, "R---R---R---R---"),
            (SD, BACKBEAT),
            (BD, "F---F---F---F---"),
        ],
        default_bpm: 120,
        ..Drill::BASE
    },
    Drill {
        id: "kicks_and_crashes",
        category: Category::Cymbals,
        lanes: &[
            (CY, "R-----R-----R---|R-----R---------"),
            (SD, "----------------|--------RLRLRLRL"),
            (BD, "F-----F-----F---|F-----F---------"),
        ],
        ..Drill::BASE
    },
    // ---- 拍子とポリリズム ----
    Drill {
        id: "waltz",
        category: Category::Meters,
        lanes: &[
            (HH, "R-R-R-R-R-R-"),
            (SD, "----L---L---"),
            (BD, "F-----------"),
        ],
        default_bpm: 120,
        meter: (3, 4),
        ..Drill::BASE
    },
    Drill {
        id: "three_four",
        category: Category::Meters,
        lanes: &[
            (HH, "R-R-R-R-R-R-"),
            (SD, "----L-------"),
            (BD, "F-------F---"),
        ],
        meter: (3, 4),
        ..Drill::BASE
    },
    Drill {
        id: "six_eight",
        category: Category::Meters,
        lanes: &[
            (HH, "R-R-R-R-R-R-"),
            (SD, "------L-----"),
            (BD, "F-----------"),
        ],
        default_bpm: 70,
        meter: (6, 8),
        click: Some("X-x-x-x-x-x-"),
        ..Drill::BASE
    },
    Drill {
        id: "five_four",
        category: Category::Meters,
        lanes: &[
            (HH, "R-R-R-R-R-R-R-R-R-R-"),
            (SD, "--------L-------L---"),
            (BD, "F-----------F-------"),
        ],
        default_bpm: 110,
        meter: (5, 4),
        ..Drill::BASE
    },
    Drill {
        id: "seven_eight",
        category: Category::Meters,
        lanes: &[
            (HH, "R-R-R-R-R-R-R-"),
            (SD, "----L-------L-"),
            (BD, "F-------F-----"),
        ],
        meter: (7, 8),
        click: Some("X---x---x-----"),
        ..Drill::BASE
    },
    Drill {
        id: "three_against_two",
        category: Category::Meters,
        lanes: &[(HH, "R-R-R-R-R-R-"), (SD, "L--L--L--L--")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "four_against_three",
        category: Category::Meters,
        lanes: &[(HH, "R--R--R--R--"), (SD, "L---L---L---")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "triplets_over_kick",
        category: Category::Meters,
        lanes: &[(SD, "RLRLRLRLRLRL"), (BD, "F---F---F---F---")],
        default_bpm: 90,
        ..Drill::BASE
    },
    Drill {
        id: "dotted_quarter_kick",
        category: Category::Meters,
        // 6 マスおきなので 3 小節でひと回り
        lanes: &[
            (HH, "R-R-R-R-R-R-R-R-|R-R-R-R-R-R-R-R-|R-R-R-R-R-R-R-R-"),
            (SD, "----L-------L---|----L-------L---|----L-------L---"),
            (BD, "F-----F-----F---|--F-----F-----F-|----F-----F-----"),
        ],
        ..Drill::BASE
    },
    // ---- 体内時計 ----
    Drill {
        id: "gap_1",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        gap: Some((1, 1)),
        ..Drill::BASE
    },
    Drill {
        id: "gap_2",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        gap: Some((2, 2)),
        ..Drill::BASE
    },
    Drill {
        id: "gap_4",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        gap: Some((4, 4)),
        ..Drill::BASE
    },
    Drill {
        id: "gap_long",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        gap: Some((2, 6)),
        ..Drill::BASE
    },
    Drill {
        id: "groove_gap",
        category: Category::InnerClock,
        lanes: &[(HH, HATS_8), (SD, BACKBEAT), (BD, "F-------F-F-----")],
        gap: Some((2, 2)),
        default_bpm: 110,
        ..Drill::BASE
    },
    Drill {
        id: "click_two_four",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        click: Some("----x-------x---"),
        ..Drill::BASE
    },
    Drill {
        id: "click_one",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        click: Some("X---------------"),
        ..Drill::BASE
    },
    Drill {
        id: "click_upbeats",
        category: Category::InnerClock,
        lanes: &[(SD, "R-L-R-L-R-L-R-L-")],
        click: Some("--x---x---x---x-"),
        ..Drill::BASE
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
    /// アクセント（強く叩く音）
    pub accent: bool,
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
            let marked = lane_has_accents(text);
            for (m, bar) in text.split('|').enumerate() {
                let cells: Vec<char> = bar.chars().filter(|c| !c.is_whitespace()).collect();
                for (i, &c) in cells.iter().enumerate() {
                    if let Some(hand) = Hand::of(c, lane) {
                        v.push(PatternNote {
                            measure: m as u32,
                            cell: i as u32,
                            cells: cells.len() as u32,
                            lane,
                            hand,
                            accent: marked && c.is_ascii_uppercase(),
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

    /// アクセントのあるレーン（型に書いた順）
    pub fn accent_lanes(&self) -> Vec<DrumLane> {
        self.lanes
            .iter()
            .filter(|(_, t)| lane_has_accents(t))
            .map(|&(l, _)| l)
            .collect()
    }

    /// アクセントのある型か
    pub fn has_accents(&self) -> bool {
        self.lanes.iter().any(|(_, t)| lane_has_accents(t))
    }

    /// テンポを上げないドリルか（クリックが消える小節があると、新しいテンポが分からないので）
    pub fn fixed_tempo(&self) -> bool {
        self.gap.is_some()
    }

    /// 小節の長さ（4/4 を 1 として）
    pub fn bar_len(&self) -> f64 {
        f64::from(self.meter.0) / f64::from(self.meter.1)
    }

    /// 型の小節のクリック（小節の中の位置 0〜1 と、強いクリックか）
    pub fn bar_clicks(&self) -> Vec<(f64, bool)> {
        match self.click {
            Some(p) => pattern_clicks(p),
            None => self.beat_clicks(),
        }
    }

    /// カウントの小節のクリック。分母が 8 の拍子は型のクリック、ほかは拍ごと
    pub fn count_in_clicks(&self) -> Vec<(f64, bool)> {
        match self.click {
            Some(p) if self.meter.1 == 8 => pattern_clicks(p),
            _ => self.beat_clicks(),
        }
    }

    /// 4 分音符ごと（小節の頭が強い）
    fn beat_clicks(&self) -> Vec<(f64, bool)> {
        let quarters = self.bar_len() * 4.0;
        let n = (quarters - 1e-9).ceil().max(1.0) as u32;
        (0..n).map(|i| (f64::from(i) / quarters, i == 0)).collect()
    }
}

/// レーンの型にアクセントがあるか（小文字の音があれば、大文字がアクセント）
fn lane_has_accents(text: &str) -> bool {
    text.contains(['r', 'l', 'f'])
}

/// クリックの型の文字（X・x・-）から位置と強さ
fn pattern_clicks(p: &str) -> Vec<(f64, bool)> {
    let cells: Vec<char> = p.chars().filter(|c| !c.is_whitespace()).collect();
    let n = cells.len().max(1) as f64;
    cells
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, 'X' | 'x'))
        .map(|(i, &c)| (i as f64 / n, c == 'X'))
        .collect()
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

/// 1 回の基礎練習（ドリル、やり方、手順、段ごとのテンポ）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub drill: &'static Drill,
    pub way: Way,
    /// 型の R と L を入れ替える（左から）
    pub lefty: bool,
    /// 選んだテンポ。テンポを上げるなら最後の段、限界チャレンジは始めの段、ほかはどの段も
    target: u32,
    /// 段ごとのテンポ
    pub bpms: Vec<u32>,
    /// 1 段の小節の数
    pub step_measures: u32,
}

impl Run {
    /// テンポを上げる 1 回（右から）
    pub fn new(drill: &'static Drill, target: u32, rise: u32) -> Self {
        Self::with(drill, Way::Climb, target, rise, false)
    }

    /// やり方 `way` の 1 回。`rise` はテンポを上げるときの 1 段ごとの上げ幅。
    /// ドリルで使えないやり方はテンポを上げる 1 回にする
    pub fn with(drill: &'static Drill, way: Way, target: u32, rise: u32, lefty: bool) -> Self {
        let way = way.for_drill(drill);
        let target = target.clamp(BPM_MIN, BPM_MAX);
        let (bpms, step_measures) = match way {
            Way::Climb => (step_bpms(drill, target, rise).to_vec(), STEP_MEASURES),
            Way::Limit => {
                let n = (((BPM_MAX - target) / LIMIT_RISE) as usize + 1).min(LIMIT_MAX_STEPS);
                let bpms = (0..n as u32).map(|i| target + LIMIT_RISE * i).collect();
                (bpms, LIMIT_STEP_MEASURES)
            }
            Way::Endurance(minutes) => {
                // 30 秒に近い小節の数で区切り、全体が `minutes` 分に近くなる区切りの数にする
                let bar_s = drill.bar_len() * 4.0 * 60.0 / f64::from(target);
                let per = (ENDURANCE_SEGMENT_S / bar_s).round().max(1.0) as u32;
                let n = (f64::from(minutes.max(1)) * 60.0 / (f64::from(per) * bar_s))
                    .round()
                    .max(1.0) as usize;
                (vec![target; n], per)
            }
            Way::Blind | Way::Echo => (vec![target; STEPS], STEP_MEASURES),
        };
        Self {
            drill,
            way,
            lefty,
            target,
            bpms,
            step_measures,
        }
    }

    /// 選んだテンポ（テンポを上げるなら最後の段、限界チャレンジは始めの段）
    pub fn target(&self) -> u32 {
        self.target
    }

    /// 段の数
    pub fn steps(&self) -> usize {
        self.bpms.len()
    }

    /// 譜面の小節の数（カウントを入れて）
    pub fn measure_count(&self) -> u32 {
        COUNT_IN_MEASURES + self.steps() as u32 * self.step_measures
    }

    /// 譜面の小節 `measure` が何段目か（カウントは None）
    pub fn step_of(&self, measure: u32) -> Option<usize> {
        let m = measure.checked_sub(COUNT_IN_MEASURES)?;
        let s = (m / self.step_measures) as usize;
        (s < self.steps()).then_some(s)
    }

    /// 段の中の何小節目か（0 から。カウントは None）
    pub fn measure_in_step(&self, measure: u32) -> Option<u32> {
        self.step_of(measure)?;
        Some((measure - COUNT_IN_MEASURES) % self.step_measures)
    }

    /// 段 `step` の譜面の小節の範囲
    pub fn step_range(&self, step: usize) -> std::ops::Range<u32> {
        let first = COUNT_IN_MEASURES + step as u32 * self.step_measures;
        first..first + self.step_measures
    }

    /// お手本の小節か（聞いてまねるの、段の中の 1・3 小節目）
    pub fn is_demo(&self, measure: u32) -> bool {
        self.way == Way::Echo && self.measure_in_step(measure).is_some_and(|i| i % 2 == 0)
    }

    /// 譜面の小節 `measure` の型の小節（お手本の小節は流す型の小節。カウントと終わりの後は None）。
    /// 聞いてまねるは、お手本と自分の 2 小節で型の同じ小節を使う
    pub fn pattern_measure(&self, measure: u32) -> Option<u32> {
        self.step_of(measure)?;
        let i = measure - COUNT_IN_MEASURES;
        let i = if self.way == Way::Echo { i / 2 } else { i };
        Some(i % self.drill.measures())
    }

    /// 型の音符（左からなら R と L を入れ替えて）
    pub fn notes(&self) -> Vec<PatternNote> {
        let mut v = self.drill.notes();
        if self.lefty {
            for n in &mut v {
                n.hand = n.hand.mirrored();
            }
        }
        v
    }

    /// 譜面の小節 `measure` の音符の濃さ（見ないでは段ごとに薄く、ほかは 1）
    pub fn note_alpha(&self, measure: u32) -> f32 {
        match (self.way, self.step_of(measure)) {
            (Way::Blind, Some(s)) => BLIND_ALPHA[s.min(STEPS - 1)],
            _ => 1.0,
        }
    }

    /// お手本の音（譜面の小節、小節の中の位置 0〜1、レーン）。聞いてまねるの、お手本の小節だけ
    pub fn demo_notes(&self) -> Vec<(u32, f64, DrumLane)> {
        if self.way != Way::Echo {
            return Vec::new();
        }
        let notes = self.drill.notes();
        (COUNT_IN_MEASURES..self.measure_count())
            .filter(|&m| self.is_demo(m))
            .flat_map(|m| {
                let pm = self.pattern_measure(m);
                notes
                    .iter()
                    .filter(move |n| Some(n.measure) == pm)
                    .map(move |n| (m, n.pos(), n.lane))
            })
            .collect()
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

    /// 譜面の小節 `measure` で鳴らすクリック（小節の中の位置 0〜1 と、強いクリックか）。消える小節は空
    pub fn clicks_in(&self, measure: u32) -> Vec<(f64, bool)> {
        if measure >= self.measure_count() || !self.clicks(measure) {
            Vec::new()
        } else if measure < COUNT_IN_MEASURES {
            self.drill.count_in_clicks()
        } else {
            self.drill.bar_clicks()
        }
    }

    /// 譜面の音符（小節と位置、レーン）を叩く手足。型に無い音符とお手本の小節は None
    pub fn hand_at(&self, at: NotePos, lane: DrumLane) -> Option<Hand> {
        self.note_at(at, lane).map(|n| n.hand)
    }

    /// 譜面の音符（小節と位置、レーン）にあたる型の音符（左からなら手を入れ替えて）。
    /// 型に無い音符とお手本の小節は None
    pub fn note_at(&self, at: NotePos, lane: DrumLane) -> Option<PatternNote> {
        if self.is_demo(at.measure) {
            return None;
        }
        let pm = self.pattern_measure(at.measure)?;
        self.notes().into_iter().find(|n| {
            n.measure == pm
                && n.lane == lane
                && u64::from(n.cell) * u64::from(at.den) == u64::from(at.num) * u64::from(n.cells)
        })
    }

    /// 譜面の文字（DTX）。0 小節目はカウントで音符を置かず、段の頭でテンポを変える。
    /// お手本の小節にも音符を置かない（音は `demo_notes` から鳴らし、判定しない）
    pub fn to_dtx(&self, title: &str) -> String {
        let mut s = String::new();
        s += &format!(
            "#TITLE: {title}\n#ARTIST: DTXmaniaNXT\n#BPM: {}\n",
            self.bpms[0]
        );
        for (i, b) in self.bpms.iter().enumerate() {
            s += &format!("#BPM{}: {b}\n", base36x2(i + 1));
        }
        s += "#00008: 01\n";
        if self.drill.meter != (4, 4) {
            let len = self.drill.bar_len();
            for m in 0..self.measure_count() {
                s += &format!("#{m:03}02: {len}\n");
            }
        }
        for step in 0..self.steps() {
            let first = self.step_range(step).start;
            s += &format!("#{first:03}08: {}\n", base36x2(step + 1));
        }
        for m in COUNT_IN_MEASURES..self.measure_count() {
            if self.is_demo(m) {
                continue;
            }
            let Some(pm) = self.pattern_measure(m) else {
                continue;
            };
            for &(lane, text) in self.drill.lanes {
                let Some(bar) = text.split('|').nth(pm as usize) else {
                    continue;
                };
                let cells: String = bar
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .map(|c| {
                        if Hand::of(c, lane).is_some() {
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

/// DTX の 2 桁の 36 進数（`#BPMzz` の番号）
fn base36x2(n: usize) -> String {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let n = n.min(36 * 36 - 1);
    format!("{}{}", DIGITS[n / 36] as char, DIGITS[n % 36] as char)
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
            m if m < MANY_MISSES => Self::Few,
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
    /// 叩いた強さ（MIDI のベロシティ）。見逃しと、強さの届かない入力（キーボード）は None
    pub velocity: Option<u8>,
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

    /// 最後まで叩いた段の出来（途中の段は None）。3 つ落とした段は、途中でも × に決まる
    pub fn grade(&self) -> Option<Grade> {
        if self.misses() >= MANY_MISSES {
            return Some(Grade::Many);
        }
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
    pub lane: DrumLane,
    /// 型でアクセントの音か
    pub accent: bool,
    /// 叩いた強さ。見逃しと強さの届かない入力は None
    pub velocity: Option<u8>,
    /// アクセントが出たか。アクセントの音で、強さが届き、まわりと比べられたときだけ
    pub stood_out: Option<bool>,
}

/// アクセントが出た数と、見た数
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccentCount {
    pub stood_out: u32,
    pub judged: u32,
}

impl AccentCount {
    /// 出た割合（0〜1）。見たアクセントが無ければ None
    pub fn rate(&self) -> Option<f64> {
        (self.judged > 0).then(|| f64::from(self.stood_out) / f64::from(self.judged))
    }
}

/// 型の 1 つの音符の、強さの平均
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeCell {
    pub note: PatternNote,
    /// 強さの平均。強さの届いた打が無ければ None
    pub mean: Option<f64>,
    /// 強さの届いた打の数
    pub hits: u32,
}

/// アクセントのあるレーン 1 つの、型の小節の強さの形
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub lane: DrumLane,
    /// ふつうの強さ（このレーンのアクセントでない音の中央値）
    pub normal: Option<f64>,
    /// 型の音符ごと（型の小節・位置の順）
    pub cells: Vec<ShapeCell>,
}

/// 叩く強さのまとめ（アクセントのある型で、強さが届いたときだけ）
#[derive(Debug, Clone, PartialEq)]
pub struct Strength {
    /// 段ごとのアクセント
    pub steps: Vec<AccentCount>,
    /// アクセントのあるレーンごとの、型の小節の強さの形（型に書いた順）
    pub shapes: Vec<Shape>,
}

impl Strength {
    /// 全部の段のアクセント
    pub fn total(&self) -> AccentCount {
        self.steps
            .iter()
            .fold(AccentCount::default(), |a, s| AccentCount {
                stood_out: a.stood_out + s.stood_out,
                judged: a.judged + s.judged,
            })
    }
}

/// 1 回の結果
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub steps: Vec<StepResult>,
    /// 型に出てくる手足だけ（右・左・足の順）
    pub hands: Vec<HandLag>,
    pub dots: Vec<Dot>,
    /// 叩く強さ。アクセントの無い型と、強さの届かない入力は None
    pub strength: Option<Strength>,
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
        !self.steps.is_empty() && self.steps.iter().all(|s| s.completed)
    }

    /// 最近叩いたアクセントのあるレーンの打（強さの届いたもの）を、古い順に `n` まで
    pub fn recent_strokes(&self, lanes: &[DrumLane], n: usize) -> Vec<&Dot> {
        let mut v: Vec<&Dot> = self
            .dots
            .iter()
            .rev()
            .filter(|d| d.velocity.is_some() && lanes.contains(&d.lane))
            .take(n)
            .collect();
        v.reverse();
        v
    }
}

/// 判定した音符の結果をまとめる。`now_measure` は今の小節（その小節より前の段は最後まで叩いた）
pub fn summarize(run: &Run, results: &[NoteOutcome], now_measure: u32) -> Summary {
    let mut steps: Vec<StepResult> = run
        .bpms
        .iter()
        .enumerate()
        .map(|(i, &bpm)| StepResult {
            bpm,
            tally: Tally::default(),
            notes: 0,
            completed: now_measure >= run.step_range(i).end,
            offset_mean_ms: None,
            offset_sd_ms: None,
        })
        .collect();
    let mut step_offsets: Vec<Vec<f64>> = vec![Vec::new(); run.steps()];
    let mut hand_offsets: Vec<(Hand, Vec<f64>)> = Vec::new();
    let notes = run.notes();
    let pattern_hands: Vec<Hand> = Hand::ALL
        .into_iter()
        .filter(|h| notes.iter().any(|n| n.hand == *h))
        .collect();
    for &h in &pattern_hands {
        hand_offsets.push((h, Vec::new()));
    }
    // 叩いた順の点と、それぞれの型の音符
    let mut dots: Vec<(Dot, Option<PatternNote>)> = Vec::new();
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
        let note = run.note_at(r.at, r.lane);
        let hand = note.map(|n| n.hand);
        if let Some(o) = offset {
            step_offsets[s].push(o);
            if let Some(h) = hand
                && let Some((_, v)) = hand_offsets.iter_mut().find(|(x, _)| *x == h)
            {
                v.push(o);
            }
        }
        dots.push((
            Dot {
                measure: r.at.measure,
                pos: f64::from(r.at.num) / f64::from(r.at.den.max(1)),
                hand,
                offset_ms: offset,
                lane: r.lane,
                accent: note.is_some_and(|n| n.accent),
                velocity: r.velocity.filter(|_| r.judgement != Judgement::Miss),
                stood_out: None,
            },
            note,
        ));
    }
    for (s, offs) in steps.iter_mut().zip(&step_offsets) {
        (s.offset_mean_ms, s.offset_sd_ms) = mean_and_sd(offs);
    }
    dots.sort_by(|a, b| {
        (a.0.measure, a.0.pos)
            .partial_cmp(&(b.0.measure, b.0.pos))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let strength = strength(run, &mut dots);
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
    Summary {
        steps,
        hands,
        dots: dots.into_iter().map(|(d, _)| d).collect(),
        strength,
    }
}

/// 叩く強さをまとめ、点ごとにアクセントが出たかを書く（`dots` は位置の順）。
/// アクセントの無い型と、強さの届かない入力は None
fn strength(run: &Run, dots: &mut [(Dot, Option<PatternNote>)]) -> Option<Strength> {
    let lanes = run.drill.accent_lanes();
    if lanes.is_empty() {
        return None;
    }
    let strokes: Vec<accent::Stroke> = dots
        .iter()
        .map(|(d, _)| accent::Stroke {
            at: f64::from(d.measure) + d.pos,
            lane: d.lane,
            accent: d.accent,
            velocity: d.velocity,
        })
        .collect();
    let on_lanes: Vec<accent::Stroke> = strokes
        .iter()
        .copied()
        .filter(|s| lanes.contains(&s.lane))
        .collect();
    if !accent::has_strength(&on_lanes) {
        return None;
    }
    let mut steps = vec![AccentCount::default(); run.steps()];
    for ((d, _), j) in dots.iter_mut().zip(accent::judge(&strokes)) {
        d.stood_out = j;
        if let (Some(b), Some(s)) = (j, run.step_of(d.measure)) {
            steps[s].judged += 1;
            steps[s].stood_out += u32::from(b);
        }
    }
    // 型の音符（小節・マス・レーン）ごとの、強さの合計と数
    let mut sums: std::collections::HashMap<(u32, u32, DrumLane), (f64, u32)> =
        std::collections::HashMap::new();
    for (d, n) in dots.iter() {
        if let (Some(n), Some(v)) = (n, d.velocity) {
            let e = sums.entry((n.measure, n.cell, n.lane)).or_default();
            e.0 += f64::from(v);
            e.1 += 1;
        }
    }
    let notes = run.notes();
    let shapes = lanes
        .iter()
        .map(|&lane| {
            let cells = notes
                .iter()
                .filter(|n| n.lane == lane)
                .map(|&note| {
                    let (sum, hits) = sums
                        .get(&(note.measure, note.cell, lane))
                        .copied()
                        .unwrap_or_default();
                    ShapeCell {
                        note,
                        mean: (hits > 0).then(|| sum / f64::from(hits)),
                        hits,
                    }
                })
                .collect();
            let normal = accent::median(
                dots.iter()
                    .filter(|(d, _)| d.lane == lane && !d.accent)
                    .filter_map(|(d, _)| d.velocity.map(f64::from))
                    .collect(),
            );
            Shape {
                lane,
                normal,
                cells,
            }
        })
        .collect();
    Some(Strength { steps, shapes })
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
            let (num, den) = d.meter;
            for (lane, text) in d.lanes {
                assert_eq!(text.split('|').count() as u32, m, "{} {lane:?}", d.id);
                for bar in text.split('|') {
                    assert!(
                        bar.chars().all(|c| "RLFrlf-".contains(c)),
                        "{} {lane:?} {bar}",
                        d.id
                    );
                    // 4 分音符ひとつのマスの数は 3〜6（3 連・16 分・5 連・6 連）
                    let cells = bar.len() as u32 * den;
                    assert_eq!(cells % (num * 4), 0, "{} {lane:?} {bar}", d.id);
                    assert!(
                        (3..=6).contains(&(cells / (num * 4))),
                        "{} {lane:?} {bar}",
                        d.id
                    );
                    // 足の文字は足のレーンだけ、手の文字は足のレーンに書かない
                    let foot = matches!(*lane, BD | LBD | LP);
                    assert!(
                        bar.chars()
                            .all(|c| c == '-' || c.eq_ignore_ascii_case(&'F') == foot),
                        "{} {lane:?} {bar}",
                        d.id
                    );
                }
            }
            if let Some(p) = d.click {
                assert!(p.chars().all(|c| "Xx-".contains(c)), "{}", d.id);
                assert!(p.contains(['X', 'x']), "{}", d.id);
            }
            // 同じ時刻に同じ手で 2 つ叩かない
            let notes = d.notes();
            for (i, a) in notes.iter().enumerate() {
                for b in &notes[i + 1..] {
                    let same_time = a.measure == b.measure
                        && u64::from(a.cell) * u64::from(b.cells)
                            == u64::from(b.cell) * u64::from(a.cells);
                    assert!(!(same_time && a.hand == b.hand), "{} {a:?} {b:?}", d.id);
                }
            }
            assert!(!notes.is_empty());
            assert!((BPM_MIN..=BPM_MAX).contains(&d.default_bpm));
            assert_eq!(d.default_bpm % BPM_STEP, 0);
        }
        // どの分け方にもドリルがある
        for c in Category::ALL {
            assert!(DRILLS.iter().any(|d| d.category == c), "{c:?}");
        }
        assert_eq!(DRILLS.len(), 118);
    }

    #[test]
    fn the_first_drills_keep_their_ids() {
        // 記録と設定は id で引くので、前からあるドリルの id は変えない
        for id in [
            "quarter",
            "eighth",
            "sixteenth",
            "triplet",
            "upbeat",
            "subdivisions",
            "single_stroke",
            "double_stroke",
            "paradiddle",
            "double_paradiddle",
            "paradiddle_diddle",
            "six_stroke",
            "eight_beat",
            "sixteen_beat",
            "four_on_the_floor",
            "shuffle",
            "hand_foot_triplet",
            "gap_2",
            "gap_4",
        ] {
            assert!(Drill::find(id).is_some(), "{id}");
        }
    }

    #[test]
    fn left_pedal_and_left_bass_are_the_left_foot() {
        let d = drill("double_bass_eighths");
        let hands: Vec<(DrumLane, Hand)> = d.notes().iter().map(|n| (n.lane, n.hand)).collect();
        assert!(hands.contains(&(BD, Hand::Foot)));
        assert!(hands.contains(&(LBD, Hand::LeftFoot)));
        let d = drill("hihat_foot");
        assert!(
            d.notes()
                .iter()
                .any(|n| n.lane == LP && n.hand == Hand::LeftFoot)
        );
        assert!(Hand::LeftFoot.is_foot() && !Hand::Left.is_foot());
    }

    #[test]
    fn meters_set_the_bar_length_and_the_clicks() {
        let run = Run::new(drill("waltz"), 120, 0);
        let chart = dtx::parse(&run.to_dtx("x"));
        let tempo = dtx::TempoMap::build(&chart);
        // 3/4 の小節は 4 分音符 3 つ（120 BPM で 1.5 秒）
        let bar = tempo.measure_start_ms(2) - tempo.measure_start_ms(1);
        assert!((bar - 1500.0).abs() < 1e-6, "{bar}");
        let notes = tempo.drum_notes(&chart);
        assert_eq!(notes.len(), (6 + 2 + 1) * 16);
        assert!(notes.iter().all(|n| run.hand_at(n.at, n.lane).is_some()));
        assert_eq!(run.clicks_in(0).len(), 3);
        assert_eq!(run.clicks_in(1).len(), 3);
        // 7/8 はカウントも 2+2+3 で鳴らす
        let run = Run::new(drill("seven_eight"), 100, 10);
        let at: Vec<f64> = run.clicks_in(0).iter().map(|c| c.0).collect();
        assert_eq!(at, [0.0, 4.0 / 14.0, 8.0 / 14.0]);
        assert_eq!(run.clicks_in(5), run.clicks_in(0));
        let chart = dtx::parse(&run.to_dtx("x"));
        let tempo = dtx::TempoMap::build(&chart);
        let bar = tempo.measure_start_ms(1) - tempo.measure_start_ms(0);
        assert!((bar - 3.5 * 60_000.0 / 70.0).abs() < 1e-6, "{bar}");
        // 2・4 拍のクリックは、カウントだけ拍ごと
        let run = Run::new(drill("click_two_four"), 100, 0);
        assert_eq!(run.clicks_in(0).len(), 4);
        assert_eq!(run.clicks_in(1), [(0.25, false), (0.75, false)]);
        // クリックの消える小節は空
        let run = Run::new(drill("gap_1"), 100, 0);
        assert!(!run.clicks_in(1).is_empty());
        assert!(run.clicks_in(2).is_empty());
        assert!(run.clicks_in(run.measure_count()).is_empty());
    }

    #[test]
    fn every_drill_makes_a_chart_whose_notes_all_have_a_hand() {
        for d in DRILLS {
            let run = Run::new(d, d.default_bpm, DEFAULT_RISE);
            let chart = dtx::parse(&run.to_dtx(d.id));
            let tempo = dtx::TempoMap::build(&chart);
            let notes = tempo.drum_notes(&chart);
            let per_pass = d.notes().len();
            let passes = (STEPS as u32 * STEP_MEASURES) as usize / d.measures() as usize;
            let rest = (STEPS as u32 * STEP_MEASURES) % d.measures();
            let extra: usize = d.notes().iter().filter(|n| n.measure < rest).count();
            assert_eq!(notes.len(), per_pass * passes + extra, "{}", d.id);
            assert!(
                notes.iter().all(|n| run.hand_at(n.at, n.lane).is_some()),
                "{}",
                d.id
            );
            assert_eq!(
                tempo.measure_count() as u32,
                run.measure_count(),
                "{}",
                d.id
            );
        }
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

    /// 小節 `m` の長さ（ミリ秒）
    fn bar_ms(tempo: &dtx::TempoMap, m: usize) -> f64 {
        tempo.measure_start_ms(m + 1) - tempo.measure_start_ms(m)
    }

    #[test]
    fn ways_go_round_and_skip_what_does_not_fit() {
        let para = drill("paradiddle");
        let mut seen = vec![Way::Climb];
        let mut w = Way::Climb;
        for _ in 0..Way::ALL.len() - 1 {
            w = w.next_for(para);
            seen.push(w);
        }
        assert_eq!(seen, Way::ALL);
        assert_eq!(Way::Echo.next_for(para), Way::Climb);
        // クリックの消えるドリルは限界チャレンジと聞いてまねるを飛ばす
        let gap = drill("gap_2");
        assert_eq!(Way::Climb.next_for(gap), Way::Endurance(1));
        assert_eq!(Way::Blind.next_for(gap), Way::Climb);
        assert_eq!(Way::Limit.for_drill(gap), Way::Climb);
        assert_eq!(Run::with(gap, Way::Echo, 100, 10, false).way, Way::Climb);
        for w in Way::ALL {
            assert_eq!(Way::from_key(&w.key()), Some(w));
        }
        assert_eq!(Way::from_key("nope"), None);
    }

    #[test]
    fn the_limit_test_adds_five_every_two_bars_up_to_the_cap() {
        let run = Run::with(drill("paradiddle"), Way::Limit, 100, 10, false);
        assert_eq!(run.target(), 100);
        assert_eq!(run.step_measures, LIMIT_STEP_MEASURES);
        assert_eq!(run.steps(), LIMIT_MAX_STEPS);
        assert_eq!(run.bpms[..3], [100, 105, 110]);
        assert_eq!(run.measure_count(), 1 + 2 * LIMIT_MAX_STEPS as u32);
        assert_eq!(run.step_of(3), Some(1));
        assert_eq!(run.step_range(1), 3..5);
        // 上の端で止まる
        let top = Run::with(drill("paradiddle"), Way::Limit, 230, 10, false);
        assert_eq!(top.bpms, vec![230, 235, 240]);
        // 段が 10 を超えても譜面のテンポが合う（#BPM の番号は 36 進数）
        let chart = dtx::parse(&run.to_dtx("limit"));
        let tempo = dtx::TempoMap::build(&chart);
        for (i, &bpm) in run.bpms.iter().enumerate() {
            let m = run.step_range(i).start as usize;
            let want = 4.0 * 60_000.0 / f64::from(bpm);
            assert!((bar_ms(&tempo, m) - want).abs() < 1e-6, "step {i}");
        }
        // 3 つ落とした段は、途中でも × に決まる
        let step = StepResult {
            bpm: 100,
            tally: Tally {
                miss: 3,
                ..Default::default()
            },
            notes: 5,
            completed: false,
            offset_mean_ms: None,
            offset_sd_ms: None,
        };
        assert_eq!(step.grade(), Some(Grade::Many));
    }

    #[test]
    fn endurance_keeps_the_tempo_in_about_thirty_second_parts() {
        let run = Run::with(drill("eighth"), Way::Endurance(2), 100, 10, false);
        assert!(run.bpms.iter().all(|&b| b == 100));
        // 100 BPM の 4/4 は 1 小節 2.4 秒。13 小節（31.2 秒）ずつ 4 つで約 2 分
        assert_eq!(run.step_measures, 13);
        assert_eq!(run.steps(), 4);
        for d in [drill("eighth"), drill("waltz"), drill("seven_eight")] {
            for minutes in ENDURANCE_MINUTES {
                for bpm in [BPM_MIN, 90, 160, BPM_MAX] {
                    let run = Run::with(d, Way::Endurance(minutes), bpm, 0, false);
                    let bar_s = d.bar_len() * 4.0 * 60.0 / f64::from(bpm);
                    let total = f64::from(run.measure_count() - 1) * bar_s;
                    let part = f64::from(run.step_measures) * bar_s;
                    let want = f64::from(minutes) * 60.0;
                    assert!(
                        (total - want).abs() <= want * 0.2,
                        "{} {minutes} {bpm}: {total}",
                        d.id
                    );
                    assert!((15.0..=45.0).contains(&part), "{} {bpm}: {part}", d.id);
                }
            }
        }
    }

    #[test]
    fn eyes_off_fades_the_notes_step_by_step() {
        let run = Run::with(drill("paradiddle"), Way::Blind, 100, 10, false);
        assert!(run.bpms.iter().all(|&b| b == 100));
        let alpha: Vec<f32> = (0..run.steps())
            .map(|i| run.note_alpha(run.step_range(i).start))
            .collect();
        assert_eq!(alpha, BLIND_ALPHA);
        assert_eq!(run.note_alpha(0), 1.0);
        let climb = Run::new(drill("paradiddle"), 100, 10);
        assert_eq!(climb.note_alpha(16), 1.0);
    }

    #[test]
    fn call_and_response_alternates_a_heard_bar_and_your_bar() {
        let run = Run::with(drill("eight_beat"), Way::Echo, 100, 10, false);
        assert!(run.bpms.iter().all(|&b| b == 100));
        let demo: Vec<bool> = (0..6).map(|m| run.is_demo(m)).collect();
        assert_eq!(demo, [false, true, false, true, false, true]);
        assert_eq!(run.pattern_measure(1), run.pattern_measure(2));
        let chart = dtx::parse(&run.to_dtx("echo"));
        let tempo = dtx::TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        // 譜面の音符は自分の小節だけ（8 小節）
        assert!(notes.iter().all(|n| !run.is_demo(n.at.measure)));
        let per_bar = drill("eight_beat").notes().len();
        assert_eq!(notes.len(), per_bar * 8);
        // お手本の音は、お手本の小節に同じ数だけ
        let heard = run.demo_notes();
        assert_eq!(heard.len(), per_bar * 8);
        assert!(heard.iter().all(|&(m, _, _)| run.is_demo(m)));
        assert!(notes.iter().all(|n| run.hand_at(n.at, n.lane).is_some()));
        // 4 小節の型は、お手本と自分の 2 小節ごとに次の小節へ
        let fill = drill("fill_four_bars");
        assert_eq!(fill.measures(), 4);
        let run = Run::with(fill, Way::Echo, 100, 10, false);
        let pm: Vec<Option<u32>> = (0..6).map(|m| run.pattern_measure(m)).collect();
        assert_eq!(pm, [None, Some(0), Some(0), Some(1), Some(1), Some(2)]);
    }

    #[test]
    fn left_lead_swaps_the_hands_but_not_the_feet() {
        let run = Run::with(drill("paradiddle"), Way::Climb, 100, 10, true);
        let chart = dtx::parse(&run.to_dtx("lefty"));
        let notes = dtx::TempoMap::build(&chart).drum_notes(&chart);
        let hands: String = notes[..8]
            .iter()
            .map(|n| run.hand_at(n.at, n.lane).unwrap().letter())
            .collect();
        assert_eq!(hands, "LRLLRLRR");
        let groove = Run::with(drill("eight_beat"), Way::Climb, 100, 10, true);
        let feet = groove.notes().iter().filter(|n| n.hand.is_foot()).count();
        let before = drill("eight_beat")
            .notes()
            .iter()
            .filter(|n| n.hand.is_foot())
            .count();
        assert_eq!(feet, before);
        assert!(
            groove
                .notes()
                .iter()
                .all(|n| n.lane != HH || n.hand == Hand::Left)
        );
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
                    velocity: (!miss).then_some(90),
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
        // アクセントの無い型は強さを見ない
        assert!(s.strength.is_none());
    }

    #[test]
    fn capital_letters_are_accents_only_in_lanes_with_small_letters() {
        let accents: String = drill("paradiddle")
            .notes()
            .iter()
            .map(|n| if n.accent { '>' } else { '-' })
            .collect();
        assert_eq!(accents, ">--->--->--->---");
        // 小文字も同じ手として読む
        let s: String = drill("paradiddle")
            .notes()
            .iter()
            .map(|n| n.hand.letter())
            .collect();
        assert_eq!(s, "RLRRLRLLRLRRLRLL");
        // 全部大文字のレーンはアクセント無し（ビートのスネアとバスドラ）
        let d = drill("eight_beat");
        assert_eq!(d.accent_lanes(), [HH]);
        assert!(d.notes().iter().filter(|n| n.lane != HH).all(|n| !n.accent));
        assert_eq!(d.notes().iter().filter(|n| n.accent).count(), 4);
        assert!(!drill("single_stroke").has_accents());
        assert!(drill("funk_ghost").accent_lanes().contains(&SD));
        // アクセントの型はアクセントの分け方に。どれもアクセントがある
        for d in DRILLS.iter().filter(|d| d.category == Category::Accents) {
            assert!(d.has_accents(), "{}", d.id);
        }
        // 左からにしても、アクセントの位置は変わらない
        let run = Run::with(drill("paradiddle"), Way::Climb, 100, 10, true);
        let n = run.note_at(NotePos::new(1, 4, 16), SD).unwrap();
        assert!(n.accent && n.hand == Hand::Right);
    }

    /// パラディドルを 16 小節。アクセントは `acc`、ふつうの音は `normal` の強さで叩いた（段ごとに変えられる）
    fn paradiddle_hits(acc: [u8; 4], normal: u8, velocity: bool) -> (Run, Vec<NoteOutcome>) {
        let run = Run::new(drill("paradiddle"), 100, 10);
        let mut results = Vec::new();
        for m in 1..=16u32 {
            for c in 0..16u32 {
                let step = ((m - 1) / 4) as usize;
                let v = if c % 4 == 0 { acc[step] } else { normal };
                results.push(NoteOutcome {
                    at: NotePos::new(m, c, 16),
                    lane: SD,
                    judgement: Judgement::Perfect,
                    offset_ms: Some(0.0),
                    velocity: velocity.then_some(v),
                });
            }
        }
        (run, results)
    }

    #[test]
    fn strength_counts_accents_per_step_and_draws_the_bar() {
        // 段が進むほどアクセントが弱くなる（ふつう 60、最後の段は 64 で出ていない）
        let (run, results) = paradiddle_hits([100, 95, 80, 64], 60, true);
        let s = summarize(&run, &results, 17);
        let st = s.strength.as_ref().unwrap();
        let counts: Vec<(u32, u32)> = st.steps.iter().map(|c| (c.stood_out, c.judged)).collect();
        assert_eq!(counts, [(16, 16), (16, 16), (16, 16), (0, 16)]);
        assert_eq!(st.total().stood_out, 48);
        assert!((st.total().rate().unwrap() - 0.75).abs() < 1e-9);
        // 1 小節の形: 型の 16 の音符、アクセントの所は 4 段の平均
        let shape = &st.shapes[0];
        assert_eq!(shape.lane, SD);
        assert_eq!(shape.cells.len(), 16);
        assert_eq!(shape.normal, Some(60.0));
        let first = shape.cells[0];
        assert!(first.note.accent);
        assert_eq!(first.hits, 16);
        assert!((first.mean.unwrap() - (100.0 + 95.0 + 80.0 + 64.0) / 4.0).abs() < 1e-9);
        assert_eq!(shape.cells[1].mean, Some(60.0));
        // 点にも、アクセントが出たかを書く
        assert_eq!(s.dots[0].stood_out, Some(true));
        assert_eq!(s.dots[1].stood_out, None);
        assert_eq!(s.dots.last().unwrap().velocity, Some(60));
        let last_accent = s.dots.iter().rev().find(|d| d.accent).unwrap();
        assert_eq!(last_accent.stood_out, Some(false));
        // 最近の打（アクセントのあるレーンだけ、古い順）
        let recent = s.recent_strokes(&[SD], 4);
        assert_eq!(recent.len(), 4);
        assert!(recent[0].accent && !recent[3].accent);
        assert!(s.recent_strokes(&[HH], 4).is_empty());
    }

    #[test]
    fn no_strength_without_velocity() {
        // キーボード（強さが届かない）
        let (run, results) = paradiddle_hits([100; 4], 60, false);
        let s = summarize(&run, &results, 17);
        assert!(s.strength.is_none());
        assert!(s.dots.iter().all(|d| d.stood_out.is_none()));
        // どの打も同じ強さで届く機器
        let (run, results) = paradiddle_hits([100; 4], 100, true);
        assert!(summarize(&run, &results, 17).strength.is_none());
        // 強さがあれば、タイミングの出来はこれまでと同じ
        let (run, results) = paradiddle_hits([60; 4], 60, true);
        let flat = summarize(&run, &results, 17);
        assert!(flat.strength.is_none());
        let (run, results) = paradiddle_hits([62; 4], 60, true);
        let weak = summarize(&run, &results, 17);
        assert_eq!(weak.strength.as_ref().unwrap().total().stood_out, 0);
        assert_eq!(weak.best_clean_bpm(), flat.best_clean_bpm());
    }
}
