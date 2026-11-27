//! ほかの DTXMania（NX・XG・本家）の設定から、このアプリに引き継げる項目を並べ、選んだものを写す
//! （設定 ＞ アプリ ＞「他の DTXMania から引き継ぐ」）。
//!
//! - 派生ごとに、値の意味を上流のソースで確かめた項目だけを引き継ぐ。NX の 1.x と v2 は同じ読み方
//!   （キーの名前・範囲・既定を両方のソースで照らした）。XG verK は名前と範囲と意味の同じ一部だけ。
//!   本家（099 系の Config.ini と 1xx 系の Config.xml）と見分けられないものは、パッドの割り当てと曲のフォルダだけ
//! - 前のアプリで既定のままの項目と、引き継いでも今と変わらない項目は、はじめは選ばない
//! - 引き継ぐ前の設定を `config.before-import.toml` に残し、「引き継ぐ前に戻す」で戻せる
//! - MIDI の割り当ては、DTXMania の「何台目の機器」（機器の名前は残っていない）を、このアプリのどの機器に
//!   写すかを選ぶ。プリセットにあってDTXMania に無いノート（リムやエッジなど）は残す
//!
//! 上流の値の読み方は `CConfigIni.tReadFromString` と同じ（ON / OFF は先頭の文字が 0 でなければ ON、
//! 数は範囲の外なら上流も既定を使う）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use dtx::DrumLane;
use i18n::{count, t, tf};
use nx::{Family, Format, Found, Ini, KeyAssign, LocateError, ReadError, Source};
use winit::keyboard::KeyCode;

use crate::config::{Config, PadNote};
use crate::config_view::{self, Setting};
use crate::inputs;
use crate::midi::PortInfo;
use crate::settings::{self, Kind};

/// 引き継ぐ項目の小見出し
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Pads,
    Songs,
    Timing,
    Play,
    Chart,
    Auto,
    Sound,
}

impl Part {
    pub const ALL: [Part; 7] = [
        Part::Pads,
        Part::Songs,
        Part::Timing,
        Part::Play,
        Part::Chart,
        Part::Auto,
        Part::Sound,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Part::Pads => t("Pad Mapping", "パッドの割り当て"),
            Part::Songs => t("Songs", "曲"),
            Part::Timing => t("Timing", "タイミング"),
            Part::Play => t("Gameplay", "ゲームプレイ"),
            Part::Chart => t("Chart", "譜面"),
            Part::Auto => t("Auto & Grouping", "自動・叩き分け"),
            Part::Sound => t("Sound", "音"),
        }
    }
}

/// 引き継ぐときに書き換えるもの
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// `Config` の欄（[`config_view::set`] の形の値）
    Value(Setting, i64),
    /// 設定の表の項目（[`settings::Values::set_raw`] の形の値）
    Table(&'static str, i64),
    /// 曲のフォルダを足す
    SongDirs(Vec<PathBuf>),
    /// キーボードの割り当てを、これにそろえる
    Keys(Vec<(KeyCode, DrumLane)>),
    /// MIDI の割り当てを写す機器の、これより弱い打撃を捨てる値（このアプリの書き方）
    MinVelocity(u8),
}

/// 引き継ぐ項目 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub part: Part,
    pub name: String,
    /// 前のアプリの書き方（`DamageLevel = 0`）
    pub raw: String,
    /// 引き継いだあとの値
    pub to: String,
    /// 今の値
    pub now: String,
    /// 補足（刻みにそろえた、など）
    pub note: Option<String>,
    /// 項目の説明
    pub desc: String,
    /// 引き継いでも今と変わらない
    pub same: bool,
    /// 前のアプリでも既定のまま
    pub untouched: bool,
    /// 引き継ぐ（画面で切り替える）
    pub on: bool,
    pub change: Change,
}

/// DTXMania の MIDI の機器 1 台ぶんの割り当て
#[derive(Debug, Clone, PartialEq)]
pub struct MidiSource {
    /// DTXMania の機器の番号（0 から。画面では「1 台目」）
    pub device: u8,
    /// ノートとレーン（書いてある順）
    pub notes: Vec<(u8, DrumLane)>,
    pub on: bool,
    /// 写す先（[`Plan::targets`] の何番目か）
    pub target: usize,
}

/// MIDI の割り当てを写す先
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub label: String,
    /// ポートの名前と ID。`None` なら、まだ設定の無い機器（これからつなぐ機器）
    pub port: Option<(String, String)>,
}

/// 引き継げないもの
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub name: String,
    /// 短い理由（一覧に出す）
    pub reason: String,
    /// くわしく（説明の列に出す）
    pub detail: String,
}

/// 読んだ設定から作った、引き継ぐ項目の一覧
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// 「DTXManiaNX v2.0.0 Beta (20260121)」など
    pub title: String,
    /// 製品の名前（「DTXManiaNX」など）
    pub product: &'static str,
    pub root: PathBuf,
    pub midi: Vec<MidiSource>,
    pub targets: Vec<Target>,
    pub items: Vec<Item>,
    pub rejected: Vec<Rejected>,
}

impl Plan {
    /// 選んでいる項目の数（MIDI の機器を含む）
    pub fn selected(&self) -> usize {
        self.items.iter().filter(|i| i.on).count() + self.midi.iter().filter(|m| m.on).count()
    }

    /// 項目の数（MIDI の機器を含む）
    pub fn total(&self) -> usize {
        self.items.len() + self.midi.len()
    }
}

/// 値の意味を確かめた範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// NX（1.x と v2）: すべて
    Full,
    /// XG verK: 名前・範囲・意味の同じ一部
    Core,
    /// 本家と見分けられないもの: パッドの割り当てと曲のフォルダだけ
    Pads,
}

impl Scope {
    fn of(family: Family) -> Self {
        match family {
            Family::NxV2 | Family::Nx1 => Self::Full,
            Family::XgVerK => Self::Core,
            Family::Classic | Family::ClassicXml | Family::Unknown => Self::Pads,
        }
    }
}

/// 上流の値の、このアプリの値への直し方
#[derive(Debug, Clone, Copy)]
enum Conv {
    /// 同じ数
    Same,
    /// ON / OFF
    Flag,
    /// 上流の値を添え字にした、このアプリの値
    Map(&'static [i64]),
    /// かける
    Times(i64),
    /// 符号を逆に（上流は叩いた時刻に足し、このアプリは引く）
    Negate,
    /// 上流の速さの段 n（0.089375 × (n + 1) ドット/ミリ秒）→ このアプリの速さ（ドット/ミリ秒の 20 倍）
    Scroll,
}

#[derive(Debug, Clone, Copy)]
enum To {
    Value(Setting),
    Table(&'static str),
}

/// 上流のキーのある所
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sec {
    /// `[System]` か `[PlayOption]`
    System,
    AutoPlay,
}

/// 上流のキー 1 つの引き継ぎ方
struct Rule {
    key: &'static str,
    sec: Sec,
    part: Part,
    to: To,
    conv: Conv,
    /// 上流の範囲（外なら上流も既定を使う）。ON / OFF は使わない
    range: (i64, i64),
    /// 上流の既定（ON / OFF は 1 と 0）
    default: i64,
    /// XG verK でも同じ意味
    xg: bool,
}

const fn rule(
    key: &'static str,
    part: Part,
    to: To,
    conv: Conv,
    range: (i64, i64),
    default: i64,
    xg: bool,
) -> Rule {
    Rule {
        key,
        sec: Sec::System,
        part,
        to,
        conv,
        range,
        default,
        xg,
    }
}

const fn flag(key: &'static str, part: Part, to: To, default: bool, xg: bool) -> Rule {
    rule(key, part, to, Conv::Flag, (0, 1), default as i64, xg)
}

const fn auto(key: &'static str, table: &'static str) -> Rule {
    Rule {
        key,
        sec: Sec::AutoPlay,
        part: Part::Auto,
        to: To::Table(table),
        conv: Conv::Flag,
        range: (0, 1),
        default: 0,
        xg: true,
    }
}

use Part::{Auto as A, Chart as C, Play as P, Sound as S, Timing as T};
use To::{Table as Tb, Value as V};

/// 上流のキーと、このアプリの設定の対応（範囲と既定は DTXManiaNX v2.0 の `CConfigIni`。1.x も同じ）
const RULES: &[Rule] = &[
    // タイミング
    rule(
        "InputAdjustTimeDrums",
        T,
        V(Setting::InputAdjust),
        Conv::Negate,
        (-99, 99),
        0,
        true,
    ),
    rule(
        "BGMAdjustTime",
        T,
        Tb(settings::BGM_ADJUST),
        Conv::Same,
        (-99, 99),
        0,
        false,
    ),
    // ゲームプレイ
    flag("StageFailed", P, V(Setting::StageFailed), true, true),
    rule(
        "DamageLevel",
        P,
        Tb(settings::DAMAGE_LEVEL),
        Conv::Same,
        (0, 2),
        1,
        true,
    ),
    rule(
        "Risky",
        P,
        Tb(settings::RISKY),
        Conv::Same,
        (0, 10),
        0,
        true,
    ),
    flag("HAZARD", P, Tb(settings::HAZARD), false, false),
    flag("DrumsTight", P, Tb(settings::TIGHT), false, true),
    rule(
        "ShowLagTime",
        P,
        V(Setting::ShowLagTime),
        Conv::Same,
        (0, 2),
        0,
        false,
    ),
    rule(
        "ShowLagTimeColor",
        P,
        V(Setting::ShowLagTimeColor),
        Conv::Same,
        (0, 1),
        0,
        false,
    ),
    flag(
        "ShowLagHitCount",
        P,
        V(Setting::ShowLagHitCount),
        false,
        false,
    ),
    rule(
        "SkipTimeMs",
        P,
        Tb(settings::SKIP_MS),
        Conv::Same,
        (100, 20000),
        5000,
        false,
    ),
    flag(
        "AutoResultCapture",
        P,
        Tb(settings::AUTO_SAVE_RESULT),
        true,
        false,
    ),
    // 譜面
    rule(
        "DrumsScrollSpeed",
        C,
        V(Setting::ScrollSpeed),
        Conv::Scroll,
        (0, 1999),
        1,
        false,
    ),
    flag("DrumsReverse", C, Tb(settings::REVERSE), false, true),
    rule(
        "DrumsJudgeLine",
        C,
        Tb(settings::JUDGE_LINE_POS),
        Conv::Same,
        (0, 100),
        0,
        false,
    ),
    rule(
        "DrumsHiddenSudden",
        C,
        Tb(settings::HID_SUD),
        Conv::Same,
        (0, 5),
        0,
        false,
    ),
    rule(
        "DrumsShutterIn",
        C,
        Tb(settings::SHUTTER_IN),
        Conv::Same,
        (0, 100),
        0,
        false,
    ),
    rule(
        "DrumsShutterOut",
        C,
        Tb(settings::SHUTTER_OUT),
        Conv::Same,
        (-100, 100),
        0,
        false,
    ),
    rule(
        "DrumsLaneDisp",
        C,
        Tb(settings::LANE_DISP),
        Conv::Same,
        (0, 4),
        0,
        false,
    ),
    flag(
        "DrumsJudgeLineDisp",
        C,
        Tb(settings::JUDGE_LINE_DISP),
        true,
        false,
    ),
    flag("DrumsLaneFlush", C, Tb(settings::LANE_FLUSH), true, false),
    rule(
        "DrumsAttackEffect",
        C,
        Tb(settings::ATTACK_EFFECT),
        Conv::Same,
        (0, 3),
        0,
        false,
    ),
    rule(
        "DrumsPosition",
        C,
        Tb(settings::JUDGE_POSITION),
        Conv::Same,
        (0, 2),
        0,
        false,
    ),
    flag("DrumComboDisp", C, Tb(settings::COMBO_DISP), true, false),
    rule(
        "MinComboDrums",
        C,
        Tb(settings::MIN_COMBO),
        Conv::Same,
        (1, 99999),
        10,
        true,
    ),
    rule(
        "DrumsLaneType",
        C,
        Tb(settings::LANE_TYPE),
        Conv::Same,
        (0, 3),
        0,
        false,
    ),
    // 上流は 0 が RD RC、1 が RC RD（既定）。このアプリは既定の RC RD が先
    rule(
        "RDPosition",
        C,
        Tb(settings::RD_POSITION),
        Conv::Map(&[1, 0]),
        (0, 1),
        1,
        false,
    ),
    rule(
        "HHOGraphics",
        C,
        Tb(settings::HHO_GRAPHICS),
        Conv::Same,
        (0, 2),
        0,
        false,
    ),
    // 上流の A・B は、このアプリの選択肢の 3 つ目と 4 つ目
    rule(
        "LBDGraphics",
        C,
        Tb(settings::LBD_GRAPHICS),
        Conv::Map(&[2, 3]),
        (0, 1),
        0,
        false,
    ),
    flag("AVI", C, Tb(settings::AVI), true, true),
    rule(
        "MovieAlpha",
        C,
        Tb(settings::MOVIE_ALPHA),
        Conv::Times(10),
        (0, 10),
        0,
        false,
    ),
    // 自動・叩き分け
    rule(
        "HHGroup",
        A,
        Tb(settings::HH_GROUP),
        Conv::Same,
        (0, 3),
        0,
        false,
    ),
    rule(
        "FTGroup",
        A,
        Tb(settings::FT_GROUP),
        Conv::Same,
        (0, 2),
        0,
        false,
    ),
    rule(
        "CYGroup",
        A,
        Tb(settings::CY_GROUP),
        Conv::Same,
        (0, 2),
        0,
        false,
    ),
    rule(
        "BDGroup",
        A,
        Tb(settings::BD_GROUP),
        Conv::Same,
        (0, 3),
        0,
        false,
    ),
    flag("CymbalFree", A, Tb(settings::CYMBAL_FREE), false, false),
    rule(
        "HitSoundPriorityHH",
        A,
        Tb(settings::HH_PRIORITY),
        Conv::Same,
        (0, 1),
        0,
        false,
    ),
    rule(
        "HitSoundPriorityFT",
        A,
        Tb(settings::FT_PRIORITY),
        Conv::Same,
        (0, 1),
        0,
        false,
    ),
    rule(
        "HitSoundPriorityCY",
        A,
        Tb(settings::CY_PRIORITY),
        Conv::Same,
        (0, 1),
        0,
        false,
    ),
    flag("MutingLP", A, Tb(settings::MUTING_LP), true, false),
    auto("LC", "auto_lc"),
    auto("HH", "auto_hh"),
    auto("LP", "auto_lp"),
    auto("LBD", "auto_lbd"),
    auto("SD", "auto_sd"),
    auto("BD", "auto_bd"),
    auto("HT", "auto_ht"),
    auto("LT", "auto_lt"),
    auto("FT", "auto_ft"),
    auto("CY", "auto_cy"),
    auto("RD", "auto_rd"),
    // 音
    flag("HitSound", S, V(Setting::HitSound), true, true),
    rule(
        "MasterVolume",
        S,
        V(Setting::MasterVolume),
        Conv::Same,
        (0, 100),
        100,
        false,
    ),
    rule(
        "ChipVolume",
        S,
        Tb(settings::CHIP_VOLUME),
        Conv::Same,
        (0, 100),
        100,
        true,
    ),
    rule(
        "AutoChipVolume",
        S,
        Tb(settings::AUTO_VOLUME),
        Conv::Same,
        (0, 100),
        80,
        true,
    ),
    flag("BGMSound", S, Tb(settings::BGM_SOUND), true, true),
    flag("Metronome", S, Tb(settings::METRONOME), false, false),
];

/// 判定の幅（上流 `[HitRange]` の `Drum…` と `DrumPedal…`。前の版の、名前の前に何も無いものは両方に効く）
const HIT_GRADES: [&str; 4] = ["Perfect", "Great", "Good", "Poor"];
/// 上流の判定の幅の既定（`STHitRanges.tCreateDefaultDTXHitRanges`）と範囲
const HIT_DEFAULTS: [i64; 4] = [34, 67, 84, 117];
const HIT_MAX: i64 = 999;

/// パッドごとの最小ベロシティのキーと、上流の既定（HH だけ 20）
const VELOCITY_KEYS: [(&str, i64); 11] = [
    ("LCVelocityMin", 0),
    ("HHVelocityMin", 20),
    ("SDVelocityMin", 0),
    ("BDVelocityMin", 0),
    ("HTVelocityMin", 0),
    ("LTVelocityMin", 0),
    ("FTVelocityMin", 0),
    ("CYVelocityMin", 0),
    ("RDVelocityMin", 0),
    ("LPVelocityMin", 0),
    ("LBDVelocityMin", 0),
];

/// 上流の既定のキーボードの割り当て（`strDefaultKeyAssign`。SlimDX のキーの番号）
const DEFAULT_KEYS: [(DrumLane, &[u8]); 12] = [
    (DrumLane::HH, &[33]),
    (DrumLane::SD, &[12, 13]),
    (DrumLane::BD, &[126, 48]),
    (DrumLane::HT, &[31, 15]),
    (DrumLane::LT, &[11, 16]),
    (DrumLane::FT, &[23, 17]),
    (DrumLane::CY, &[22, 19]),
    (DrumLane::HHO, &[28]),
    (DrumLane::RD, &[47, 20]),
    (DrumLane::LC, &[35, 10]),
    (DrumLane::LP, &[87]),
    (DrumLane::LBD, &[77]),
];

/// 音の出し方（このアプリと仕組みが違う）のキー
const SOUND_DEVICE_KEYS: [&str; 6] = [
    "SoundDeviceType",
    "WASAPIBufferSizeMs",
    "ASIODevice",
    "ASIOBufferSizeMs",
    "EventDrivenWASAPI",
    "SoundTimerType",
];

/// `[System]` か `[PlayOption]` の値
fn sys<'a>(ini: &'a Ini, key: &str) -> Option<&'a str> {
    ini.get("System", key)
        .or_else(|| ini.get("PlayOption", key))
}

/// 上流 `CConversion.bONorOFF`（先頭の文字が 0 でなければ ON。空なら上流は読まない）
fn on_off(v: &str) -> Option<bool> {
    v.chars().next().map(|c| c != '0')
}

/// 上流の数（読めないか範囲の外なら、上流も既定を使うので `None`）
fn number(v: &str, (min, max): (i64, i64)) -> Option<i64> {
    v.trim()
        .parse::<i64>()
        .ok()
        .filter(|n| (min..=max).contains(n))
}

/// 読んだ設定から、引き継ぐ項目を並べる。`ports` は今つながっている MIDI の機器（OS の並び）
pub fn plan(source: &Source, config: &Config, ports: &[PortInfo]) -> Plan {
    let scope = Scope::of(source.family);
    let mut p = Plan {
        title: source.title(),
        product: source.family.product(),
        root: source.found.root.clone(),
        midi: Vec::new(),
        targets: targets(ports),
        items: Vec::new(),
        rejected: Vec::new(),
    };
    pads(&mut p, &source.ini, config, ports);
    songs(&mut p, &source.ini, &source.found.root, config);
    let ini = &source.ini;
    if scope != Scope::Pads {
        for r in RULES {
            if scope == Scope::Full || r.xg {
                add_rule(&mut p, ini, config, r);
            }
        }
    }
    if scope == Scope::Full {
        hit_ranges(&mut p, ini, config);
        velocity(&mut p, ini, config, ports);
    }
    rejected_extras(&mut p, ini, scope, source.found.format);
    p.items.sort_by_key(|i| i.part);
    p
}

/// MIDI の割り当てを写す先（つながっている機器と、これからつなぐ機器）
fn targets(ports: &[PortInfo]) -> Vec<Target> {
    let mut out: Vec<Target> = ports
        .iter()
        .map(|p| Target {
            label: p.name.clone(),
            port: Some((p.name.clone(), p.id.clone())),
        })
        .collect();
    out.push(Target {
        label: t("Next Device You Connect", "これからつなぐ機器").to_owned(),
        port: None,
    });
    out
}

/// パッドの割り当て（`[DrumsKeyAssign]`）
fn pads(p: &mut Plan, ini: &Ini, config: &Config, ports: &[PortInfo]) {
    // 同じキーが何度もあれば後のものが勝つ（上流と同じ）
    let mut lanes: BTreeMap<DrumLane, Vec<KeyAssign>> = BTreeMap::new();
    let mut order: Vec<DrumLane> = Vec::new();
    for (key, value) in ini.entries("DrumsKeyAssign") {
        if let Some(lane) = nx::lane_of(key) {
            if !order.contains(&lane) {
                order.push(lane);
            }
            lanes.insert(lane, nx::parse_assigns(value));
        }
    }
    let mut midi: BTreeMap<u8, Vec<(u8, DrumLane)>> = BTreeMap::new();
    let mut doubled: Vec<String> = Vec::new();
    let mut keys: Vec<(KeyCode, DrumLane)> = Vec::new();
    let mut bad_keys: Vec<&'static str> = Vec::new();
    let mut others = 0usize;
    for lane in &order {
        for a in &lanes[lane] {
            match *a {
                KeyAssign::Midi { device, note } => {
                    let list = midi.entry(device).or_default();
                    match list.iter().find(|(n, _)| *n == note) {
                        // 1 つのノートは 1 つのレーンだけ（先に書いてある方）
                        Some((_, first)) if first != lane => doubled.push(format!(
                            "{} ({}: {} / {})",
                            note,
                            device + 1,
                            first.name(),
                            lane.name()
                        )),
                        Some(_) => {}
                        None => list.push((note, *lane)),
                    }
                }
                KeyAssign::Keyboard(code) => match slimdx_key(code) {
                    Some((Some(k), _)) if inputs::assignable(k) => {
                        if !keys.iter().any(|(c, _)| *c == k) {
                            keys.push((k, *lane));
                        }
                    }
                    Some((_, name)) => {
                        if !bad_keys.contains(&name) {
                            bad_keys.push(name);
                        }
                    }
                    None => others += 1,
                },
                KeyAssign::Joypad { .. } | KeyAssign::Mouse(_) => others += 1,
            }
        }
    }
    // MIDI の機器ごと（DTXMania の機器の番号 n を、つながっている n 番目の機器へ。1 台だけなら 1 台目へ）
    let later = p.targets.len() - 1;
    let single = midi.len() == 1;
    for (device, notes) in midi {
        let target = if usize::from(device) < ports.len() {
            usize::from(device)
        } else if single && !ports.is_empty() {
            0
        } else {
            later
        };
        p.midi.push(MidiSource {
            device,
            notes,
            on: true,
            target,
        });
    }
    if !lanes.is_empty() {
        add_keys(p, &lanes, keys, config);
    }
    if !doubled.is_empty() {
        p.rejected.push(Rejected {
            name: t("Notes Mapped to Two Lanes", "2 つのレーンに割り当てたノート").to_owned(),
            reason: tf!(
                "{} notes, first lane kept",
                "{} 個（先のレーンだけ）",
                doubled.len()
            ),
            detail: tf!(
                "In this app, one note goes to one lane. The lane written first is used.\n{}",
                "このアプリでは、1 つのノートは 1 つのレーンにだけ割り当てます。先に書いてあるレーンを使います。\n{}",
                doubled.join(", ")
            ),
        });
    }
    if !bad_keys.is_empty() {
        p.rejected.push(Rejected {
            name: tf!("Keys: {}", "キー: {}", bad_keys.join(" / ")),
            reason: t("Used for menus here", "画面の操作に使うキー").to_owned(),
            detail: t(
                "These keys are used to move through menus in this app (or can't be read), so they can't be mapped to pads. Map other keys in Pad & Key Mapping.",
                "このアプリでは画面の操作に使うキー（か、読めないキー）なので、パッドに割り当てられません。「パッドとキーの割り当て」でほかのキーを割り当ててください。",
            )
            .to_owned(),
        });
    }
    if others > 0 {
        p.rejected.push(Rejected {
            name: t("Joypad & Mouse Mapping", "ジョイパッドとマウスの割り当て").to_owned(),
            reason: count(others, "mapping", "mappings", "件"),
            detail: t(
                "This app doesn't support joypads or mice for playing.",
                "このアプリは、ジョイパッドとマウスでの演奏に対応していません。",
            )
            .to_owned(),
        });
    }
}

/// キーボードの割り当ての項目
fn add_keys(
    p: &mut Plan,
    lanes: &BTreeMap<DrumLane, Vec<KeyAssign>>,
    keys: Vec<(KeyCode, DrumLane)>,
    config: &Config,
) {
    // 上流の既定と同じか（レーンごとのキーボードのキーの組）
    let untouched = DEFAULT_KEYS.iter().all(|(lane, codes)| {
        let mut got: Vec<u8> = lanes
            .get(lane)
            .map(|a| {
                a.iter()
                    .filter_map(|k| match k {
                        KeyAssign::Keyboard(c) => Some(*c),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        got.sort_unstable();
        let mut want = codes.to_vec();
        want.sort_unstable();
        got == want
    });
    let mut after = config.clone();
    apply_keys(&mut after, &keys);
    let sorted = |c: &Config| {
        let mut m: Vec<(String, DrumLane)> = inputs::keyboard_map(c)
            .into_iter()
            .map(|(k, l)| (inputs::key_name(k), l))
            .collect();
        m.sort();
        m
    };
    let same = sorted(&after) == sorted(config);
    let list: Vec<String> = keys
        .iter()
        .map(|(k, l)| format!("{} {}", l.name(), inputs::key_label(*k)))
        .collect();
    let now = if config.keys.is_empty() {
        t("Default", "既定").to_owned()
    } else {
        t("Changed", "変更あり").to_owned()
    };
    p.items.push(Item {
        part: Part::Pads,
        name: t("Keyboard", "キーボード").to_owned(),
        raw: list.join("  "),
        to: count(keys.len(), "key", "keys", "キー"),
        now,
        note: untouched.then(|| {
            t(
                "Still the default in the previous app",
                "前のアプリでも既定のまま",
            )
            .to_owned()
        }),
        desc: t(
            "Keyboard keys for each lane. Keys not listed here are cleared.",
            "レーンごとのキーボードのキーです。ここに無いキーの割り当ては外します。",
        )
        .to_owned(),
        same,
        untouched,
        on: !same && !untouched,
        change: Change::Keys(keys),
    });
}

/// 曲のフォルダ（`DTXPath`。`;` で区切り、相対パスは DTXMania のフォルダから）
fn songs(p: &mut Plan, ini: &Ini, root: &Path, config: &Config) {
    let Some(value) = ini.get("System", "DTXPath") else {
        return;
    };
    let mut add = Vec::new();
    let mut already = 0;
    for part in value.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let dir = nx::resolve(root, part);
        if !dir.is_dir() {
            p.rejected.push(Rejected {
                name: tf!("Song Folder {}", "曲のフォルダ {}", dir.display()),
                reason: t("Not found", "見つからない").to_owned(),
                detail: tf!(
                    "The folder {} doesn't exist (it may have been moved or deleted, or the drive isn't connected).",
                    "フォルダ {} がありません（動かしたか消したか、ドライブがつながっていないようです）。",
                    dir.display()
                ),
            });
            continue;
        }
        if config.song_dirs.iter().any(|d| same_dir(d, &dir)) || add.contains(&dir) {
            already += 1;
            continue;
        }
        add.push(dir);
    }
    if add.is_empty() && already == 0 {
        return;
    }
    let same = add.is_empty();
    let to = match add.as_slice() {
        [] => t("Already added", "追加済み").to_owned(),
        [one] => format!(
            "+ {}",
            one.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| one.display().to_string())
        ),
        many => tf!("+ {} folders", "＋ {} フォルダ", many.len()),
    };
    let list: Vec<String> = add.iter().map(|d| d.display().to_string()).collect();
    p.items.push(Item {
        part: Part::Songs,
        name: t("Song Folders", "曲のフォルダ").to_owned(),
        raw: format!("DTXPath={value}"),
        to,
        now: count(config.song_dirs.len(), "folder", "folders", "フォルダ"),
        note: None,
        desc: if list.is_empty() {
            t(
                "These folders are already in your song folder list.",
                "どのフォルダも、曲のフォルダの一覧にあります。",
            )
            .to_owned()
        } else {
            tf!(
                "Adds these to your song folders and rescans songs. Folders you already have stay as they are.\n{}",
                "曲のフォルダに追加して、曲を再読み込みします。今あるフォルダはそのままです。\n{}",
                list.join("\n")
            )
        },
        same,
        untouched: false,
        on: !same,
        change: Change::SongDirs(add),
    });
}

/// 同じフォルダか（書き方の違いは OS に任せる）
fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// 上流の値を、このアプリの値にする（無い値なら `None`）
fn convert(conv: Conv, v: i64) -> Option<i64> {
    match conv {
        Conv::Same | Conv::Flag => Some(v),
        Conv::Map(m) => usize::try_from(v).ok().and_then(|i| m.get(i).copied()),
        Conv::Times(k) => Some(v * k),
        Conv::Negate => Some(-v),
        Conv::Scroll => Some((0.089_375 * (v as f64 + 1.0) * 20.0).round() as i64),
    }
}

/// このアプリの範囲と刻み（表の数と `Config` の欄の数）
fn bounds(to: To) -> Option<(i64, i64, i64)> {
    match to {
        To::Table(key) => match settings::def(key).kind {
            Kind::Number { min, max, step, .. } => Some((min, max, step)),
            Kind::Choice { choices, .. } => Some((0, choices.len() as i64 - 1, 1)),
            Kind::Toggle { .. } | Kind::All { .. } => None,
        },
        To::Value(s) => match s {
            Setting::InputAdjust | Setting::VisualAdjust => {
                let r = config_view::OFFSET_RANGE as i64;
                Some((-r, r, 1))
            }
            Setting::ScrollSpeed => Some((2, 60, 1)),
            Setting::MasterVolume => Some((0, 100, i64::from(crate::config::MASTER_VOLUME_STEP))),
            Setting::ShowLagTime => Some((0, 2, 1)),
            Setting::ShowLagTimeColor => Some((0, 1, 1)),
            _ => None,
        },
    }
}

/// 名前と、値の見せ方と、説明（`v` は [`config_view::get`] か [`settings::Values::raw`] の形）
fn describe(to: To, config: &Config, v: Option<i64>) -> (String, String, String) {
    match to {
        To::Table(key) => {
            let d = settings::def(key);
            let mut vals = config.settings.clone();
            if let Some(v) = v {
                vals.set_raw(key, v);
            }
            (
                d.name.get().trim().to_owned(),
                vals.label(key),
                d.desc.get().to_owned(),
            )
        }
        To::Value(s) => {
            let mut c = config.clone();
            if let Some(v) = v {
                config_view::set(s, &mut c, v);
            }
            config_view::value_text(s, &c)
        }
    }
}

/// 今の値（[`config_view::get`] か [`settings::Values::raw`] の形）
fn current(to: To, config: &Config) -> i64 {
    match to {
        To::Table(key) => config.settings.raw(key).unwrap_or_default(),
        To::Value(s) => config_view::get(s, config),
    }
}

fn add_rule(p: &mut Plan, ini: &Ini, config: &Config, r: &Rule) {
    let raw = match r.sec {
        Sec::System => sys(ini, r.key),
        Sec::AutoPlay => ini.get("AutoPlay", r.key),
    };
    let Some(raw) = raw else {
        return;
    };
    let value = match r.conv {
        Conv::Flag => on_off(raw).map(i64::from),
        _ => number(raw, r.range),
    };
    // 読めない値は上流も既定を使う
    let value = value.unwrap_or(r.default);
    let (name, _, desc) = describe(r.to, config, None);
    let name = match r.sec {
        Sec::AutoPlay => tf!("Auto: {}", "自動: {}", name),
        Sec::System => name,
    };
    let raw_text = format!("{}={}", r.key, raw.trim());
    let Some(mut to) = convert(r.conv, value) else {
        p.rejected.push(not_here(&name, &raw_text));
        return;
    };
    let mut note = None;
    if let Some((min, max, step)) = bounds(r.to) {
        if to < min || to > max {
            // 数は端にそろえる。選択肢に無い値と、このアプリに無い負の値（意味が違う）は引き継がない
            let numeric = match r.to {
                To::Value(_) => true,
                To::Table(key) => matches!(settings::def(key).kind, Kind::Number { .. }),
            };
            if !numeric || (to < 0 && min >= 0) {
                p.rejected.push(not_here(&name, &raw_text));
                return;
            }
            to = to.clamp(min, max);
            note = Some(
                t(
                    "Set to the nearest value available here",
                    "このアプリで選べる一番近い値にします",
                )
                .to_owned(),
            );
        }
        let snapped = min + ((to - min) as f64 / step as f64).round() as i64 * step;
        let snapped = snapped.clamp(min, max);
        if snapped != to {
            note = Some(tf!("Rounded to steps of {step}", "{step} 刻みにそろえます"));
            to = snapped;
        }
    }
    let now = current(r.to, config);
    let (_, now_label, _) = describe(r.to, config, Some(now));
    let (_, to_label, _) = describe(r.to, config, Some(to));
    let same = now == to;
    let untouched = value == r.default;
    let change = match r.to {
        To::Table(key) => Change::Table(key, to),
        To::Value(s) => Change::Value(s, to),
    };
    p.items.push(Item {
        part: r.part,
        name,
        raw: raw_text,
        to: to_label,
        now: now_label,
        note,
        desc,
        same,
        untouched,
        on: !same && !untouched,
        change,
    });
}

/// このアプリに無い値
fn not_here(name: &str, raw: &str) -> Rejected {
    Rejected {
        name: name.to_owned(),
        reason: t("Value not available here", "このアプリに無い値").to_owned(),
        detail: tf!(
            "{raw}\nThis app has no matching choice for this value, so it's left as is.",
            "{raw}\nこの値に当たる選択肢がこのアプリに無いので、今のままにします。"
        ),
    }
}

/// 判定の幅（`[HitRange]`）
fn hit_ranges(p: &mut Plan, ini: &Ini, config: &Config) {
    for (i, grade) in HIT_GRADES.iter().enumerate() {
        let legacy = ini.get("HitRange", grade);
        for (prefix, key, pedal) in [
            ("Drum", settings::HIT_KEYS[i], false),
            ("DrumPedal", settings::PEDAL_HIT_KEYS[i], true),
        ] {
            let name_key = format!("{prefix}{grade}");
            let (raw_key, raw) = match ini.get("HitRange", &name_key) {
                Some(v) => (name_key, v),
                None => match legacy {
                    Some(v) => ((*grade).to_owned(), v),
                    None => continue,
                },
            };
            let value = number(raw, (0, HIT_MAX)).unwrap_or(HIT_DEFAULTS[i]);
            let Kind::Number { max, .. } = settings::def(key).kind else {
                continue;
            };
            let to = value.min(max);
            let note =
                (to != value).then(|| tf!("Up to {max} ms here", "このアプリは {max} ms まで"));
            let d = settings::def(key);
            let name = if pedal {
                tf!("{} (Pedals)", "{}（ペダル）", d.name.get())
            } else {
                d.name.get().to_owned()
            };
            let now = config.settings.number(key);
            let tb = To::Table(key);
            p.items.push(Item {
                part: Part::Play,
                name,
                raw: format!("{raw_key}={}", raw.trim()),
                to: describe(tb, config, Some(to)).1,
                now: describe(tb, config, Some(now)).1,
                note,
                desc: d.desc.get().to_owned(),
                same: now == to,
                untouched: value == HIT_DEFAULTS[i],
                on: now != to && value != HIT_DEFAULTS[i],
                change: Change::Table(key, to),
            });
        }
    }
}

/// 最小ベロシティ（上流はパッドごと、このアプリは機器ごとに 1 つ。ほかのパッドの打撃を落とさないよう一番小さいもの）
fn velocity(p: &mut Plan, ini: &Ini, config: &Config, ports: &[PortInfo]) {
    let got: Vec<(&str, i64, i64)> = VELOCITY_KEYS
        .iter()
        .filter_map(|&(key, default)| {
            let v = sys(ini, key)?;
            Some((key, number(v, (0, 127)).unwrap_or(default), default))
        })
        .collect();
    if got.is_empty() || p.midi.is_empty() {
        return;
    }
    let untouched = got.iter().all(|&(_, v, d)| v == d);
    let least = got.iter().map(|&(_, v, _)| v).min().unwrap_or(0);
    // 上流は「この値以下」を捨て、このアプリは「この値より弱い」を捨てる
    let to = if least == 0 { 0 } else { (least + 1).min(127) } as u8;
    let differs = got.iter().any(|&(_, v, _)| v != least);
    let raw: Vec<String> = got
        .iter()
        .map(|(k, v, _)| format!("{}={v}", k.trim_end_matches("VelocityMin")))
        .collect();
    // 今の値は、写す先の 1 台目の機器のもの
    let now = p
        .midi
        .iter()
        .find_map(|m| p.targets.get(m.target)?.port.as_ref())
        .and_then(|(name, id)| {
            let i = ports.iter().position(|q| &q.name == name && &q.id == id)?;
            inputs::device(config, ports, i).map(|d| d.min_velocity)
        })
        .unwrap_or(0);
    let off = |v: u8| {
        if v == 0 {
            "OFF".to_owned()
        } else {
            v.to_string()
        }
    };
    if differs && !untouched {
        p.rejected.push(Rejected {
            name: t("Per-Pad Minimum Velocity", "パッドごとの最小ベロシティ").to_owned(),
            reason: tf!("Uses the lowest ({least})", "一番小さい {least} を使う"),
            detail: tf!(
                "{}\nThis app has one value per MIDI device. To avoid dropping hits on other pads, the lowest value is used with the MIDI mapping.",
                "{}\nこのアプリは MIDI 機器ごとに 1 つです。ほかのパッドの弱い打撃が無視されないよう、一番小さい値を MIDI の割り当てと一緒に引き継ぎます。",
                raw.join("  ")
            ),
        });
    }
    p.items.push(Item {
        part: Part::Pads,
        name: t("Minimum Velocity", "最小ベロシティ").to_owned(),
        raw: raw.join("  "),
        to: off(to),
        now: off(now),
        note: None,
        desc: t(
            "Hits softer than this are ignored (to filter crosstalk). Applied to the devices that receive the MIDI mapping above.",
            "これより弱い打撃を無視します（隣のパッドにつられて入る信号を防ぎます）。上の MIDI の割り当てを使う機器に設定します。",
        )
        .to_owned(),
        same: now == to,
        untouched,
        on: now != to && !untouched,
        change: Change::MinVelocity(to),
    });
}

/// 項目にならないけれど、引き継げないと知らせておくもの
fn rejected_extras(p: &mut Plan, ini: &Ini, scope: Scope, format: Format) {
    let in_sys = |key: &str| sys(ini, key).is_some();
    if scope != Scope::Full {
        let others = RULES
            .iter()
            .filter(|r| scope == Scope::Pads || !r.xg)
            .filter(|r| match r.sec {
                Sec::System => in_sys(r.key),
                Sec::AutoPlay => ini.get("AutoPlay", r.key).is_some(),
            })
            .count()
            + VELOCITY_KEYS.iter().filter(|(k, _)| in_sys(k)).count()
            + ini.entries("HitRange").count();
        if others > 0 || format == Format::Xml {
            p.rejected.push(Rejected {
                name: t("Other Settings", "そのほかの設定").to_owned(),
                reason: if others > 0 {
                    count(others, "setting", "settings", "項目")
                } else {
                    t("Not read", "読まない").to_owned()
                },
                detail: t(
                    "From this version of DTXMania, only settings whose meaning is certain are imported (pad mapping and song folders, plus a few more for XG).",
                    "この DTXMania からは、値の意味が確かな項目だけを引き継ぎます（パッドの割り当てと曲のフォルダ。XG はほかにもいくつか）。",
                )
                .to_owned(),
            });
        }
    }
    if scope == Scope::Full
        && sys(ini, "PedalLagTime").is_some_and(|v| number(v, (-100, 100)).is_some_and(|n| n != 0))
    {
        p.rejected.push(Rejected {
            name: t("Pedal Lag Offset", "ペダルのずれの補正").to_owned(),
            reason: t("No matching setting", "同じ設定が無い").to_owned(),
            detail: format!(
                "PedalLagTime={}\n{}",
                sys(ini, "PedalLagTime").unwrap_or_default(),
                t(
                    "This app has no separate offset for pedals. Use Input Offset or the per-song offset instead.",
                    "このアプリにはペダルだけの補正がありません。入力の補正か、曲ごとの補正を使ってください。",
                )
            ),
        });
    }
    if SOUND_DEVICE_KEYS.iter().any(|k| in_sys(k)) {
        p.rejected.push(Rejected {
            name: t("Audio Output (ASIO, WASAPI)", "音の出し方（ASIO・WASAPI）").to_owned(),
            reason: t("Works differently here", "仕組みが違う").to_owned(),
            detail: t(
                "This app plays sound in its own way, so the audio device settings aren't imported.",
                "このアプリは音の出し方の仕組みが違うので、音の機器の設定は引き継ぎません。",
            )
            .to_owned(),
        });
    }
    if sys(ini, "SkinPath").is_some_and(|v| !v.trim().is_empty()) {
        p.rejected.push(Rejected {
            name: t("Skin", "スキン").to_owned(),
            reason: t("Built into this app", "このアプリに組み込み").to_owned(),
            detail: t(
                "This app uses its own built-in skin.",
                "このアプリは組み込みのスキンを使います。",
            )
            .to_owned(),
        });
    }
    let guitar = ini.entries("GuitarKeyAssign").count()
        + ini.entries("BassKeyAssign").count()
        + ["System", "PlayOption", "AutoPlay", "HitRange"]
            .iter()
            .flat_map(|s| ini.entries(s))
            .filter(|(k, _)| {
                k.starts_with("Guitar") || k.starts_with("Bass") || k.starts_with("Gt")
            })
            .count();
    if guitar > 0 {
        p.rejected.push(Rejected {
            name: t("Guitar & Bass Settings", "ギター・ベースの設定").to_owned(),
            reason: count(guitar, "setting", "settings", "項目"),
            detail: t(
                "This app is for drums only.",
                "このアプリはドラムだけです。",
            )
            .to_owned(),
        });
    }
}

/// SlimDX のキーの番号（上流の Config.ini の `K0…`）→ このアプリのキーと、画面に出す名前。
/// 対応するキーが無いもの（メディアキーなど）は `(None, 名前)`、範囲の外は `None`
fn slimdx_key(code: u8) -> Option<(Option<KeyCode>, &'static str)> {
    use KeyCode as K;
    const DIGITS: [KeyCode; 10] = [
        K::Digit0,
        K::Digit1,
        K::Digit2,
        K::Digit3,
        K::Digit4,
        K::Digit5,
        K::Digit6,
        K::Digit7,
        K::Digit8,
        K::Digit9,
    ];
    const LETTERS: [KeyCode; 26] = [
        K::KeyA,
        K::KeyB,
        K::KeyC,
        K::KeyD,
        K::KeyE,
        K::KeyF,
        K::KeyG,
        K::KeyH,
        K::KeyI,
        K::KeyJ,
        K::KeyK,
        K::KeyL,
        K::KeyM,
        K::KeyN,
        K::KeyO,
        K::KeyP,
        K::KeyQ,
        K::KeyR,
        K::KeyS,
        K::KeyT,
        K::KeyU,
        K::KeyV,
        K::KeyW,
        K::KeyX,
        K::KeyY,
        K::KeyZ,
    ];
    const NAMES_09: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const NAMES_AZ: [&str; 26] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z",
    ];
    const NUMPAD: [KeyCode; 10] = [
        K::Numpad0,
        K::Numpad1,
        K::Numpad2,
        K::Numpad3,
        K::Numpad4,
        K::Numpad5,
        K::Numpad6,
        K::Numpad7,
        K::Numpad8,
        K::Numpad9,
    ];
    const NAMES_NUM: [&str; 10] = [
        "Num0", "Num1", "Num2", "Num3", "Num4", "Num5", "Num6", "Num7", "Num8", "Num9",
    ];
    let c = usize::from(code);
    Some(match code {
        0..=9 => (Some(DIGITS[c]), NAMES_09[c]),
        10..=35 => (Some(LETTERS[c - 10]), NAMES_AZ[c - 10]),
        // ABNT_C1 は日本語のキーボードの「ろ」
        36 => (Some(K::IntlRo), "Ro"),
        38 => (Some(K::Quote), "'"),
        42 => (Some(K::Backspace), "Backspace"),
        43 => (Some(K::Backslash), "\\"),
        45 => (Some(K::CapsLock), "CapsLock"),
        47 => (Some(K::Comma), ","),
        48 => (Some(K::Convert), "Convert"),
        49 => (Some(K::Delete), "Delete"),
        50 => (Some(K::ArrowDown), "Down"),
        51 => (Some(K::End), "End"),
        52 => (Some(K::Equal), "="),
        53 => (Some(K::Escape), "Esc"),
        54..=65 => (None, "F1-F12"),
        69 => (Some(K::Backquote), "`"),
        70 => (Some(K::Home), "Home"),
        71 => (Some(K::Insert), "Insert"),
        72 => (Some(K::KanaMode), "Kana"),
        74 => (Some(K::BracketLeft), "["),
        75 => (Some(K::ControlLeft), "LeftCtrl"),
        76 => (Some(K::ArrowLeft), "Left"),
        77 => (Some(K::AltLeft), "LeftAlt"),
        78 => (Some(K::ShiftLeft), "LeftShift"),
        83 => (Some(K::Minus), "-"),
        87 => (Some(K::NonConvert), "NoConvert"),
        89..=98 => (Some(NUMPAD[c - 89]), NAMES_NUM[c - 89]),
        100 => (Some(K::NumpadEnter), "NumEnter"),
        102 => (Some(K::NumpadSubtract), "Num-"),
        103 => (Some(K::NumpadDecimal), "Num."),
        104 => (Some(K::NumpadAdd), "Num+"),
        105 => (Some(K::NumpadDivide), "Num/"),
        106 => (Some(K::NumpadMultiply), "Num*"),
        108 => (Some(K::PageDown), "PageDown"),
        109 => (Some(K::PageUp), "PageUp"),
        111 => (Some(K::Period), "."),
        115 => (Some(K::BracketRight), "]"),
        116 => (Some(K::ControlRight), "RightCtrl"),
        117 => (Some(K::Enter), "Enter"),
        118 => (Some(K::ArrowRight), "Right"),
        119 => (Some(K::AltRight), "RightAlt"),
        120 => (Some(K::ShiftRight), "RightShift"),
        123 => (Some(K::Semicolon), ";"),
        124 => (Some(K::Slash), "/"),
        126 => (Some(K::Space), "Space"),
        129 => (Some(K::Tab), "Tab"),
        132 => (Some(K::ArrowUp), "Up"),
        143 => (Some(K::IntlYen), "Yen"),
        _ if code <= 144 => (None, "Other"),
        _ => return None,
    })
}

/// キーボードの割り当てを `keys` にそろえる（無いキーは外す。既定と同じものは書かない）
fn apply_keys(config: &mut Config, keys: &[(KeyCode, DrumLane)]) {
    config.keys.clear();
    for (code, _) in inputs::DEFAULT_KEYS {
        if !keys.iter().any(|(k, _)| *k == code) {
            inputs::set_key(config, code, None);
        }
    }
    for &(code, lane) in keys {
        inputs::set_key(config, code, Some(lane));
    }
}

/// 引き継いだ結果
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Applied {
    /// 引き継いだ項目の数
    pub count: usize,
    /// 曲のフォルダを足した（探し直す）
    pub songs: bool,
    /// パッドの割り当てか機器の設定を変えた（入力を作り直す）
    pub pads: bool,
}

/// 選んだ項目を設定に写す（保存は呼ぶ側で）。`ports` は今つながっている MIDI の機器
pub fn apply(plan: &Plan, config: &mut Config, ports: &[PortInfo]) -> Applied {
    let mut out = Applied::default();
    let port_of = |t: &Target| {
        let (name, id) = t.port.as_ref()?;
        ports
            .iter()
            .position(|p| &p.name == name && &p.id == id)
            .or_else(|| ports.iter().position(|p| &p.name == name))
    };
    let mut touched_ports: Vec<usize> = Vec::new();
    for m in plan.midi.iter().filter(|m| m.on) {
        out.count += 1;
        out.pads = true;
        match plan.targets.get(m.target).and_then(port_of) {
            Some(port) => {
                for &(note, lane) in &m.notes {
                    inputs::assign_note(config, ports, port, note, lane);
                }
                touched_ports.push(port);
            }
            // まだ設定の無い機器に重ねる（つないだときに、その機器の設定に写る）
            None => {
                for &(note, lane) in &m.notes {
                    config.pad_notes.retain(|p| p.note != note);
                    config.pad_notes.push(PadNote {
                        note,
                        lane: lane.name().to_owned(),
                    });
                }
            }
        }
    }
    for item in plan.items.iter().filter(|i| i.on) {
        out.count += 1;
        match &item.change {
            Change::Value(s, v) => config_view::set(*s, config, *v),
            Change::Table(key, v) => config.settings.set_raw(key, *v),
            Change::SongDirs(dirs) => {
                for d in dirs {
                    out.songs |= config.add_song_dir(d.clone());
                }
            }
            Change::Keys(keys) => apply_keys(config, keys),
            Change::MinVelocity(v) => {
                out.pads = true;
                for &port in &touched_ports {
                    if let Some(d) = inputs::device_mut(config, ports, port) {
                        d.min_velocity = *v;
                    }
                }
            }
        }
    }
    out
}

/// 引き継ぐ前の設定を残すファイル
pub fn backup_path() -> Option<PathBuf> {
    Some(Config::dir()?.join("config.before-import.toml"))
}

/// 引き継ぐ前の設定を残す
pub fn save_backup(config: &Config) -> bool {
    let Some(path) = backup_path() else {
        return false;
    };
    let result = (|| -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(config)?)?;
        Ok(())
    })();
    match result {
        Ok(()) => true,
        Err(e) => {
            log::warn!("cannot save {}: {e:#}", path.display());
            false
        }
    }
}

/// 残しておいた引き継ぐ前の設定（読めなければ `None`）
pub fn load_backup() -> Option<Config> {
    let text = std::fs::read_to_string(backup_path()?).ok()?;
    toml::from_str(&text).ok()
}

/// 引き継ぐ前の設定を消す（戻したあと）
pub fn remove_backup() {
    if let Some(p) = backup_path()
        && p.exists()
        && let Err(e) = std::fs::remove_file(&p)
    {
        log::warn!("cannot remove {}: {e}", p.display());
    }
}

/// 引き継いだ日時（「11/25 18:40」。残しておいた設定が無ければ `None`）
pub fn backup_time() -> Option<String> {
    let modified = std::fs::metadata(backup_path()?).ok()?.modified().ok()?;
    let local: chrono::DateTime<chrono::Local> = modified.into();
    Some(local.format("%m/%d %H:%M").to_string())
}

/// 見つけた DTXMania を探すフォルダ（ショートカットを探すフォルダと、DTXMania のフォルダを探すフォルダ）
pub fn search_dirs() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut shortcuts = Vec::new();
    let mut folders = Vec::new();
    if let Some(d) = dirs::desktop_dir() {
        shortcuts.push(d.clone());
        folders.push(d);
    }
    // みんなのデスクトップとスタートメニュー（Windows）
    if let Some(p) = std::env::var_os("PUBLIC") {
        shortcuts.push(PathBuf::from(p).join("Desktop"));
    }
    if let Some(d) = dirs::data_dir() {
        shortcuts.push(
            d.join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    if let Some(p) = std::env::var_os("ProgramData") {
        shortcuts.push(
            PathBuf::from(p)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    for d in [dirs::document_dir(), dirs::download_dir(), dirs::home_dir()]
        .into_iter()
        .flatten()
    {
        if !folders.contains(&d) {
            folders.push(d);
        }
    }
    (shortcuts, folders)
}

/// 見つけて読めた DTXMania（設定の画面の「見つかった DTXMania」）
pub struct Search {
    pub sources: Vec<Source>,
    rx: Option<Receiver<Vec<Source>>>,
}

impl Search {
    /// 別のスレッドで探し始める
    pub fn start() -> Self {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("find DTXMania".to_owned())
            .spawn(move || {
                let (shortcuts, folders) = search_dirs();
                let found = nx::discover(&shortcuts, &folders);
                let sources: Vec<Source> = found
                    .iter()
                    .filter_map(|f| match nx::source::read(f) {
                        Ok(s) => Some(s),
                        Err(e) => {
                            log::info!("skipped {}: {e:?}", f.config.display());
                            None
                        }
                    })
                    .collect();
                let _ = tx.send(sources);
            });
        Self {
            sources: Vec::new(),
            rx: spawned.is_ok().then_some(rx),
        }
    }

    /// 探し終えたか見る（終えたら true を 1 回だけ）
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(s) => {
                self.sources = s;
                self.rx = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                true
            }
        }
    }

    pub fn searching(&self) -> bool {
        self.rx.is_some()
    }
}

/// 他の DTXMania からの引き継ぎのようす（App が持つ）
#[derive(Default)]
pub struct Import {
    /// 見つけた DTXMania（起動したときと「もう一度探す」で探す）
    pub search: Option<Search>,
    /// 読んだ設定から作った項目
    pub plan: Option<Plan>,
    /// 読めなかったわけ
    pub error: Option<String>,
    /// 引き継ぐ前の設定を残した日時
    pub backup: Option<String>,
}

impl Import {
    /// 探し始める
    pub fn start() -> Self {
        Self {
            search: Some(Search::start()),
            backup: backup_time(),
            ..Self::default()
        }
    }

    /// 見つけて読めた DTXMania
    pub fn sources(&self) -> &[Source] {
        self.search.as_ref().map_or(&[], |s| s.sources.as_slice())
    }

    /// 探し終えたか見る（終えたら true を 1 回だけ）
    pub fn poll(&mut self) -> bool {
        self.search.as_mut().is_some_and(Search::poll)
    }

    /// 設定の画面に出すもの
    pub fn info(&self) -> config_view::ImportInfo<'_> {
        config_view::ImportInfo {
            sources: self.sources(),
            searching: self.search.as_ref().is_some_and(Search::searching),
            plan: self.plan.as_ref(),
            error: self.error.as_deref(),
            backup: self.backup.as_deref(),
        }
    }
}

/// 指定したもの（ショートカット・exe・フォルダ・設定のファイル）から読む。読めなければ画面に出す文
pub fn open(path: &Path) -> Result<Source, String> {
    let found = nx::locate(path).map_err(|e| locate_message(&e))?;
    nx::source::read(&found).map_err(|e| match e {
        ReadError::Io(why) => tf!(
            "Couldn't read {}: {why}",
            "{} を読めませんでした: {why}",
            found.config.display()
        ),
        ReadError::NotDtxmania => tf!(
            "{} isn't a DTXMania settings file.",
            "{} は DTXMania の設定ではありません。",
            found.config.display()
        ),
    })
}

/// 見つからなかった理由の文
fn locate_message(e: &LocateError) -> String {
    match e {
        LocateError::Missing(p) => tf!("{} doesn't exist.", "{} がありません。", p.display()),
        LocateError::BadShortcut(p) => tf!(
            "Couldn't read the shortcut {}.",
            "ショートカット {} を読めませんでした。",
            p.display()
        ),
        LocateError::ShortcutTargetMissing { target, .. } => tf!(
            "The shortcut points to {}, which no longer exists. Choose the DTXMania folder instead.",
            "ショートカットの行き先 {} がありません。DTXMania のフォルダを選んでください。",
            target.display()
        ),
        LocateError::NoConfig { root } => tf!(
            "No settings file in {}. Start that DTXMania once, then try again.",
            "{} に設定のファイルがありません。その DTXMania を一度起動してから、もう一度読んでください。",
            root.display()
        ),
        LocateError::NotDtxmania(p) => tf!(
            "No DTXMania found in {}.",
            "{} に DTXMania が見つかりません。",
            p.display()
        ),
        LocateError::ItsUs(_) => t(
            "That's this app. Choose another DTXMania.",
            "これはこのアプリです。ほかの DTXMania を選んでください。",
        )
        .to_owned(),
        LocateError::Several(list) => tf!(
            "Found {} copies of DTXMania inside. Choose one of their folders.",
            "中に DTXMania が {} つあります。どれかのフォルダを選んでください。",
            list.len()
        ),
    }
}

/// 見つけた所の文（「デスクトップのショートカット」、フォルダの名前など）
pub fn found_by(f: &Found) -> String {
    match &f.shortcut {
        Some(s) => {
            let name = s
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            tf!("Shortcut \"{name}\"", "ショートカット「{name}」")
        }
        None => {
            let name = f
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.root.display().to_string());
            tf!("Folder \"{name}\"", "フォルダ「{name}」")
        }
    }
}

/// 設定のファイルを保存した日時（「2026/11/20 21:05」）
pub fn saved_at(s: &Source) -> Option<String> {
    let local: chrono::DateTime<chrono::Local> = s.saved?.into();
    Some(local.format("%Y/%m/%d %H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn source(text: &str, family: Family) -> Source {
        let root = std::env::temp_dir().join("nxt-takeover-none");
        Source {
            found: Found {
                config: root.join("Config.ini"),
                root,
                format: Format::Ini,
                exe: Some("DTXManiaNX.exe".to_owned()),
                shortcut: None,
            },
            ini: Ini::parse(text),
            family,
            version: Some("v2.0.0 Beta (20260121)".to_owned()),
            saved: Some(SystemTime::now()),
        }
    }

    fn port(name: &str) -> PortInfo {
        PortInfo {
            name: name.to_owned(),
            id: format!("{name}-id"),
            open: true,
            error: None,
        }
    }

    fn item<'a>(p: &'a Plan, raw_key: &str) -> &'a Item {
        p.items
            .iter()
            .find(|i| i.raw.starts_with(&format!("{raw_key}=")))
            .unwrap_or_else(|| panic!("no item for {raw_key}"))
    }

    const NX: &str = "\
[System]
Version=v2.0.0 Beta (20260121)
DamageLevel=0
Risky=0
HHGroup=1
FTGroup=2
RDPosition=0
DrumsScrollSpeed=5
InputAdjustTimeDrums=-12
ChipVolume=73
MasterVolume=100
HHVelocityMin=20
SDVelocityMin=5
SoundDeviceType=2
GuitarScrollSpeed=3
PedalLagTime=0
[PlayOption]
DrumsHiddenSudden=2
LBDGraphics=1
[AutoPlay]
LC=0
BD=1
GuitarR=1
[HitRange]
DrumPerfect=30
DrumPedalPerfect=34
Great=600
[DrumsKeyAssign]
HH=K033,M042,M022
SD=K012,K013,M038
BD=K0126,K048,M036
HO=M046,M142
CY=M049,M042,J012
LC=K035,K010,K054
[GuitarKeyAssign]
R=K054
";

    #[test]
    fn reads_nx_values_with_their_meaning() {
        let _en = i18n::scoped(i18n::Lang::En);
        let config = Config::default();
        let p = plan(&source(NX, Family::NxV2), &config, &[]);
        // ゲージの減り方: 上流 0（少ない）は選んで引き継ぐ
        let d = item(&p, "DamageLevel");
        assert_eq!(d.change, Change::Table(settings::DAMAGE_LEVEL, 0));
        assert!(d.on && !d.same && !d.untouched);
        // 既定のまま（Risky=0）は選ばない
        let r = item(&p, "Risky");
        assert!(r.untouched && r.same && !r.on);
        // RD の位置は並びが逆（上流 0 = RD RC → このアプリの 1）
        assert_eq!(
            item(&p, "RDPosition").change,
            Change::Table(settings::RD_POSITION, 1)
        );
        // LBD の絵 B は 4 つ目
        assert_eq!(
            item(&p, "LBDGraphics").change,
            Change::Table(settings::LBD_GRAPHICS, 3)
        );
        assert_eq!(
            item(&p, "DrumsHiddenSudden").change,
            Change::Table(settings::HID_SUD, 2)
        );
        // 補正は符号が逆
        assert_eq!(
            item(&p, "InputAdjustTimeDrums").change,
            Change::Value(Setting::InputAdjust, 12)
        );
        // 速さ: 0.089375 × 6 = 0.53625 → ×20 = 10.7 → 11
        assert_eq!(
            item(&p, "DrumsScrollSpeed").change,
            Change::Value(Setting::ScrollSpeed, 11)
        );
        // 音量は 5 刻みにそろえる
        let v = item(&p, "ChipVolume");
        assert_eq!(v.change, Change::Table(settings::CHIP_VOLUME, 75));
        assert!(v.note.is_some());
        // 全体の音量の 100 は上流の既定なので選ばない
        assert!(!item(&p, "MasterVolume").on);
        // 自動: BD だけ ON
        assert_eq!(item(&p, "BD").change, Change::Table("auto_bd", 1));
        assert!(item(&p, "BD").on);
        assert!(!item(&p, "LC").on);
        // 判定の幅: 名前つきが先、前の版の名前（Great）はドラムとペダルの両方。このアプリは 500 まで
        assert_eq!(
            item(&p, "DrumPerfect").change,
            Change::Table(settings::HIT_KEYS[0], 30)
        );
        assert!(!item(&p, "DrumPedalPerfect").on);
        let great: Vec<&Item> = p.items.iter().filter(|i| i.raw == "Great=600").collect();
        assert_eq!(great.len(), 2);
        assert_eq!(great[0].change, Change::Table(settings::HIT_KEYS[1], 500));
        assert_eq!(
            great[1].change,
            Change::Table(settings::PEDAL_HIT_KEYS[1], 500)
        );
        // FT の叩き分けの 2 はこのアプリに無い
        assert!(p.rejected.iter().any(|r| r.detail.starts_with("FTGroup=2")));
        assert!(!p.items.iter().any(|i| i.raw.starts_with("FTGroup")));
        // 音の機器・ギター（GuitarKeyAssign 1、GuitarScrollSpeed、GuitarR）・ジョイパッド・ペダルの補正（0 は出さない）
        let names: Vec<&str> = p.rejected.iter().map(|r| r.name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("ASIO")));
        let guitar = p
            .rejected
            .iter()
            .find(|r| r.name.contains("Guitar"))
            .unwrap();
        assert!(guitar.reason.starts_with('3'));
        assert!(names.iter().any(|n| n.contains("Joypad")));
        assert!(!names.iter().any(|n| n.contains("Pedal Lag")));
    }

    #[test]
    fn midi_goes_to_the_same_numbered_device_and_keeps_one_lane_per_note() {
        let _en = i18n::scoped(i18n::Lang::En);
        let config = Config::default();
        let ports = [port("V71"), port("FGDP-30")];
        let p = plan(&source(NX, Family::NxV2), &config, &ports);
        assert_eq!(p.midi.len(), 2);
        let first = &p.midi[0];
        assert_eq!(first.device, 0);
        assert_eq!(first.target, 0);
        // 42 は HH が先（CY の 42 は入れない）
        assert!(first.notes.contains(&(42, DrumLane::HH)));
        assert!(!first.notes.contains(&(42, DrumLane::CY)));
        assert!(first.notes.contains(&(49, DrumLane::CY)));
        assert!(first.notes.contains(&(46, DrumLane::HHO)));
        // 2 台目（M142）は 2 台目の機器へ
        assert_eq!(p.midi[1].device, 1);
        assert_eq!(p.midi[1].notes, vec![(42, DrumLane::HHO)]);
        assert_eq!(p.midi[1].target, 1);
        assert!(p.rejected.iter().any(|r| r.name.contains("Two Lanes")));
        // つながっていなければ、これからつなぐ機器へ
        let none = plan(&source(NX, Family::NxV2), &config, &[]);
        assert!(none.midi.iter().all(|m| m.target == none.targets.len() - 1));
    }

    #[test]
    fn applies_midi_keys_and_values() {
        let _en = i18n::scoped(i18n::Lang::En);
        let mut config = Config::default();
        let ports = [port("V71")];
        let mut p = plan(&source(NX, Family::NxV2), &config, &ports);
        // 1 台だけつながっていれば、DTXMania の 1 台目をそこへ。2 台目はこれからつなぐ機器へ
        assert_eq!(p.midi[0].target, 0);
        assert_eq!(p.midi[1].target, 1);
        // キーボード: LC の K054（F1）は割り当てられないキー、BD の Space も
        let keys = p
            .items
            .iter_mut()
            .find(|i| matches!(i.change, Change::Keys(_)))
            .unwrap();
        assert!(!keys.untouched);
        keys.on = true;
        let a = apply(&p, &mut config, &ports);
        assert!(a.pads);
        let d = inputs::device(&config, &ports, 0).unwrap();
        assert!(d.notes.contains(&PadNote {
            note: 22,
            lane: "HH".to_owned()
        }));
        assert!(config.pad_notes.contains(&PadNote {
            note: 42,
            lane: "HHO".to_owned()
        }));
        let map = inputs::keyboard_map(&config);
        assert!(map.contains(&(KeyCode::KeyX, DrumLane::HH)));
        assert!(map.contains(&(KeyCode::KeyC, DrumLane::SD)));
        // 上流に無いこのアプリの既定のキー（F → BD）は外す
        assert!(!map.iter().any(|(k, _)| *k == KeyCode::KeyF));
        assert!(!map.iter().any(|(k, _)| *k == KeyCode::Space));
        assert!(p.rejected.iter().any(|r| r.name.contains("Space")));
        assert_eq!(config.settings.choice(settings::DAMAGE_LEVEL), 0);
        assert_eq!(config.input_offset_ms, 12.0);
        // 選ばなかったもの（全体の音量）は変えない
        assert_eq!(config.master_volume, Config::default().master_volume);
    }

    #[test]
    fn untouched_keyboard_is_not_selected() {
        let text = "[System]\nVersion=v1.4.2 20240519\n[DrumsKeyAssign]\nHH=K033\nSD=K012,K013\nBD=K0126,K048\nHT=K031,K015\nLT=K011,K016\nFT=K023,K017\nCY=K022,K019\nHO=K028\nRD=K047,K020\nLC=K035,K010\nLP=K087\nLBD=K077\n";
        let p = plan(&source(text, Family::Nx1), &Config::default(), &[]);
        let k = p
            .items
            .iter()
            .find(|i| matches!(i.change, Change::Keys(_)))
            .unwrap();
        assert!(k.untouched && !k.on);
        assert!(p.midi.is_empty());
    }

    #[test]
    fn xg_and_classic_take_only_checked_items() {
        let _en = i18n::scoped(i18n::Lang::En);
        let text = "[System]\nVersion=Ver4.12(231107)\nDamageLevel=2\nHHGroup=2\nHitSound=0\n[DrumsKeyAssign]\nSD=M038\n";
        let xg = plan(&source(text, Family::XgVerK), &Config::default(), &[]);
        assert!(xg.items.iter().any(|i| i.raw == "DamageLevel=2"));
        assert!(xg.items.iter().any(|i| i.raw == "HitSound=0"));
        // 叩き分けは XG では意味を確かめていない
        assert!(!xg.items.iter().any(|i| i.raw.starts_with("HHGroup")));
        assert!(xg.rejected.iter().any(|r| r.name.contains("Other")));
        let classic = plan(&source(text, Family::Classic), &Config::default(), &[]);
        assert!(classic.items.iter().all(|i| i.part == Part::Pads));
        assert_eq!(classic.midi.len(), 1);
    }

    #[test]
    fn song_folders_are_resolved_and_checked() {
        let dir = std::env::temp_dir().join(format!("nxt-takeover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("DTXFiles")).unwrap();
        let mut s = source(
            "[System]\nVersion=v2.0.0\nDTXPath=.\\DTXFiles\\;Z:\\nowhere\\;\n",
            Family::NxV2,
        );
        s.found.root = dir.clone();
        let mut config = Config::default();
        let p = plan(&s, &config, &[]);
        let songs = p.items.iter().find(|i| i.part == Part::Songs).unwrap();
        assert_eq!(songs.change, Change::SongDirs(vec![dir.join("DTXFiles")]));
        assert!(songs.on);
        assert!(p.rejected.iter().any(|r| r.name.contains("nowhere")));
        assert!(apply(&p, &mut config, &[]).songs);
        // もう足してあれば同じ
        let again = plan(&s, &config, &[]);
        assert!(
            again
                .items
                .iter()
                .find(|i| i.part == Part::Songs)
                .unwrap()
                .same
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn velocity_takes_the_lowest_and_counts_from_the_next_value() {
        let _en = i18n::scoped(i18n::Lang::En);
        let text = "[System]\nVersion=v2.0.0\nHHVelocityMin=30\nSDVelocityMin=12\n[DrumsKeyAssign]\nSD=M038\n";
        let mut config = Config::default();
        let ports = [port("TD-17")];
        let p = plan(&source(text, Family::NxV2), &config, &ports);
        let v = p
            .items
            .iter()
            .find(|i| matches!(i.change, Change::MinVelocity(_)))
            .unwrap();
        // 上流は 12 以下を捨てる → このアプリは 13 より弱いものを捨てる
        assert_eq!(v.change, Change::MinVelocity(13));
        assert!(v.on);
        assert!(p.rejected.iter().any(|r| r.name.contains("Per-Pad")));
        apply(&p, &mut config, &ports);
        assert_eq!(inputs::device(&config, &ports, 0).unwrap().min_velocity, 13);
    }

    #[test]
    fn slimdx_keys_map_to_the_same_keys() {
        assert_eq!(slimdx_key(33), Some((Some(KeyCode::KeyX), "X")));
        assert_eq!(slimdx_key(0), Some((Some(KeyCode::Digit0), "0")));
        assert_eq!(slimdx_key(89), Some((Some(KeyCode::Numpad0), "Num0")));
        assert_eq!(slimdx_key(143), Some((Some(KeyCode::IntlYen), "Yen")));
        assert_eq!(slimdx_key(200), None);
        // 上流の既定の LP（無変換）は、このアプリでは割り当てられない
        let (k, _) = slimdx_key(87).unwrap();
        assert!(!inputs::assignable(k.unwrap()));
    }
}
