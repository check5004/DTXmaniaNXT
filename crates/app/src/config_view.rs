//! 設定の画面（2026-11-22 に作り直した。見本から選んだ案 2「3 列」）。
//! 左に分類、真ん中に項目の一覧、右に説明と見本を出す。背景と上下の帯は NX v2.0 のスキン（4_ の画像）のままで、
//! 行や板は絵を足さずにコードで描く（[`crate::settings_ui`]）。
//!
//! パッドの役目は固定して、画面の下に出す。HT 上、LT 下、SD 左、FT 右、CY / RD 決定、LC 戻る
//! （キーボードは矢印、Enter、Esc）。
//! - 値は左右（◀▶）でその場で変える（値を変える状態は無い）。4 つのパッドだけで操作するときは、決定で選択肢の一覧か
//!   数を選ぶ窓を開き、上下で変えて決定で閉じる（戻るなら元に戻す）
//! - 一覧は端で回らず、「3 / 12」と何番目かを出す
//! - 割り当ては「順に叩いて割り当てる」か、レーンを決定してから叩く（そのレーンに割り当て済みなら外す）。
//!   叩かずに 5 秒たてば取りやめ
//! - 取り消せない操作（割り当てを戻す、曲のフォルダを外す）は確かめる
//!
//! 項目の多くは設定の表（[`crate::settings`]）から作る。表に足した項目は、分類と小見出しのとおりにここへ出る。
//! 出入りは NX v2.0 と同じくシャッター（`shutter`）。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use dtx::DrumLane;
use egui::Color32;
use engine::input::Preset;
use engine::practice::{
    LoopStyle, PassRule, SPEED_MAX, SPEED_MIN, SPEED_STEP, SpeedPlan, snap_speed,
};
use library::Progress;
use winit::keyboard::KeyCode;

use crate::config::{Config, MASTER_VOLUME_STEP};
use crate::gfx::{Canvas, Gfx};
use crate::inputs::{KEYBOARD, LaneSource, lane_label};
use crate::settings::{self, Group, Kind};
use crate::settings_ui::{self as ui, Icon, Pad};
use crate::sfx::Se;
use crate::stretch::Pitch;
use crate::text::Style;
use crate::title_view::draw_scanning;

/// 補正の値の範囲（ミリ秒）
pub const OFFSET_RANGE: f64 = 200.0;

/// レーンを決定してから叩くのを待つ時間
const WAIT_FOR: Duration = Duration::from_secs(5);
/// 順に割り当てるとき、叩かなければ次のレーンへ飛ばすまで
const FLOW_STEP: Duration = Duration::from_secs(4);
/// 順に割り当てるとき、1 つ割り当ててから次の打撃を受けるまで（1 回の打撃でノートが 2 つ届く機器があるので）
const FLOW_GUARD: Duration = Duration::from_millis(300);
/// 画面の下の短い知らせを出しておく時間
const TOAST_FOR: Duration = Duration::from_millis(2500);

/// 分類（左の列）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cat {
    /// よく使う（選曲の簡易設定と同じ並び）
    Fav,
    Play,
    View,
    Timing,
    Sound,
    Input,
    Songs,
    Practice,
    App,
}

/// 左の列の並び
pub const CATS: [Cat; 9] = [
    Cat::Fav,
    Cat::Play,
    Cat::View,
    Cat::Timing,
    Cat::Sound,
    Cat::Input,
    Cat::Songs,
    Cat::Practice,
    Cat::App,
];

impl Cat {
    /// 設定の表の分類（よく使うは表に無い）
    fn group(self) -> Option<Group> {
        match self {
            Cat::Fav => None,
            Cat::Play => Some(Group::Play),
            Cat::View => Some(Group::View),
            Cat::Timing => Some(Group::Timing),
            Cat::Sound => Some(Group::Sound),
            Cat::Input => Some(Group::Input),
            Cat::Songs => Some(Group::Songs),
            Cat::Practice => Some(Group::Practice),
            Cat::App => Some(Group::App),
        }
    }

    pub fn name(self) -> &'static str {
        self.group().map_or("よく使う", Group::label)
    }

    pub fn desc(self) -> &'static str {
        match self {
            Cat::Fav => "よく変える項目です。",
            Cat::Play => "ゲームの決まり（STAGE FAILED、自動演奏、判定）。",
            Cat::View => "譜面の流れ方と、画面に出すもの。",
            Cat::Timing => "叩いた時刻と表示の補正。",
            Cat::Sound => "音量と、鳴らすもの。",
            Cat::Input => "パッドとキーの割り当てと、MIDI 機器ごとの設定。",
            Cat::Songs => "曲のフォルダと、曲の探し直し。",
            Cat::Practice => {
                "区間練習の決めごとの既定です。選曲の練習のポップアップで変えたものも、ここに残ります。"
            }
            Cat::App => "更新など、アプリ全体のこと。",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Cat::Fav => Icon::Star,
            Cat::Play => Icon::Stick,
            Cat::View => Icon::Lanes,
            Cat::Timing => Icon::Metronome,
            Cat::Sound => Icon::Speaker,
            Cat::Input => Icon::Midi,
            Cat::Songs => Icon::Folder,
            Cat::Practice => Icon::Loop,
            Cat::App => Icon::Gear,
        }
    }
}

/// 一覧（分類の一覧か、その中で開いた一覧）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Cat(Cat),
    /// パッドとキーの割り当て
    Lanes,
    /// MIDI の機器（ポートの何番目か）
    Device(usize),
}

/// 値を変えられる設定（`Config` の欄）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    MasterVolume,
    InputAdjust,
    VisualAdjust,
    ScrollSpeed,
    StageFailed,
    HitSound,
    ShowLagTime,
    ShowLagTimeColor,
    ShowLagHitCount,
    AutoUpdate,
}

/// 練習の決めごと（練習のポップアップと同じ項目。区間のほか）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drill {
    Style,
    Rule,
    /// 速度の決め方（段階アップか固定か）
    Ladder,
    Speed,
    Target,
    Pitch,
}

/// 一覧の項目
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemId {
    Value(Setting),
    /// 設定の表（[`crate::settings`]）の項目（キー）
    Table(&'static str),
    /// 練習の決めごとの既定
    Practice(Drill),
    AddSongDir,
    SongDir(usize),
    Rescan,
    /// パッドとキーの割り当ての一覧へ
    Lanes,
    /// 順に叩いて割り当てる
    AssignAll,
    Lane(DrumLane),
    ResetPads,
    /// MIDI の機器（ポートの何番目か）の一覧へ
    Device(usize),
    DeviceState(usize),
    DeviceUse(usize),
    DevicePreset(usize),
    DeviceChannel(usize),
    DeviceMinVelocity(usize),
    DeviceReset(usize),
    /// MIDI の機器を探し直す
    ReconnectMidi,
}

/// 数の見せ方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fmt {
    Plain,
    /// 20 で割って小数 2 桁（譜面の速さ）
    Speed,
    /// 符号つきのミリ秒
    Ms,
    /// 0 は「すべて」（MIDI のチャンネル）
    Channel,
    /// 0 は OFF
    OffZero,
    /// 単位をつける
    Unit(&'static str),
}

impl Fmt {
    pub fn show(self, v: i64) -> String {
        match self {
            Fmt::Plain => v.to_string(),
            Fmt::Speed => format!("{:.2}", v as f64 / 20.0),
            Fmt::Ms => format!("{v:+} ms"),
            Fmt::Channel if v == 0 => "すべて".to_owned(),
            Fmt::OffZero if v == 0 => "OFF".to_owned(),
            Fmt::Channel | Fmt::OffZero => v.to_string(),
            Fmt::Unit(u) => format!("{v}{u}"),
        }
    }
}

/// 行の操作のしかた
#[derive(Debug, Clone, PartialEq)]
pub enum Ctl {
    /// 小見出し（選べない）。名前が空なら隙間だけ
    Heading,
    /// ON / OFF（左が ON、右が OFF）。`None` は混ざっている（まとめて切り替える項目）
    Toggle {
        on: Option<bool>,
        default: Option<bool>,
    },
    /// 選択肢から 1 つ
    Choice {
        labels: Vec<&'static str>,
        at: usize,
        default: Option<usize>,
    },
    /// 数
    Number {
        at: i64,
        min: i64,
        max: i64,
        step: i64,
        default: Option<i64>,
        fmt: Fmt,
    },
    /// 決定で一覧を開く
    Page(Page),
    /// 決定で動く（確かめる文があれば先に聞く）
    Action { confirm: Option<String> },
    /// レーンの割り当て（決定してから叩いて足す・外す）
    Lane(DrumLane),
    /// 見るだけ
    Info,
}

/// 行の色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Normal,
    /// 勧める操作（順に叩いて割り当てる）
    Strong,
    /// 取り消せない操作
    Danger,
}

/// 一覧の 1 行
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// 小見出しは `None`
    pub id: Option<ItemId>,
    pub name: String,
    /// 一覧に出す値（開く一覧なら今の様子）
    pub value: String,
    pub desc: String,
    pub ctl: Ctl,
    pub tone: Tone,
}

impl Row {
    pub fn selectable(&self) -> bool {
        self.id.is_some()
    }

    /// 左右で値を変えられる
    pub fn editable(&self) -> bool {
        matches!(
            self.ctl,
            Ctl::Toggle { .. } | Ctl::Choice { .. } | Ctl::Number { .. }
        )
    }

    /// 既定から変えている
    pub fn changed(&self) -> bool {
        match &self.ctl {
            Ctl::Toggle { on, default } => default.is_some() && on != default,
            Ctl::Choice { at, default, .. } => default.is_some_and(|d| d != *at),
            Ctl::Number { at, default, .. } => default.is_some_and(|d| d != *at),
            _ => false,
        }
    }

    /// 既定の値の表示
    pub fn default_label(&self) -> Option<String> {
        match &self.ctl {
            Ctl::Toggle { default, .. } => default.map(on_off),
            Ctl::Choice {
                labels, default, ..
            } => default.and_then(|d| labels.get(d)).map(|s| (*s).to_owned()),
            Ctl::Number { default, fmt, .. } => default.map(|d| fmt.show(d)),
            _ => None,
        }
    }

    /// 選択肢の名前と、今は何番目か（ON / OFF も 2 つの選択肢。混ざっていれば何番目でもない）
    pub fn options(&self) -> Option<(Vec<&'static str>, Option<usize>)> {
        match &self.ctl {
            Ctl::Toggle { on, .. } => Some((
                vec!["ON", "OFF"],
                on.map(|b| if b { 0 } else { 1 }),
            )),
            Ctl::Choice { labels, at, .. } => Some((labels.clone(), Some(*at))),
            _ => None,
        }
    }
}

fn on_off(b: bool) -> String {
    (if b { "ON" } else { "OFF" }).to_owned()
}

fn heading(name: &str) -> Row {
    Row {
        id: None,
        name: name.to_owned(),
        value: String::new(),
        desc: String::new(),
        ctl: Ctl::Heading,
        tone: Tone::Normal,
    }
}

fn row(id: ItemId, name: &str, value: String, desc: String, ctl: Ctl) -> Row {
    Row {
        id: Some(id),
        name: name.to_owned(),
        value,
        desc,
        ctl,
        tone: Tone::Normal,
    }
}

fn toggle(id: ItemId, name: &str, on: bool, default: bool, desc: &str) -> Row {
    row(
        id,
        name,
        on_off(on),
        desc.to_owned(),
        Ctl::Toggle {
            on: Some(on),
            default: Some(default),
        },
    )
}

fn choice(
    id: ItemId,
    name: &str,
    labels: &[&'static str],
    at: usize,
    default: Option<usize>,
    desc: &str,
) -> Row {
    let at = at.min(labels.len().saturating_sub(1));
    row(
        id,
        name,
        labels.get(at).copied().unwrap_or("").to_owned(),
        desc.to_owned(),
        Ctl::Choice {
            labels: labels.to_vec(),
            at,
            default,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn number(
    id: ItemId,
    name: &str,
    at: i64,
    (min, max, step): (i64, i64, i64),
    default: Option<i64>,
    fmt: Fmt,
    desc: String,
) -> Row {
    row(
        id,
        name,
        fmt.show(at),
        desc,
        Ctl::Number {
            at,
            min,
            max,
            step,
            default,
            fmt,
        },
    )
}

fn page_row(id: ItemId, name: &str, p: Page, summary: String, desc: String) -> Row {
    row(id, name, summary, desc, Ctl::Page(p))
}

fn action(id: ItemId, name: &str, value: String, desc: String, confirm: Option<String>) -> Row {
    let tone = if confirm.is_some() {
        Tone::Danger
    } else {
        Tone::Normal
    };
    Row {
        tone,
        ..row(id, name, value, desc, Ctl::Action { confirm })
    }
}

/// 設定の値を一覧の数にする（ON / OFF は 1 と 0、選択肢は何番目か、譜面の速さは 20 倍）
pub fn get(s: Setting, c: &Config) -> i64 {
    match s {
        Setting::MasterVolume => i64::from(c.master_volume()),
        Setting::InputAdjust => c.input_offset_ms.round() as i64,
        Setting::VisualAdjust => c.visual_offset_ms.round() as i64,
        Setting::ScrollSpeed => (f64::from(c.scroll_px_per_ms) * 20.0).round() as i64,
        Setting::StageFailed => i64::from(c.stage_failed()),
        Setting::HitSound => i64::from(c.drum_sounds),
        Setting::ShowLagTime => i64::from(c.show_lag_time.min(2)),
        Setting::ShowLagTimeColor => i64::from(c.show_lag_time_color.min(1)),
        Setting::ShowLagHitCount => i64::from(c.show_lag_hit_count),
        Setting::AutoUpdate => i64::from(c.auto_update),
    }
}

/// [`get`] の形の値を設定に書く（範囲の外は端に）
pub fn set(s: Setting, c: &mut Config, v: i64) {
    let r = OFFSET_RANGE as i64;
    match s {
        Setting::MasterVolume => c.set_master_volume(v),
        Setting::InputAdjust => c.input_offset_ms = v.clamp(-r, r) as f64,
        Setting::VisualAdjust => c.visual_offset_ms = v.clamp(-r, r) as f64,
        Setting::ScrollSpeed => c.scroll_px_per_ms = (v.clamp(2, 60) as f64 / 20.0) as f32,
        Setting::StageFailed => c.stage_failed = Some(v != 0),
        Setting::HitSound => c.drum_sounds = v != 0,
        Setting::ShowLagTime => c.show_lag_time = v.clamp(0, 2) as u8,
        Setting::ShowLagTimeColor => c.show_lag_time_color = v.clamp(0, 1) as u8,
        Setting::ShowLagHitCount => c.show_lag_hit_count = v != 0,
        Setting::AutoUpdate => c.auto_update = v != 0,
    }
}

/// `Config` の欄の項目の行
fn value_row(s: Setting, c: &Config) -> Row {
    let d = Config::default();
    let id = ItemId::Value(s);
    let at = get(s, c);
    let def = get(s, &d);
    let flag = |name: &str, desc: &str| toggle(id, name, at != 0, def != 0, desc);
    match s {
        Setting::ScrollSpeed => number(
            id,
            "譜面の速さ",
            at,
            (2, 60, 1),
            Some(def),
            Fmt::Speed,
            "音符の流れる速さです。大きいほど速く流れ、音符の間が広がります。\n演奏中は ↑↓ キーでも変えられます。"
                .to_owned(),
        ),
        Setting::InputAdjust => number(
            id,
            "入力の補正",
            at,
            (-(OFFSET_RANGE as i64), OFFSET_RANGE as i64, 1),
            Some(def),
            Fmt::Ms,
            "叩いた時刻の補正です。結果の画面のずれがいつも遅めなら大きく、早めなら小さくします。\n演奏中は - と = キーでも変えられます。"
                .to_owned(),
        ),
        Setting::VisualAdjust => number(
            id,
            "表示の補正",
            at,
            (-(OFFSET_RANGE as i64), OFFSET_RANGE as i64, 1),
            Some(def),
            Fmt::Ms,
            "譜面の表示の補正です。音と見た目がずれて見えるときに直します。判定は変わりません。\n演奏中は [ と ] キーでも変えられます。"
                .to_owned(),
        ),
        Setting::MasterVolume => number(
            id,
            "全体の音量",
            at,
            (0, 100, i64::from(MASTER_VOLUME_STEP)),
            Some(def),
            Fmt::Plain,
            format!(
                "全体の音量です。0 が無音で、100 が最大です。{MASTER_VOLUME_STEP} ずつ変わります。"
            ),
        ),
        Setting::HitSound => flag(
            "叩いた音",
            &format!(
                "叩いたときに譜面の音を鳴らします。電子ドラムの音源の音を使うなら OFF にします。\n演奏中は {} キーでも切り替えられます。",
                crate::HIT_SOUND_KEY_NAME
            ),
        ),
        Setting::StageFailed => flag(
            "ゲージ切れで中断",
            "ON にすると、ゲージがなくなったときに STAGE FAILED（演奏失敗）となり、演奏が中断されます。練習では中断しません。",
        ),
        Setting::ShowLagTime => choice(
            id,
            "ずれの表示",
            &["OFF", "ON", "PERFECT 以外"],
            at as usize,
            Some(def as usize),
            "判定の下に、ちょうどの時刻からのずれ（ms）を出します。\n「PERFECT 以外」は PERFECT でなかったときだけ出します。",
        ),
        Setting::ShowLagTimeColor => choice(
            id,
            "ずれの色",
            &["早いと青", "早いと赤"],
            at as usize,
            Some(def as usize),
            "ずれの数字の色です。\n早いと青: 早いと青、遅いと赤。\n早いと赤: 早いと赤、遅いと青。",
        ),
        Setting::ShowLagHitCount => flag(
            "早い・遅いの数",
            "ON にすると、結果の画面に早かった数と遅かった数を出します。",
        ),
        Setting::AutoUpdate => flag(
            "自動更新",
            "ON にすると、起動したときに新しい版を GitHub から落とし、終了したときに入れ替えます。\nSetup.exe で入れたときだけ働きます。切り替えは次の起動から効きます。",
        ),
    }
}

/// 練習の決めごとを一覧の数にする（選択肢は何番目か。段階アップは 0、固定は 1）
pub fn get_drill(d: Drill, c: &Config) -> i64 {
    let at = |i: Option<usize>| i.unwrap_or(0) as i64;
    match d {
        Drill::Style => at(LoopStyle::ALL.iter().position(|&s| s == c.loop_style())),
        Drill::Rule => at(PassRule::ALL.iter().position(|&r| r == c.pass_rule())),
        Drill::Ladder => i64::from(!c.practice_ladder),
        Drill::Speed => i64::from(snap_speed(i64::from(c.practice_speed))),
        Drill::Target => at(SpeedPlan::TARGETS
            .iter()
            .position(|&t| t == snap_speed(i64::from(c.practice_target)))),
        Drill::Pitch => at(Pitch::ALL.iter().position(|&p| p == c.pitch())),
    }
}

/// [`get_drill`] の形の値を設定に書く
pub fn set_drill(d: Drill, c: &mut Config, v: i64) {
    let pick = |n: usize| v.clamp(0, n as i64 - 1) as usize;
    match d {
        Drill::Style => c.practice_style = LoopStyle::ALL[pick(2)].name().to_owned(),
        Drill::Rule => c.practice_rule = PassRule::ALL[pick(3)].name().to_owned(),
        Drill::Ladder => c.practice_ladder = v == 0,
        Drill::Speed => c.practice_speed = snap_speed(v),
        Drill::Target => {
            c.practice_target = SpeedPlan::TARGETS[pick(SpeedPlan::TARGETS.len())];
        }
        Drill::Pitch => c.practice_pitch = Pitch::ALL[pick(2)].name().to_owned(),
    }
}

/// 練習の決めごとの行
fn drill_row(d: Drill, c: &Config) -> Row {
    let id = ItemId::Practice(d);
    let at = get_drill(d, c);
    let def = Some(get_drill(d, &Config::default()));
    let pick = |name: &str, labels: &[&'static str], desc: &str| {
        choice(id, name, labels, at as usize, def.map(|v| v as usize), desc)
    };
    match d {
        Drill::Style => pick(
            "つなぎ方",
            &["切れ目なく", "毎回区切る"],
            "切れ目なく: 区間の終わりから切れ目なく頭へ戻ります（ビート向き）。\n毎回区切る: 次の小節の頭の 1 打まで叩き、助走からやり直します（フィル向き）。",
        ),
        Drill::Rule => pick(
            "合格の基準",
            &["Easy", "Normal", "Strict"],
            "Easy: MISS が 0 なら合格。\nNormal: フルコンボ（MISS と POOR が 0）で合格。\nStrict: フルコンボで、PERFECT が 8 割以上なら合格。",
        ),
        Drill::Ladder => pick(
            "速度の決め方",
            &["段階アップ", "固定"],
            "段階アップ: 合格を重ねるたびに速度を上げ、不合格が続くと下げます。\n固定: 同じ速度のまま回ります（練習中も , と . キーで 5% ずつ変えられます）。",
        ),
        Drill::Speed => number(
            id,
            if c.practice_ladder {
                "始めの速度"
            } else {
                "速度"
            },
            at,
            (i64::from(SPEED_MIN), i64::from(SPEED_MAX), i64::from(SPEED_STEP)),
            def,
            Fmt::Unit("%"),
            "練習の速度です（曲の速さの何 % か）。段階アップなら 1 周目の速度です。".to_owned(),
        ),
        Drill::Target => pick(
            "目標の速度",
            &["100%", "105%", "110%"],
            "段階アップの目標の速度です。100% より上にすると、本番に余裕ができます。固定のときは使いません。",
        ),
        Drill::Pitch => pick(
            "遅くしたときの音程",
            &["保つ", "一緒に下げる"],
            "保つ: 音程を保ったまま遅くします。\n一緒に下げる: 音程ごと遅くします（テープと同じ）。",
        ),
    }
}

/// 設定の表の項目の行
fn table_row(d: &'static settings::Def, v: &settings::Values) -> Row {
    let id = ItemId::Table(d.key);
    let at = v.raw(d.key);
    let def = settings::default_raw(d.key);
    let ctl = match d.kind {
        Kind::Toggle { .. } | Kind::All { .. } => Ctl::Toggle {
            on: at.map(|a| a != 0),
            default: def.map(|a| a != 0),
        },
        Kind::Choice { choices, .. } => Ctl::Choice {
            labels: choices.iter().map(|c| c.label).collect(),
            at: at.unwrap_or(0) as usize,
            default: def.map(|a| a as usize),
        },
        Kind::Number {
            min,
            max,
            step,
            unit,
            ..
        } => Ctl::Number {
            at: at.unwrap_or(0),
            min,
            max,
            step,
            default: def,
            fmt: Fmt::Unit(unit),
        },
    };
    row(id, d.name, v.label(d.key), d.desc.to_owned(), ctl)
}

/// 小見出しごとに行を集める（同じ小見出しの行は、後から足しても同じ所に入る）
#[derive(Default)]
struct Sections(Vec<(&'static str, Vec<Row>)>);

impl Sections {
    fn at(&mut self, section: &'static str) -> &mut Vec<Row> {
        let i = match self.0.iter().position(|(s, _)| *s == section) {
            Some(i) => i,
            None => {
                self.0.push((section, Vec::new()));
                self.0.len() - 1
            }
        };
        &mut self.0[i].1
    }

    /// 設定の表の分類 `group` の項目を、それぞれの小見出しへ
    fn table(&mut self, group: Group, v: &settings::Values) {
        for d in settings::in_group(group) {
            self.at(d.section).push(table_row(d, v));
        }
    }

    fn rows(self) -> Vec<Row> {
        let mut out = Vec::new();
        for (i, (name, rows)) in self.0.into_iter().enumerate() {
            if rows.is_empty() {
                continue;
            }
            // 先頭の小見出しの無い組には隙間も入れない
            if i > 0 || !name.is_empty() {
                out.push(heading(name));
            }
            out.extend(rows);
        }
        out
    }
}

/// MIDI の機器 1 つ（OS が見せるポート）の様子
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub name: String,
    /// OS のポートの ID
    pub id: String,
    pub open: bool,
    /// 開けなかったわけ
    pub error: Option<String>,
    /// 使わない設定
    pub disabled: bool,
    pub preset: Preset,
    /// 名前から選ぶ出発点（既定）
    pub default_preset: Preset,
    /// 受けるチャンネル（1〜16、0 はすべて）
    pub channel: u8,
    /// これより弱い打撃は捨てる（0 は捨てない）
    pub min_velocity: u8,
    /// 出発点から変えた割り当ての数
    pub changes: usize,
    /// 最後に届いた打撃
    pub last: Option<String>,
}

/// 項目を作るのに要るもの
pub struct ConfigInfo<'a> {
    pub config: &'a Config,
    /// レーンを叩ける入力（機器ごと）
    pub lanes: BTreeMap<DrumLane, Vec<LaneSource>>,
    /// MIDI の機器（OS が見せるポートの並び）
    pub devices: Vec<DeviceRow>,
    pub scanning: Option<Progress>,
    pub song_count: usize,
}

/// パッドの割り当ての並び（上流 `tSetupItemList_KeyAssignDrums`）。順に割り当てるときもこの順
pub const PADS: [DrumLane; 11] = [
    DrumLane::LC,
    DrumLane::HH,
    DrumLane::HHO,
    DrumLane::SD,
    DrumLane::BD,
    DrumLane::HT,
    DrumLane::LT,
    DrumLane::FT,
    DrumLane::CY,
    DrumLane::RD,
    DrumLane::LP,
];

fn folder_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// 機器の一覧の値（出発点か、使っていないわけ）
fn device_state(d: &DeviceRow) -> String {
    if d.disabled {
        "使わない".to_owned()
    } else if !d.open {
        "開けません".to_owned()
    } else {
        d.preset.label().to_owned()
    }
}

/// 機器の様子の説明
fn device_summary(d: &DeviceRow) -> String {
    if d.disabled {
        "使わない設定です。".to_owned()
    } else if let Some(e) = &d.error {
        format!("開けません（ほかのアプリが使っているかも）: {e}")
    } else {
        format!(
            "{} / チャンネル {} / 変更 {} 個",
            d.preset.label(),
            Fmt::Channel.show(i64::from(d.channel)),
            d.changes
        )
    }
}

/// レーンに割り当てた入力（機器ごと。キーボードは「キー」）
pub fn lane_value(sources: &[LaneSource]) -> String {
    if sources.is_empty() {
        return "-".to_owned();
    }
    sources
        .iter()
        .map(|s| {
            if s.source == KEYBOARD {
                format!("キー {}", s.inputs.join(" "))
            } else {
                s.inputs.join(" ")
            }
        })
        .collect::<Vec<_>>()
        .join("　")
}

/// 一覧の行
pub fn rows(page: Page, info: &ConfigInfo<'_>) -> Vec<Row> {
    let c = info.config;
    let v = &c.settings;
    let mut s = Sections::default();
    match page {
        Page::Cat(Cat::Fav) => {
            let r = s.at("");
            for x in [
                Setting::ScrollSpeed,
                Setting::InputAdjust,
                Setting::HitSound,
                Setting::MasterVolume,
                Setting::ShowLagTime,
                Setting::StageFailed,
            ] {
                r.push(value_row(x, c));
            }
            r.push(table_row(settings::def(settings::AUTO_ALL), v));
        }
        Page::Cat(Cat::Play) => {
            s.at("演奏の決まり")
                .push(value_row(Setting::StageFailed, c));
            s.table(Group::Play, v);
            s.at("判定").push(value_row(Setting::ShowLagHitCount, c));
        }
        Page::Cat(Cat::View) => {
            s.at("流れ方").push(value_row(Setting::ScrollSpeed, c));
            let lag = s.at("ずれの表示");
            lag.push(value_row(Setting::ShowLagTime, c));
            lag.push(value_row(Setting::ShowLagTimeColor, c));
            s.table(Group::View, v);
        }
        Page::Cat(Cat::Timing) => {
            let r = s.at("");
            r.push(value_row(Setting::InputAdjust, c));
            r.push(value_row(Setting::VisualAdjust, c));
            s.table(Group::Timing, v);
        }
        Page::Cat(Cat::Sound) => {
            let r = s.at("");
            r.push(value_row(Setting::MasterVolume, c));
            r.push(value_row(Setting::HitSound, c));
            s.table(Group::Sound, v);
        }
        Page::Cat(Cat::Input) => {
            let assigned = PADS.iter().filter(|l| info.lanes.contains_key(l)).count();
            s.at("").push(page_row(
                ItemId::Lanes,
                "パッドとキーの割り当て",
                Page::Lanes,
                format!("{assigned} / {} レーン", PADS.len()),
                "レーンごとに、叩くパッド（MIDI のノート番号）とキーボードのキーを決めます。\n「順に叩いて割り当てる」で、全部のレーンをまとめて決められます。"
                    .to_owned(),
            ));
            let devices = s.at("MIDI 機器");
            for (i, d) in info.devices.iter().enumerate() {
                devices.push(page_row(
                    ItemId::Device(i),
                    &d.name,
                    Page::Device(i),
                    device_state(d),
                    format!(
                        "{}\n決定でこの機器の設定（使うか、プリセット、チャンネル、最小ベロシティ）を開きます。",
                        device_summary(d)
                    ),
                ));
            }
            devices.push(action(
                ItemId::ReconnectMidi,
                "機器を探し直す",
                if info.devices.is_empty() {
                    "見つかりません".to_owned()
                } else {
                    format!("{} 台", info.devices.len())
                },
                "MIDI の機器を探し直して、開き直します。後からつないだ機器は、この画面にいれば自動でも見つけます。"
                    .to_owned(),
                None,
            ));
            s.table(Group::Input, v);
        }
        Page::Cat(Cat::Songs) => {
            let dirs = s.at("曲のフォルダ");
            for (i, d) in c.song_dirs.iter().enumerate() {
                let name = folder_name(d);
                dirs.push(action(
                    ItemId::SongDir(i),
                    &format!("フォルダ {}", i + 1),
                    name.clone(),
                    format!(
                        "{}\n決定で一覧から外します（ファイルは消しません）。",
                        d.display()
                    ),
                    Some(format!(
                        "曲のフォルダ「{name}」を一覧から外しますか？\nフォルダの中のファイルは消しません。"
                    )),
                ));
            }
            dirs.push(action(
                ItemId::AddSongDir,
                "フォルダを足す",
                String::new(),
                "曲のフォルダを足します。決定でフォルダを選ぶ窓を開きます（マウスで選びます）。窓にドロップしても足せます。"
                    .to_owned(),
                None,
            ));
            s.at(" ").push(action(
                ItemId::Rescan,
                "曲を探し直す",
                match info.scanning {
                    Some(p) => format!("{} 曲…", p.songs),
                    None => format!("{} 曲", info.song_count),
                },
                "曲のフォルダを探し直して、曲の一覧を作り直します。".to_owned(),
                None,
            ));
            s.table(Group::Songs, v);
        }
        Page::Cat(Cat::Practice) => {
            let r = s.at("");
            for d in [
                Drill::Style,
                Drill::Rule,
                Drill::Ladder,
                Drill::Speed,
                Drill::Target,
                Drill::Pitch,
            ] {
                r.push(drill_row(d, c));
            }
            s.table(Group::Practice, v);
        }
        Page::Cat(Cat::App) => {
            s.at("").push(value_row(Setting::AutoUpdate, c));
            s.table(Group::App, v);
        }
        Page::Lanes => {
            s.at("").push(Row {
                tone: Tone::Strong,
                ..action(
                    ItemId::AssignAll,
                    "順に叩いて割り当てる",
                    String::new(),
                    "左シンバルからライドまで、言われたレーンのパッドを順に叩いて割り当てます。叩いた機器の割り当てになります。\n叩かずに 4 秒たつと次のレーンへ飛ばします。"
                        .to_owned(),
                    None,
                )
            });
            let lanes = s.at("レーン");
            for lane in PADS {
                let sources = info.lanes.get(&lane).map_or(&[][..], Vec::as_slice);
                let mut desc: String = sources
                    .iter()
                    .map(|s| format!("{}: {}\n", s.source, s.inputs.join(", ")))
                    .collect();
                if sources.is_empty() {
                    desc.push_str("割り当てがありません。\n");
                }
                desc.push_str(
                    "決定してからパッドを叩くかキーを押すと足します。このレーンに割り当て済みなら外します。",
                );
                lanes.push(row(
                    ItemId::Lane(lane),
                    lane_label(lane),
                    lane_value(sources),
                    desc,
                    Ctl::Lane(lane),
                ));
            }
            let changed = c.has_custom_assignments();
            s.at(" ").push(action(
                ItemId::ResetPads,
                "割り当てをすべて戻す",
                String::new(),
                format!(
                    "ここで変えた割り当てを全部消して、機器ごとのプリセットとキーボードの既定に戻します。{}",
                    if changed { "" } else { "\n（今は変えていません）" }
                ),
                Some(
                    "パッドとキーの割り当てを、すべての機器で既定に戻しますか？\nここで変えた割り当ては消えます。"
                        .to_owned(),
                ),
            ));
        }
        Page::Device(i) => {
            let Some(d) = info.devices.get(i) else {
                return Vec::new();
            };
            let r = s.at("");
            r.push(row(
                ItemId::DeviceState(i),
                "状態",
                if d.disabled {
                    "使っていません".to_owned()
                } else if d.open {
                    "つながっています".to_owned()
                } else {
                    "開けません".to_owned()
                },
                format!("{}\n{}", d.name, device_summary(d)),
                Ctl::Info,
            ));
            r.push(toggle(
                ItemId::DeviceUse(i),
                "使う",
                !d.disabled,
                true,
                "OFF にすると、このアプリはこの機器を開きません（ほかのアプリで使うときなど）。",
            ));
            r.push(choice(
                ItemId::DevicePreset(i),
                "プリセット",
                &Preset::ALL.map(Preset::label),
                Preset::ALL.iter().position(|&p| p == d.preset).unwrap_or(0),
                Preset::ALL.iter().position(|&p| p == d.default_preset),
                "割り当ての出発点です。\nRoland: V-Drums の既定（リムやエッジも）。\nGM: General MIDI のドラムの番号。\nなし: 何も割り当てない。",
            ));
            r.push(number(
                ItemId::DeviceChannel(i),
                "チャンネル",
                i64::from(d.channel),
                (0, 16, 1),
                Some(0),
                Fmt::Channel,
                "受ける MIDI のチャンネルです。「すべて」ならどのチャンネルでも受けます。".to_owned(),
            ));
            r.push(number(
                ItemId::DeviceMinVelocity(i),
                "最小ベロシティ",
                i64::from(d.min_velocity),
                (0, 127, 1),
                Some(0),
                Fmt::OffZero,
                "これより弱い打撃（ベロシティ）を捨てます。隣のパッドにつられて鳴る弱い音を除きます。".to_owned(),
            ));
            r.push(action(
                ItemId::DeviceReset(i),
                "割り当てを戻す",
                if d.changes == 0 {
                    String::new()
                } else {
                    format!("{} 個", d.changes)
                },
                "この機器で変えた割り当てを消して、プリセットに戻します。".to_owned(),
                Some(format!(
                    "{} の割り当てをプリセットに戻しますか？\nこの機器で変えた割り当て（{} 個）は消えます。",
                    d.name, d.changes
                )),
            ));
        }
    }
    s.rows()
}

/// 一覧の題
fn page_title(page: Page, info: &ConfigInfo<'_>) -> String {
    match page {
        Page::Cat(c) => c.name().to_owned(),
        Page::Lanes => "パッドとキーの割り当て".to_owned(),
        Page::Device(i) => info
            .devices
            .get(i)
            .map_or_else(|| "MIDI 機器".to_owned(), |d| d.name.clone()),
    }
}

/// 操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigInput {
    Up,
    Down,
    Left,
    Right,
    Decide,
    Cancel,
}

/// 画面から App に頼むこと
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfigAction {
    /// 値を決める（ON / OFF は 1 と 0、選択肢は何番目か、数はそのまま）
    Set(ItemId, i64),
    /// 動かす（曲のフォルダを足す・外す、探し直す、割り当てを戻す）
    Run(ItemId),
    /// MIDI のポート（何番目か）のノートを、レーンに足す。もう割り当ててあれば外す
    TogglePad(DrumLane, usize, u8),
    /// キーボードのキーを、レーンに足す。もう割り当ててあれば外す
    ToggleKey(DrumLane, KeyCode),
    /// MIDI のポートのノートをレーンに割り当てる（順に叩いて割り当てるとき）
    AssignPad(DrumLane, usize, u8),
    /// キーをレーンに割り当てる（順に叩いて割り当てるとき）
    AssignKey(DrumLane, KeyCode),
    /// レーンの割り当てを、すべての機器とキーボードから外す（待っているあいだの Delete）
    ClearLane(DrumLane),
    /// 設定の画面を出る（App がシャッターを閉じてから移る）
    Leave,
}

/// 操作の結果（鳴らす音と、App に頼むこと）
#[derive(Debug, Default, PartialEq)]
pub struct Response {
    pub se: Option<Se>,
    pub action: Option<ConfigAction>,
}

impl Response {
    fn se(se: Se) -> Self {
        Self {
            se: Some(se),
            action: None,
        }
    }

    fn action(se: Se, action: ConfigAction) -> Self {
        Self {
            se: Some(se),
            action: Some(action),
        }
    }
}

/// 決定で開いた、選択肢か数を選ぶ窓
#[derive(Debug, Clone, Copy, PartialEq)]
struct Picker {
    id: ItemId,
    /// 開いたときの値（戻るなら戻す）
    original: i64,
}

/// 取り消せない操作の確かめ
#[derive(Debug, Clone, PartialEq)]
struct Confirm {
    id: ItemId,
    text: String,
    yes: bool,
}

/// 順に叩いて割り当てているところ
#[derive(Debug, Clone, PartialEq)]
struct Flow {
    /// 今のレーン（[`PADS`] の何番目か）
    at: usize,
    /// 今のレーンを待ち始めた時刻
    since: Instant,
    /// この時刻までの打撃は受けない
    guard: Instant,
    /// レーンごとに割り当てたもの
    got: Vec<Option<String>>,
}

/// 開いている一覧と、その中で選んでいる行
#[derive(Debug, Clone, Copy, PartialEq)]
struct Level {
    page: Page,
    cursor: usize,
}

pub struct ConfigView {
    /// 左の列の何番目か
    cat: usize,
    /// 真ん中の一覧を操作している（でなければ左の列）
    focus_list: bool,
    /// 開いている一覧（先頭は分類の一覧）
    stack: Vec<Level>,
    picker: Option<Picker>,
    confirm: Option<Confirm>,
    /// 決定してから叩くのを待っているレーンと、待ち始めた時刻
    wait: Option<(DrumLane, Instant)>,
    flow: Option<Flow>,
    toast: Option<(String, Instant)>,
    entered: Instant,
    /// 出ると決めた（あとは操作を受け付けない）
    leaving: bool,
}

impl Default for ConfigView {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigView {
    pub fn new() -> Self {
        Self {
            cat: 0,
            focus_list: false,
            stack: vec![Level {
                page: Page::Cat(CATS[0]),
                cursor: 0,
            }],
            picker: None,
            confirm: None,
            wait: None,
            flow: None,
            toast: None,
            entered: Instant::now(),
            leaving: false,
        }
    }

    /// 真ん中に出している一覧
    pub fn page(&self) -> Page {
        self.level().page
    }

    fn level(&self) -> Level {
        self.stack.last().copied().unwrap_or(Level {
            page: Page::Cat(CATS[self.cat]),
            cursor: 0,
        })
    }

    fn cursor_mut(&mut self) -> &mut usize {
        &mut self.stack.last_mut().expect("the category level").cursor
    }

    /// 叩いたノートやキーを、そのまま受け取りたい（割り当てを待っている）
    pub fn wants_raw(&self) -> bool {
        self.wait.is_some() || self.flow.is_some()
    }

    /// 画面の下に短い知らせを出す
    pub fn toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now()));
    }

    /// MIDI の機器がつながったり外れたりした（機器の番号が変わるので、機器の設定を開いていたら分類の一覧へ戻る）
    pub fn devices_changed(&mut self) {
        if self
            .stack
            .iter()
            .any(|l| matches!(l.page, Page::Device(_)))
        {
            self.stack.truncate(1);
            self.picker = None;
            self.confirm = None;
        }
    }

    /// 選んでいる行を、選べる行に合わせる（一覧が変わったとき、開いたばかりのとき）
    fn sync(&mut self, rows: &[Row]) {
        if rows.is_empty() {
            return;
        }
        let at = (*self.cursor_mut()).min(rows.len() - 1);
        let fixed = if rows[at].selectable() {
            at
        } else {
            next_selectable(rows, at, 1)
                .or_else(|| next_selectable(rows, at, -1))
                .unwrap_or(at)
        };
        *self.cursor_mut() = fixed;
    }

    /// 出る（v2 と同じく、白へは消さずに App がシャッターを閉じる）
    fn leave(&mut self, se: Se) -> Response {
        self.leaving = true;
        Response::action(se, ConfigAction::Leave)
    }

    pub fn input(&mut self, input: ConfigInput, rows: &[Row]) -> Response {
        if self.leaving {
            return Response::default();
        }
        if self.flow.is_some() {
            return match input {
                // キーボードの Esc でやめ、Enter か Space で飛ばす
                ConfigInput::Cancel => {
                    self.end_flow();
                    Response::se(Se::Cancel)
                }
                ConfigInput::Decide => {
                    self.advance_flow(None);
                    Response::se(Se::Move)
                }
                _ => Response::default(),
            };
        }
        if self.wait.is_some() {
            // 叩くのを待っているあいだは、キーボードの Esc で取りやめるだけ
            if input == ConfigInput::Cancel {
                self.wait = None;
                return Response::se(Se::Cancel);
            }
            return Response::default();
        }
        if self.confirm.is_some() {
            return self.confirm_input(input);
        }
        if self.picker.is_some() {
            return self.picker_input(input, rows);
        }
        if !self.focus_list {
            return self.rail_input(input);
        }
        self.sync(rows);
        self.list_input(input, rows)
    }

    fn rail_input(&mut self, input: ConfigInput) -> Response {
        match input {
            ConfigInput::Up | ConfigInput::Down => {
                let to = if input == ConfigInput::Up {
                    self.cat.checked_sub(1)
                } else {
                    Some(self.cat + 1).filter(|&c| c < CATS.len())
                };
                let Some(to) = to else {
                    return Response::default();
                };
                self.cat = to;
                self.stack = vec![Level {
                    page: Page::Cat(CATS[to]),
                    cursor: 0,
                }];
                Response::se(Se::Move)
            }
            ConfigInput::Decide | ConfigInput::Right => {
                self.focus_list = true;
                Response::se(Se::Decide)
            }
            ConfigInput::Left => Response::default(),
            ConfigInput::Cancel => self.leave(Se::Cancel),
        }
    }

    fn list_input(&mut self, input: ConfigInput, rows: &[Row]) -> Response {
        let at = self.level().cursor;
        let Some(r) = rows.get(at) else {
            // 何も無い一覧（機器が外れたなど）は戻るだけ
            return match input {
                ConfigInput::Cancel => self.back(),
                _ => Response::default(),
            };
        };
        let Some(id) = r.id else {
            return Response::default();
        };
        match input {
            ConfigInput::Up | ConfigInput::Down => {
                let d = if input == ConfigInput::Up { -1 } else { 1 };
                match next_selectable(rows, at, d) {
                    Some(to) => {
                        *self.cursor_mut() = to;
                        Response::se(Se::Move)
                    }
                    None => Response::default(),
                }
            }
            ConfigInput::Left | ConfigInput::Right => {
                let d = if input == ConfigInput::Left { -1 } else { 1 };
                if let Ctl::Page(p) = r.ctl {
                    return if d > 0 {
                        self.open(p)
                    } else {
                        Response::default()
                    };
                }
                match stepped(r, d) {
                    Some(v) => Response::action(Se::Move, ConfigAction::Set(id, v)),
                    None => Response::default(),
                }
            }
            ConfigInput::Decide => match &r.ctl {
                Ctl::Heading | Ctl::Info => Response::default(),
                Ctl::Toggle { on, .. } => {
                    // 混ざっているときは全部 ON に
                    let v = i64::from(*on != Some(true));
                    Response::action(Se::Decide, ConfigAction::Set(id, v))
                }
                Ctl::Choice { at, .. } => {
                    self.picker = Some(Picker {
                        id,
                        original: *at as i64,
                    });
                    Response::se(Se::Decide)
                }
                Ctl::Number { at, .. } => {
                    self.picker = Some(Picker { id, original: *at });
                    Response::se(Se::Decide)
                }
                Ctl::Page(p) => self.open(*p),
                Ctl::Action { confirm: Some(text) } => {
                    self.confirm = Some(Confirm {
                        id,
                        text: text.clone(),
                        yes: false,
                    });
                    Response::se(Se::Decide)
                }
                Ctl::Action { confirm: None } => {
                    if id == ItemId::AssignAll {
                        let now = Instant::now();
                        self.flow = Some(Flow {
                            at: 0,
                            since: now,
                            guard: now + FLOW_GUARD,
                            got: vec![None; PADS.len()],
                        });
                        return Response::se(Se::Decide);
                    }
                    Response::action(Se::Decide, ConfigAction::Run(id))
                }
                Ctl::Lane(lane) => {
                    self.wait = Some((*lane, Instant::now()));
                    Response::se(Se::Decide)
                }
            },
            ConfigInput::Cancel => self.back(),
        }
    }

    fn open(&mut self, page: Page) -> Response {
        self.stack.push(Level { page, cursor: 0 });
        Response::se(Se::Decide)
    }

    /// 一つ上の一覧へ（分類の一覧なら左の列へ）
    fn back(&mut self) -> Response {
        if self.stack.len() > 1 {
            self.stack.pop();
        } else {
            self.focus_list = false;
        }
        Response::se(Se::Cancel)
    }

    fn picker_input(&mut self, input: ConfigInput, rows: &[Row]) -> Response {
        let Some(p) = self.picker else {
            return Response::default();
        };
        let Some(r) = rows.iter().find(|r| r.id == Some(p.id)) else {
            self.picker = None;
            return Response::default();
        };
        let number = matches!(r.ctl, Ctl::Number { .. });
        let d = match input {
            ConfigInput::Decide => {
                self.picker = None;
                return Response::se(Se::Decide);
            }
            ConfigInput::Cancel => {
                self.picker = None;
                return Response::action(Se::Cancel, ConfigAction::Set(p.id, p.original));
            }
            // 数は上で増やし、選択肢は上で前の選択肢へ（一覧の並びのとおり）
            ConfigInput::Up => {
                if number {
                    1
                } else {
                    -1
                }
            }
            ConfigInput::Down => {
                if number {
                    -1
                } else {
                    1
                }
            }
            ConfigInput::Left => -1,
            ConfigInput::Right => 1,
        };
        match stepped(r, d) {
            Some(v) => Response::action(Se::Move, ConfigAction::Set(p.id, v)),
            None => Response::default(),
        }
    }

    fn confirm_input(&mut self, input: ConfigInput) -> Response {
        let Some(c) = &mut self.confirm else {
            return Response::default();
        };
        match input {
            ConfigInput::Up | ConfigInput::Down | ConfigInput::Left | ConfigInput::Right => {
                c.yes = !c.yes;
                Response::se(Se::Move)
            }
            ConfigInput::Decide => {
                let c = self.confirm.take().expect("confirm");
                if c.yes {
                    Response::action(Se::Decide, ConfigAction::Run(c.id))
                } else {
                    Response::se(Se::Cancel)
                }
            }
            ConfigInput::Cancel => {
                self.confirm = None;
                Response::se(Se::Cancel)
            }
        }
    }

    /// 割り当てを待っているあいだに、MIDI のポート（何番目か）のノートが届いた
    pub fn pad_hit(&mut self, port: usize, note: u8) -> Response {
        if let Some((lane, _)) = self.wait.take() {
            return Response::action(Se::Decide, ConfigAction::TogglePad(lane, port, note));
        }
        if let Some(f) = &self.flow {
            if Instant::now() < f.guard {
                return Response::default();
            }
            let lane = PADS[f.at];
            self.advance_flow(Some(format!("ノート {note}")));
            return Response::action(Se::Decide, ConfigAction::AssignPad(lane, port, note));
        }
        Response::default()
    }

    /// 割り当てを待っているあいだに押したキー。割り当てられるキーなら割り当てる。
    /// 待っていなければ、またはほかのキー（Esc など）なら `None`（ふつうの操作として扱う）
    pub fn key_hit(&mut self, code: KeyCode) -> Option<Response> {
        if code == KeyCode::Delete
            && let Some((lane, _)) = self.wait.take()
        {
            return Some(Response::action(Se::Decide, ConfigAction::ClearLane(lane)));
        }
        if !crate::inputs::assignable(code) {
            return None;
        }
        if let Some((lane, _)) = self.wait.take() {
            return Some(Response::action(
                Se::Decide,
                ConfigAction::ToggleKey(lane, code),
            ));
        }
        let lane = PADS[self.flow.as_ref()?.at];
        self.advance_flow(Some(format!(
            "キー {}",
            crate::inputs::key_label(code)
        )));
        Some(Response::action(
            Se::Decide,
            ConfigAction::AssignKey(lane, code),
        ))
    }

    /// 待つ時間が過ぎたかを見る（毎回の処理で呼ぶ）
    pub fn tick(&mut self) -> Option<Response> {
        let now = Instant::now();
        if let Some((_, since)) = self.wait
            && now.duration_since(since) >= WAIT_FOR
        {
            self.wait = None;
            self.toast("5 秒叩かなかったので取りやめました".to_owned());
            return Some(Response::se(Se::Cancel));
        }
        if let Some(f) = &self.flow
            && now.duration_since(f.since) >= FLOW_STEP
        {
            self.advance_flow(None);
            return Some(Response::se(Se::Move));
        }
        None
    }

    /// 順に割り当てるのを次のレーンへ（`got` は今のレーンに割り当てたもの。飛ばしたら `None`）
    fn advance_flow(&mut self, got: Option<String>) {
        let Some(f) = &mut self.flow else {
            return;
        };
        f.got[f.at] = got;
        f.at += 1;
        let now = Instant::now();
        f.since = now;
        f.guard = now + FLOW_GUARD;
        if f.at >= PADS.len() {
            self.end_flow();
        }
    }

    fn end_flow(&mut self) {
        if let Some(f) = self.flow.take() {
            let n = f.got.iter().flatten().count();
            self.toast(format!("順に割り当てるのを終えました（{n} レーン）"));
        }
    }
}

/// `at` から `d`（-1 か 1）の向きにある、次の選べる行
fn next_selectable(rows: &[Row], at: usize, d: i64) -> Option<usize> {
    let mut i = at as i64 + d;
    while (0..rows.len() as i64).contains(&i) {
        if rows[i as usize].selectable() {
            return Some(i as usize);
        }
        i += d;
    }
    None
}

/// 行の値を `d`（-1 か 1）だけ動かした値（端で止まる。動かなければ `None`）。ON / OFF は左が ON
fn stepped(r: &Row, d: i64) -> Option<i64> {
    match &r.ctl {
        Ctl::Toggle { on, .. } => {
            let to = d < 0;
            (*on != Some(to)).then_some(i64::from(to))
        }
        Ctl::Choice { labels, at, .. } => {
            let to = (*at as i64 + d).clamp(0, labels.len() as i64 - 1);
            (to != *at as i64).then_some(to)
        }
        Ctl::Number {
            at, min, max, step, ..
        } => {
            let to = (at + d * step).clamp(*min, *max);
            (to != *at).then_some(to)
        }
        _ => None,
    }
}

// ここから描く

/// 一覧の板
const LIST_X: f32 = 284.0;
const LIST_Y: f32 = 104.0;
const LIST_W: f32 = 572.0;
const LIST_H: f32 = 576.0;
/// 説明の板
const DETAIL_X: f32 = 872.0;
const DETAIL_W: f32 = 376.0;
/// 行の高さ、小見出しの高さ、名前の無い小見出し（隙間）の高さ、題の高さ
const ROW_H: f32 = 42.0;
const HEADING_H: f32 = 30.0;
const GAP_H: f32 = 12.0;
const HEAD_H: f32 = 46.0;

fn row_height(r: &Row) -> f32 {
    match r.ctl {
        Ctl::Heading if r.name.trim().is_empty() => GAP_H,
        Ctl::Heading => HEADING_H,
        _ => ROW_H,
    }
}

/// 一覧の一番上に出す行（選んでいる行の下に 2 行半の余りが残るように）
fn first_visible(rows: &[Row], cursor: Option<usize>, avail: f32) -> usize {
    let Some(cursor) = cursor else {
        return 0;
    };
    let hs: Vec<f32> = rows.iter().map(row_height).collect();
    let bottom: f32 = hs.iter().take(cursor + 1).sum();
    let margin = ROW_H * 2.5;
    let mut top = 0;
    let mut cut = 0.0;
    while bottom - cut > avail - margin && top < cursor {
        cut += hs[top];
        top += 1;
    }
    top
}

/// 画面の下の案内
fn hints_for(view: &ConfigView, row: Option<&Row>) -> Vec<ui::Hint<'static>> {
    const ALL: &[Pad] = &[
        Pad::LC,
        Pad::HH,
        Pad::SD,
        Pad::HT,
        Pad::LT,
        Pad::FT,
        Pad::CY,
        Pad::RD,
        Pad::BD,
    ];
    if view.flow.is_some() {
        return vec![(
            ALL,
            "叩くと割り当て（4 秒で飛ばす・キーボードは Space で飛ばす / Esc でやめる）",
        )];
    }
    if view.wait.is_some() {
        return vec![(
            ALL,
            "足すパッドを叩く（割り当て済みなら外す・5 秒で取りやめ・キーボードは Esc、Delete で全部外す）",
        )];
    }
    if view.confirm.is_some() {
        return vec![
            (&[Pad::HT, Pad::LT], "選ぶ"),
            (&[Pad::CY, Pad::RD], "決める（Enter）"),
            (&[Pad::LC], "やめる（Esc）"),
        ];
    }
    if view.picker.is_some() {
        return if row.is_some_and(|r| matches!(r.ctl, Ctl::Number { .. })) {
            vec![
                (&[Pad::HT], "増やす"),
                (&[Pad::LT], "減らす"),
                (&[Pad::CY, Pad::RD], "決める（Enter）"),
                (&[Pad::LC], "やめて元に戻す（Esc）"),
            ]
        } else {
            vec![
                (&[Pad::HT], "上"),
                (&[Pad::LT], "下"),
                (&[Pad::CY, Pad::RD], "決める（Enter）"),
                (&[Pad::LC], "やめて元に戻す（Esc）"),
            ]
        };
    }
    if !view.focus_list {
        return vec![
            (&[Pad::HT], "上"),
            (&[Pad::LT], "下"),
            (&[Pad::CY, Pad::RD], "開く（Enter）"),
            (&[Pad::LC], "設定を出る（Esc）"),
        ];
    }
    let mut v: Vec<ui::Hint<'static>> = vec![(&[Pad::HT], "上"), (&[Pad::LT], "下")];
    let decide = match row.map(|r| &r.ctl) {
        Some(Ctl::Toggle { .. }) => "切り替え（Enter）",
        Some(Ctl::Choice { .. } | Ctl::Number { .. }) => "選ぶ（Enter）",
        Some(Ctl::Page(_)) => "開く（Enter）",
        Some(Ctl::Lane(_)) => "叩いて足す・外す（Enter）",
        Some(Ctl::Action { .. }) => "実行（Enter）",
        _ => "決定（Enter）",
    };
    if row.is_some_and(Row::editable) {
        v.push((&[Pad::SD, Pad::FT], "←→ 値を変える"));
    } else if row.is_some_and(|r| matches!(r.ctl, Ctl::Page(_))) {
        v.push((&[Pad::FT], "→ 開く"));
    }
    v.push((&[Pad::CY, Pad::RD], decide));
    v.push((&[Pad::LC], "戻る（Esc）"));
    v
}

impl ConfigView {
    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, rows: &[Row], info: &ConfigInfo<'_>) {
        if self.focus_list {
            self.sync(rows);
        }
        match g.skin.tex("Graphics/4_background.png") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(20, 24, 40)),
        }
        // 背景の絵を沈めて、文字を読みやすく
        c.cover(Color32::from_rgba_premultiplied(5, 3, 12, 148));
        if let Some(t) = g.skin.tex("Graphics/4_header panel.png") {
            c.draw(&t, 0.0, 0.0);
        }
        self.draw_title(c, g, info);
        self.draw_rail(c, g);
        let cursor = self.focus_list.then(|| self.level().cursor);
        self.draw_list(c, g, rows, cursor, info);
        let row = cursor.and_then(|i| rows.get(i));
        self.draw_detail(c, g, row, info);
        if let Some(t) = g.skin.tex("Graphics/4_footer panel.png") {
            c.draw(&t, 0.0, 720.0 - t.h);
        }
        let picked = self
            .picker
            .and_then(|p| rows.iter().find(|r| r.id == Some(p.id)));
        if let Some(r) = picked {
            draw_picker(c, g, r);
        }
        if let Some(cf) = &self.confirm {
            draw_confirm(c, g, cf);
        }
        if let Some((lane, since)) = self.wait {
            draw_wait(c, g, lane, since, info);
        }
        if let Some(f) = &self.flow {
            draw_flow(c, g, f, info);
        }
        ui::legend(c, g, &hints_for(self, picked.or(row)));
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= TOAST_FOR)
        {
            self.toast = None;
        }
        if let Some((t, _)) = &self.toast {
            draw_toast(c, g, t);
        }
        if info.scanning.is_some() {
            draw_scanning(c, g, self.entered);
        }
    }

    /// 上の帯の題と、今どこか
    fn draw_title(&self, c: &Canvas, g: &mut Gfx, info: &ConfigInfo<'_>) {
        let w = ui::text(c, g, "設定", 40.0, 22.0, &ui::edged(34.0), 200.0);
        let mut x = 40.0 + w + 14.0;
        let crumb = ui::edged(20.0);
        let mut parts = vec![CATS[self.cat].name().to_owned()];
        if self.focus_list {
            parts.extend(
                self.stack
                    .iter()
                    .skip(1)
                    .map(|l| page_title(l.page, info)),
            );
        }
        for p in parts {
            ui::chevron(c, x + 4.0, 47.0, 14.0, 2.5, ui::DIM);
            x += 16.0;
            x += ui::text(c, g, &p, x, 35.0, &crumb, 400.0) + 6.0;
        }
    }

    /// 左の列（分類）
    fn draw_rail(&self, c: &Canvas, g: &mut Gfx) {
        for (i, cat) in CATS.iter().enumerate() {
            let y = 112.0 + i as f32 * 54.0;
            let on = i == self.cat;
            if on {
                ui::selection(c, 32.0, y, 232.0, 46.0, if self.focus_list { 0.45 } else { 1.0 });
            }
            let color = if on {
                ui::YELLOW
            } else {
                Color32::from_rgb(216, 212, 230)
            };
            ui::icon(c, cat.icon(), 46.0, y + 10.0, 26.0, color);
            let style = if on {
                ui::glow(20.0)
            } else {
                Style {
                    edge: Some((Color32::BLACK, 2.0)),
                    ..ui::bold(20.0, Color32::from_rgb(233, 229, 243))
                }
            };
            ui::text(c, g, cat.name(), 86.0, y + 11.0, &style, 170.0);
        }
    }

    /// 真ん中の一覧
    fn draw_list(
        &self,
        c: &Canvas,
        g: &mut Gfx,
        rows: &[Row],
        cursor: Option<usize>,
        info: &ConfigInfo<'_>,
    ) {
        let (x, y, w, h) = (LIST_X, LIST_Y, LIST_W, LIST_H);
        ui::panel(c, x, y, w, h);
        ui::text(
            c,
            g,
            &page_title(self.page(), info),
            x + 22.0,
            y + 12.0,
            &ui::bold(22.0, Color32::WHITE),
            w - 160.0,
        );
        let selectable: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].selectable()).collect();
        let count = match cursor.and_then(|at| selectable.iter().position(|&i| i == at)) {
            Some(n) => format!("{} / {}", n + 1, selectable.len()),
            None => format!("{} 項目", selectable.len()),
        };
        ui::text_right(
            c,
            g,
            &count,
            x + w - 22.0,
            y + 16.0,
            &ui::bold(15.0, ui::DIM),
            120.0,
        );
        let avail = h - HEAD_H - 12.0;
        let top = first_visible(rows, cursor, avail);
        let clip = c.with_clip(c.rect(x, y + HEAD_H - 2.0, w, avail + 4.0));
        let mut ry = y + HEAD_H;
        for (i, r) in rows.iter().enumerate().skip(top) {
            if ry >= y + HEAD_H + avail {
                break;
            }
            draw_row(&clip, g, r, x, ry, w, cursor == Some(i));
            ry += row_height(r);
        }
        // スクロールの位置
        let total: f32 = rows.iter().map(row_height).sum();
        if total > avail {
            let cut: f32 = rows.iter().take(top).map(row_height).sum();
            let th = (avail * avail / total).max(30.0);
            let ty = cut / total * avail;
            c.fill(
                x + w - 9.0,
                y + HEAD_H,
                3.0,
                avail,
                Color32::from_white_alpha(26),
            );
            c.fill(x + w - 9.0, y + HEAD_H + ty, 3.0, th, ui::ORANGE);
        }
    }

    /// 右の列（説明と見本）
    fn draw_detail(&self, c: &Canvas, g: &mut Gfx, row: Option<&Row>, info: &ConfigInfo<'_>) {
        let (x, y, w, h) = (DETAIL_X, LIST_Y, DETAIL_W, LIST_H);
        ui::panel(c, x, y, w, h);
        let pv = (x + 16.0, y + h - 236.0, w - 32.0, 220.0);
        let desc_style = ui::plain(16.0, Color32::from_rgb(226, 221, 239));
        let Some(r) = row else {
            let cat = CATS[self.cat];
            ui::text(c, g, cat.name(), x + 22.0, y + 16.0, &ui::bold(24.0, Color32::WHITE), w - 44.0);
            let text = format!("{}\n\n決定か → で中へ。", cat.desc());
            ui::paragraph(c, g, &text, x + 22.0, y + 62.0, w - 44.0, 25.0, pv.1 - 8.0, &desc_style);
            preview_box(c, pv);
            ui::pad_roles(c, g, pv.0, pv.1, pv.2, pv.3);
            return;
        };
        let mut ty = ui::paragraph(
            c,
            g,
            r.name.trim_start(),
            x + 22.0,
            y + 16.0,
            w - 44.0,
            30.0,
            y + 80.0,
            &ui::bold(24.0, Color32::WHITE),
        );
        let show_value = !r.value.is_empty() && !matches!(r.ctl, Ctl::Action { .. });
        if show_value {
            ui::text(c, g, &r.value, x + 22.0, ty + 6.0, &ui::glow(30.0), w - 44.0);
            ty += 48.0;
        }
        if let Some(d) = r.default_label() {
            let dw = ui::text(
                c,
                g,
                &format!("既定: {d}"),
                x + 22.0,
                ty + 4.0,
                &ui::plain(14.0, ui::DIM),
                w - 160.0,
            );
            if r.changed() {
                let chip = ui::bold(12.0, Color32::from_rgb(26, 10, 0));
                let cw = ui::text_width(g, "変えています", &chip) + 12.0;
                ui::round_fill(c, x + 22.0 + dw + 12.0, ty + 4.0, cw, 19.0, 3.0, ui::ORANGE);
                ui::text(c, g, "変えています", x + 28.0 + dw + 12.0, ty + 6.0, &chip, cw);
            }
            ty += 30.0;
        }
        ui::paragraph(c, g, &r.desc, x + 22.0, ty + 6.0, w - 44.0, 25.0, pv.1 - 8.0, &desc_style);
        preview_box(c, pv);
        draw_preview(c, g, r, info, pv, self.entered);
    }
}

/// 一覧の 1 行
fn draw_row(c: &Canvas, g: &mut Gfx, r: &Row, x: f32, y: f32, w: f32, sel: bool) {
    if let Ctl::Heading = r.ctl {
        let name = r.name.trim();
        if !name.is_empty() {
            let tw = ui::text(c, g, name, x + 22.0, y + 9.0, &ui::bold(13.0, ui::MUTED), w);
            c.fill(
                x + 22.0 + tw + 10.0,
                y + 17.0,
                (w - 60.0 - tw).max(0.0),
                1.0,
                Color32::from_white_alpha(36),
            );
        }
        return;
    }
    if sel {
        ui::selection(c, x + 8.0, y + 1.0, w - 22.0, ROW_H - 2.0, 1.0);
    }
    if r.changed() {
        ui::dot(c, x + 17.0, y + ROW_H / 2.0, 3.0, ui::ORANGE);
    }
    let color = match (r.tone, sel) {
        (_, true) => ui::YELLOW,
        (Tone::Strong, _) => ui::STRONG,
        (Tone::Danger, _) => ui::DANGER,
        _ => ui::TEXT,
    };
    let name_style = if sel || r.tone == Tone::Strong {
        ui::bold(18.0, color)
    } else {
        ui::plain(18.0, color)
    };
    ui::text(c, g, &r.name, x + 28.0, y + 10.0, &name_style, w * 0.5);
    let right = x + w - 30.0;
    match &r.ctl {
        Ctl::Toggle { .. } | Ctl::Choice { .. } | Ctl::Number { .. } => {
            let style = if sel {
                ui::bold(18.0, ui::YELLOW)
            } else {
                ui::plain(18.0, Color32::WHITE)
            };
            let tri = if sel {
                ui::ORANGE
            } else {
                Color32::from_white_alpha(70)
            };
            let options = r.options();
            let dy = if options.as_ref().is_some_and(|(o, _)| o.len() > 1) {
                -4.0
            } else {
                0.0
            };
            let vw = ui::text_width(g, &r.value, &style).min(170.0);
            ui::triangle(c, right - 4.0, y + ROW_H / 2.0 + dy, 9.0, 1, tri);
            ui::text_right(c, g, &r.value, right - 16.0, y + 10.0 + dy, &style, 170.0);
            ui::triangle(c, right - 28.0 - vw, y + ROW_H / 2.0 + dy, 9.0, -1, tri);
            if let Some((o, at)) = options
                && o.len() > 1
            {
                let n = o.len();
                for i in 0..n {
                    let color = if Some(i) == at {
                        ui::ORANGE
                    } else if sel {
                        Color32::from_white_alpha(115)
                    } else {
                        Color32::from_white_alpha(46)
                    };
                    let dx = right - 16.0 - (n - i) as f32 * 11.0 + 3.0;
                    ui::dot(c, dx + 3.0, y + ROW_H - 8.0, 3.0, color);
                }
            }
        }
        Ctl::Page(_) => {
            ui::text_right(
                c,
                g,
                &r.value,
                right - 16.0,
                y + 12.0,
                &ui::plain(15.0, ui::DIM),
                190.0,
            );
            ui::chevron(c, right - 2.0, y + ROW_H / 2.0, 12.0, 2.0, ui::DIM);
        }
        Ctl::Lane(_) => {
            ui::text_right(
                c,
                g,
                &r.value,
                right,
                y + 11.0,
                &ui::plain(16.0, if sel { ui::YELLOW } else { Color32::WHITE }),
                240.0,
            );
        }
        _ => {
            ui::text_right(
                c,
                g,
                &r.value,
                right,
                y + 12.0,
                &ui::plain(15.0, ui::DIM),
                250.0,
            );
        }
    }
}

fn preview_box(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32)) {
    c.fill(x, y, w, h, Color32::from_rgba_premultiplied(0, 0, 0, 110));
    c.frame(x, y, w, h, Color32::from_white_alpha(26));
}

/// 説明の列の下の見本
fn draw_preview(
    c: &Canvas,
    g: &mut Gfx,
    r: &Row,
    info: &ConfigInfo<'_>,
    (x, y, w, h): (f32, f32, f32, f32),
    since: Instant,
) {
    let cfg = info.config;
    let cap = ui::plain(13.0, ui::MUTED);
    match r.id {
        Some(ItemId::Value(Setting::ScrollSpeed)) => {
            preview_scroll(c, g, cfg.scroll_px_per_ms, (x, y, w, h), since);
        }
        Some(ItemId::Value(Setting::ShowLagTime | Setting::ShowLagTimeColor)) => {
            preview_lag(c, g, cfg, (x, y, w, h));
        }
        Some(ItemId::Value(s @ (Setting::InputAdjust | Setting::VisualAdjust))) => {
            preview_offset(c, g, get(s, cfg), s == Setting::InputAdjust, (x, y, w, h));
        }
        Some(ItemId::Value(Setting::MasterVolume)) => {
            let v = cfg.master_volume() as f32 / 100.0;
            let (bx, by, bw) = (x + 24.0, y + 90.0, w - 48.0);
            ui::round_fill(c, bx, by, bw, 18.0, 9.0, Color32::from_white_alpha(30));
            if v > 0.0 {
                ui::round_fill(c, bx, by, bw * v, 18.0, 9.0, ui::ORANGE);
            }
            ui::text_center(
                c,
                g,
                &format!("{} %", cfg.master_volume()),
                x + w / 2.0,
                y + 44.0,
                &ui::bold(24.0, Color32::WHITE),
                w,
            );
            ui::text(c, g, "全体の音量", x + 12.0, y + h - 22.0, &cap, w - 24.0);
        }
        Some(
            ItemId::Device(i)
            | ItemId::DeviceState(i)
            | ItemId::DeviceUse(i)
            | ItemId::DevicePreset(i)
            | ItemId::DeviceChannel(i)
            | ItemId::DeviceMinVelocity(i)
            | ItemId::DeviceReset(i),
        ) => {
            let d = info.devices.get(i);
            ui::text(c, g, "最後に届いた打撃", x + 14.0, y + 14.0, &cap, w - 28.0);
            let last = d
                .and_then(|d| d.last.clone())
                .unwrap_or_else(|| "（まだ届いていません。叩くとここに出ます）".to_owned());
            ui::paragraph(
                c,
                g,
                &last,
                x + 14.0,
                y + 40.0,
                w - 28.0,
                28.0,
                y + h - 30.0,
                &ui::bold(20.0, Color32::WHITE),
            );
            if let Some(d) = d {
                ui::text(c, g, &d.name, x + 12.0, y + h - 22.0, &cap, w - 24.0);
            }
        }
        Some(ItemId::Lanes | ItemId::AssignAll | ItemId::ResetPads) => {
            preview_kit(c, g, info, None, (x, y, w, h));
        }
        Some(ItemId::Lane(lane)) => preview_kit(c, g, info, Some(lane), (x, y, w, h)),
        Some(ItemId::Table(key)) if key.starts_with("auto_") => {
            preview_auto(c, g, cfg, (x, y, w, h));
        }
        _ => ui::pad_roles(c, g, x, y, w, h),
    }
}

/// 譜面の速さの見本（7 本のレーンに音符を流す）
fn preview_scroll(
    c: &Canvas,
    g: &mut Gfx,
    speed: f32,
    (x, y, w, h): (f32, f32, f32, f32),
    since: Instant,
) {
    let clip = c.with_clip(c.rect(x, y, w, h));
    let lanes = 7;
    let lw = (w - 40.0) / lanes as f32;
    let judge = y + h - 34.0;
    // 本当の速さの 3 割（見本の窓は演奏の画面より短いので）
    let px_per_ms = speed * 0.3;
    let now = since.elapsed().as_secs_f32() * 1000.0;
    let beat = 250.0;
    for i in 0..lanes {
        let lx = x + 20.0 + i as f32 * lw;
        clip.fill(lx, y + 8.0, lw - 2.0, h - 16.0, Color32::from_white_alpha(10));
        let color = [
            Pad::LC, Pad::HH, Pad::SD, Pad::BD, Pad::HT, Pad::FT, Pad::CY,
        ][i]
            .color();
        // レーンごとにずらした拍に音符を置く
        let phase = ((i * 37) % 8) as f32 * beat / 2.0;
        let first = ((now - phase) / (beat * 2.0)).floor() as i64;
        for k in first..first + 12 {
            let t = phase + k as f32 * beat * 2.0;
            let ny = judge - (t - now) * px_per_ms;
            if ny < y || ny > judge + 6.0 {
                continue;
            }
            clip.fill(lx + 2.0, ny - 4.0, lw - 6.0, 7.0, color);
        }
    }
    clip.fill(x + 20.0, judge, w - 40.0, 3.0, ui::YELLOW);
    ui::text(
        c,
        g,
        &format!("速さ {speed:.2}"),
        x + 10.0,
        y + h - 22.0,
        &ui::plain(13.0, ui::MUTED),
        w - 20.0,
    );
}

/// ずれの表示の見本（判定の文字と、ずれの数字）
fn preview_lag(c: &Canvas, g: &mut Gfx, cfg: &Config, (x, y, w, h): (f32, f32, f32, f32)) {
    let blue = Color32::from_rgb(74, 168, 255);
    let red = Color32::from_rgb(255, 74, 74);
    let (early, late) = if cfg.show_lag_time_color == 1 {
        (red, blue)
    } else {
        (blue, red)
    };
    let samples = [
        ("PERFECT", -3, Color32::from_rgb(255, 232, 106)),
        ("GREAT", 18, Color32::from_rgb(125, 255, 156)),
        ("GOOD", -41, Color32::from_rgb(122, 215, 255)),
    ];
    for (i, (judge, lag, color)) in samples.into_iter().enumerate() {
        let cx = x + w / 6.0 + i as f32 * w / 3.0;
        ui::text_center(c, g, judge, cx, y + 56.0, &ui::bold(19.0, color), w / 3.0 - 6.0);
        let shown = match cfg.show_lag_time {
            0 => false,
            2 => judge != "PERFECT",
            _ => true,
        };
        if shown {
            let style = ui::bold(22.0, if lag < 0 { early } else { late });
            ui::text_center(c, g, &format!("{lag:+}"), cx, y + 90.0, &style, w / 3.0);
        }
    }
    ui::text(
        c,
        g,
        "早い（-）と遅い（+）の色",
        x + 12.0,
        y + h - 22.0,
        &ui::plain(13.0, ui::MUTED),
        w - 24.0,
    );
}

/// 補正の見本（ちょうどの線と、補正でずらした線）
fn preview_offset(
    c: &Canvas,
    g: &mut Gfx,
    ms: i64,
    input: bool,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    let cx = x + w / 2.0;
    let scale = (w / 2.0 - 30.0) / OFFSET_RANGE as f32;
    let ay = y + 100.0;
    c.fill(x + 20.0, ay, w - 40.0, 2.0, Color32::from_white_alpha(60));
    c.fill(cx - 1.0, ay - 40.0, 2.0, 80.0, Color32::from_white_alpha(140));
    ui::text_center(c, g, "ちょうど", cx, ay - 62.0, &ui::plain(13.0, ui::DIM), 100.0);
    let mx = cx + ms as f32 * scale;
    c.fill(mx - 2.0, ay - 30.0, 4.0, 60.0, ui::ORANGE);
    let label = if input {
        match ms {
            0 => "補正なし".to_owned(),
            m if m > 0 => format!("叩いた時刻を {m} ms 早めて判定"),
            m => format!("叩いた時刻を {} ms 遅らせて判定", -m),
        }
    } else {
        match ms {
            0 => "補正なし".to_owned(),
            m if m > 0 => format!("譜面を {m} ms 遅らせて表示"),
            m => format!("譜面を {} ms 早めて表示", -m),
        }
    };
    ui::text_center(c, g, &label, cx, ay + 44.0, &ui::bold(16.0, Color32::WHITE), w - 24.0);
    ui::text(
        c,
        g,
        &format!("-{0:.0} 〜 +{0:.0} ms", OFFSET_RANGE),
        x + 12.0,
        y + h - 22.0,
        &ui::plain(13.0, ui::MUTED),
        w - 24.0,
    );
}

/// ドラムの配置図。`lit` のレーンを光らせ、割り当てを添える
fn preview_kit(
    c: &Canvas,
    g: &mut Gfx,
    info: &ConfigInfo<'_>,
    lit: Option<DrumLane>,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    for lane in PADS {
        let p = ui::lane_spot_in(lane, x + 30.0, y + 26.0, w - 60.0, h - 66.0);
        let sources = info.lanes.get(&lane).map_or(&[][..], Vec::as_slice);
        let notes = sources
            .iter()
            .filter(|s| s.source != KEYBOARD)
            .flat_map(|s| s.inputs.iter().cloned())
            .collect::<Vec<_>>()
            .join(",");
        ui::pad_circle(
            c,
            g,
            p.x,
            p.y,
            19.0,
            ui::lane_color(lane),
            short_lane(lane),
            &notes,
            lit == Some(lane),
        );
    }
    ui::text(
        c,
        g,
        "丸の下は割り当てたノート番号",
        x + 12.0,
        y + h - 22.0,
        &ui::plain(13.0, ui::MUTED),
        w - 24.0,
    );
}

/// 自動演奏の見本（自動で叩くレーンを光らせる）
fn preview_auto(c: &Canvas, g: &mut Gfx, cfg: &Config, (x, y, w, h): (f32, f32, f32, f32)) {
    let auto = cfg.settings.auto_lanes();
    for lane in PADS {
        let p = ui::lane_spot_in(lane, x + 30.0, y + 26.0, w - 60.0, h - 66.0);
        let on = auto.contains(&lane);
        ui::pad_circle(
            c,
            g,
            p.x,
            p.y,
            19.0,
            ui::lane_color(lane),
            short_lane(lane),
            if on { "自動" } else { "" },
            on,
        );
    }
    ui::text(
        c,
        g,
        "光っているレーンを自動で叩きます",
        x + 12.0,
        y + h - 22.0,
        &ui::plain(13.0, ui::MUTED),
        w - 24.0,
    );
}

/// 配置図に書くレーンの短い名前
fn short_lane(lane: DrumLane) -> &'static str {
    match lane {
        DrumLane::LC => "LC",
        DrumLane::HH => "HH",
        DrumLane::HHO => "HHO",
        DrumLane::SD => "SD",
        DrumLane::BD => "BD",
        DrumLane::HT => "HT",
        DrumLane::LT => "LT",
        DrumLane::FT => "FT",
        DrumLane::CY => "CY",
        DrumLane::RD => "RD",
        DrumLane::LP => "LP",
        DrumLane::LBD => "LBD",
    }
}

/// 選択肢か数を選ぶ窓
fn draw_picker(c: &Canvas, g: &mut Gfx, r: &Row) {
    const X: f32 = 520.0;
    const Y: f32 = 200.0;
    const W: f32 = 330.0;
    c.cover(Color32::from_black_alpha(90));
    let title = ui::bold(20.0, Color32::WHITE);
    let note = ui::plain(14.0, ui::DIM);
    if let Ctl::Number { fmt, default, .. } = &r.ctl {
        ui::pick_panel(c, X, Y, W, 210.0);
        ui::text(c, g, r.name.trim_start(), X + 20.0, Y + 14.0, &title, W - 40.0);
        ui::triangle(c, X + W / 2.0, Y + 66.0, 18.0, -2, ui::YELLOW);
        ui::text_center(c, g, &r.value, X + W / 2.0, Y + 86.0, &ui::glow(40.0), W - 20.0);
        ui::triangle(c, X + W / 2.0, Y + 150.0, 18.0, 2, ui::YELLOW);
        let text = match default {
            Some(d) => format!("既定 {}", fmt.show(*d)),
            None => String::new(),
        };
        ui::text(c, g, &text, X + 20.0, Y + 178.0, &note, W - 40.0);
        return;
    }
    let Some((labels, at)) = r.options() else {
        return;
    };
    let default = match &r.ctl {
        Ctl::Toggle { default, .. } => default.map(|d| if d { 0 } else { 1 }),
        Ctl::Choice { default, .. } => *default,
        _ => None,
    };
    let h = 64.0 + labels.len() as f32 * 38.0;
    ui::pick_panel(c, X, Y, W, h);
    ui::text(c, g, r.name.trim_start(), X + 20.0, Y + 14.0, &title, W - 40.0);
    for (i, label) in labels.iter().enumerate() {
        let ly = Y + 54.0 + i as f32 * 38.0;
        let sel = Some(i) == at;
        if sel {
            ui::selection(c, X + 10.0, ly, W - 20.0, 34.0, 1.0);
        }
        let style = if sel {
            ui::bold(18.0, ui::YELLOW)
        } else {
            ui::plain(18.0, ui::TEXT)
        };
        let lw = ui::text(c, g, label, X + 26.0, ly + 6.0, &style, W - 120.0);
        if Some(i) == default {
            ui::text(c, g, "（既定）", X + 30.0 + lw, ly + 9.0, &note, 80.0);
        }
    }
}

/// 取り消せない操作の確かめ
fn draw_confirm(c: &Canvas, g: &mut Gfx, cf: &Confirm) {
    c.cover(Color32::from_black_alpha(128));
    ui::pick_panel(c, 390.0, 250.0, 500.0, 190.0);
    ui::paragraph(
        c,
        g,
        &cf.text,
        416.0,
        274.0,
        448.0,
        26.0,
        366.0,
        &ui::plain(17.0, Color32::WHITE),
    );
    for (i, label) in ["はい", "いいえ"].into_iter().enumerate() {
        let bx = 440.0 + i as f32 * 210.0;
        let sel = (i == 0) == cf.yes;
        if sel {
            ui::selection(c, bx, 372.0, 180.0, 44.0, 1.0);
        } else {
            ui::round_frame(c, bx, 372.0, 180.0, 44.0, 3.0, 1.0, Color32::from_white_alpha(77));
        }
        let style = if sel {
            ui::bold(18.0, ui::YELLOW)
        } else {
            ui::plain(18.0, ui::TEXT)
        };
        ui::text_center(c, g, label, bx + 90.0, 383.0, &style, 170.0);
    }
}

/// 残りの時間の帯
fn countdown(c: &Canvas, x: f32, y: f32, w: f32, h: f32, left: f32) {
    ui::round_fill(c, x, y, w, h, h / 2.0, Color32::from_white_alpha(30));
    if left > 0.0 {
        ui::round_fill(c, x, y, w * left.min(1.0), h, h / 2.0, ui::ORANGE);
    }
}

/// レーンを決定してから叩くのを待つ窓
fn draw_wait(c: &Canvas, g: &mut Gfx, lane: DrumLane, since: Instant, info: &ConfigInfo<'_>) {
    c.cover(Color32::from_black_alpha(140));
    ui::pick_panel(c, 390.0, 230.0, 500.0, 230.0);
    ui::text_center(
        c,
        g,
        &format!("{}に足すパッドを叩いてください", lane_label(lane)),
        640.0,
        254.0,
        &ui::bold(22.0, Color32::WHITE),
        460.0,
    );
    let now = lane_value(info.lanes.get(&lane).map_or(&[][..], Vec::as_slice));
    ui::text_center(
        c,
        g,
        &format!("今: {now}"),
        640.0,
        296.0,
        &ui::plain(15.0, ui::DIM),
        460.0,
    );
    ui::text_center(
        c,
        g,
        "このレーンに割り当て済みのパッドやキーなら外します",
        640.0,
        322.0,
        &ui::plain(15.0, ui::DIM),
        460.0,
    );
    let left = 1.0 - since.elapsed().as_secs_f32() / WAIT_FOR.as_secs_f32();
    countdown(c, 450.0, 370.0, 380.0, 10.0, left);
    let secs = (left * WAIT_FOR.as_secs_f32()).ceil().max(0.0);
    ui::text_center(
        c,
        g,
        &format!("あと {secs:.0} 秒で取りやめ（キーボードは Esc）"),
        640.0,
        396.0,
        &ui::plain(14.0, ui::DIM),
        460.0,
    );
}

/// 順に叩いて割り当てる画面
fn draw_flow(c: &Canvas, g: &mut Gfx, f: &Flow, info: &ConfigInfo<'_>) {
    c.cover(Color32::from_rgba_premultiplied(4, 2, 11, 237));
    let lane = PADS[f.at.min(PADS.len() - 1)];
    ui::text_center(
        c,
        g,
        &format!("順に叩いて割り当てる　{} / {}", f.at + 1, PADS.len()),
        640.0,
        112.0,
        &ui::plain(16.0, ui::DIM),
        800.0,
    );
    ui::text_center(
        c,
        g,
        &format!("{}を叩いてください", lane_label(lane)),
        640.0,
        142.0,
        &ui::edged(40.0),
        1000.0,
    );
    for (i, l) in PADS.iter().enumerate() {
        let (px, py) = ui::lane_spot(*l);
        let sub = if i < f.at {
            f.got[i].clone().unwrap_or_else(|| "飛ばした".to_owned())
        } else if i == f.at {
            "今ここ".to_owned()
        } else {
            lane_value(info.lanes.get(l).map_or(&[][..], Vec::as_slice))
        };
        ui::pad_circle(
            c,
            g,
            px,
            py,
            46.0,
            ui::lane_color(*l),
            lane_label(*l),
            &sub,
            i == f.at,
        );
    }
    let left = 1.0 - f.since.elapsed().as_secs_f32() / FLOW_STEP.as_secs_f32();
    countdown(c, 490.0, 646.0, 300.0, 6.0, left);
    let secs = (left * FLOW_STEP.as_secs_f32()).ceil().max(0.0);
    ui::text_center(
        c,
        g,
        &format!("叩かずに {secs:.0} 秒で次のレーンへ"),
        640.0,
        658.0,
        &ui::plain(14.0, ui::DIM),
        600.0,
    );
}

/// 画面の下の短い知らせ
fn draw_toast(c: &Canvas, g: &mut Gfx, text: &str) {
    let style = ui::plain(18.0, Color32::WHITE);
    let w = ui::text_width(g, text, &style).min(900.0) + 40.0;
    let x = 640.0 - w / 2.0;
    c.fill(x, 626.0, w, 40.0, Color32::from_rgba_premultiplied(10, 8, 18, 235));
    ui::round_frame(c, x, 626.0, w, 40.0, 2.0, 1.0, ui::ORANGE);
    ui::text_center(c, g, text, 640.0, 635.0, &style, 900.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(config: &Config) -> ConfigInfo<'_> {
        ConfigInfo {
            config,
            lanes: BTreeMap::from([(
                DrumLane::SD,
                vec![
                    LaneSource {
                        source: "V51".to_owned(),
                        inputs: vec!["38".to_owned(), "40".to_owned()],
                    },
                    LaneSource {
                        source: KEYBOARD.to_owned(),
                        inputs: vec!["D".to_owned()],
                    },
                ],
            )]),
            devices: vec![
                DeviceRow {
                    name: "V51".to_owned(),
                    id: "v51".to_owned(),
                    open: true,
                    error: None,
                    disabled: false,
                    preset: Preset::Roland,
                    default_preset: Preset::Roland,
                    channel: 0,
                    min_velocity: 0,
                    changes: 0,
                    last: Some("ch10 ノート 38 ベロシティ 100 → スネア".to_owned()),
                },
                DeviceRow {
                    name: "Finger Drum Pad".to_owned(),
                    id: "fgdp".to_owned(),
                    open: true,
                    error: None,
                    disabled: false,
                    preset: Preset::GeneralMidi,
                    default_preset: Preset::GeneralMidi,
                    channel: 10,
                    min_velocity: 8,
                    changes: 3,
                    last: None,
                },
            ],
            scanning: None,
            song_count: 3,
        }
    }

    fn find(rows: &[Row], id: ItemId) -> usize {
        rows.iter().position(|r| r.id == Some(id)).unwrap()
    }

    /// 一覧を開いて `id` の行を選んだ画面
    fn at(page: Page, rows: &[Row], id: ItemId) -> ConfigView {
        let mut v = ConfigView::new();
        v.focus_list = true;
        let cat = match page {
            Page::Cat(c) => c,
            _ => Cat::Input,
        };
        v.cat = CATS.iter().position(|&c| c == cat).unwrap();
        v.stack = vec![Level {
            page: Page::Cat(cat),
            cursor: 0,
        }];
        if !matches!(page, Page::Cat(_)) {
            v.stack.push(Level { page, cursor: 0 });
        }
        *v.cursor_mut() = find(rows, id);
        v
    }

    #[test]
    fn every_name_is_japanese_and_no_nx_folder() {
        let c = Config::default();
        let info = info(&c);
        for cat in CATS {
            for r in rows(Page::Cat(cat), &info) {
                assert!(!r.name.contains("NX"), "{}", r.name);
                if r.selectable() {
                    // 英字だけの名前は無い（機器の名前は除く）
                    let device = info.devices.iter().any(|d| d.name == r.name);
                    assert!(
                        device || !r.name.chars().all(|ch| ch.is_ascii() || ch == '　'),
                        "{}",
                        r.name
                    );
                }
            }
        }
    }

    #[test]
    fn values_round_trip_through_the_list_numbers() {
        let mut c = Config::default();
        for (s, v) in [
            (Setting::ScrollSpeed, 25),
            (Setting::InputAdjust, -12),
            (Setting::VisualAdjust, 7),
            (Setting::MasterVolume, 35),
            (Setting::ShowLagTime, 2),
            (Setting::ShowLagTimeColor, 1),
            (Setting::StageFailed, 0),
            (Setting::HitSound, 1),
            (Setting::ShowLagHitCount, 1),
            (Setting::AutoUpdate, 0),
        ] {
            set(s, &mut c, v);
            assert_eq!(get(s, &c), v, "{s:?}");
        }
        assert_eq!(c.scroll_px_per_ms, 1.25);
        // 範囲の外は端に
        set(Setting::InputAdjust, &mut c, 999);
        assert_eq!(c.input_offset_ms, OFFSET_RANGE);
        set(Setting::ScrollSpeed, &mut c, 0);
        assert_eq!(c.scroll_px_per_ms, 0.1);
    }

    #[test]
    fn left_and_right_change_values_in_place_and_stop_at_the_ends() {
        let c = Config::default();
        let list = rows(Page::Cat(Cat::Fav), &info(&c));
        let speed = ItemId::Value(Setting::ScrollSpeed);
        let mut v = at(Page::Cat(Cat::Fav), &list, speed);
        assert_eq!(
            v.input(ConfigInput::Right, &list).action,
            Some(ConfigAction::Set(speed, 13))
        );
        assert_eq!(
            v.input(ConfigInput::Left, &list).action,
            Some(ConfigAction::Set(speed, 11))
        );
        // ON / OFF は左が ON。ON のまま左は何もしない
        let hit = ItemId::Value(Setting::HitSound);
        *v.cursor_mut() = find(&list, hit);
        assert_eq!(v.input(ConfigInput::Left, &list), Response::default());
        assert_eq!(
            v.input(ConfigInput::Right, &list).action,
            Some(ConfigAction::Set(hit, 0))
        );
        // 選択肢は端で止まる（回らない）
        let lag = ItemId::Value(Setting::ShowLagTime);
        *v.cursor_mut() = find(&list, lag);
        assert_eq!(
            v.input(ConfigInput::Right, &list).action,
            Some(ConfigAction::Set(lag, 2))
        );
        let mut c2 = Config::default();
        c2.show_lag_time = 2;
        let list2 = rows(Page::Cat(Cat::Fav), &info(&c2));
        assert_eq!(v.input(ConfigInput::Right, &list2), Response::default());
    }

    #[test]
    fn the_list_does_not_wrap_and_skips_headings() {
        let c = Config::default();
        let list = rows(Page::Cat(Cat::View), &info(&c));
        assert_eq!(list[0].ctl, Ctl::Heading);
        let mut v = at(Page::Cat(Cat::View), &list, ItemId::Value(Setting::ScrollSpeed));
        assert_eq!(v.input(ConfigInput::Up, &list), Response::default());
        v.input(ConfigInput::Down, &list);
        // 小見出し「ずれの表示」を飛ばす
        assert_eq!(list[v.level().cursor].id, Some(ItemId::Value(Setting::ShowLagTime)));
        let last = list.iter().rposition(Row::selectable).unwrap();
        *v.cursor_mut() = last;
        assert_eq!(v.input(ConfigInput::Down, &list), Response::default());
        assert_eq!(v.level().cursor, last);
    }

    #[test]
    fn decide_opens_a_picker_that_reverts_on_back() {
        let c = Config::default();
        let list = rows(Page::Cat(Cat::Fav), &info(&c));
        let vol = ItemId::Value(Setting::MasterVolume);
        let mut v = at(Page::Cat(Cat::Fav), &list, vol);
        assert_eq!(v.input(ConfigInput::Decide, &list).action, None);
        assert!(v.picker.is_some());
        // 数は上で増やす（その場で変える）
        assert_eq!(
            v.input(ConfigInput::Up, &list).action,
            Some(ConfigAction::Set(vol, 55))
        );
        // 戻るなら元の値に
        assert_eq!(
            v.input(ConfigInput::Cancel, &list).action,
            Some(ConfigAction::Set(vol, 50))
        );
        assert!(v.picker.is_none() && v.focus_list);
        // 選択肢の窓は下で次の選択肢へ、決定で閉じる
        let lag = ItemId::Value(Setting::ShowLagTime);
        *v.cursor_mut() = find(&list, lag);
        v.input(ConfigInput::Decide, &list);
        assert_eq!(
            v.input(ConfigInput::Down, &list).action,
            Some(ConfigAction::Set(lag, 2))
        );
        assert_eq!(v.input(ConfigInput::Decide, &list).action, None);
        assert!(v.picker.is_none());
        // ON / OFF は決定で切り替える
        let hit = ItemId::Value(Setting::HitSound);
        *v.cursor_mut() = find(&list, hit);
        assert_eq!(
            v.input(ConfigInput::Decide, &list).action,
            Some(ConfigAction::Set(hit, 0))
        );
    }

    #[test]
    fn auto_play_is_in_favorites_and_play() {
        let mut config = Config::default();
        let fav = rows(Page::Cat(Cat::Fav), &info(&config));
        let auto = ItemId::Table(settings::AUTO_ALL);
        let i = find(&fav, auto);
        assert_eq!(fav[i].value, "OFF");
        let play = rows(Page::Cat(Cat::Play), &info(&config));
        let names: Vec<&str> = play.iter().map(|r| r.name.as_str()).collect();
        let at_auto = names.iter().position(|n| *n == "自動で叩く").unwrap();
        assert_eq!(play[at_auto].ctl, Ctl::Heading);
        assert_eq!(play[at_auto + 1].id, Some(auto));
        assert_eq!(names[at_auto + 2], "　左シンバル");
        // 混ざっていれば「-」で、決定で全部 ON に
        config.settings.set_raw("auto_bd", 1);
        let fav = rows(Page::Cat(Cat::Fav), &info(&config));
        assert_eq!(fav[i].value, "-");
        assert!(fav[i].changed());
        let mut v = at(Page::Cat(Cat::Fav), &fav, auto);
        assert_eq!(
            v.input(ConfigInput::Decide, &fav).action,
            Some(ConfigAction::Set(auto, 1))
        );
    }

    #[test]
    fn lanes_are_assigned_by_hitting_after_decide() {
        let config = Config::default();
        let info = info(&config);
        let input = rows(Page::Cat(Cat::Input), &info);
        let mut v = at(Page::Cat(Cat::Input), &input, ItemId::Lanes);
        assert_eq!(input[find(&input, ItemId::Lanes)].value, "1 / 11 レーン");
        // → でも開く
        v.input(ConfigInput::Right, &input);
        assert_eq!(v.page(), Page::Lanes);
        let lanes = rows(Page::Lanes, &info);
        let sd = &lanes[find(&lanes, ItemId::Lane(DrumLane::SD))];
        assert_eq!(sd.value, "38 40　キー D");
        assert_eq!(sd.name, "スネア");
        assert!(sd.desc.starts_with("V51: 38, 40\nキーボード: D\n"));
        *v.cursor_mut() = find(&lanes, ItemId::Lane(DrumLane::HT));
        v.input(ConfigInput::Decide, &lanes);
        assert!(v.wants_raw());
        // 待っているあいだは動かない
        assert_eq!(v.input(ConfigInput::Down, &lanes), Response::default());
        assert_eq!(
            v.pad_hit(1, 48).action,
            Some(ConfigAction::TogglePad(DrumLane::HT, 1, 48))
        );
        assert!(!v.wants_raw());
        // キーでも。演奏の操作のキーは割り当てない
        v.input(ConfigInput::Decide, &lanes);
        assert_eq!(v.key_hit(KeyCode::KeyR), None);
        assert_eq!(
            v.key_hit(KeyCode::KeyX).and_then(|r| r.action),
            Some(ConfigAction::ToggleKey(DrumLane::HT, KeyCode::KeyX))
        );
        // Delete で全部外す
        v.input(ConfigInput::Decide, &lanes);
        assert_eq!(
            v.key_hit(KeyCode::Delete).and_then(|r| r.action),
            Some(ConfigAction::ClearLane(DrumLane::HT))
        );
        // Esc で取りやめ
        v.input(ConfigInput::Decide, &lanes);
        v.input(ConfigInput::Cancel, &lanes);
        assert!(!v.wants_raw());
        assert_eq!(v.pad_hit(0, 48).action, None);
        // 戻ると入力機器の一覧の、割り当ての行へ
        v.input(ConfigInput::Cancel, &lanes);
        assert_eq!(v.page(), Page::Cat(Cat::Input));
        assert_eq!(v.level().cursor, find(&input, ItemId::Lanes));
    }

    #[test]
    fn assign_all_walks_every_lane_in_order() {
        let config = Config::default();
        let info = info(&config);
        let lanes = rows(Page::Lanes, &info);
        let mut v = at(Page::Lanes, &lanes, ItemId::AssignAll);
        v.input(ConfigInput::Decide, &lanes);
        assert!(v.flow.is_some());
        // 始めてすぐの打撃は受けない（決定に叩いたパッドを割り当てない）
        assert_eq!(v.pad_hit(0, 49).action, None);
        v.flow.as_mut().unwrap().guard = Instant::now();
        assert_eq!(
            v.pad_hit(0, 49).action,
            Some(ConfigAction::AssignPad(DrumLane::LC, 0, 49))
        );
        // 続けての打撃も少しのあいだ受けない
        assert_eq!(v.pad_hit(0, 49).action, None);
        // Space（決定）で飛ばす
        v.input(ConfigInput::Decide, &lanes);
        assert_eq!(v.flow.as_ref().unwrap().at, 2);
        v.flow.as_mut().unwrap().guard = Instant::now();
        assert_eq!(
            v.key_hit(KeyCode::KeyQ).and_then(|r| r.action),
            Some(ConfigAction::AssignKey(DrumLane::HHO, KeyCode::KeyQ))
        );
        // 叩かずに時間が過ぎれば次へ
        v.flow.as_mut().unwrap().since = Instant::now() - FLOW_STEP;
        assert!(v.tick().is_some());
        assert_eq!(v.flow.as_ref().unwrap().at, 4);
        // Esc でやめる
        v.input(ConfigInput::Cancel, &lanes);
        assert!(v.flow.is_none());
        assert!(v.toast.as_ref().unwrap().0.contains("2 レーン"));
    }

    #[test]
    fn waiting_gives_up_after_five_seconds() {
        let config = Config::default();
        let lanes = rows(Page::Lanes, &info(&config));
        let mut v = at(Page::Lanes, &lanes, ItemId::Lane(DrumLane::SD));
        v.input(ConfigInput::Decide, &lanes);
        assert!(v.tick().is_none());
        v.wait = Some((DrumLane::SD, Instant::now() - WAIT_FOR));
        assert_eq!(v.tick(), Some(Response::se(Se::Cancel)));
        assert!(!v.wants_raw());
    }

    #[test]
    fn irreversible_actions_ask_first() {
        let config = Config::default();
        let lanes = rows(Page::Lanes, &info(&config));
        let mut v = at(Page::Lanes, &lanes, ItemId::ResetPads);
        assert_eq!(v.input(ConfigInput::Decide, &lanes).action, None);
        // 初めは「いいえ」
        assert_eq!(v.input(ConfigInput::Decide, &lanes).action, None);
        assert!(v.confirm.is_none());
        v.input(ConfigInput::Decide, &lanes);
        v.input(ConfigInput::Left, &lanes);
        assert_eq!(
            v.input(ConfigInput::Decide, &lanes).action,
            Some(ConfigAction::Run(ItemId::ResetPads))
        );
        // 曲を探し直すのは確かめない
        let mut c = Config::default();
        c.song_dirs.push("/songs".into());
        let songs = rows(Page::Cat(Cat::Songs), &info(&c));
        let mut v = at(Page::Cat(Cat::Songs), &songs, ItemId::Rescan);
        assert_eq!(
            v.input(ConfigInput::Decide, &songs).action,
            Some(ConfigAction::Run(ItemId::Rescan))
        );
        *v.cursor_mut() = find(&songs, ItemId::SongDir(0));
        assert_eq!(v.input(ConfigInput::Decide, &songs).action, None);
        assert!(v.confirm.as_ref().unwrap().text.contains("songs"));
    }

    #[test]
    fn devices_have_their_own_page() {
        let config = Config::default();
        let info = info(&config);
        let input = rows(Page::Cat(Cat::Input), &info);
        let fgdp = find(&input, ItemId::Device(1));
        assert_eq!(input[fgdp].name, "Finger Drum Pad");
        assert_eq!(input[fgdp].value, "GM");
        let mut v = at(Page::Cat(Cat::Input), &input, ItemId::Device(1));
        v.input(ConfigInput::Decide, &input);
        assert_eq!(v.page(), Page::Device(1));
        let dev = rows(Page::Device(1), &info);
        let names: Vec<&str> = dev.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["状態", "使う", "プリセット", "チャンネル", "最小ベロシティ", "割り当てを戻す"]
        );
        assert_eq!(dev[2].value, "GM");
        assert_eq!(dev[3].value, "10");
        assert!(dev[3].changed());
        assert_eq!(dev[4].value, "8");
        // 「状態」は選べるが動かない。下へ動くと「使う」
        v.sync(&dev);
        assert_eq!(v.input(ConfigInput::Decide, &dev), Response::default());
        v.input(ConfigInput::Down, &dev);
        v.input(ConfigInput::Down, &dev);
        // プリセットは左右で替える（Roland ← GM → なし）
        assert_eq!(
            v.input(ConfigInput::Left, &dev).action,
            Some(ConfigAction::Set(ItemId::DevicePreset(1), 0))
        );
        v.input(ConfigInput::Down, &dev);
        assert_eq!(
            v.input(ConfigInput::Left, &dev).action,
            Some(ConfigAction::Set(ItemId::DeviceChannel(1), 9))
        );
        // つながる機器が変わったら、機器の一覧から出る
        v.devices_changed();
        assert_eq!(v.page(), Page::Cat(Cat::Input));
    }

    #[test]
    fn rail_moves_categories_and_escape_leaves() {
        let config = Config::default();
        let info = info(&config);
        let mut v = ConfigView::new();
        let list = rows(v.page(), &info);
        assert_eq!(v.input(ConfigInput::Up, &list), Response::default());
        v.input(ConfigInput::Down, &list);
        assert_eq!(v.page(), Page::Cat(Cat::Play));
        v.input(ConfigInput::Right, &list);
        assert!(v.focus_list);
        let list = rows(v.page(), &info);
        v.input(ConfigInput::Cancel, &list);
        assert!(!v.focus_list);
        assert_eq!(
            v.input(ConfigInput::Cancel, &list),
            Response::action(Se::Cancel, ConfigAction::Leave)
        );
        // 出ると決めたあとは動かない
        assert_eq!(v.input(ConfigInput::Down, &list), Response::default());
    }

    #[test]
    fn the_list_scrolls_to_keep_the_cursor_visible() {
        let rows: Vec<Row> = (0..30)
            .map(|i| {
                row(
                    ItemId::SongDir(i),
                    "x",
                    String::new(),
                    String::new(),
                    Ctl::Info,
                )
            })
            .collect();
        assert_eq!(first_visible(&rows, Some(0), 500.0), 0);
        assert_eq!(first_visible(&rows, None, 500.0), 0);
        let top = first_visible(&rows, Some(20), 500.0);
        let below: f32 = (top..=20).map(|_| ROW_H).sum();
        assert!(below <= 500.0 - ROW_H * 2.5 + 0.1 && top > 0);
    }
}
