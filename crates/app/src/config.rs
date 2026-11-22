//! 設定。OS の設定のフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の config.toml に置く。
//! 読めないときは既定の値で始め、ファイルは壊さない（書くのは値を変えたときだけ）。

use std::path::PathBuf;

use dtx::DrumLane;
use engine::input::MidiProfile;
use engine::practice::{LoopStyle, PassRule, SET_LAPS, SET_LAPS_CHOICES, SpeedPlan, snap_speed};
use serde::{Deserialize, Serialize};

use crate::practice::PracticePrefs;
use crate::settings::{DisplayMode, Values};
use crate::stretch::Pitch;
use crate::update::UpdateMode;

/// 設定の画面で全体の音量を 1 回で変える幅（%）
pub const MASTER_VOLUME_STEP: u8 = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 0.1 の設定で選んでいた DTXManiaNX のフォルダ。読むだけで書かない
    /// （起動したときにその NX の設定を一度だけ引き継ぐ。[`Config::take_over_nx`]）
    #[serde(skip_serializing)]
    pub nx_dir: Option<PathBuf>,
    /// 曲のフォルダ
    pub song_dirs: Vec<PathBuf>,
    /// 入力の遅れの補正（ミリ秒）。叩いた時刻からこの分を引く
    pub input_offset_ms: f64,
    /// 表示の遅れの補正（ミリ秒）
    pub visual_offset_ms: f64,
    /// 1 ミリ秒あたりに落ちるピクセル
    pub scroll_px_per_ms: f32,
    /// 叩いたときに譜面の音を鳴らす（モジュールの音を使うなら切る）
    pub drum_sounds: bool,
    /// 全体の音量（0〜100 %、上流 `MasterVolume`）。書いていなければ 50。
    /// NX の Config.ini からは引き継がない（NX の既定の 100 だと大きすぎる）
    pub master_volume: u8,
    /// 最後に選んだ難易度（0〜4）
    pub level: usize,
    /// 練習の周のつなぎ方（"connect" か "restart"）
    pub practice_style: String,
    /// 練習の苦手の見方（"easy"、"normal"、"strict"。同じ拍を 3・2・1 回落としたら苦手）
    pub practice_rule: String,
    /// 練習の速度を段階アップで上げる（false なら固定）
    pub practice_ladder: bool,
    /// 練習の速度（%）。段階アップなら始めの速度
    pub practice_speed: u32,
    /// 段階アップの目標の速度（%）
    pub practice_target: u32,
    /// 遅くするときの音程（"keep" は保つ、"tape" は音程ごと遅く）
    pub practice_pitch: String,
    /// 練習でメトロノームを鳴らす（区間の拍ごとに小さく鳴る）
    pub practice_metronome: bool,
    /// 練習の 1 セットの周の数
    pub practice_set_laps: u32,
    /// ゲージが尽きたら途中で終わる（STAGE FAILED）。書いていなければ ON（上流の既定）
    pub stage_failed: Option<bool>,
    /// 判定の下のずれの数字（0 は出さない、1 は出す、2 は PERFECT 以外だけ。上流 `ShowLagTime`）
    pub show_lag_time: u8,
    /// ずれの数字の色（0 は TYPE-A：早いを青・遅いを赤、1 は TYPE-B：早いを赤・遅いを青。上流 `ShowLagTimeColor`）
    pub show_lag_time_color: u8,
    /// 結果の画面に早かった数と遅かった数を出す（上流 `ShowLagHitCount`）
    pub show_lag_hit_count: bool,
    /// 前の版（0.2 まで）の、機器を区別しないパッドの割り当て。まだ設定の無い MIDI の機器に重ね、
    /// その機器の設定を作るときに写す（[`crate::inputs`]）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pad_notes: Vec<PadNote>,
    /// MIDI の入力機器ごとの設定
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub midi_devices: Vec<MidiDevice>,
    /// PC のキーボードの割り当てのうち、既定から変えたもの（順に重ねる）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyBind>,
    /// 0.5.0 までの自動更新の ON/OFF。読むだけで書かない（OFF にしていたら `update` を「確かめない」にする）
    #[serde(skip_serializing)]
    pub auto_update: Option<bool>,
    /// 新しい版の扱い（"notify"、"auto"、"off"。[`crate::update::UpdateMode`]）。書いていなければ知らせる
    #[serde(skip_serializing_if = "String::is_empty")]
    pub update: String,
    /// 最後に起動した版（更新のあとに「新しい版になりました」を出す）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_version: String,
    /// 最後に使った全画面の出し方（"borderless" か "fullscreen"）。Alt+Enter と F11 でウィンドウから戻る先
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_full_mode: String,
    /// 設定の表（[`crate::settings`]）の項目の値（config.toml の `[settings]`）。書いていないものは既定
    #[serde(skip_serializing_if = "Values::is_empty")]
    pub settings: Values,
}

/// パッドの割り当て 1 つ（ノート番号 → レーン）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadNote {
    pub note: u8,
    /// レーンの名前（HH、SD など）。[`UNASSIGNED`] なら割り当てを外す
    pub lane: String,
}

/// 割り当てを外したことを表すレーンの名前
pub const UNASSIGNED: &str = "-";

/// MIDI の入力機器（ポート）1 つの設定。ポートの名前で探し、同じ名前が何台もあれば OS の ID で見分ける
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiDevice {
    /// ポートの名前（同じ機種の 2 台目に OS が付ける印は除く。[`crate::midi::base_name`]）
    pub name: String,
    /// OS のポートの ID（Windows はデバイスのパス、macOS は CoreMIDI の番号、Linux は client:port）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// 割り当ての出発点（"roland"、"gm"、"empty"）
    pub preset: String,
    /// 受けるチャンネル（1〜16）。0 ならすべて
    #[serde(skip_serializing_if = "is_zero")]
    pub channel: u8,
    /// これより弱い打撃は捨てる（隣のパッドにつられて鳴る弱い音を除く）。0 なら捨てない
    #[serde(skip_serializing_if = "is_zero")]
    pub min_velocity: u8,
    /// 開かない（ほかのアプリに使わせるときなど）
    #[serde(skip_serializing_if = "is_false")]
    pub disabled: bool,
    /// 出発点から変えた割り当て（順に重ねる）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<PadNote>,
}

/// キーボードの割り当て 1 つ（キー → レーン）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBind {
    /// キーの名前（winit の `KeyCode` の名前。KeyZ、Digit1 など）
    pub key: String,
    /// レーンの名前。[`UNASSIGNED`] なら割り当てを外す
    pub lane: String,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nx_dir: None,
            song_dirs: Vec::new(),
            input_offset_ms: 0.0,
            visual_offset_ms: 0.0,
            scroll_px_per_ms: 0.6,
            drum_sounds: true,
            master_volume: crate::audio::DEFAULT_MASTER_VOLUME,
            level: 2,
            practice_style: LoopStyle::default().name().to_owned(),
            practice_rule: PassRule::default().name().to_owned(),
            practice_ladder: true,
            practice_speed: SpeedPlan::DEFAULT_START,
            practice_target: SpeedPlan::DEFAULT_TARGET,
            practice_pitch: Pitch::default().name().to_owned(),
            practice_metronome: true,
            practice_set_laps: SET_LAPS,
            stage_failed: None,
            show_lag_time: 1,
            show_lag_time_color: 0,
            show_lag_hit_count: false,
            pad_notes: Vec::new(),
            midi_devices: Vec::new(),
            keys: Vec::new(),
            auto_update: None,
            update: String::new(),
            last_version: String::new(),
            last_full_mode: String::new(),
            settings: Values::default(),
        }
    }
}

impl Config {
    /// 設定のフォルダ（Windows は %APPDATA%\DTXmaniaNXT）
    pub fn dir() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("DTXmaniaNXT"))
    }

    pub fn path() -> Option<PathBuf> {
        Some(Self::dir()?.join("config.toml"))
    }

    /// 0.1 で DTXManiaNX のフォルダを選んでいたら、その NX のパッドの割り当てと表示の設定を一度だけ引き継ぐ
    /// （曲のフォルダは 0.1 が `song_dirs` に足してある）。0.1 が設定のフォルダの `nx` へ写した NX のスキンと
    /// Config.ini は、もう使わないので消す。引き継いだら true（保存は呼ぶ側で）
    pub fn take_over_nx(&mut self) -> bool {
        let Some(nx_dir) = self.nx_dir.take() else {
            return false;
        };
        let copy = Self::dir()
            .map(|d| d.join("nx"))
            .filter(|d| nx::is_install(d));
        let install = copy
            .as_deref()
            .and_then(nx::Install::open)
            .or_else(|| nx::Install::open(&nx_dir));
        if let Some(i) = &install {
            self.adopt_nx(&i.config);
            log::info!(
                "took over the settings of DTXManiaNX ({})",
                i.root.display()
            );
        }
        if let Some(c) = copy
            && let Err(e) = std::fs::remove_dir_all(&c)
        {
            log::warn!("cannot remove {}: {e}", c.display());
        }
        true
    }

    /// NX の Config.ini のパッドの割り当てと表示の設定を引き継ぐ。
    /// パッドは Roland の既定と違うものだけ足し、この設定の画面で変えた割り当てをそのまま優先する
    fn adopt_nx(&mut self, nx: &nx::NxConfig) {
        let roland = MidiProfile::roland_vdrums().lane_notes();
        let mut pads: Vec<PadNote> = nx
            .midi_notes()
            .into_iter()
            .filter(|&(note, lane)| {
                lane != DrumLane::LBD
                    && !self.pad_notes.iter().any(|p| p.note == note)
                    && !roland.get(&lane).is_some_and(|n| n.contains(&note))
            })
            .map(|(note, lane)| PadNote {
                note,
                lane: lane.name().to_owned(),
            })
            .collect();
        pads.append(&mut self.pad_notes);
        self.pad_notes = pads;
        if self.stage_failed.is_none() {
            self.stage_failed = nx.stage_failed;
        }
        // NX で書いていなければ上流の既定（出さない）
        self.show_lag_time = nx.show_lag_time.unwrap_or(0);
        self.show_lag_time_color = u8::from(nx.lag_color_swapped);
        self.show_lag_hit_count = nx.show_lag_hit_count;
    }

    /// 全体の音量（%）。ファイルに 100 より大きく書いてあれば 100
    pub fn master_volume(&self) -> u8 {
        self.master_volume.min(100)
    }

    /// 全体の音量を決める（0〜100 に収め、1 段 5 % の刻みにそろえる）
    pub fn set_master_volume(&mut self, v: i64) {
        let step = i64::from(MASTER_VOLUME_STEP);
        self.master_volume = ((v.clamp(0, 100) + step / 2) / step * step).min(100) as u8;
    }

    /// STAGE FAILED にする（書いていなければ ON）
    pub fn stage_failed(&self) -> bool {
        self.stage_failed.unwrap_or(true)
    }

    /// Alt+Enter と F11 でウィンドウから切り替える先（最後に使った方。初めは全画面）
    pub fn full_mode(&self) -> DisplayMode {
        DisplayMode::from_name(&self.last_full_mode)
            .filter(|m| *m != DisplayMode::Window)
            .unwrap_or(DisplayMode::Fullscreen)
    }

    /// 全画面の出し方を使ったら覚える。覚え直したら true
    pub fn remember_full_mode(&mut self, mode: DisplayMode) -> bool {
        if mode == DisplayMode::Window || self.last_full_mode == mode.name() {
            return false;
        }
        mode.name().clone_into(&mut self.last_full_mode);
        true
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(mut c) => {
                    c.upgrade_update();
                    // 前の版のキーを写したら書き直す
                    if c.settings.upgrade() {
                        c.save();
                    }
                    c
                }
                Err(e) => {
                    log::warn!("{}: {e}", path.display());
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // 途中で落ちても前のファイルが残るように、別の名前に書いてから置き換える
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("cannot save {}: {e:#}", path.display());
        }
    }

    /// 0.5.0 までの自動更新を OFF にしていたら、新しい版の扱いを「確かめない」にする（次に書くときに残る）
    fn upgrade_update(&mut self) {
        if self.update.is_empty() && self.auto_update == Some(false) {
            self.update = UpdateMode::Off.name().to_owned();
        }
    }

    /// 新しい版の扱い（読めない値なら既定の「知らせる」）
    pub fn update_mode(&self) -> UpdateMode {
        UpdateMode::from_name(&self.update).unwrap_or_default()
    }

    pub fn set_update_mode(&mut self, mode: UpdateMode) {
        mode.name().clone_into(&mut self.update);
    }

    /// 起動した版を覚える。前と違う版なら、前の版（分かれば）を返す（呼ぶ側が知らせて保存する）。
    /// 初めての起動なら何も返さない。0.5.0 までの設定（前の版を覚えていない）なら空の文字を返す
    pub fn remember_version(&mut self, current: &str) -> Option<String> {
        if self.last_version == current {
            return None;
        }
        let previous = std::mem::replace(&mut self.last_version, current.to_owned());
        (!previous.is_empty() || self.auto_update.is_some()).then_some(previous)
    }

    /// 書いてあるファイルが読める（または無い）ときだけ保存する。起動しただけで書くときに使い、
    /// 読めないファイルを既定の値で上書きしない
    pub fn save_unless_broken(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Ok(text) = std::fs::read_to_string(&path)
            && toml::from_str::<Self>(&text).is_err()
        {
            return;
        }
        self.save();
    }

    /// 練習の周のつなぎ方（読めない値なら既定）
    pub fn loop_style(&self) -> LoopStyle {
        LoopStyle::from_name(&self.practice_style).unwrap_or_default()
    }

    /// 練習の苦手の見方（読めない値なら既定）
    pub fn pass_rule(&self) -> PassRule {
        PassRule::from_name(&self.practice_rule).unwrap_or_default()
    }

    /// 練習の速度の決め方
    pub fn speed_plan(&self) -> SpeedPlan {
        let speed = snap_speed(i64::from(self.practice_speed));
        if self.practice_ladder {
            SpeedPlan::Ladder {
                start: speed,
                target: snap_speed(i64::from(self.practice_target)),
            }
        } else {
            SpeedPlan::Fixed(speed)
        }
    }

    /// 遅くするときの音程（読めない値なら既定）
    pub fn pitch(&self) -> Pitch {
        Pitch::from_name(&self.practice_pitch).unwrap_or_default()
    }

    /// 練習の決めごと（区間のほか）
    pub fn practice_prefs(&self) -> PracticePrefs {
        PracticePrefs {
            style: self.loop_style(),
            rule: self.pass_rule(),
            speed: self.speed_plan(),
            pitch: self.pitch(),
            metronome: self.practice_metronome,
            set_laps: self.set_laps(),
        }
    }

    /// 練習の決めごとを残す（次の練習でも使う）
    pub fn set_practice_prefs(&mut self, p: PracticePrefs) {
        self.practice_style = p.style.name().to_owned();
        self.practice_rule = p.rule.name().to_owned();
        match p.speed {
            SpeedPlan::Fixed(s) => {
                self.practice_ladder = false;
                self.practice_speed = s;
            }
            SpeedPlan::Ladder { start, target } => {
                self.practice_ladder = true;
                self.practice_speed = start;
                self.practice_target = target;
            }
        }
        self.practice_pitch = p.pitch.name().to_owned();
        self.practice_metronome = p.metronome;
        self.practice_set_laps = p.set_laps;
    }

    /// 練習の 1 セットの周の数（選べる数のうち一番近いもの）
    pub fn set_laps(&self) -> u32 {
        SET_LAPS_CHOICES
            .into_iter()
            .min_by_key(|&n| n.abs_diff(self.practice_set_laps))
            .unwrap_or(SET_LAPS)
    }

    /// 前の版の、機器を区別しないパッドの割り当て（読めないレーンの名前は飛ばす）
    pub fn pad_overrides(&self) -> Vec<(u8, Option<DrumLane>)> {
        note_overrides(&self.pad_notes)
    }

    /// 設定の画面で変えた割り当てがあるか
    pub fn has_custom_assignments(&self) -> bool {
        !self.pad_notes.is_empty()
            || !self.keys.is_empty()
            || self.midi_devices.iter().any(|d| !d.notes.is_empty())
    }

    /// 曲のフォルダを足す（同じものは足さない）。足したら true
    pub fn add_song_dir(&mut self, dir: PathBuf) -> bool {
        if self.song_dirs.contains(&dir) {
            return false;
        }
        self.song_dirs.push(dir);
        true
    }
}

/// 割り当ての並び（ノート → レーン、`None` は外す）。読めないレーンの名前は飛ばす
pub fn note_overrides(notes: &[PadNote]) -> Vec<(u8, Option<DrumLane>)> {
    notes
        .iter()
        .filter_map(|p| {
            if p.lane == UNASSIGNED {
                Some((p.note, None))
            } else {
                Some((p.note, Some(DrumLane::from_name(&p.lane)?)))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_use_defaults() {
        let c: Config = toml::from_str("input_offset_ms = 12.0\n").unwrap();
        assert_eq!(c.input_offset_ms, 12.0);
        assert_eq!(c.scroll_px_per_ms, 0.6);
        assert!(c.drum_sounds);
        // 前の版の設定（音量の項目が無い）でも 50 % で鳴らす
        assert_eq!(c.master_volume(), 50);
        assert_eq!(c.loop_style(), LoopStyle::Connect);
        assert_eq!(c.pass_rule(), PassRule::Normal);
        assert_eq!(c.speed_plan(), SpeedPlan::default());
        assert_eq!(c.pitch(), Pitch::Keep);
    }

    #[test]
    fn practice_prefs_round_trip() {
        let mut c = Config::default();
        let p = PracticePrefs {
            style: LoopStyle::Restart,
            rule: PassRule::Strict,
            speed: SpeedPlan::Fixed(85),
            pitch: Pitch::Tape,
            metronome: false,
            set_laps: 6,
        };
        c.set_practice_prefs(p);
        assert_eq!(c.practice_prefs(), p);
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.practice_prefs(), p);
        // 段階アップに戻しても目標は前の値のまま
        c.practice_ladder = true;
        assert_eq!(
            c.speed_plan(),
            SpeedPlan::Ladder {
                start: 85,
                target: 100
            }
        );
        // 刻みに合わない速度は直す
        c.practice_speed = 7;
        assert_eq!(c.speed_plan().first_speed(), 25);
    }

    #[test]
    fn unknown_practice_names_fall_back() {
        let c: Config =
            toml::from_str("practice_style = \"loop\"\npractice_rule = \"strict\"\n").unwrap();
        assert_eq!(c.loop_style(), LoopStyle::Connect);
        assert_eq!(c.pass_rule(), PassRule::Strict);
    }

    #[test]
    fn pad_notes_skip_unknown_lanes() {
        let c: Config = toml::from_str(
            "[[pad_notes]]\nnote = 1\nlane = \"XX\"\n[[pad_notes]]\nnote = 2\nlane = \"-\"\n",
        )
        .unwrap();
        assert_eq!(c.pad_overrides(), vec![(2, None)]);
    }

    #[test]
    fn devices_and_keys_are_saved() {
        let text = "\
[[midi_devices]]
name = \"Finger Drum Pad\"
id = \"usb-1\"
preset = \"empty\"
channel = 10

[[midi_devices.notes]]
note = 3
lane = \"FT\"

[[midi_devices]]
name = \"V51\"
preset = \"roland\"
disabled = true

[[keys]]
key = \"KeyX\"
lane = \"LP\"
";
        let c: Config = toml::from_str(text).unwrap();
        assert_eq!(c.midi_devices.len(), 2);
        assert_eq!(c.midi_devices[0].channel, 10);
        assert_eq!(
            note_overrides(&c.midi_devices[0].notes),
            vec![(3, Some(DrumLane::FT))]
        );
        assert!(c.midi_devices[1].disabled);
        assert!(c.midi_devices[1].id.is_empty());
        assert!(c.has_custom_assignments());
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        // 書いていない項目は既定（すべてのチャンネル、使う）で、書き出しにも出さない
        let out = toml::to_string_pretty(&c).unwrap();
        assert_eq!(out.matches("channel").count(), 1);
        assert_eq!(out.matches("disabled").count(), 1);
    }

    #[test]
    fn takes_over_nx_settings_once() {
        // 0.1 の設定：NX のフォルダは読むが、書き戻さない
        let mut c: Config =
            toml::from_str("nx_dir = \"/nx\"\n[[pad_notes]]\nnote = 50\nlane = \"FT\"\n").unwrap();
        assert_eq!(c.nx_dir.as_deref(), Some(std::path::Path::new("/nx")));
        assert!(!toml::to_string_pretty(&c).unwrap().contains("nx_dir"));
        let nx = nx::NxConfig::parse(
            "[System]\nStageFailed=0\nShowLagTime=2\nShowLagTimeColor=1\nShowLagHitCount=1\n\
             [DrumsKeyAssign]\nSD=M038,M040,M099\nHT=M050\nLBD=M033\n",
        );
        c.adopt_nx(&nx);
        // 38 と 40 は Roland の既定と同じなので足さない。50 はここで変えたほうを残す。LBD は扱わない
        assert_eq!(
            c.pad_overrides(),
            vec![(99, Some(DrumLane::SD)), (50, Some(DrumLane::FT))]
        );
        assert!(!c.stage_failed());
        assert_eq!(c.show_lag_time, 2);
        assert_eq!(c.show_lag_time_color, 1);
        assert!(c.show_lag_hit_count);
        // ここで決めた STAGE FAILED は NX より優先する
        let mut c = Config {
            stage_failed: Some(true),
            ..Config::default()
        };
        c.adopt_nx(&nx);
        assert!(c.stage_failed());
    }

    #[test]
    fn master_volume_defaults_to_half_and_moves_in_steps() {
        let mut c = Config::default();
        assert_eq!(c.master_volume(), 50);
        c.set_master_volume(35);
        assert_eq!(c.master_volume(), 35);
        c.set_master_volume(-100);
        assert_eq!(c.master_volume(), 0);
        c.set_master_volume(300);
        assert_eq!(c.master_volume(), 100);
        // ファイルに書いた半端な値から 1 段動かすと、刻みに合わせて直す
        let mut c: Config = toml::from_str("master_volume = 42\n").unwrap();
        c.set_master_volume(i64::from(c.master_volume()) + 5);
        assert_eq!(c.master_volume(), 45);
        let mut c: Config = toml::from_str("master_volume = 200\n").unwrap();
        assert_eq!(c.master_volume(), 100);
        c.set_master_volume(i64::from(c.master_volume()) - 5);
        assert_eq!(c.master_volume(), 95);
    }

    #[test]
    fn master_volume_is_kept_for_the_next_version() {
        // 一度下げた音量は保存して読み直しても同じ（版を上げても設定のファイルは同じ所）
        let mut c = Config::default();
        c.set_master_volume(20);
        let text = toml::to_string_pretty(&c).unwrap();
        assert!(text.contains("master_volume = 20"));
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.master_volume(), 20);
        // NX の設定を引き継いでも音量は変わらない
        let mut c = back;
        c.adopt_nx(&nx::NxConfig::parse("[System]\nMasterVolume=100\n"));
        assert_eq!(c.master_volume(), 20);
    }

    #[test]
    fn f11_returns_to_the_last_full_mode() {
        let mut c = Config::default();
        assert_eq!(c.full_mode(), DisplayMode::Fullscreen);
        assert!(!c.remember_full_mode(DisplayMode::Window));
        assert!(c.remember_full_mode(DisplayMode::Borderless));
        assert!(!c.remember_full_mode(DisplayMode::Borderless));
        assert_eq!(c.full_mode(), DisplayMode::Borderless);
        let back: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.full_mode(), DisplayMode::Borderless);
        // 読めない名前やウィンドウは全画面に
        c.last_full_mode = "window".into();
        assert_eq!(c.full_mode(), DisplayMode::Fullscreen);
    }

    #[test]
    fn round_trip() {
        let mut c = Config::default();
        c.song_dirs.push(PathBuf::from("曲"));
        c.level = 4;
        c.master_volume = 70;
        c.stage_failed = Some(false);
        c.show_lag_time = 2;
        c.show_lag_time_color = 1;
        c.show_lag_hit_count = true;
        c.pad_notes.push(PadNote {
            note: 60,
            lane: "HT".to_owned(),
        });
        c.keys.push(KeyBind {
            key: "KeyX".to_owned(),
            lane: "LP".to_owned(),
        });
        c.midi_devices.push(MidiDevice {
            name: "TD-27".to_owned(),
            preset: "roland".to_owned(),
            ..MidiDevice::default()
        });
        c.set_update_mode(UpdateMode::Auto);
        c.last_version = "0.5.0".to_owned();
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn auto_update_off_becomes_no_checks() {
        let mut c: Config = toml::from_str("auto_update = false").unwrap();
        c.upgrade_update();
        assert_eq!(c.update_mode(), UpdateMode::Off);
        // 書くのは新しいキーだけ
        let text = toml::to_string_pretty(&c).unwrap();
        assert!(text.contains("update = \"off\""), "{text}");
        assert!(!text.contains("auto_update"), "{text}");
        // ON のままだった人は、勝手に入れ替えない「知らせる」になる
        let mut c: Config = toml::from_str("auto_update = true").unwrap();
        c.upgrade_update();
        assert_eq!(c.update_mode(), UpdateMode::Notify);
        // 読めない値は既定
        let c: Config = toml::from_str("update = \"later\"").unwrap();
        assert_eq!(c.update_mode(), UpdateMode::Notify);
    }

    #[test]
    fn a_new_version_is_noticed_once() {
        // 初めての起動
        let mut c = Config::default();
        assert_eq!(c.remember_version("0.5.1"), None);
        assert_eq!(c.remember_version("0.5.1"), None);
        assert_eq!(c.remember_version("0.5.2"), Some("0.5.1".to_owned()));
        assert_eq!(c.last_version, "0.5.2");
        // 0.5.0 までの設定（自動更新のキーはあるが、前の版を覚えていない）
        let mut c: Config = toml::from_str("auto_update = true").unwrap();
        assert_eq!(c.remember_version("0.5.1"), Some(String::new()));
    }
}
