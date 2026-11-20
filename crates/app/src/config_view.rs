//! 設定の画面（上流 `CStageConfig` と `CActConfigList`）。
//! スキンの 4_ の画像で、左のメニュー（System / Drums / Exit）と、右の項目の一覧、説明のパネルを出す。
//! 項目は NXT のもの（NX のフォルダ、曲のフォルダ、入力と表示の補正、スクロールの速さ、パッドの割り当てなど）。
//!
//! 操作は上流と同じ。↑↓（HT / LT）で選び、Enter（CY / RD）で決め、Esc（LC）で戻る。
//! 数の項目は Enter で値を変える状態になり、↑↓ で値を変えて、もう一度 Enter で戻る。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use dtx::DrumLane;
use egui::Color32;
use library::Progress;

use crate::config::Config;
use crate::gfx::{Canvas, Gfx, src};
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

/// 白いフェードの長さ（上流 `CActFIFOWhite`）
const FADE: Duration = Duration::from_millis(500);

/// 説明のパネルを出すまでの間（上流 `ctDisplayWait`：0〜350 を 1 ミリ秒ずつ）
const DESCRIPTION_WAIT: Duration = Duration::from_millis(350);

/// 説明の文字を置ける下の端（4_Description Panel.png の白い所）
const DESCRIPTION_BOTTOM: f32 = 252.0 + 155.0;

/// 補正の値の範囲（ミリ秒）
pub const OFFSET_RANGE: f64 = 200.0;

/// 画面から App に頼むこと
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfigAction {
    ChooseNx,
    AddSongDir,
    RemoveSongDir(usize),
    Rescan,
    ReconnectMidi,
    /// 数の値を 1 段変える
    Nudge(Setting, i32),
    /// ON と OFF などを切り替える
    Toggle(Setting),
    /// ノート番号をレーンに割り当てる
    AssignPad(DrumLane, u8),
    /// このアプリで変えたパッドの割り当てを消す
    ResetPads,
    /// 設定の画面を出る（フェードを終えた）
    Leave,
}

/// 値を変えられる設定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    InputAdjust,
    VisualAdjust,
    ScrollSpeed,
    StageFailed,
    HitSound,
}

/// 一覧の項目
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemId {
    /// `<< ReturnTo Menu`
    Return,
    NxDir,
    AddSongDir,
    SongDir(usize),
    Rescan,
    Midi,
    Value(Setting),
    /// パッドの割り当ての一覧へ
    DrumsKeys,
    Pad(DrumLane),
    ResetPads,
}

impl ItemId {
    /// Enter で値を変える状態になる数の項目
    fn is_integer(self) -> bool {
        matches!(
            self,
            ItemId::Value(Setting::InputAdjust | Setting::VisualAdjust | Setting::ScrollSpeed)
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
}

/// 項目を作るのに要るもの
pub struct ConfigInfo<'a> {
    pub config: &'a Config,
    /// 使っている DTXManiaNX のフォルダ
    pub nx_root: Option<&'a Path>,
    /// NX の Config.ini の STAGE FAILED
    pub nx_stage_failed: Option<bool>,
    /// 今の割り当て（レーン → ノート番号）
    pub lanes: BTreeMap<DrumLane, Vec<u8>>,
    pub midi_ports: &'a [String],
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

/// 一覧の項目
pub fn items(list: List, info: &ConfigInfo<'_>) -> Vec<Item> {
    let c = info.config;
    let mut v = vec![return_item()];
    match list {
        List::System => {
            v.push(item(
                ItemId::NxDir,
                "NX Folder",
                info.nx_root.map_or("未設定".to_owned(), folder_name),
                format!(
                    "DTXManiaNX のフォルダです。画像と音、曲のフォルダ、パッドの割り当てをここから読みます。\n{}",
                    info.nx_root
                        .map_or("未設定".to_owned(), |p| p.display().to_string())
                ),
            ));
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
            let stage_failed = match c.stage_failed {
                Some(b) => on_off(b),
                None => format!("NX ({})", on_off(info.nx_stage_failed.unwrap_or(true))),
            };
            v.push(item(
                ItemId::Value(Setting::StageFailed),
                "StageFailed",
                stage_failed,
                "ON にするとゲージがなくなった時に STAGE FAILED となり演奏が中断されます。\nEnter で ON → OFF → NX の設定に従う。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::Midi,
                "MIDI Input",
                if info.midi_ports.is_empty() {
                    "なし".to_owned()
                } else {
                    format!("{} 個", info.midi_ports.len())
                },
                format!(
                    "MIDI の入力: {}\nEnter でつなぎ直します（電子ドラムを後からつないだとき）。",
                    if info.midi_ports.is_empty() {
                        "なし".to_owned()
                    } else {
                        info.midi_ports.join("、")
                    }
                ),
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
                "叩いたときに譜面の音を鳴らします。電子ドラムの音源の音を使うなら OFF にします。演奏中は M。"
                    .to_owned(),
            ));
            v.push(item(
                ItemId::DrumsKeys,
                "Drums Keys",
                String::new(),
                "ドラムのパッドの割り当て（MIDI のノート番号）を設定します。".to_owned(),
            ));
        }
        List::DrumsKeys => {
            for (lane, name) in PADS {
                let notes = info
                    .lanes
                    .get(&lane)
                    .filter(|n| !n.is_empty())
                    .map_or("-".to_owned(), |n| {
                        n.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
                    });
                let key = crate::keyboard_key(lane)
                    .map_or(String::new(), |k| format!("キーボードでは {k}。"));
                v.push(item(
                    ItemId::Pad(lane),
                    name,
                    notes.clone(),
                    format!(
                        "{name} に割り当てる MIDI のノート番号です。\nEnter を押してからパッドを叩くと、そのノートをここに割り当てます。{key}"
                    ),
                ));
            }
            let changed = c.pad_notes.len();
            v.push(item(
                ItemId::ResetPads,
                "Reset Keys",
                if changed == 0 {
                    String::new()
                } else {
                    format!("{changed} 個")
                },
                "ここで変えた割り当てを消して、Roland の既定と NX の Config.ini の割り当てに戻します。"
                    .to_owned(),
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
    /// 出ていくフェードの始まり
    leaving: Option<Instant>,
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
            leaving: None,
        }
    }

    pub fn list(&self) -> List {
        self.list
    }

    /// パッドを叩くのを待っている
    pub fn waiting_pad(&self) -> bool {
        self.waiting.is_some()
    }

    /// 待っているパッドにノートを割り当てる
    pub fn pad_hit(&mut self, note: u8) -> Response {
        match self.waiting.take() {
            Some(lane) => Response::action(Se::Decide, ConfigAction::AssignPad(lane, note)),
            None => Response::default(),
        }
    }

    /// 出ていくフェードを終えたら `Leave`
    pub fn poll(&mut self) -> Option<ConfigAction> {
        self.leaving
            .filter(|at| at.elapsed() >= FADE)
            .map(|_| ConfigAction::Leave)
    }

    fn ready(&self) -> bool {
        self.leaving.is_none() && self.entered.elapsed() >= FADE
    }

    fn leave(&mut self) {
        self.leaving = Some(Instant::now());
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

    pub fn input(&mut self, input: ConfigInput, items: &[Item]) -> Response {
        if !self.ready() {
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
                    if let ItemId::Value(s) = it.id {
                        return Response::action(Se::Move, ConfigAction::Nudge(s, -d as i32));
                    }
                    return Response::default();
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
                        if self.list == List::DrumsKeys {
                            self.set_list(List::Drums);
                        } else {
                            self.focus_list = false;
                        }
                        self.moved();
                        return Response::se(Se::Decide);
                    }
                    ItemId::DrumsKeys => {
                        self.set_list(List::DrumsKeys);
                        self.moved();
                        return Response::se(Se::Decide);
                    }
                    ItemId::Pad(lane) => {
                        self.waiting = Some(lane);
                        return Response::se(Se::Decide);
                    }
                    ItemId::NxDir => ConfigAction::ChooseNx,
                    ItemId::AddSongDir => ConfigAction::AddSongDir,
                    ItemId::SongDir(i) => ConfigAction::RemoveSongDir(i),
                    ItemId::Rescan => ConfigAction::Rescan,
                    ItemId::Midi => ConfigAction::ReconnectMidi,
                    ItemId::Value(s) => ConfigAction::Toggle(s),
                    ItemId::ResetPads => ConfigAction::ResetPads,
                };
                Response::action(Se::Decide, action)
            }
            ConfigInput::Cancel => {
                if self.editing {
                    self.editing = false;
                } else if self.list == List::DrumsKeys {
                    self.set_list(List::Drums);
                } else {
                    self.focus_list = false;
                }
                self.moved();
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
                    self.leave();
                } else {
                    self.focus_list = true;
                    self.moved();
                }
                Response::se(Se::Decide)
            }
            ConfigInput::Cancel => {
                self.leave();
                Response::se(Se::Cancel)
            }
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
        // 白いフェード
        let a = match self.leaving {
            Some(at) => (at.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0),
            None => 1.0 - (self.entered.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0),
        };
        if a > 0.0 {
            c.cover(Color32::from_white_alpha((a * 255.0) as u8));
        }
    }

    /// 左のメニュー（上流：4_menu panel.png に、4_menu cursor.png を 170 の幅に伸ばして重ねる）
    fn draw_menu(&self, c: &Canvas, g: &mut Gfx) {
        if let Some(t) = g.skin.tex("Graphics/4_menu panel.png") {
            // 上流は 5 行ぶんの枠。3 行ぶんに縮め、下の縁は残す
            let h = 12.0 + MENU.len() as f32 * 32.0;
            let edge = 8.0f32.min(t.h);
            c.part(&t, 245.0, 140.0, src(0.0, 0.0, t.w, h - edge));
            c.part(&t, 245.0, 140.0 + h - edge, src(0.0, t.h - edge, t.w, edge));
        }
        if let Some(t) = g.skin.tex("Graphics/4_menu cursor.png") {
            let alpha = if self.focus_list { 0.5 } else { 1.0 };
            let (x, y, w) = (250.0, 146.0 + self.menu as f32 * 32.0, 170.0);
            let h = t.h.min(32.0);
            let part = |x: f32, s| {
                c.draw_opt(
                    &t,
                    x,
                    y,
                    crate::gfx::Opt {
                        src: Some(s),
                        alpha,
                        ..Default::default()
                    },
                );
            };
            part(x, src(0.0, 0.0, 16.0, h));
            part(x + w - 16.0, src(16.0, 0.0, 16.0, h));
            let mut left = w - 32.0;
            let mut px = x + 16.0;
            while left > 0.0 {
                let pw = left.min(16.0);
                part(px, src(8.0, 0.0, pw, h));
                px += pw;
                left -= pw;
            }
        }
        for (i, name) in MENU.iter().enumerate() {
            let style = if i == self.menu {
                Style {
                    gradient: Some(Color32::from_rgb(255, 69, 0)),
                    ..Style::edged(24.0, Color32::from_rgb(255, 255, 0), Color32::BLACK, 2.0)
                }
            } else {
                Style::edged(24.0, Color32::WHITE, Color32::BLACK, 2.0)
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
            for (i, line) in ["パッドを叩いてください", "（Esc で取りやめ）"]
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
            nx_root: None,
            nx_stage_failed: None,
            lanes: BTreeMap::from([(DrumLane::SD, vec![38, 40])]),
            midi_ports: &[],
            scanning: None,
            song_count: 3,
        }
    }

    fn ready() -> ConfigView {
        let mut v = ConfigView::new();
        v.entered -= FADE;
        v
    }

    #[test]
    fn lists_start_with_return_to_menu() {
        let config = Config::default();
        for list in [List::System, List::Drums, List::DrumsKeys] {
            assert_eq!(items(list, &info(&config))[0].id, ItemId::Return);
        }
        let keys = items(List::DrumsKeys, &info(&config));
        let sd = keys
            .iter()
            .find(|i| i.id == ItemId::Pad(DrumLane::SD))
            .unwrap();
        assert_eq!(sd.value, "38, 40");
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
            v.pad_hit(48).action,
            Some(ConfigAction::AssignPad(DrumLane::HT, 48))
        );
        assert!(!v.waiting_pad());
        // Esc で取りやめ
        v.input(ConfigInput::Decide, &list);
        v.input(ConfigInput::Cancel, &list);
        assert!(!v.waiting_pad());
        assert_eq!(v.pad_hit(48).action, None);
    }

    #[test]
    fn exit_and_escape_leave_after_the_fade() {
        let config = Config::default();
        let list = items(List::System, &info(&config));
        let mut v = ready();
        v.input(ConfigInput::Up, &list);
        assert_eq!(v.menu, MENU_EXIT);
        // Exit を選んでも一覧は前のまま
        assert_eq!(v.list(), List::System);
        v.input(ConfigInput::Decide, &list);
        assert_eq!(v.poll(), None);
        v.leaving = v.leaving.map(|t| t - FADE);
        assert_eq!(v.poll(), Some(ConfigAction::Leave));
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
