//! 設定の項目の表。1 項目 1 件で、config.toml のキー・分類・日本語の名前・種類（ON/OFF・選択肢・数・まとめて切り替え）・
//! 選択肢か範囲と刻み・既定・説明・演奏中に変えてよいか を持つ。設定の画面はこの表から行を作る。
//! 値は config.toml の `[settings]` の表にキーごとに書く。書いていないもの、読めないものは既定。
//!
//! NX から移す設定はここに足していく（棚卸し: https://claude.ai/artifact/AH4xxgyx2hqybH6Q5Kb1rw ）。
//! 前からある設定（曲のフォルダ、補正、音量など）は `Config` の欄のまま。

use std::collections::BTreeMap;

use dtx::DrumLane;
use engine::judge::{BdGroup, CyGroup, FtGroup, HhGroup, HitWindows, LaneGroups};
use serde::{Deserialize, Serialize};

/// 分類（設定の画面の左の列。名前は [`Group::label`]）。
/// 左の列は NX v2.0 と同じく「システム」と「ドラム」の 2 段に分ける（2026-11-22 ユーザーが見本から案 A を選んだ）。
/// システムは NX の System ＞ Graphics・Audio・Gameplay と同じ小分け、ドラムは NX の Drums にあったもの
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// システム ＞ 画面（窓の出し方、垂直同期、譜面の動画。NX の System ＞ Graphics）
    Display,
    /// システム ＞ 音（音量、鳴らすもの。NX の System ＞ Audio）
    Sound,
    /// システム ＞ ゲームプレイ（演奏の決まり、判定の幅、ずれの表示、記録。NX の System ＞ Gameplay と HAZARD・Tight）
    Play,
    /// システム ＞ 曲
    Songs,
    /// システム ＞ アプリ
    App,
    /// ドラム ＞ 譜面（流れ方、見えにくくする、レーンとチップ、画面の部品）
    View,
    /// ドラム ＞ 自動・叩き分け（自動で叩く、叩き分け、共通にしたときの音、Muting LP）
    Pads,
    /// ドラム ＞ タイミング（補正）
    Timing,
    /// ドラム ＞ 入力（パッドとキーの割り当て、MIDI 機器）
    Input,
    /// 練習
    Practice,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::Display => "画面",
            Self::Sound => "音",
            Self::Play => "ゲームプレイ",
            Self::Songs => "曲",
            Self::App => "アプリ",
            Self::View => "譜面",
            Self::Pads => "自動・叩き分け",
            Self::Timing => "タイミング",
            Self::Input => "入力",
            Self::Practice => "練習",
        }
    }
}

/// 選択肢 1 つ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// config.toml に書く名前
    pub key: &'static str,
    /// 画面に出す名前
    pub label: &'static str,
}

/// 項目の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// ON / OFF
    Toggle { default: bool },
    /// 選択肢から 1 つ（設定の画面では ◀▶ で隣へ。端で止まる）
    Choice {
        choices: &'static [Choice],
        default: usize,
    },
    /// 数（設定の画面では ◀▶ で `step` ずつ）
    Number {
        min: i64,
        max: i64,
        step: i64,
        default: i64,
        unit: &'static str,
        /// 0 のときに数の代わりに出す名前（Risky の「OFF」など）
        zero: Option<&'static str>,
    },
    /// ほかの ON / OFF をまとめて切り替える。値は全部 ON なら ON、全部 OFF なら OFF、混ざっていれば「-」
    /// （上流の三つの状態の項目 `CItemThreeState`）
    All { members: &'static [&'static str] },
}

/// 設定の項目 1 つ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Def {
    /// config.toml の `[settings]` のキー
    pub key: &'static str,
    pub group: Group,
    /// 分類の中の小見出し（無ければ空）
    pub section: &'static str,
    /// 画面に出す名前
    pub name: &'static str,
    pub kind: Kind,
    /// 説明のパネルに出す文
    pub desc: &'static str,
    /// 演奏中に変えてよいか
    pub in_play: bool,
}

/// 自動演奏のレーン 1 つ
const fn auto_lane(key: &'static str, name: &'static str) -> Def {
    Def {
        key,
        group: Group::Pads,
        section: AUTO_SECTION,
        name,
        kind: Kind::Toggle { default: false },
        desc: AUTO_LANE_DESC,
        in_play: false,
    }
}

const AUTO_SECTION: &str = "自動で叩く";

/// 早送り・巻き戻しの幅（ミリ秒）
pub const SKIP_MS: &str = "skip_ms";
/// 窓と画面の保存（上流の FullScreen、VSyncWait、AutoSaveResult）
pub const DISPLAY_MODE: &str = "display_mode";
pub const VSYNC: &str = "vsync";
pub const AUTO_SAVE_RESULT: &str = "auto_save_result";

/// ゲージの減り方（MISS の減りにかける数を選ぶ）
pub const DAMAGE_LEVEL: &str = "damage_level";
/// ミスの許容数（0 なら使わない）
pub const RISKY: &str = "risky";
/// GREAT と GOOD でもゲージが減る
pub const HAZARD: &str = "hazard";
/// 空打ちをミスにする
pub const TIGHT: &str = "tight";

/// 判定の幅（PERFECT・GREAT・GOOD・POOR）のキー。ペダル以外とペダル
pub const HIT_KEYS: [&str; 4] = ["hit_perfect", "hit_great", "hit_good", "hit_poor"];
pub const PEDAL_HIT_KEYS: [&str; 4] = [
    "pedal_hit_perfect",
    "pedal_hit_great",
    "pedal_hit_good",
    "pedal_hit_poor",
];

/// 譜面の見た目（上流のドラムの設定）
pub const JUDGE_LINE_POS: &str = "judge_line_pos";
pub const REVERSE: &str = "reverse";
pub const HID_SUD: &str = "hid_sud";
pub const SHUTTER_IN: &str = "shutter_in";
pub const SHUTTER_OUT: &str = "shutter_out";
pub const DARK: &str = "dark";
pub const LANE_DISP: &str = "lane_disp";
pub const JUDGE_LINE_DISP: &str = "judge_line_disp";
pub const LANE_FLUSH: &str = "lane_flush";
pub const ATTACK_EFFECT: &str = "attack_effect";
pub const JUDGE_POSITION: &str = "judge_position";
pub const COMBO_DISP: &str = "combo_disp";
pub const MIN_COMBO: &str = "min_combo";
pub const LANE_TYPE: &str = "lane_type";
pub const RD_POSITION: &str = "rd_position";
pub const HHO_GRAPHICS: &str = "hho_graphics";
pub const LBD_GRAPHICS: &str = "lbd_graphics";
/// スネアのリムの音符の出し方
pub const RIM_GRAPHICS: &str = "rim_graphics";
/// ずれのバー（叩いた音符のずれを、判定の線の時刻を真ん中にした横の目盛りに印で出す）
pub const HIT_ERROR_BAR: &str = "hit_error_bar";

/// 選択肢の表を `[key, label]` の並びから作る
macro_rules! choices {
    ($($key:literal => $label:literal),+ $(,)?) => {
        &[$(Choice { key: $key, label: $label }),+]
    };
}

/// 画面の出し方（並びは [`DisplayMode`] と同じ）
const DISPLAY_MODE_CHOICES: &[Choice] = choices![
    "window" => "ウィンドウ",
    "borderless" => "ボーダーレス（疑似全画面）",
    "fullscreen" => "全画面",
];

/// 0.4.2 までの全画面の ON / OFF のキー（読むだけ。[`Values::upgrade`] で画面の出し方に写す）
const OLD_FULLSCREEN: &str = "fullscreen";

/// 画面の出し方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    /// 枠のある窓
    Window,
    /// 枠のない窓を画面いっぱいに（OS とドライバーには普通の窓）
    Borderless,
    /// OS の全画面（解像度は変えない）
    Fullscreen,
}

impl DisplayMode {
    const ALL: [Self; 3] = [Self::Window, Self::Borderless, Self::Fullscreen];

    /// config.toml に書く名前
    pub fn name(self) -> &'static str {
        DISPLAY_MODE_CHOICES[self as usize].key
    }

    /// [`DisplayMode::name`] から（読めなければ `None`）
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// 上流 `nHidSud`（並びは上流と同じ）
pub const HID_SUD_CHOICES: &[Choice] = choices![
    "off" => "OFF",
    "hidden" => "HIDDEN（途中から消える）",
    "sudden" => "SUDDEN（途中から出る）",
    "hid_sud" => "HID-SUD（途中だけ見える）",
    "stealth" => "STEALTH（見えない）",
];

/// 上流 `EDarkMode`
const DARK_CHOICES: &[Choice] = choices![
    "off" => "OFF",
    "half" => "HALF",
    "full" => "FULL",
];

/// 上流 `nLaneDisp`
pub const LANE_DISP_CHOICES: &[Choice] = choices![
    "all_on" => "全部出す",
    "lane_off" => "レーンを暗く",
    "line_off" => "小節線を消す",
    "all_off" => "両方",
];

/// 上流 `eAttackEffect`
pub const ATTACK_EFFECT_CHOICES: &[Choice] = choices![
    "all_on" => "全部出す",
    "chip_off" => "飛ぶチップを消す",
    "effect_only" => "火花だけ",
    "all_off" => "全部消す",
];

/// 上流 `JudgementStringPosition`
pub const JUDGE_POSITION_CHOICES: &[Choice] = choices![
    "lane" => "レーンの上（P-A）",
    "below" => "判定ラインの下（P-B）",
    "off" => "出さない",
];

/// 上流 `eLaneType.Drums`（名前は HH と LT のあいだの並び）
const LANE_TYPE_CHOICES: &[Choice] = choices![
    "a" => "A（LP SD HT BD）",
    "b" => "B（SD LP BD HT）",
    "c" => "C（LP SD BD HT）",
    "d" => "D（SD HT LP BD）",
];

/// 上流 `eRDPosition`（並びは上流と逆で、既定の RCRD が先）
const RD_POSITION_CHOICES: &[Choice] = choices![
    "right" => "CY RD（右端が RD）",
    "left_of_cy" => "RD CY（右端が CY）",
];

/// 上流 `eHHOGraphics.Drums`
const HHO_GRAPHICS_CHOICES: &[Choice] = choices![
    "a" => "A（NX の絵）",
    "b" => "B（丸なし）",
    "c" => "C（クローズと同じ）",
];

/// 左バスドラムの出し方。先頭の 2 つは NXT のもの（BD のレーンに水色で出す、LP の右に水色のレーンを足す）、
/// あとの 2 つは上流 `eLBDGraphics.Drums`
const LBD_GRAPHICS_CHOICES: &[Choice] = choices![
    "bd_lane" => "BD のレーンに水色で",
    "own_lane" => "LP の右にレーンを足す（水色）",
    "a" => "LP のレーンに同じ絵（NX の A）",
    "b" => "LP のレーンに紫の絵（NX の B）",
];

/// スネアのリムの音符の出し方（並びは [`crate::rim_look::RimLook`] と同じ）
const RIM_GRAPHICS_CHOICES: &[Choice] = choices![
    "ears" => "両はしに印",
    "hollow" => "縁だけ",
    "white" => "白",
    "off" => "出さない",
];

const VIEW_LANES: &str = "画面の部品";
const VIEW_ORDER: &str = "レーンとチップ";
const VIEW_HIDE: &str = "見えにくくする";

/// 0〜100 の数（判定ラインの高さ、シャッター）
const fn percent(
    key: &'static str,
    section: &'static str,
    name: &'static str,
    desc: &'static str,
) -> Def {
    Def {
        key,
        group: Group::View,
        section,
        name,
        kind: Kind::Number {
            min: 0,
            max: 100,
            step: 1,
            default: 0,
            unit: "",
            zero: None,
        },
        desc,
        in_play: false,
    }
}

/// 譜面の見た目の ON / OFF（既定は ON）
const fn view_toggle(key: &'static str, name: &'static str, desc: &'static str) -> Def {
    Def {
        key,
        group: Group::View,
        section: VIEW_LANES,
        name,
        kind: Kind::Toggle { default: true },
        desc,
        in_play: false,
    }
}

/// 譜面の見た目の選択肢
const fn view_choice(
    key: &'static str,
    section: &'static str,
    name: &'static str,
    choices: &'static [Choice],
    desc: &'static str,
) -> Def {
    Def {
        key,
        group: Group::View,
        section,
        name,
        kind: Kind::Choice {
            choices,
            default: 0,
        },
        desc,
        in_play: false,
    }
}

/// 叩き分け（上流の HH / FT / CY / BD Group と CymbalFree）
pub const HH_GROUP: &str = "hh_group";
pub const FT_GROUP: &str = "ft_group";
pub const CY_GROUP: &str = "cy_group";
pub const BD_GROUP: &str = "bd_group";
pub const CYMBAL_FREE: &str = "cymbal_free";
/// 叩き分けを共通にしたとき、譜面の音とパッドの音のどちらを鳴らすか（上流の HH / FT / CY Priority）
pub const HH_PRIORITY: &str = "hh_priority";
pub const FT_PRIORITY: &str = "ft_priority";
pub const CY_PRIORITY: &str = "cy_priority";
/// 左ペダルで鳴っているハイハットの音を止める（上流の Muting LP）
pub const MUTING_LP: &str = "muting_lp";

const GROUP_SECTION: &str = "叩き分け";
/// 叩き分けを共通にしたとき鳴らす音（NX では Group の隣）
const PRIORITY_SECTION: &str = "共通にしたときの音";

/// 上流 `EHHGroup` の並び
const HH_GROUP_CHOICES: &[Choice] = choices![
    "hh0" => "HH-0（全部叩き分ける）",
    "hh1" => "HH-1（左シンバルはハイハットと共通）",
    "hh2" => "HH-2（ハイハットの開閉は共通）",
    "hh3" => "HH-3（全部共通）",
];
const FT_GROUP_CHOICES: &[Choice] = choices![
    "ft0" => "FT-0（叩き分ける）",
    "ft1" => "FT-1（ロータムと共通）",
];
const CY_GROUP_CHOICES: &[Choice] = choices![
    "cy0" => "CY-0（叩き分ける）",
    "cy1" => "CY-1（ライドと共通）",
];
/// 上流 `EBDGroup` の並び
const BD_GROUP_CHOICES: &[Choice] = choices![
    "bd0" => "BD-0（全部叩き分ける）",
    "bd1" => "BD-1（左バスドラムは右と共通）",
    "bd2" => "BD-2（左ペダルと左バスドラムが共通）",
    "bd3" => "BD-3（全部共通）",
];
/// 上流 `EPlaybackPriority`
const PRIORITY_CHOICES: &[Choice] = choices![
    "chip" => "C>P（譜面の音）",
    "pad" => "P>C（叩いたパッドの音）",
];

/// 叩き分けの選択肢
const fn group_choice(
    key: &'static str,
    name: &'static str,
    choices: &'static [Choice],
    desc: &'static str,
) -> Def {
    Def {
        key,
        group: Group::Pads,
        section: GROUP_SECTION,
        name,
        kind: Kind::Choice {
            choices,
            default: 0,
        },
        desc,
        in_play: false,
    }
}

/// 鳴らす音の優先
const fn priority(key: &'static str, name: &'static str, desc: &'static str) -> Def {
    Def {
        key,
        group: Group::Pads,
        section: PRIORITY_SECTION,
        name,
        kind: Kind::Choice {
            choices: PRIORITY_CHOICES,
            default: 0,
        },
        desc,
        in_play: false,
    }
}

/// 音量と鳴らすもの（上流の ChipVolume、AutoVolume、BGM Sound、BGMAdjust、Metronome）
pub const CHIP_VOLUME: &str = "chip_volume";
pub const AUTO_VOLUME: &str = "auto_volume";
pub const BGM_SOUND: &str = "bgm_sound";
pub const BGM_ADJUST: &str = "bgm_adjust";
pub const METRONOME: &str = "metronome";

/// 音量（0〜100 %、5 刻み）
const fn volume(key: &'static str, name: &'static str, default: i64, desc: &'static str) -> Def {
    Def {
        key,
        group: Group::Sound,
        section: "音量",
        name,
        kind: Kind::Number {
            min: 0,
            max: 100,
            step: 5,
            default,
            unit: " %",
            zero: None,
        },
        desc,
        in_play: false,
    }
}

const RULE_SECTION: &str = "演奏の決まり";

/// ゲージの減り方の選択肢（上流 `EDamageLevel`。並びは MISS の減りにかける数 [`DAMAGE_FACTORS`] と同じ）
const DAMAGE_CHOICES: &[Choice] = &[
    Choice {
        key: "small",
        label: "少ない",
    },
    Choice {
        key: "normal",
        label: "ふつう",
    },
    Choice {
        key: "large",
        label: "多い",
    },
];

/// MISS のゲージの減りにかける数（上流 `fDamageLevelFactor`）
const DAMAGE_FACTORS: [f64; 3] = [0.25, 0.5, 0.75];

/// 判定の幅 1 つ（上流 Config.ini の `[HitRange]`。上流は 0〜999）
const fn hit_window(
    key: &'static str,
    section: &'static str,
    name: &'static str,
    default: i64,
    desc: &'static str,
) -> Def {
    Def {
        key,
        group: Group::Play,
        section,
        name,
        kind: Kind::Number {
            min: 0,
            max: 500,
            step: 1,
            default,
            unit: " ms",
            zero: None,
        },
        desc,
        in_play: false,
    }
}

const HIT_DESC: &str = "音符の時刻から前後この幅までに叩くと、この判定になります（上流の既定は PERFECT 34、GREAT 67、GOOD 84、POOR 117）。\nPOOR の幅の外は空打ちです。狭い判定の幅が広い判定を越えたら、広い判定もそこまで広げて使います。";
const PEDAL_HIT_DESC: &str = "バスドラム・左ペダル・左バスドラムの判定の幅です（上流の DrumPedal〜）。\nほかのレーンと同じく、狭い判定の幅が広い判定を越えたら、広い判定もそこまで広げて使います。";

/// オートのまとめての切り替え
pub const AUTO_ALL: &str = "auto_all";

/// オートの項目のキーと、そのキーで自動で叩くレーン（上流 `bAutoPlay` の LC〜RD。HH は閉と開の両方）。
/// 並びは上流の設定画面と同じ
pub const AUTO_LANES: [(&str, &[DrumLane]); 11] = [
    ("auto_lc", &[DrumLane::LC]),
    ("auto_hh", &[DrumLane::HH, DrumLane::HHO]),
    ("auto_lp", &[DrumLane::LP]),
    ("auto_lbd", &[DrumLane::LBD]),
    ("auto_sd", &[DrumLane::SD]),
    ("auto_bd", &[DrumLane::BD]),
    ("auto_ht", &[DrumLane::HT]),
    ("auto_lt", &[DrumLane::LT]),
    ("auto_ft", &[DrumLane::FT]),
    ("auto_cy", &[DrumLane::CY]),
    ("auto_rd", &[DrumLane::RD]),
];

const AUTO_KEYS: [&str; 11] = {
    let mut keys = [""; 11];
    let mut i = 0;
    while i < AUTO_LANES.len() {
        keys[i] = AUTO_LANES[i].0;
        i += 1;
    }
    keys
};

const AUTO_LANE_DESC: &str = "ON にすると、このレーンの音符を自動で叩きます（音も鳴らします）。ほかのレーンの練習に。\n自動のレーンがあるプレイは、ベストの記録に入れません（ハイハットは閉と開の両方）。";

/// すべての項目（設定の画面の並び）
pub const ALL: &[Def] = &[
    Def {
        key: AUTO_ALL,
        group: Group::Pads,
        section: AUTO_SECTION,
        name: "自動演奏（AutoPlay）",
        kind: Kind::All {
            members: &AUTO_KEYS,
        },
        desc: "ドラムのすべてのレーンの自動演奏を、まとめて ON / OFF にします。下の各レーンで 1 つずつも選べます。\n全部を自動にしたプレイは記録に残しません。",
        in_play: false,
    },
    auto_lane("auto_lc", "　左シンバル"),
    auto_lane("auto_hh", "　ハイハット"),
    auto_lane("auto_lp", "　左ペダル"),
    auto_lane("auto_lbd", "　左バスドラム"),
    auto_lane("auto_sd", "　スネア"),
    auto_lane("auto_bd", "　バスドラム"),
    auto_lane("auto_ht", "　ハイタム"),
    auto_lane("auto_lt", "　ロータム"),
    auto_lane("auto_ft", "　フロアタム"),
    auto_lane("auto_cy", "　右シンバル"),
    auto_lane("auto_rd", "　ライド"),
    Def {
        key: DAMAGE_LEVEL,
        group: Group::Play,
        section: RULE_SECTION,
        name: "ゲージの減り方（DamageLevel）",
        kind: Kind::Choice {
            choices: DAMAGE_CHOICES,
            default: 1,
        },
        desc: "見逃したとき（MISS）にゲージがどれだけ減るかを選びます。\nミスの許容数（Risky）を使うときは効きません。",
        in_play: false,
    },
    Def {
        key: RISKY,
        group: Group::Play,
        section: RULE_SECTION,
        name: "ミスの許容数（Risky）",
        kind: Kind::Number {
            min: 0,
            max: 10,
            step: 1,
            default: 0,
            unit: " 回",
            zero: Some("OFF"),
        },
        desc: "POOR と MISS がこの回数になると、ゲージが尽きて STAGE FAILED になります（STAGE FAILED が ON のとき）。\nゲージは満タンから始まり、ミスのたびに同じだけ減ります。OFF ならふつうのゲージです。",
        in_play: false,
    },
    Def {
        key: HAZARD,
        group: Group::Play,
        section: RULE_SECTION,
        name: "GREAT・GOOD でも減る（HAZARD）",
        kind: Kind::Toggle { default: false },
        desc: "GREAT と GOOD でも、MISS と同じだけゲージが減ります（Risky なら残りの回数が減ります）。\nPERFECT だけを狙うときに。",
        in_play: false,
    },
    Def {
        key: TIGHT,
        group: Group::Play,
        section: RULE_SECTION,
        name: "空打ちをミスに（Tight）",
        kind: Kind::Toggle { default: false },
        desc: "音符の無いところを叩くと MISS を出し、コンボが切れてゲージが減ります（MISS の数には入れません）。\n区間の練習では効きません。",
        in_play: false,
    },
    group_choice(
        HH_GROUP,
        "ハイハットの叩き分け（HH Group）",
        HH_GROUP_CHOICES,
        "左シンバル・ハイハット（閉）・ハイハット（開）を叩き分けるかを選びます。共通にしたものは、どれを叩いても取れます。\n譜面に無いレーンは、叩き分けようがないので共通にします。",
    ),
    group_choice(
        FT_GROUP,
        "フロアタムの叩き分け（FT Group）",
        FT_GROUP_CHOICES,
        "ロータムとフロアタムを叩き分けるかを選びます。",
    ),
    group_choice(
        CY_GROUP,
        "シンバルの叩き分け（CY Group）",
        CY_GROUP_CHOICES,
        "右シンバルとライドを叩き分けるかを選びます。譜面にライドが無ければ共通にします。",
    ),
    group_choice(
        BD_GROUP,
        "ペダルの叩き分け（BD Group）",
        BD_GROUP_CHOICES,
        "左ペダル・左バスドラム・バスドラムを叩き分けるかを選びます。\n左バスドラムにパッドもキーも割り当てていなければ、BD-0 でもバスドラムで左バスドラムを取れます。",
    ),
    Def {
        key: CYMBAL_FREE,
        group: Group::Pads,
        section: GROUP_SECTION,
        name: "シンバルフリー（CymbalFree）",
        kind: Kind::Toggle { default: false },
        desc: "左シンバルと右シンバルを叩き分けません（どちらで叩いても取れます）。\nライドも共通にするかは、シンバルの叩き分けに従います。",
        in_play: false,
    },
    Def {
        key: SKIP_MS,
        group: Group::Play,
        section: "演奏中のキー",
        name: "早送り・巻き戻しの幅（SkipTimeMs）",
        kind: Kind::Number {
            min: 1000,
            max: 20000,
            step: 1000,
            default: 5000,
            unit: " ms",
            zero: None,
        },
        desc: "演奏中に → で先へ、← で前へ、この幅だけ飛びます（練習では区間をずらします）。\n飛んだプレイは記録に残しません。Space か Pause で一時停止します。",
        in_play: true,
    },
    Def {
        key: DISPLAY_MODE,
        group: Group::Display,
        section: "",
        name: "画面の出し方（FullScreen）",
        kind: Kind::Choice {
            choices: DISPLAY_MODE_CHOICES,
            default: 0,
        },
        desc: "ボーダーレスは枠のない窓を画面いっぱいに広げ、ほかの窓へすぐ切り替えたり上に重ねたりできます。\n全画面は絵が少し早く出ることがありますが、切り替えで画面が一瞬暗くなることがあります。どの画面でも Alt+Enter か F11 で、ウィンドウと最後に使った方を行き来します。",
        in_play: true,
    },
    Def {
        key: VSYNC,
        group: Group::Display,
        section: "",
        name: "垂直同期（VSyncWait）",
        kind: Kind::Toggle { default: true },
        desc: "画面の書き換えをモニターに合わせます。OFF にすると絵が少し早く出ますが、横に裂けて見えることがあります。\n判定は MIDI の時刻と音で行うので、どちらでも変わりません。",
        in_play: true,
    },
    Def {
        key: AUTO_SAVE_RESULT,
        group: Group::Play,
        section: "記録",
        name: "記録を更新した結果を画像で残す（AutoSaveResult）",
        kind: Kind::Toggle { default: true },
        desc: "記録を更新したとき、結果の画面を譜面と同じフォルダに PNG で残します（譜面の名前.日時_DRUMS_ランク.png）。\nF12 ではいつでも画面を「ピクチャ」の DTXmaniaNXT に残せます。",
        in_play: false,
    },
    percent(
        JUDGE_LINE_POS,
        "流れ方",
        "判定ラインの高さ（JudgeLinePos）",
        "判定ラインを下から何ドット上げるか（0〜100）。チップと小節線、火花も一緒に動きます。",
    ),
    Def {
        key: REVERSE,
        group: Group::View,
        section: "流れ方",
        name: "下から上へ流す（Reverse）",
        kind: Kind::Toggle { default: false },
        desc: "判定ラインを上に置き、チップを下から上へ流します。判定ラインの高さは下げる向きに。\nシャッターは、チップが出てくる側（下）が「上のシャッター」の値になります。",
        in_play: false,
    },
    view_choice(
        LANE_TYPE,
        VIEW_ORDER,
        "レーンの並び（LaneType）",
        LANE_TYPE_CHOICES,
        "HH と LT のあいだの並び。B はペダルとタム、C はタムをまとめ、D は左右対称。\nCY と RD は「RD の位置」で決まり、パッドや判定の文字も一緒に動きます。",
    ),
    view_choice(
        RD_POSITION,
        VIEW_ORDER,
        "RD の位置（RDPosition）",
        RD_POSITION_CHOICES,
        "ライドシンバルのレーンを右端に置くか、シンバル（CY）の左に置くか。",
    ),
    view_choice(
        HHO_GRAPHICS,
        VIEW_ORDER,
        "オープンハイハットの絵（HHOGraphics）",
        HHO_GRAPHICS_CHOICES,
        "A は NX の絵、B は A のまわりの丸を消した絵、C はクローズのハイハットと同じ絵です。",
    ),
    view_choice(
        LBD_GRAPHICS,
        VIEW_ORDER,
        "左バスドラムの出し方（LBDGraphics）",
        LBD_GRAPHICS_CHOICES,
        "左バスドラム（LBD）の音符を BD のレーンに水色で出すか、LP の右に足したレーン（実物のツインペダルと同じ並び）に出すか、NX と同じく LP のレーンに出すか。\n光・判定の文字・火花も、音符を出したレーンに出します。",
    ),
    view_choice(
        RIM_GRAPHICS,
        VIEW_ORDER,
        "スネアのリムの音符",
        RIM_GRAPHICS_CHOICES,
        "音の名前が Rim や SideStick のスネアの音符を、リムの音符として見分けて出します。\n出さないにすると、普通の音符と同じ絵です。",
    ),
    view_choice(
        HID_SUD,
        VIEW_HIDE,
        "見えにくくする（HID-SUD）",
        HID_SUD_CHOICES,
        "HIDDEN は判定ラインの近くでチップが消え、SUDDEN は判定ラインの近くまで来てから出ます。\nHID-SUD は両方、STEALTH はずっと見えません（譜面を覚える練習に）。",
    ),
    percent(
        SHUTTER_IN,
        VIEW_HIDE,
        "上のシャッター（ShutterInPos）",
        "チップが出てくる側（上）をシャッターでどこまで隠すか（0〜100。100 で画面の高さ全部）。",
    ),
    percent(
        SHUTTER_OUT,
        VIEW_HIDE,
        "下のシャッター（ShutterOutPos）",
        "判定ラインの側（下）をシャッターでどこまで隠すか（0〜100。100 で画面の高さ全部）。",
    ),
    view_choice(
        DARK,
        VIEW_LANES,
        "暗くする（Dark）",
        DARK_CHOICES,
        "下の 3 つをまとめて切り替えます。HALF はレーンを暗くし、FULL はさらに小節線・判定ライン・レーンの光・進み具合のバーを消します。\n選んだあとで下の 3 つを 1 つずつ変えることもできます。",
    ),
    view_choice(
        LANE_DISP,
        VIEW_LANES,
        "レーンと小節線（LaneDisp）",
        LANE_DISP_CHOICES,
        "レーンの背景を暗くするか、小節線と拍線を消すかを選びます。",
    ),
    view_toggle(
        JUDGE_LINE_DISP,
        "判定ライン（JudgeLineDisp）",
        "判定ラインを出すかどうか。",
    ),
    view_toggle(
        LANE_FLUSH,
        "レーンの光（LaneFlush）",
        "叩いたときにレーンを光らせるかどうか。",
    ),
    view_choice(
        ATTACK_EFFECT,
        VIEW_LANES,
        "叩いたときの効果（AttackEffect）",
        ATTACK_EFFECT_CHOICES,
        "音符を取ったときの火花・星・飛び散るチップをどこまで出すか。",
    ),
    view_choice(
        JUDGE_POSITION,
        VIEW_LANES,
        "判定の文字の位置（JudgePosition）",
        JUDGE_POSITION_CHOICES,
        "PERFECT などの文字を、レーンの上か、判定ラインの下に出します。出さないこともできます。",
    ),
    view_toggle(COMBO_DISP, "コンボ（Combo）", "コンボの数を出すかどうか。"),
    Def {
        key: HIT_ERROR_BAR,
        group: Group::Play,
        section: "ずれの表示",
        name: "ずれのバー",
        kind: Kind::Toggle { default: true },
        desc: "左のスコアの下（練習では左下）に、叩いたずれを印で出します。真ん中が ±0 で、左が早く、右が遅いです。\n印の色は判定の色で、少しずつ消えます。▼ と数字は直近 20 打の平均です。",
        in_play: true,
    },
    Def {
        key: MIN_COMBO,
        group: Group::View,
        section: VIEW_LANES,
        name: "コンボを出す数（D-MinCombo）",
        kind: Kind::Number {
            min: 1,
            max: 99999,
            step: 1,
            default: 10,
            unit: "",
            zero: None,
        },
        desc: "コンボがこの数になったら、コンボの数を出します。",
        in_play: false,
    },
    priority(
        HH_PRIORITY,
        "ハイハットの音（HH Priority）",
        "ハイハットや左シンバルを共通にしたとき、別のレーンの音符を取ったら、譜面の音符の音を鳴らすか、叩いたパッドの音を鳴らすかを選びます。",
    ),
    priority(
        FT_PRIORITY,
        "フロアタムの音（FT Priority）",
        "ロータムとフロアタムを共通にしたとき、譜面の音符の音を鳴らすか、叩いたパッドの音を鳴らすかを選びます。",
    ),
    priority(
        CY_PRIORITY,
        "シンバルの音（CY Priority）",
        "シンバルとライドを共通にしたとき、譜面の音符の音を鳴らすか、叩いたパッドの音を鳴らすかを選びます。",
    ),
    Def {
        key: MUTING_LP,
        group: Group::Pads,
        section: "ペダル",
        name: "左ペダルでハイハットを止める（Muting LP）",
        kind: Kind::Toggle { default: true },
        desc: "左ペダルを踏むと、鳴っているハイハットの音（叩いた音）を止めます。",
        in_play: false,
    },
    volume(
        CHIP_VOLUME,
        "叩いた音の音量（ChipVolume）",
        100,
        "叩いたときに鳴らす譜面の音の音量です（全体の音量にさらにかけます）。",
    ),
    volume(
        AUTO_VOLUME,
        "自動で鳴らす音の音量（AutoVolume）",
        80,
        "BGM・効果音・自動で叩くレーンなど、自動で鳴らす譜面の音の音量です（全体の音量にさらにかけます）。NX と同じく既定は 80 %。",
    ),
    Def {
        key: BGM_SOUND,
        group: Group::Sound,
        section: "鳴らすもの",
        name: "BGM（BGM Sound）",
        kind: Kind::Toggle { default: true },
        desc: "OFF にすると、譜面の BGM（チャンネル 01）を鳴らしません。効果音やギター・ベースの音は鳴らします。",
        in_play: false,
    },
    Def {
        key: METRONOME,
        group: Group::Sound,
        section: "鳴らすもの",
        name: "メトロノーム（Metronome）",
        kind: Kind::Toggle { default: false },
        desc: "小節の頭と拍でクリックを鳴らします（拍は小さめ）。区間の練習では、遅くした速さに合わせて鳴ります。",
        in_play: false,
    },
    Def {
        key: BGM_ADJUST,
        group: Group::Timing,
        section: "",
        name: "BGM のずらし（BGMAdjust）",
        kind: Kind::Number {
            min: -99,
            max: 99,
            step: 1,
            default: 0,
            unit: " ms",
            zero: None,
        },
        desc: "BGM と効果音を鳴らす時刻をずらします（プラスで遅く）。譜面と BGM が合っていない曲に。\n-99 〜 99 ms。",
        in_play: false,
    },
    hit_window(
        HIT_KEYS[0],
        "判定の幅",
        "PERFECT（DrumPerfect）",
        34,
        HIT_DESC,
    ),
    hit_window(HIT_KEYS[1], "判定の幅", "GREAT（DrumGreat）", 67, HIT_DESC),
    hit_window(HIT_KEYS[2], "判定の幅", "GOOD（DrumGood）", 84, HIT_DESC),
    hit_window(HIT_KEYS[3], "判定の幅", "POOR（DrumPoor）", 117, HIT_DESC),
    hit_window(
        PEDAL_HIT_KEYS[0],
        "判定の幅（ペダル）",
        "PERFECT（DrumPedalPerfect）",
        34,
        PEDAL_HIT_DESC,
    ),
    hit_window(
        PEDAL_HIT_KEYS[1],
        "判定の幅（ペダル）",
        "GREAT（DrumPedalGreat）",
        67,
        PEDAL_HIT_DESC,
    ),
    hit_window(
        PEDAL_HIT_KEYS[2],
        "判定の幅（ペダル）",
        "GOOD（DrumPedalGood）",
        84,
        PEDAL_HIT_DESC,
    ),
    hit_window(
        PEDAL_HIT_KEYS[3],
        "判定の幅（ペダル）",
        "POOR（DrumPedalPoor）",
        117,
        PEDAL_HIT_DESC,
    ),
];

/// キーの項目。表に無いキーはプログラムの誤り
pub fn def(key: &str) -> &'static Def {
    ALL.iter()
        .find(|d| d.key == key)
        .unwrap_or_else(|| panic!("no setting named {key}"))
}

/// 分類 `group` の項目（表の並び）
pub fn in_group(group: Group) -> impl Iterator<Item = &'static Def> {
    ALL.iter().filter(move |d| d.group == group)
}

/// 既定の値（[`Values::raw`] と同じ形）。まとめての項目は、ほかの既定が全部同じならその値
pub fn default_raw(key: &str) -> Option<i64> {
    match def(key).kind {
        Kind::Toggle { default } => Some(i64::from(default)),
        Kind::Choice { default, .. } => Some(default as i64),
        Kind::Number { default, .. } => Some(default),
        Kind::All { members } => {
            let first = default_raw(members.first()?)?;
            members
                .iter()
                .all(|m| default_raw(m) == Some(first))
                .then_some(first)
        }
    }
}

/// 項目の値（config.toml の `[settings]`）。書いていないキーは既定の値
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Values(BTreeMap<String, toml::Value>);

impl Values {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// ON / OFF の値。まとめての項目は、全部 ON なら true
    pub fn flag(&self, key: &str) -> bool {
        match def(key).kind {
            Kind::Toggle { default } => self
                .0
                .get(key)
                .and_then(toml::Value::as_bool)
                .unwrap_or(default),
            Kind::All { members } => members.iter().all(|m| self.flag(m)),
            _ => false,
        }
    }

    /// 数の値（範囲に収め、刻みにそろえる）
    pub fn number(&self, key: &str) -> i64 {
        let Kind::Number {
            min,
            max,
            step,
            default,
            ..
        } = def(key).kind
        else {
            return 0;
        };
        let v = self
            .0
            .get(key)
            .and_then(toml::Value::as_integer)
            .unwrap_or(default);
        snap(v, min, max, step)
    }

    /// 選択肢の何番目か（読めない名前なら既定）
    pub fn choice(&self, key: &str) -> usize {
        let Kind::Choice { choices, default } = def(key).kind else {
            return 0;
        };
        self.0
            .get(key)
            .and_then(toml::Value::as_str)
            .and_then(|s| choices.iter().position(|c| c.key == s))
            .unwrap_or(default)
    }

    /// 画面に出す値
    pub fn label(&self, key: &str) -> String {
        let on_off = |b: bool| (if b { "ON" } else { "OFF" }).to_owned();
        match def(key).kind {
            Kind::Toggle { .. } => on_off(self.flag(key)),
            Kind::Choice { choices, .. } => choices[self.choice(key)].label.to_owned(),
            Kind::Number { unit, zero, .. } => match (self.number(key), zero) {
                (0, Some(z)) => z.to_owned(),
                (n, _) => format!("{n}{unit}"),
            },
            Kind::All { members } => {
                let on = members.iter().filter(|m| self.flag(m)).count();
                if on == members.len() {
                    on_off(true)
                } else if on == 0 {
                    on_off(false)
                } else {
                    "-".to_owned()
                }
            }
        }
    }

    /// 設定の画面で扱う形の値。ON / OFF は 1 と 0、選択肢は何番目か、数はそのまま。
    /// まとめての項目は全部 ON なら 1、全部 OFF なら 0、混ざっていれば `None`
    pub fn raw(&self, key: &str) -> Option<i64> {
        match def(key).kind {
            Kind::Toggle { .. } => Some(i64::from(self.flag(key))),
            Kind::Choice { .. } => Some(self.choice(key) as i64),
            Kind::Number { .. } => Some(self.number(key)),
            Kind::All { members } => {
                let on = members.iter().filter(|m| self.flag(m)).count();
                match on {
                    0 => Some(0),
                    n if n == members.len() => Some(1),
                    _ => None,
                }
            }
        }
    }

    /// [`Values::raw`] の形で値を決める（範囲の外は端に、数は刻みにそろえる）。
    /// まとめての項目は、ほかの ON / OFF をすべて同じにする
    pub fn set_raw(&mut self, key: &str, v: i64) {
        match def(key).kind {
            Kind::Toggle { .. } => self.set(key, toml::Value::Boolean(v != 0)),
            Kind::Choice { choices, .. } => {
                let i = v.clamp(0, choices.len() as i64 - 1) as usize;
                self.set(key, toml::Value::String(choices[i].key.to_owned()));
                if key == DARK {
                    self.apply_dark(i);
                }
            }
            Kind::Number { min, max, step, .. } => {
                self.set(key, toml::Value::Integer(snap(v, min, max, step)));
            }
            Kind::All { members } => {
                for m in members {
                    self.set(m, toml::Value::Boolean(v != 0));
                }
            }
        }
    }

    /// Dark を選んだら、レーンと小節線・判定ライン・レーンの光をそろえる（上流の設定画面と同じ）
    fn apply_dark(&mut self, dark: usize) {
        let (lanes, lines) = match dark {
            2 => (3, false),
            1 => (1, true),
            _ => (0, true),
        };
        self.set(
            LANE_DISP,
            toml::Value::String(LANE_DISP_CHOICES[lanes].key.to_owned()),
        );
        self.set(JUDGE_LINE_DISP, toml::Value::Boolean(lines));
        self.set(LANE_FLUSH, toml::Value::Boolean(lines));
    }

    /// Dark が FULL（進み具合のバーも消す）
    pub fn dark_full(&self) -> bool {
        self.choice(DARK) == 2
    }

    /// 画面の出し方
    pub fn display_mode(&self) -> DisplayMode {
        DisplayMode::ALL[self.choice(DISPLAY_MODE)]
    }

    pub fn set_display_mode(&mut self, mode: DisplayMode) {
        self.set_raw(DISPLAY_MODE, mode as i64);
    }

    /// 前の版のキーを今のキーに写す（読んだときに 1 回）。写したら true
    ///
    /// - 0.4.2 までの `fullscreen = true` は、画面の出し方の「全画面」
    pub fn upgrade(&mut self) -> bool {
        let Some(old) = self.0.remove(OLD_FULLSCREEN) else {
            return false;
        };
        if !self.0.contains_key(DISPLAY_MODE) && old.as_bool() == Some(true) {
            self.set_display_mode(DisplayMode::Fullscreen);
        }
        true
    }

    fn set(&mut self, key: &str, v: toml::Value) {
        self.0.insert(key.to_owned(), v);
    }

    /// LP の右に左バスドラムのレーンを足す（出し方の 2 つめ）
    pub fn left_bass_lane(&self) -> bool {
        self.choice(LBD_GRAPHICS) == 1
    }

    /// 左バスドラムを LP と分けて出す（BD のレーンか、足したレーン）。分けないのは NX の 2 つ
    pub fn left_bass_apart(&self) -> bool {
        self.choice(LBD_GRAPHICS) <= 1
    }

    /// 叩き分け（上流の HH / FT / CY / BD Group と CymbalFree）
    pub fn lane_groups(&self) -> LaneGroups {
        LaneGroups {
            hh: match self.choice(HH_GROUP) {
                1 => HhGroup::HiHatOnly,
                2 => HhGroup::LeftCymbalOnly,
                3 => HhGroup::Common,
                _ => HhGroup::Separate,
            },
            ft: match self.choice(FT_GROUP) {
                1 => FtGroup::Common,
                _ => FtGroup::Separate,
            },
            cy: match self.choice(CY_GROUP) {
                1 => CyGroup::Common,
                _ => CyGroup::Separate,
            },
            bd: match self.choice(BD_GROUP) {
                1 => BdGroup::BassDrumsCommon,
                2 => BdGroup::LeftPedalsCommon,
                3 => BdGroup::Common,
                _ => BdGroup::Separate,
            },
            cymbal_free: self.flag(CYMBAL_FREE),
        }
    }

    /// 叩き分けを共通にしたとき、叩いたパッドの音を先にするか（ハイハット・フロアタム・シンバルの順）
    pub fn pad_sound_first(&self) -> [bool; 3] {
        [HH_PRIORITY, FT_PRIORITY, CY_PRIORITY].map(|k| self.choice(k) == 1)
    }

    /// MISS のゲージの減りにかける数
    pub fn damage_factor(&self) -> f64 {
        DAMAGE_FACTORS[self.choice(DAMAGE_LEVEL)]
    }

    /// 判定の幅。`pedal` ならバスドラム・左ペダル・左バスドラムの幅
    pub fn hit_windows(&self, pedal: bool) -> HitWindows {
        let keys = if pedal { PEDAL_HIT_KEYS } else { HIT_KEYS };
        let [perfect, great, good, poor] = keys.map(|k| self.number(k) as f64);
        HitWindows {
            perfect,
            great,
            good,
            poor,
        }
        .ordered()
    }

    /// 自動で叩くレーン
    pub fn auto_lanes(&self) -> Vec<DrumLane> {
        AUTO_LANES
            .iter()
            .filter(|(key, _)| self.flag(key))
            .flat_map(|(_, lanes)| lanes.iter().copied())
            .collect()
    }
}

/// `v` を `min`〜`max` に収め、`min` からの `step` 刻みにそろえる
fn snap(v: i64, min: i64, max: i64, step: i64) -> i64 {
    let step = step.max(1);
    let v = v.clamp(min, max);
    min + (v - min) / step * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_members_exist() {
        for (i, d) in ALL.iter().enumerate() {
            assert!(
                ALL[..i].iter().all(|e| e.key != d.key),
                "duplicate key {}",
                d.key
            );
            if let Kind::All { members } = d.kind {
                for m in members {
                    assert!(matches!(def(m).kind, Kind::Toggle { .. }), "{m}");
                }
            }
            if let Kind::Choice { choices, default } = d.kind {
                assert!(default < choices.len(), "{}", d.key);
            }
            if let Kind::Number {
                min, max, default, ..
            } = d.kind
            {
                assert!((min..=max).contains(&default), "{}", d.key);
            }
        }
        for (key, _) in AUTO_LANES {
            def(key);
        }
    }

    #[test]
    fn auto_all_switches_every_lane() {
        let mut v = Values::default();
        assert!(v.auto_lanes().is_empty());
        assert_eq!(v.label(AUTO_ALL), "OFF");
        assert_eq!(v.raw(AUTO_ALL), Some(0));
        assert_eq!(default_raw(AUTO_ALL), Some(0));
        v.set_raw("auto_bd", 1);
        assert_eq!(v.auto_lanes(), vec![DrumLane::BD]);
        assert_eq!(v.label(AUTO_ALL), "-");
        assert_eq!(v.raw(AUTO_ALL), None);
        v.set_raw(AUTO_ALL, 1);
        assert_eq!(v.label(AUTO_ALL), "ON");
        assert_eq!(v.auto_lanes().len(), 12);
        v.set_raw(AUTO_ALL, 0);
        assert!(v.auto_lanes().is_empty());
        v.set_raw("auto_hh", 1);
        assert_eq!(v.auto_lanes(), vec![DrumLane::HH, DrumLane::HHO]);
    }

    #[test]
    fn values_round_trip_through_toml() {
        let mut v = Values::default();
        v.set_raw("auto_sd", 1);
        let text = toml::to_string(&v).unwrap();
        assert_eq!(text.trim(), "auto_sd = true");
        let back: Values = toml::from_str(&text).unwrap();
        assert_eq!(back, v);
        // 型の違う値は既定に
        let odd: Values = toml::from_str("auto_sd = 3\n").unwrap();
        assert!(!odd.flag("auto_sd"));
    }

    #[test]
    fn skip_width_steps_by_a_second() {
        let mut v = Values::default();
        assert_eq!(v.number(SKIP_MS), 5000);
        assert_eq!(v.label(SKIP_MS), "5000 ms");
        v.set_raw(SKIP_MS, 7400);
        assert_eq!(v.number(SKIP_MS), 7000);
        v.set_raw(SKIP_MS, -100);
        assert_eq!(v.number(SKIP_MS), 1000);
    }

    #[test]
    fn rules_default_to_upstream() {
        let mut v = Values::default();
        assert_eq!(v.damage_factor(), 0.5);
        assert_eq!(v.label(DAMAGE_LEVEL), "ふつう");
        v.set_raw(DAMAGE_LEVEL, 2);
        assert_eq!(v.damage_factor(), 0.75);
        // 端で止まる
        v.set_raw(DAMAGE_LEVEL, 3);
        assert_eq!(v.damage_factor(), 0.75);
        assert_eq!(v.label(RISKY), "OFF");
        v.set_raw(RISKY, 3);
        assert_eq!(v.label(RISKY), "3 回");
        assert!(!v.flag(HAZARD) && !v.flag(TIGHT));
    }

    #[test]
    fn hit_windows_come_from_the_table_in_order() {
        let mut v = Values::default();
        assert_eq!(v.hit_windows(false), HitWindows::default());
        assert_eq!(v.hit_windows(true), HitWindows::default());
        // ペダルの GREAT を PERFECT より狭くしても、PERFECT までは広げる
        v.set_raw(PEDAL_HIT_KEYS[0], 44);
        v.set_raw(PEDAL_HIT_KEYS[1], 27);
        let w = v.hit_windows(true);
        assert_eq!((w.perfect, w.great), (44.0, 44.0));
        assert_eq!(v.hit_windows(false), HitWindows::default());
    }

    #[test]
    fn dark_sets_the_lane_parts() {
        let mut v = Values::default();
        assert!(v.flag(JUDGE_LINE_DISP) && v.flag(LANE_FLUSH));
        assert_eq!(v.number(MIN_COMBO), 10);
        v.set_raw(DARK, 2);
        assert!(v.dark_full());
        assert_eq!(v.choice(LANE_DISP), 3);
        assert!(!v.flag(JUDGE_LINE_DISP) && !v.flag(LANE_FLUSH));
        // あとから 1 つずつ変えられる
        v.set_raw(LANE_FLUSH, 1);
        assert!(v.flag(LANE_FLUSH));
        v.set_raw(DARK, 1);
        assert_eq!(v.choice(LANE_DISP), 1);
        assert!(v.flag(JUDGE_LINE_DISP));
        v.set_raw(DARK, 0);
        assert_eq!(v.choice(LANE_DISP), 0);
    }

    #[test]
    fn lane_groups_follow_the_upstream_order() {
        let mut v = Values::default();
        assert_eq!(v.lane_groups(), LaneGroups::default());
        assert_eq!(v.pad_sound_first(), [false; 3]);
        assert!(v.flag(MUTING_LP));
        v.set_raw(HH_GROUP, 1);
        v.set_raw(BD_GROUP, 1);
        v.set_raw(CYMBAL_FREE, 1);
        v.set_raw(CY_PRIORITY, 1);
        let g = v.lane_groups();
        // HH-1 は左シンバルとハイハットが共通、BD-1 は左バスドラムとバスドラムが共通
        assert_eq!(g.hh, HhGroup::HiHatOnly);
        assert_eq!(g.bd, BdGroup::BassDrumsCommon);
        assert!(g.cymbal_free);
        assert_eq!(v.pad_sound_first(), [false, false, true]);
        v.set_raw(BD_GROUP, 2);
        assert_eq!(v.lane_groups().bd, BdGroup::LeftPedalsCommon);
    }

    #[test]
    fn sound_settings_default_to_upstream() {
        let mut v = Values::default();
        assert_eq!(v.number(CHIP_VOLUME), 100);
        assert_eq!(v.label(AUTO_VOLUME), "80 %");
        assert!(v.flag(BGM_SOUND) && !v.flag(METRONOME));
        v.set_raw(BGM_ADJUST, -150);
        assert_eq!(v.number(BGM_ADJUST), -99);
    }

    #[test]
    fn old_fullscreen_becomes_the_display_mode() {
        let mut v: Values = toml::from_str("fullscreen = true\nvsync = false\n").unwrap();
        assert!(v.upgrade());
        assert_eq!(v.display_mode(), DisplayMode::Fullscreen);
        assert_eq!(v.label(DISPLAY_MODE), "全画面");
        assert!(!v.flag(VSYNC));
        let text = toml::to_string(&v).unwrap();
        assert!(!text.contains("fullscreen ="), "{text}");
        assert!(text.contains("display_mode = \"fullscreen\""), "{text}");
        // 2 回目は何もしない
        assert!(!v.upgrade());

        let mut off: Values = toml::from_str("fullscreen = false\n").unwrap();
        assert!(off.upgrade());
        assert_eq!(off.display_mode(), DisplayMode::Window);
        assert!(off.is_empty());

        // 新しいキーがあればそちらのまま
        let mut both: Values =
            toml::from_str("fullscreen = true\ndisplay_mode = \"borderless\"\n").unwrap();
        assert!(both.upgrade());
        assert_eq!(both.display_mode(), DisplayMode::Borderless);
    }

    #[test]
    fn display_modes_match_their_choices() {
        let mut v = Values::default();
        assert_eq!(v.display_mode(), DisplayMode::Window);
        for m in DisplayMode::ALL {
            v.set_display_mode(m);
            assert_eq!(v.display_mode(), m);
            assert_eq!(DisplayMode::from_name(m.name()), Some(m));
        }
        assert_eq!(DisplayMode::from_name("exclusive"), None);
    }

    #[test]
    fn numbers_snap_to_their_steps() {
        assert_eq!(snap(17, 0, 100, 5), 15);
        assert_eq!(snap(-3, 0, 100, 5), 0);
        assert_eq!(snap(130, 0, 100, 5), 100);
        assert_eq!(snap(7, 1, 9, 2), 7);
    }
}
