//! 設定の画面（上流 `CStageConfig` と `CActConfigList`）。
//! スキンの 4_ の画像で、左のメニュー（System / Drums / Exit）と、右の項目の一覧、説明のパネルを出す。
//! 項目は NXT のもの（曲のフォルダ、入力と表示の補正、スクロールの速さ、パッドの割り当てなど）。
//! 割り当ては入力機器（MIDI のポートと PC のキーボード）ごと。Drums の「Drums Keys」でレーンごとに、叩いたパッドか
//! 押したキーを足し、「MIDI Devices」で機器ごとの出発点（Roland / GM / なし）・チャンネル・使うかどうかを決める。
//! 出入りは NX v2.0 と同じくシャッター（`shutter`）で、白いフェードは使わない。
//!
//! 操作は上流と同じ。↑↓（HT / LT）で選び、Enter（CY / RD）で決め、Esc（LC）で戻る。
//! 数の項目は Enter で値を変える状態になり、↑↓ で値を変えて、もう一度 Enter で戻る。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use dtx::DrumLane;
use egui::Color32;
use engine::input::Preset;
use library::Progress;
use winit::keyboard::KeyCode;

use crate::config::{Config, MASTER_VOLUME_STEP};
use crate::gfx::{Canvas, Gfx, src};
use crate::inputs::LaneSource;
use crate::sfx::Se;
use crate::text::Style;
use crate::title_view::draw_scanning;

/// 左のメニュー（上流は System / Drums / Guitar / Bass / Exit。ギターとベースは扱わない）
const MENU: [&str; 3] = ["System", "Drums", "Exit"];
const MENU_EXIT: usize = 2;

/// 項目の箱の左と、選んでいる行（0 行目）の上。行の間は 67（上流 `pt新パネルの基本座標`）
const ITEM_X: f32 = 420.0;
const ITEM_Y: f32 = 189.0;
const ITEM_STEP: f32 = 67.0;

/// 説明のパネルを出すまでの間（上流 `ctDisplayWait`：0〜350 を 1 ミリ秒ずつ）
const DESCRIPTION_WAIT: Duration = Duration::from_millis(350);

/// 説明の文字を置ける下の端（4_Description Panel.png の白い所）
const DESCRIPTION_BOTTOM: f32 = 252.0 + 155.0;

/// 補正の値の範囲（ミリ秒）
pub const OFFSET_RANGE: f64 = 200.0;

/// 画面から App に頼むこと
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfigAction {
    AddSongDir,
    RemoveSongDir(usize),
    Rescan,
    ReconnectMidi,
    /// 数の値を 1 段変える
    Nudge(Setting, i32),
    /// ON と OFF などを切り替える
    Toggle(Setting),
    /// MIDI のポート（何番目か）のノート番号をレーンに割り当てる
    AssignPad(DrumLane, usize, u8),
    /// キーボードのキーをレーンに割り当てる
    AssignKey(DrumLane, KeyCode),
    /// レーンの割り当てを、すべての機器とキーボードから外す
    ClearLane(DrumLane),
    /// このアプリで変えた割り当てをすべて消す
    ResetPads,
    /// 機器（ポートの何番目か）を使うかどうかを切り替える
    ToggleDevice(usize),
    /// 機器の割り当ての出発点を次のものにする
    NextPreset(usize),
    /// 機器の受けるチャンネルを 1 段変える（すべて → 1 → … → 16）
    NudgeChannel(usize, i32),
    /// 機器の捨てる弱さ（ベロシティ）を 1 段変える
    NudgeMinVelocity(usize, i32),
    /// 機器の割り当てを出発点に戻す
    ResetDevice(usize),
    /// 設定の画面を出る（App がシャッターを閉じてから移る）
    Leave,
}

/// 値を変えられる設定
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
}

/// 一覧の項目
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemId {
    /// `<< ReturnTo Menu`
    Return,
    AddSongDir,
    SongDir(usize),
    Rescan,
    Value(Setting),
    /// パッドの割り当ての一覧へ
    DrumsKeys,
    Pad(DrumLane),
    ResetPads,
    /// MIDI の機器の一覧へ
    MidiDevices,
    /// MIDI の機器を探し直す
    ReconnectMidi,
    /// MIDI の機器（ポートの何番目か）の設定へ
    Device(usize),
    /// 機器の名前と最後に届いた入力（見るだけ）
    DeviceInfo(usize),
    DeviceUse(usize),
    DevicePreset(usize),
    DeviceChannel(usize),
    DeviceMinVelocity(usize),
    DeviceReset(usize),
}

impl ItemId {
    /// Enter で値を変える状態になる数の項目
    fn is_integer(self) -> bool {
        matches!(
            self,
            ItemId::Value(
                Setting::MasterVolume
                    | Setting::InputAdjust
                    | Setting::VisualAdjust
                    | Setting::ScrollSpeed
            ) | ItemId::DeviceChannel(_)
                | ItemId::DeviceMinVelocity(_)
        )
    }
}

/// 一覧の 1 行
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: ItemId,
    pub name: String,
    pub value: String,
    pub desc: String,
}

/// どの一覧を出しているか
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    System,
    Drums,
    /// Drums の中のパッドの割り当て
    DrumsKeys,
    /// Drums の中の MIDI の機器の一覧
    MidiDevices,
    /// MIDI の機器 1 つ（ポートの何番目か）の設定
    Device(usize),
}

impl List {
    /// Esc や「戻る」で行く一覧（`None` なら左のメニューへ）
    fn parent(self) -> Option<List> {
        match self {
            List::System | List::Drums => None,
            List::DrumsKeys | List::MidiDevices => Some(List::Drums),
            List::Device(_) => Some(List::MidiDevices),
        }
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

/// パッドの割り当ての一覧の並び（上流 `tSetupItemList_KeyAssignDrums`）
const PADS: [(DrumLane, &str); 11] = [
    (DrumLane::LC, "LeftCymbal"),
    (DrumLane::HH, "HiHat(Close)"),
    (DrumLane::HHO, "HiHat(Open)"),
    (DrumLane::SD, "Snare"),
    (DrumLane::BD, "Bass"),
    (DrumLane::HT, "HighTom"),
    (DrumLane::LT, "LowTom"),
    (DrumLane::FT, "FloorTom"),
    (DrumLane::CY, "RightCymbal"),
    (DrumLane::RD, "RideCymbal"),
    (DrumLane::LP, "LeftPedal"),
];

fn on_off(b: bool) -> String {
    (if b { "ON" } else { "OFF" }).to_owned()
}

fn folder_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn item(id: ItemId, name: &str, value: String, desc: String) -> Item {
    Item {
        id,
        name: name.to_owned(),
        value,
        desc,
    }
}

fn return_item() -> Item {
    item(
        ItemId::Return,
        "<< ReturnTo Menu",
        String::new(),
        "左側のメニューに戻ります。".to_owned(),
    )
}

/// チャンネルの表示（0 はすべて）
fn channel_label(ch: u8) -> String {
    if (1..=16).contains(&ch) {
        ch.to_string()
    } else {
        "ALL".to_owned()
    }
}

/// 機器の一覧の値（出発点か、使っていないわけ）
fn device_state(d: &DeviceRow) -> String {
    if d.disabled {
        "OFF".to_owned()
    } else if !d.open {
        "Error".to_owned()
    } else {
        d.preset.label().to_owned()
    }
}

/// 機器の様子の説明（説明のパネルに出す）
fn device_summary(d: &DeviceRow) -> String {
    let state = if d.disabled {
        "使わない設定です。".to_owned()
    } else if let Some(e) = &d.error {
        format!("開けません（ほかのアプリが使っているかも）: {e}")
    } else {
        format!(
            "{} / ch {} / 変更 {} 個",
            d.preset.label(),
            channel_label(d.channel),
            d.changes
        )
    };
    match &d.last {
        Some(last) => format!("{state}\n最後: {last}"),
        None => format!("{state}\n叩くとここに届いた内容が出ます。"),
    }
}

/// 機器の名前の並び（説明のパネルに出す）
fn device_names(devices: &[DeviceRow]) -> String {
    if devices.is_empty() {
        return "つながっている機器はありません。".to_owned();
    }
    devices
        .iter()
        .map(|d| d.name.as_str())
        .collect::<Vec<_>>()
        .join("、")
}

/// 一覧の項目
pub fn items(list: List, info: &ConfigInfo<'_>) -> Vec<Item> {
    let c = info.config;
    let mut v = vec![return_item()];
    match list {
        List::System => {
            v.push(item(
                ItemId::AddSongDir,
                "Add Song Folder",
                String::new(),
                "曲のフォルダを足します。Enter でフォルダを選びます。窓にドロップしても足せます。"
                    .to_owned(),
            ));
            for (i, d) in c.song_dirs.iter().enumerate() {
                v.push(item(
                    ItemId::SongDir(i),
                    &format!("Song Folder {}", i + 1),
                    folder_name(d),
                    format!(
                        "{}\nEnter で一覧から外します（ファイルは消しません）。",
                        d.display()
                    ),
                ));
            }
            v.push(item(
                ItemId::Rescan,
                "Reload Songs",
                match info.scanning {
                    Some(p) => format!("{} 曲…", p.songs),
                    None => format!("{} 曲", info.song_count),
                },
                "曲のフォルダを探し直して、曲の一覧を作り直します。".to_owned(),
            ));
            v.push(item(
                ItemId::Value(Setting::MasterVolume),
                "MasterVolume",
                c.master_volume().to_string(),
                format!(
                    "全体の音量です。0 が無音で、100 が最大です。\nEnter を押してから ↑↓ で {MASTER_VOLUME_STEP} ずつ変えます。"
                ),
            ));
            v.push(item(
                ItemId::Value(Setting::StageFailed),
                "StageFailed",
                on_off(c.stage_failed()),
                "ON にするとゲージがなくなった時に STAGE FAILED となり演奏が中断されます。練習では中断しません。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::Value(Setting::ShowLagTime),
                "ShowLagTime",
                match c.show_lag_time {
                    0 => "OFF",
                    2 => "GREAT-",
                    _ => "ON",
                }
                .to_owned(),
                "判定の下に、ちょうどの時刻からのずれ（ms）を出します。\nGREAT- は PERFECT 以外のときだけ出します。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::Value(Setting::ShowLagTimeColor),
                "ShowLagTimeColor",
                (if c.show_lag_time_color == 1 {
                    "TYPE-B"
                } else {
                    "TYPE-A"
                })
                .to_owned(),
                "ずれの数字の色です。\nTYPE-A: 早いと青、遅いと赤。\nTYPE-B: 早いと赤、遅いと青。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::Value(Setting::ShowLagHitCount),
                "ShowLagHitCount",
                on_off(c.show_lag_hit_count),
                "ON にすると、結果の画面に早かった数と遅かった数を出します。".to_owned(),
            ));
        }
        List::Drums => {
            v.push(item(
                ItemId::Value(Setting::ScrollSpeed),
                "ScrollSpeed",
                format!("{:.2}", c.scroll_px_per_ms),
                "ノーツの流れるスピードです。数字が大きいほど速くなり、ノーツの間隔が広がります。演奏中は ↑↓。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::Value(Setting::InputAdjust),
                "InputAdjust",
                format!("{:+.0} ms", c.input_offset_ms),
                format!(
                    "叩いた時刻の補正です。結果の画面のずれが遅めなら大きく、早めなら小さくします。\n-{0:.0} ～ {0:.0}ms。演奏中は - と =。",
                    OFFSET_RANGE
                ),
            ));
            v.push(item(
                ItemId::Value(Setting::VisualAdjust),
                "VisualAdjust",
                format!("{:+.0} ms", c.visual_offset_ms),
                format!(
                    "譜面の表示の補正です。音と見た目がずれて見えるときに直します。\n-{0:.0} ～ {0:.0}ms。演奏中は [ と ]。",
                    OFFSET_RANGE
                ),
            ));
            v.push(item(
                ItemId::Value(Setting::HitSound),
                "HitSound",
                on_off(c.drum_sounds),
                format!(
                    "叩いたときに譜面の音を鳴らします。電子ドラムの音源の音を使うなら OFF にします。演奏中は {}。",
                    crate::HIT_SOUND_KEY_NAME
                ),
            ));
            v.push(item(
                ItemId::DrumsKeys,
                "Drums Keys",
                String::new(),
                "レーンごとに、叩くパッド（MIDI のノート番号）とキーボードのキーを割り当てます。機器ごとに別々に持ちます。"
                    .to_owned(),
            ));
            let open = info.devices.iter().filter(|d| d.open).count();
            v.push(item(
                ItemId::MidiDevices,
                "MIDI Devices",
                format!("{open} 台"),
                format!(
                    "MIDI の機器ごとに、割り当ての出発点、受けるチャンネル、使うかどうかを決めます。\n{}",
                    device_names(&info.devices)
                ),
            ));
        }
        List::DrumsKeys => {
            for (lane, name) in PADS {
                let sources = info.lanes.get(&lane).map_or(&[][..], Vec::as_slice);
                let value = if sources.is_empty() {
                    "-".to_owned()
                } else {
                    sources
                        .iter()
                        .map(|s| s.inputs.join(" "))
                        .collect::<Vec<_>>()
                        .join(" / ")
                };
                let mut desc: String = sources
                    .iter()
                    .map(|s| format!("{}: {}\n", s.source, s.inputs.join(", ")))
                    .collect();
                if sources.is_empty() {
                    desc.push_str("割り当てがありません。\n");
                }
                desc.push_str("Enter のあとパッドを叩くかキーを押すと足します。Delete でこのレーンの割り当てを全部外します。");
                v.push(item(ItemId::Pad(lane), name, value, desc));
            }
            let changed = c.has_custom_assignments();
            v.push(item(
                ItemId::ResetPads,
                "Reset Keys",
                String::new(),
                format!(
                    "ここで変えた割り当てを全部消して、機器ごとの出発点とキーボードの既定に戻します。{}",
                    if changed { "" } else { "\n（今は変えていません）" }
                ),
            ));
        }
        List::MidiDevices => {
            for (i, d) in info.devices.iter().enumerate() {
                v.push(item(
                    ItemId::Device(i),
                    &d.name,
                    device_state(d),
                    format!(
                        "{}\n{}\nEnter でこの機器の設定を開きます。",
                        d.name,
                        device_summary(d)
                    ),
                ));
            }
            v.push(item(
                ItemId::ReconnectMidi,
                "Rescan Devices",
                if info.devices.is_empty() {
                    "なし".to_owned()
                } else {
                    format!("{} 台", info.devices.len())
                },
                "MIDI の機器を探し直して、開き直します。後からつないだ機器は、この画面にいれば自動でも見つけます。"
                    .to_owned(),
            ));
        }
        List::Device(i) => {
            let Some(d) = info.devices.get(i) else {
                return v;
            };
            v.push(item(
                ItemId::DeviceInfo(i),
                "Device",
                d.name.clone(),
                format!("{}\n{}", d.name, device_summary(d)),
            ));
            v.push(item(
                ItemId::DeviceUse(i),
                "Use",
                on_off(!d.disabled),
                "OFF にすると、このアプリはこの機器を開きません（ほかのアプリで使うときなど）。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::DevicePreset(i),
                "Preset",
                d.preset.label().to_owned(),
                "割り当ての出発点です。Enter で替えます。\nRoland: V-Drums の既定（リムやエッジも）。\nGM: General MIDI のドラムの番号。\nNone: 何もなし。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::DeviceChannel(i),
                "Channel",
                channel_label(d.channel),
                "受ける MIDI のチャンネルです。ALL ならどのチャンネルでも受けます。\nEnter を押してから ↑↓ で変えます。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::DeviceMinVelocity(i),
                "MinVelocity",
                if d.min_velocity == 0 {
                    "OFF".to_owned()
                } else {
                    d.min_velocity.to_string()
                },
                "これより弱い打撃（ベロシティ）を捨てます。隣のパッドにつられて鳴る弱い音を除きます。\nEnter を押してから ↑↓ で変えます。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::DeviceReset(i),
                "Reset Keys",
                if d.changes == 0 {
                    String::new()
                } else {
                    format!("{} 個", d.changes)
                },
                "この機器で変えた割り当てを消して、出発点に戻します。".to_owned(),
            ));
        }
    }
    v
}

/// 操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigInput {
    Up,
    Down,
    Decide,
    Cancel,
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

pub struct ConfigView {
    menu: usize,
    /// 出している一覧（Exit を選んでいるあいだは前の一覧のまま）
    list: List,
    /// 項目の一覧の側を操作している（でなければ左のメニュー）
    focus_list: bool,
    /// 数の値を変えている
    editing: bool,
    cursor: usize,
    /// スクロールの残り（1 行 = 100。正なら下の行へ動いている途中）
    pending: i64,
    scroll_tick: Instant,
    /// 最後にカーソルを動かした時刻（説明のパネルはしばらく止まってから出す）
    moved_at: Instant,
    /// 叩くのを待っているパッド
    waiting: Option<DrumLane>,
    entered: Instant,
    /// 出ると決めた（あとは操作を受け付けない）
    leaving: bool,
}

impl ConfigView {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            menu: 0,
            list: List::System,
            focus_list: false,
            editing: false,
            cursor: 0,
            pending: 0,
            scroll_tick: now,
            moved_at: now,
            waiting: None,
            entered: now,
            leaving: false,
        }
    }

    pub fn list(&self) -> List {
        self.list
    }

    /// パッドを叩くのを待っている
    pub fn waiting_pad(&self) -> bool {
        self.waiting.is_some()
    }

    /// 待っているレーンに、MIDI のポート（何番目か）のノートを割り当てる
    pub fn pad_hit(&mut self, port: usize, note: u8) -> Response {
        match self.waiting.take() {
            Some(lane) => Response::action(Se::Decide, ConfigAction::AssignPad(lane, port, note)),
            None => Response::default(),
        }
    }

    /// 待っているあいだに押したキー。割り当てられるキーなら割り当て、Delete ならレーンの割り当てを全部外す。
    /// 待っていなければ、またはほかのキー（Esc など）なら `None`（ふつうの操作として扱う）
    pub fn key_hit(&mut self, code: KeyCode) -> Option<Response> {
        let lane = self.waiting?;
        if code == KeyCode::Delete {
            self.waiting = None;
            return Some(Response::action(Se::Decide, ConfigAction::ClearLane(lane)));
        }
        if crate::inputs::assignable(code) {
            self.waiting = None;
            return Some(Response::action(
                Se::Decide,
                ConfigAction::AssignKey(lane, code),
            ));
        }
        None
    }

    /// MIDI の機器がつながったり外れたりした（機器の番号が変わるので、機器の設定を開いていたら一覧へ戻る）
    pub fn devices_changed(&mut self) {
        if matches!(self.list, List::Device(_)) {
            self.list = List::MidiDevices;
            self.cursor = 0;
            self.pending = 0;
            self.editing = false;
        }
    }

    /// 出る（v2 と同じく、白へは消さずに App がシャッターを閉じる）
    fn leave(&mut self, se: Se) -> Response {
        self.leaving = true;
        Response::action(se, ConfigAction::Leave)
    }

    fn set_list(&mut self, list: List) {
        if self.list != list {
            self.list = list;
            self.cursor = 0;
            self.pending = 0;
        }
    }

    fn moved(&mut self) {
        self.moved_at = Instant::now();
    }

    /// 一つ上の一覧へ（一番上なら左のメニューへ）。機器の設定からは、その機器の行に戻る
    fn back(&mut self) {
        let from = self.list;
        match from.parent() {
            Some(parent) => {
                self.set_list(parent);
                if let List::Device(i) = from {
                    // 0 行目は「戻る」
                    self.cursor = i + 1;
                }
            }
            None => self.focus_list = false,
        }
        self.moved();
    }

    pub fn input(&mut self, input: ConfigInput, items: &[Item]) -> Response {
        if self.leaving {
            return Response::default();
        }
        if self.waiting.is_some() {
            // パッドを待っているあいだは Esc で取りやめるだけ（パッドは叩いたものを割り当てる）
            if input == ConfigInput::Cancel {
                self.waiting = None;
                return Response::se(Se::Cancel);
            }
            return Response::default();
        }
        if !self.focus_list {
            return self.menu_input(input);
        }
        let n = items.len().max(1);
        self.cursor = self.cursor.min(n - 1);
        let Some(it) = items.get(self.cursor) else {
            return Response::default();
        };
        match input {
            ConfigInput::Up | ConfigInput::Down => {
                let d: i64 = if input == ConfigInput::Up { -1 } else { 1 };
                if self.editing {
                    // 上流と同じく ↑ で値を増やす
                    return match it.id {
                        ItemId::Value(s) => {
                            Response::action(Se::Move, ConfigAction::Nudge(s, -d as i32))
                        }
                        ItemId::DeviceChannel(i) => {
                            Response::action(Se::Move, ConfigAction::NudgeChannel(i, -d as i32))
                        }
                        ItemId::DeviceMinVelocity(i) => {
                            Response::action(Se::Move, ConfigAction::NudgeMinVelocity(i, -d as i32))
                        }
                        _ => Response::default(),
                    };
                }
                self.cursor = (self.cursor as i64 + d).rem_euclid(n as i64) as usize;
                self.pending += d * 100;
                self.moved();
                Response::se(Se::Move)
            }
            ConfigInput::Decide => {
                if self.editing {
                    self.editing = false;
                    return Response::se(Se::Decide);
                }
                if it.id.is_integer() {
                    self.editing = true;
                    return Response::se(Se::Decide);
                }
                let action = match it.id {
                    ItemId::Return => {
                        self.back();
                        return Response::se(Se::Decide);
                    }
                    ItemId::DrumsKeys | ItemId::MidiDevices | ItemId::Device(_) => {
                        self.set_list(match it.id {
                            ItemId::DrumsKeys => List::DrumsKeys,
                            ItemId::Device(i) => List::Device(i),
                            _ => List::MidiDevices,
                        });
                        self.moved();
                        return Response::se(Se::Decide);
                    }
                    ItemId::DeviceInfo(_) => return Response::default(),
                    ItemId::Pad(lane) => {
                        self.waiting = Some(lane);
                        return Response::se(Se::Decide);
                    }
                    ItemId::AddSongDir => ConfigAction::AddSongDir,
                    ItemId::SongDir(i) => ConfigAction::RemoveSongDir(i),
                    ItemId::Rescan => ConfigAction::Rescan,
                    ItemId::ReconnectMidi => ConfigAction::ReconnectMidi,
                    ItemId::Value(s) => ConfigAction::Toggle(s),
                    ItemId::ResetPads => ConfigAction::ResetPads,
                    ItemId::DeviceUse(i) => ConfigAction::ToggleDevice(i),
                    ItemId::DevicePreset(i) => ConfigAction::NextPreset(i),
                    ItemId::DeviceReset(i) => ConfigAction::ResetDevice(i),
                    ItemId::DeviceChannel(_) | ItemId::DeviceMinVelocity(_) => {
                        return Response::default();
                    }
                };
                Response::action(Se::Decide, action)
            }
            ConfigInput::Cancel => {
                if self.editing {
                    self.editing = false;
                    self.moved();
                } else {
                    self.back();
                }
                Response::se(Se::Cancel)
            }
        }
    }

    fn menu_input(&mut self, input: ConfigInput) -> Response {
        match input {
            ConfigInput::Up | ConfigInput::Down => {
                let d = if input == ConfigInput::Up {
                    MENU.len() - 1
                } else {
                    1
                };
                self.menu = (self.menu + d) % MENU.len();
                match self.menu {
                    0 => self.set_list(List::System),
                    1 => self.set_list(List::Drums),
                    _ => {}
                }
                self.moved();
                Response::se(Se::Move)
            }
            ConfigInput::Decide => {
                if self.menu == MENU_EXIT {
                    return self.leave(Se::Decide);
                }
                self.focus_list = true;
                self.moved();
                Response::se(Se::Decide)
            }
            ConfigInput::Cancel => self.leave(Se::Cancel),
        }
    }

    /// スクロールを進める（上流：2 ミリ秒ごとに、残りが多いほど速く）
    fn update_scroll(&mut self) {
        let now = Instant::now();
        while now.duration_since(self.scroll_tick) >= Duration::from_millis(2) {
            self.scroll_tick += Duration::from_millis(2);
            let left = self.pending.abs();
            if left == 0 {
                self.scroll_tick = now;
                break;
            }
            let step = match left {
                0..=100 => 2,
                101..=300 => 3,
                301..=500 => 4,
                _ => 8,
            };
            self.pending -= self.pending.signum() * step.min(left);
        }
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, items: &[Item], scanning: bool) {
        self.update_scroll();
        match g.skin.tex("Graphics/4_background.png") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(20, 24, 40)),
        }
        if let Some(t) = g.skin.tex("Graphics/4_item bar.png") {
            c.draw(&t, 400.0, 0.0);
        }
        self.draw_menu(c, g);
        self.draw_items(c, g, items);
        if self.focus_list {
            match g.skin.tex("Graphics/4_itembox cursor.png") {
                Some(t) => c.draw(&t, 413.0, 193.0),
                None => c.frame(413.0, 193.0, 444.0, 67.0, Color32::from_rgb(230, 30, 160)),
            }
            let settled = self.pending == 0 && self.moved_at.elapsed() >= DESCRIPTION_WAIT;
            if settled && let Some(it) = items.get(self.cursor) {
                draw_description(c, g, &it.desc);
            }
            if let Some(t) = g.skin.tex("Graphics/4_Arrow.png") {
                c.part(&t, 394.0, 174.0, src(0.0, 0.0, 40.0, 40.0));
                c.part(&t, 394.0, 240.0, src(0.0, 40.0, 40.0, 40.0));
            }
        }
        if let Some(t) = g.skin.tex("Graphics/4_header panel.png") {
            c.draw(&t, 0.0, 0.0);
        }
        if let Some(t) = g.skin.tex("Graphics/4_footer panel.png") {
            c.draw(&t, 0.0, 720.0 - t.h);
        }
        if self.waiting.is_some() {
            draw_hit_key_dialog(c, g);
        }
        if scanning {
            draw_scanning(c, g, self.entered);
        }
    }

    /// 左のメニュー（v2：4_menu panel.png を (245, 140) に、4_menu cursor.png を 170×28 に伸ばして重ね、
    /// 18 ピクセルの文字を x = 340 の中央、y = 144 から 32 ずつに）
    fn draw_menu(&self, c: &Canvas, g: &mut Gfx) {
        if let Some(t) = g.skin.tex("Graphics/4_menu panel.png") {
            // v2 は 5 行ぶんの枠。3 行ぶんに縮め、下の縁は残す
            let h = 12.0 + MENU.len() as f32 * 32.0;
            let edge = 8.0f32.min(t.h);
            c.part(&t, 245.0, 140.0, src(0.0, 0.0, t.w, h - edge));
            c.part(&t, 245.0, 140.0 + h - edge, src(0.0, t.h - edge, t.w, edge));
        }
        if let Some(t) = g.skin.tex("Graphics/4_menu cursor.png") {
            // v2 は左右の 16 ピクセルを伸ばさずに、間だけ横へ伸ばす（`ERenderMode.Sliced`）
            let alpha = if self.focus_list { 0.5 } else { 1.0 };
            let (x, y, w, h) = (250.0, 146.0 + self.menu as f32 * 32.0, 170.0, 28.0);
            let cap = 16.0f32.min(t.w / 2.0);
            let mid = t.w - cap * 2.0;
            let sy = h / t.h;
            let part = |x: f32, s: crate::gfx::Src, sx: f32| {
                c.draw_opt(
                    &t,
                    x,
                    y,
                    crate::gfx::Opt {
                        src: Some(s),
                        sx,
                        sy,
                        alpha,
                        ..Default::default()
                    },
                );
            };
            part(x, src(0.0, 0.0, cap, t.h), 1.0);
            if mid > 0.0 {
                part(x + cap, src(cap, 0.0, mid, t.h), (w - cap * 2.0) / mid);
            }
            part(x + w - cap, src(t.w - cap, 0.0, cap, t.h), 1.0);
        }
        for (i, name) in MENU.iter().enumerate() {
            // v2 `UIBasicButton`：ふだんは縁取りした白、選んでいると黄から橙のグラデーション
            let style = if i == self.menu {
                Style {
                    gradient: Some(Color32::from_rgb(255, 69, 0)),
                    ..Style::edged(18.0, Color32::from_rgb(255, 255, 0), Color32::BLACK, 2.0)
                }
            } else {
                Style::edged(18.0, Color32::WHITE, Color32::BLACK, 2.0)
            };
            if let Some(t) = g.texts.get(name, &style) {
                t.draw(c, 340.0 - t.w / 2.0, 144.0 + i as f32 * 32.0, 1.0, 1.0);
            }
        }
    }

    /// 項目の箱を 14 行（上流 `n行番号` -4〜9）
    fn draw_items(&self, c: &Canvas, g: &mut Gfx, items: &[Item]) {
        let n = items.len() as i64;
        if n == 0 {
            return;
        }
        let pos = self.cursor as i64 * 100 - self.pending;
        let sel = pos.div_euclid(100);
        let frac = pos.rem_euclid(100) as f32 / 100.0;
        let normal = g.skin.tex("Graphics/4_itembox.png");
        let other = g.skin.tex("Graphics/4_itembox other.png");
        let name_style = Style::plain(20.0, Color32::WHITE);
        let value_style = Style::plain(20.0, Color32::BLACK);
        let edit_style = Style {
            gradient: Some(Color32::from_rgb(255, 69, 0)),
            ..Style::edged(20.0, Color32::from_rgb(255, 255, 0), Color32::BLACK, 2.0)
        };
        for row in -4..10i64 {
            let it = &items[(sel + row).rem_euclid(n) as usize];
            let y = ITEM_Y + ITEM_STEP * (row as f32 - frac);
            if !(-80.0..=720.0).contains(&y) {
                continue;
            }
            let panel = if it.id == ItemId::Return {
                &other
            } else {
                &normal
            };
            match panel {
                Some(t) => c.draw(t, ITEM_X, y),
                None => {
                    // スキンが無いときは箱と、項目名の黒い所だけ
                    c.fill(ITEM_X, y + 4.0, 438.0, 72.0, Color32::from_gray(230));
                    c.fill(ITEM_X + 18.0, y + 13.0, 180.0, 35.0, Color32::BLACK);
                }
            }
            if let Some(t) = g.texts.get(&it.name, &name_style) {
                t.draw_fit(c, ITEM_X + 20.0, y + 24.0, 176.0);
            }
            let editing = self.editing && row == 0 && frac == 0.0;
            if editing {
                if let Some(t) = g.texts.get(&it.value, &edit_style) {
                    t.draw_fit(c, ITEM_X + 260.0, y + 20.0, 168.0);
                }
            } else if let Some(t) = g.texts.get(&it.value, &value_style) {
                t.draw_fit(c, ITEM_X + 260.0, y + 24.0, 168.0);
            }
        }
    }
}

/// 説明のパネル（上流：4_Description Panel.png を (781, 252) に、黒い 17 ピクセルの文字を (800, 270) から 230 の幅で）
fn draw_description(c: &Canvas, g: &mut Gfx, desc: &str) {
    if let Some(t) = g.skin.tex("Graphics/4_Description Panel.png") {
        c.draw(&t, 781.0, 252.0);
    } else {
        c.fill(781.0, 252.0, 280.0, 360.0, Color32::from_gray(240));
    }
    let style = Style::plain(17.0, Color32::BLACK);
    let mut y = 270.0;
    for line in g.texts.wrap(desc, style.px, 230.0) {
        // パネルの白い所（上から 155）に収まる分だけ
        if y + 18.0 > DESCRIPTION_BOTTOM {
            break;
        }
        if let Some(t) = g.texts.get(&line, &style) {
            t.draw(c, 804.0, y, 1.0, 1.0);
        }
        y += 18.0;
    }
}

/// 「パッドを叩いてください」の窓（上流 `CActConfigKeyAssign`：(509, 215)）
fn draw_hit_key_dialog(c: &Canvas, g: &mut Gfx) {
    let t = g
        .skin
        .tex_optional("Graphics/ScreenConfig hit key to assign dialog.png")
        .or_else(|| g.skin.tex("Graphics/4_hit key to assign dialog.png"));
    match t {
        Some(t) => c.draw(&t, 509.0, 215.0),
        None => {
            c.fill(509.0, 215.0, 200.0, 100.0, Color32::from_black_alpha(220));
            let style = Style::plain(18.0, Color32::WHITE);
            for (i, line) in ["パッドを叩くかキーを押す", "（Esc で取りやめ）"]
                .iter()
                .enumerate()
            {
                if let Some(t) = g.texts.get(line, &style) {
                    t.draw(c, 609.0 - t.w / 2.0, 240.0 + i as f32 * 26.0, 1.0, 1.0);
                }
            }
        }
    }
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
                        source: "Keyboard".to_owned(),
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
                    channel: 0,
                    min_velocity: 0,
                    changes: 0,
                    last: Some("ch10 note 38 vel 100 → SD".to_owned()),
                },
                DeviceRow {
                    name: "Finger Drum Pad".to_owned(),
                    id: "fgdp".to_owned(),
                    open: true,
                    error: None,
                    disabled: false,
                    preset: Preset::GeneralMidi,
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

    #[test]
    fn system_list_has_no_nx_folder() {
        let c = Config::default();
        let names: Vec<String> = items(List::System, &info(&c))
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert!(!names.iter().any(|n| n.contains("NX")));
        assert!(names.contains(&"ShowLagTime".to_owned()));
        let lag = items(List::System, &info(&c))
            .into_iter()
            .find(|i| i.id == ItemId::Value(Setting::ShowLagTime))
            .unwrap();
        assert_eq!(lag.value, "ON");
    }

    fn ready() -> ConfigView {
        ConfigView::new()
    }

    #[test]
    fn lists_start_with_return_to_menu() {
        let config = Config::default();
        for list in [
            List::System,
            List::Drums,
            List::DrumsKeys,
            List::MidiDevices,
            List::Device(0),
        ] {
            assert_eq!(items(list, &info(&config))[0].id, ItemId::Return);
        }
        let keys = items(List::DrumsKeys, &info(&config));
        let sd = keys
            .iter()
            .find(|i| i.id == ItemId::Pad(DrumLane::SD))
            .unwrap();
        // 機器ごとに並べる
        assert_eq!(sd.value, "38 40 / D");
        assert!(sd.desc.starts_with("V51: 38, 40\nKeyboard: D\n"));
    }

    #[test]
    fn integer_items_are_edited_with_up_and_down() {
        let config = Config::default();
        let list = items(List::Drums, &info(&config));
        let mut v = ready();
        // メニューの Drums から一覧へ
        v.input(ConfigInput::Down, &list);
        assert_eq!(v.list(), List::Drums);
        v.input(ConfigInput::Decide, &list);
        // ScrollSpeed を選んで値を変える状態に
        v.input(ConfigInput::Down, &list);
        assert_eq!(list[v.cursor].id, ItemId::Value(Setting::ScrollSpeed));
        assert_eq!(v.input(ConfigInput::Decide, &list).action, None);
        assert_eq!(
            v.input(ConfigInput::Up, &list).action,
            Some(ConfigAction::Nudge(Setting::ScrollSpeed, 1))
        );
        assert_eq!(
            v.input(ConfigInput::Down, &list).action,
            Some(ConfigAction::Nudge(Setting::ScrollSpeed, -1))
        );
        // Esc で値の変更をやめても一覧に残る
        v.input(ConfigInput::Cancel, &list);
        assert!(v.focus_list && !v.editing);
    }

    #[test]
    fn master_volume_is_in_system_and_starts_at_half() {
        let config = Config::default();
        let list = items(List::System, &info(&config));
        let at = list
            .iter()
            .position(|i| i.id == ItemId::Value(Setting::MasterVolume))
            .unwrap();
        assert_eq!(list[at].name, "MasterVolume");
        assert_eq!(list[at].value, "50");
        let mut v = ready();
        v.input(ConfigInput::Decide, &list);
        v.cursor = at;
        assert_eq!(v.input(ConfigInput::Decide, &list).action, None);
        assert_eq!(
            v.input(ConfigInput::Down, &list).action,
            Some(ConfigAction::Nudge(Setting::MasterVolume, -1))
        );
    }

    #[test]
    fn pads_are_assigned_by_the_next_hit() {
        let config = Config::default();
        let list = items(List::DrumsKeys, &info(&config));
        let mut v = ready();
        v.list = List::DrumsKeys;
        v.focus_list = true;
        v.cursor = list
            .iter()
            .position(|i| i.id == ItemId::Pad(DrumLane::HT))
            .unwrap();
        v.input(ConfigInput::Decide, &list);
        assert!(v.waiting_pad());
        // 待っているあいだは動かない
        assert_eq!(v.input(ConfigInput::Down, &list), Response::default());
        assert_eq!(
            v.pad_hit(1, 48).action,
            Some(ConfigAction::AssignPad(DrumLane::HT, 1, 48))
        );
        assert!(!v.waiting_pad());
        // キーでも割り当てる。演奏の操作のキーは割り当てない
        v.input(ConfigInput::Decide, &list);
        assert_eq!(v.key_hit(KeyCode::KeyR), None);
        assert_eq!(
            v.key_hit(KeyCode::KeyX).and_then(|r| r.action),
            Some(ConfigAction::AssignKey(DrumLane::HT, KeyCode::KeyX))
        );
        // Delete で全部外す
        v.input(ConfigInput::Decide, &list);
        assert_eq!(
            v.key_hit(KeyCode::Delete).and_then(|r| r.action),
            Some(ConfigAction::ClearLane(DrumLane::HT))
        );
        // 待っていなければキーは扱わない
        assert_eq!(v.key_hit(KeyCode::KeyX), None);
        // Esc で取りやめ
        v.input(ConfigInput::Decide, &list);
        v.input(ConfigInput::Cancel, &list);
        assert!(!v.waiting_pad());
        assert_eq!(v.pad_hit(0, 48).action, None);
    }

    #[test]
    fn devices_have_their_own_settings() {
        let config = Config::default();
        let info = info(&config);
        let drums = items(List::Drums, &info);
        let mut v = ready();
        v.list = List::Drums;
        v.focus_list = true;
        v.cursor = drums
            .iter()
            .position(|i| i.id == ItemId::MidiDevices)
            .unwrap();
        assert_eq!(drums[v.cursor].value, "2 台");
        v.input(ConfigInput::Decide, &drums);
        assert_eq!(v.list(), List::MidiDevices);
        let list = items(List::MidiDevices, &info);
        assert_eq!(list[1].name, "V51");
        assert_eq!(list[1].value, "Roland");
        assert!(list[1].desc.contains("最後: ch10 note 38 vel 100 → SD"));
        // 2 台目（FGDP）の設定へ
        v.input(ConfigInput::Down, &list);
        v.input(ConfigInput::Down, &list);
        v.input(ConfigInput::Decide, &list);
        assert_eq!(v.list(), List::Device(1));
        let dev = items(List::Device(1), &info);
        let names: Vec<&str> = dev.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "<< ReturnTo Menu",
                "Device",
                "Use",
                "Preset",
                "Channel",
                "MinVelocity",
                "Reset Keys"
            ]
        );
        assert_eq!(dev[3].value, "GM");
        assert_eq!(dev[4].value, "10");
        assert_eq!(dev[5].value, "8");
        // 出発点は Enter で替える
        v.cursor = 3;
        assert_eq!(
            v.input(ConfigInput::Decide, &dev).action,
            Some(ConfigAction::NextPreset(1))
        );
        // チャンネルは Enter のあと ↑↓
        v.cursor = 4;
        assert_eq!(v.input(ConfigInput::Decide, &dev).action, None);
        assert_eq!(
            v.input(ConfigInput::Up, &dev).action,
            Some(ConfigAction::NudgeChannel(1, 1))
        );
        v.input(ConfigInput::Decide, &dev);
        // Esc で機器の一覧の、その機器の行へ戻る
        v.input(ConfigInput::Cancel, &dev);
        assert_eq!(v.list(), List::MidiDevices);
        assert_eq!(v.cursor, 2);
        v.input(ConfigInput::Cancel, &list);
        assert_eq!(v.list(), List::Drums);
        // つながる機器が変わったら、機器の設定から一覧へ戻る
        v.list = List::Device(0);
        v.devices_changed();
        assert_eq!(v.list(), List::MidiDevices);
    }

    #[test]
    fn exit_and_escape_leave() {
        let config = Config::default();
        let list = items(List::System, &info(&config));
        let mut v = ready();
        v.input(ConfigInput::Up, &list);
        assert_eq!(v.menu, MENU_EXIT);
        // Exit を選んでも一覧は前のまま
        assert_eq!(v.list(), List::System);
        assert_eq!(
            v.input(ConfigInput::Decide, &list).action,
            Some(ConfigAction::Leave)
        );
        // 出ると決めたあとは動かない
        assert_eq!(v.input(ConfigInput::Down, &list), Response::default());
        let mut v = ready();
        assert_eq!(
            v.input(ConfigInput::Cancel, &list),
            Response::action(Se::Cancel, ConfigAction::Leave)
        );
    }

    #[test]
    fn cursor_wraps_around_the_list() {
        let config = Config::default();
        let list = items(List::Drums, &info(&config));
        let mut v = ready();
        v.list = List::Drums;
        v.focus_list = true;
        v.input(ConfigInput::Up, &list);
        assert_eq!(v.cursor, list.len() - 1);
        v.input(ConfigInput::Down, &list);
        assert_eq!(v.cursor, 0);
    }
}
