//! 入力機器ごとの割り当て。MIDI のポート（機器）と PC のキーボードを、それぞれ別の入力として扱う。
//!
//! - MIDI の機器はポートの名前で設定を探し、同じ名前が何台もあれば OS の ID で見分ける。
//!   設定が無ければ、名前から出発点（Roland か GM）を選び、前の版の割り当て（`pad_notes`）を重ねる
//! - 割り当ては機器ごと。同じノート番号・同じチャンネルで送る機器どうしも混ざらない。
//!   チャンネルは既定ではすべて受け、機器ごとに 1 つに絞れる
//! - どのレーンも、いくつの機器やキーからでも叩ける（LP だけ別の機器やキーボード、など）

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use dtx::DrumLane;
use engine::input::{MidiMessage, MidiProfile, Normalizer, Preset};
use winit::keyboard::KeyCode;

use crate::config::{Config, KeyBind, MidiDevice, PadNote, UNASSIGNED, note_overrides};
use crate::midi::{PortInfo, base_name};

/// キーボードの入力の名前（設定の画面に出す）
pub const KEYBOARD: &str = "キーボード";

/// レーンの画面に出す名前（設定の画面の割り当てや、結果の分析に出す）
pub fn lane_label(lane: DrumLane) -> &'static str {
    match lane {
        DrumLane::LC => "左シンバル",
        DrumLane::HH => "ハイハット（閉）",
        DrumLane::HHO => "ハイハット（開）",
        DrumLane::SD => "スネア",
        DrumLane::BD => "バスドラム",
        DrumLane::HT => "ハイタム",
        DrumLane::LT => "ロータム",
        DrumLane::FT => "フロアタム",
        DrumLane::CY => "右シンバル",
        DrumLane::RD => "ライド",
        DrumLane::LP => "左ペダル",
        DrumLane::LBD => "左バスドラム",
    }
}

/// ポートの名前と ID の組
pub type PortKey<'a> = (&'a str, &'a str);

pub fn port_keys(ports: &[PortInfo]) -> Vec<PortKey<'_>> {
    ports
        .iter()
        .map(|p| (p.name.as_str(), p.id.as_str()))
        .collect()
}

/// 設定のどの機器がどのポートか（ポートごとに `devices` の添え字）。
/// まず名前と ID がどちらも同じものを、次に名前だけ同じものを当てる（別の USB の口に挿して ID が変わったときなど）
pub fn match_devices(devices: &[MidiDevice], ports: &[PortKey<'_>]) -> Vec<Option<usize>> {
    let mut out = vec![None; ports.len()];
    let mut used = vec![false; devices.len()];
    for (i, &(name, id)) in ports.iter().enumerate() {
        if id.is_empty() {
            continue;
        }
        let base = base_name(name);
        if let Some(j) =
            (0..devices.len()).find(|&j| !used[j] && devices[j].name == base && devices[j].id == id)
        {
            used[j] = true;
            out[i] = Some(j);
        }
    }
    for (i, &(name, _)) in ports.iter().enumerate() {
        if out[i].is_some() {
            continue;
        }
        let base = base_name(name);
        let free = |j: usize| !used[j] && devices[j].name == base;
        // 今つながっている別のポートの ID を持つ設定は、そのポートのために残しておく
        let elsewhere = |j: usize| {
            !devices[j].id.is_empty() && ports.iter().any(|&(_, id)| id == devices[j].id)
        };
        let pick = (0..devices.len())
            .find(|&j| free(j) && !elsewhere(j))
            .or_else(|| (0..devices.len()).find(|&j| free(j)));
        if let Some(j) = pick {
            used[j] = true;
            out[i] = Some(j);
        }
    }
    out
}

/// 機器の設定（無ければ `None`）
pub fn device<'a>(config: &'a Config, ports: &[PortInfo], port: usize) -> Option<&'a MidiDevice> {
    let i = (*match_devices(&config.midi_devices, &port_keys(ports)).get(port)?)?;
    config.midi_devices.get(i)
}

/// ポートを開くか（使わない設定にした機器は開かない）。ポートの並びと同じ順に
pub fn ports_to_open(config: &Config, ports: &[PortKey<'_>]) -> Vec<bool> {
    match_devices(&config.midi_devices, ports)
        .into_iter()
        .map(|d| d.is_none_or(|i| !config.midi_devices[i].disabled))
        .collect()
}

/// 機器の割り当ての出発点（設定が無ければ名前から選ぶ）
pub fn preset_for(device: Option<&MidiDevice>, port_name: &str) -> Preset {
    device
        .and_then(|d| Preset::from_name(&d.preset))
        .unwrap_or_else(|| Preset::detect(base_name(port_name)))
}

/// 機器の割り当て
pub fn profile_for(config: &Config, device: Option<&MidiDevice>, port_name: &str) -> MidiProfile {
    let mut p = preset_for(device, port_name).profile();
    match device {
        Some(d) => {
            p.channel = (1..=16).contains(&d.channel).then(|| d.channel - 1);
            p.min_velocity = d.min_velocity.min(127);
            p.apply_overrides(&note_overrides(&d.notes));
        }
        None => p.apply_overrides(&config.pad_overrides()),
    }
    p
}

/// ポートごとの割り当て（ポートの並びと同じ順）
pub fn profiles(config: &Config, ports: &[PortInfo]) -> Vec<MidiProfile> {
    let matched = match_devices(&config.midi_devices, &port_keys(ports));
    ports
        .iter()
        .zip(matched)
        .map(|(p, d)| profile_for(config, d.map(|i| &config.midi_devices[i]), &p.name))
        .collect()
}

/// ポートごとの入力の読み手
pub fn normalizers(config: &Config, ports: &[PortInfo]) -> Vec<Normalizer> {
    profiles(config, ports)
        .into_iter()
        .map(Normalizer::new)
        .collect()
}

/// 機器の設定（無ければ作る）。作るときは名前から選んだ出発点と、前の版の割り当てを写す。
/// ID が変わっていれば（別の USB の口に挿したなど）今のものにする
pub fn device_mut<'a>(
    config: &'a mut Config,
    ports: &[PortInfo],
    port: usize,
) -> Option<&'a mut MidiDevice> {
    let info = ports.get(port)?;
    let matched = match_devices(&config.midi_devices, &port_keys(ports))[port];
    let i = match matched {
        Some(i) => i,
        None => {
            let notes = config.pad_notes.clone();
            config.midi_devices.push(MidiDevice {
                name: base_name(&info.name).to_owned(),
                id: info.id.clone(),
                preset: Preset::detect(base_name(&info.name)).name().to_owned(),
                channel: 0,
                min_velocity: 0,
                disabled: false,
                notes,
            });
            config.midi_devices.len() - 1
        }
    };
    let d = &mut config.midi_devices[i];
    d.id.clone_from(&info.id);
    Some(d)
}

/// 機器のノートをレーンに割り当てる（そのノートの前の割り当ては消す）
pub fn assign_note(config: &mut Config, ports: &[PortInfo], port: usize, note: u8, lane: DrumLane) {
    if let Some(d) = device_mut(config, ports, port) {
        set_note(d, note, lane.name());
    }
}

fn set_note(d: &mut MidiDevice, note: u8, lane: &str) {
    d.notes.retain(|p| p.note != note);
    d.notes.push(PadNote {
        note,
        lane: lane.to_owned(),
    });
}

/// レーンの割り当てを、今つながっている機器とキーボードからすべて外す
pub fn clear_lane(config: &mut Config, ports: &[PortInfo], lane: DrumLane) {
    let profiles = profiles(config, ports);
    for (port, p) in profiles.iter().enumerate() {
        let notes = p.lane_notes().remove(&lane).unwrap_or_default();
        if notes.is_empty() {
            continue;
        }
        if let Some(d) = device_mut(config, ports, port) {
            for note in notes {
                set_note(d, note, UNASSIGNED);
            }
        }
    }
    for (code, l) in keyboard_map(config) {
        if l == lane {
            set_key(config, code, None);
        }
    }
}

/// 機器の割り当てを出発点に戻す（設定で変えたものを消す）
pub fn reset_device(config: &mut Config, ports: &[PortInfo], port: usize) {
    if let Some(d) = device_mut(config, ports, port) {
        d.notes.clear();
    }
}

/// 割り当てをすべて既定に戻す（機器ごとの変更、前の版の割り当て、キーボードの変更を消す）。
/// 機器の出発点・チャンネル・使うかどうかは残す
pub fn reset_all(config: &mut Config) {
    config.pad_notes.clear();
    config.keys.clear();
    for d in &mut config.midi_devices {
        d.notes.clear();
    }
}

/// キーボードの既定の割り当て（DTX Web Editor と同じ並び。Q は HH オープン）
pub const DEFAULT_KEYS: [(KeyCode, DrumLane); 11] = [
    (KeyCode::KeyA, DrumLane::LC),
    (KeyCode::KeyS, DrumLane::HH),
    (KeyCode::KeyQ, DrumLane::HHO),
    (KeyCode::KeyZ, DrumLane::LP),
    (KeyCode::KeyD, DrumLane::SD),
    (KeyCode::KeyG, DrumLane::HT),
    (KeyCode::KeyF, DrumLane::BD),
    (KeyCode::KeyH, DrumLane::LT),
    (KeyCode::KeyJ, DrumLane::FT),
    (KeyCode::KeyK, DrumLane::CY),
    (KeyCode::KeyL, DrumLane::RD),
];

/// 叩くのに割り当てられるキー。演奏中の操作に使うキー（Esc、R、I、矢印、F2、- = [ ] , .）と、
/// 画面の操作に使うキー（Enter、Space、Backspace、Delete、Tab、F1〜F12）は除く
const ASSIGNABLE: &[KeyCode] = &[
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
    KeyCode::NumpadAdd,
    KeyCode::NumpadSubtract,
    KeyCode::NumpadMultiply,
    KeyCode::NumpadDivide,
    KeyCode::NumpadDecimal,
    KeyCode::Semicolon,
    KeyCode::Quote,
    KeyCode::Slash,
    KeyCode::Backslash,
    KeyCode::Backquote,
    KeyCode::IntlRo,
    KeyCode::IntlYen,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
    KeyCode::Insert,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::PageUp,
    KeyCode::PageDown,
];

/// 叩くのに割り当てられるキーか
pub fn assignable(code: KeyCode) -> bool {
    ASSIGNABLE.contains(&code)
}

/// 設定のファイルに書くキーの名前
pub fn key_name(code: KeyCode) -> String {
    format!("{code:?}")
}

/// [`key_name`] の逆（割り当てられないキーは `None`）
pub fn parse_key(name: &str) -> Option<KeyCode> {
    ASSIGNABLE.iter().copied().find(|&k| key_name(k) == name)
}

/// 画面に出すキーの名前（KeyZ → Z、Digit1 → 1、Numpad1 → Num1）
pub fn key_label(code: KeyCode) -> String {
    let n = key_name(code);
    if let Some(rest) = n.strip_prefix("Key").or_else(|| n.strip_prefix("Digit")) {
        return rest.to_owned();
    }
    if let Some(rest) = n.strip_prefix("Numpad") {
        return format!("Num{rest}");
    }
    n
}

/// キーボードの割り当て（既定に、設定で変えたものを順に重ねる）
pub fn keyboard_map(config: &Config) -> Vec<(KeyCode, DrumLane)> {
    let mut map = DEFAULT_KEYS.to_vec();
    for b in &config.keys {
        let Some(code) = parse_key(&b.key) else {
            continue;
        };
        map.retain(|(k, _)| *k != code);
        if let Some(lane) = DrumLane::from_name(&b.lane) {
            map.push((code, lane));
        }
    }
    map
}

/// キーをレーンに割り当てる（`None` なら外す）。既定と同じになるなら書かない
pub fn set_key(config: &mut Config, code: KeyCode, lane: Option<DrumLane>) {
    let key = key_name(code);
    config.keys.retain(|b| b.key != key);
    let default = DEFAULT_KEYS
        .iter()
        .find(|(k, _)| *k == code)
        .map(|(_, l)| *l);
    if default != lane {
        config.keys.push(KeyBind {
            key,
            lane: lane.map_or(UNASSIGNED, |l| l.name()).to_owned(),
        });
    }
}

/// レーンを叩く入力 1 つぶん（設定の画面に出す）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSource {
    /// 機器の名前（キーボードなら [`KEYBOARD`]）
    pub source: String,
    /// ノート番号かキーの名前
    pub inputs: Vec<String>,
}

/// レーンごとの、叩ける入力（開いている MIDI の機器と、キーボード）
pub fn lane_sources(config: &Config, ports: &[PortInfo]) -> BTreeMap<DrumLane, Vec<LaneSource>> {
    let mut out: BTreeMap<DrumLane, Vec<LaneSource>> = BTreeMap::new();
    for (port, p) in ports.iter().zip(profiles(config, ports)) {
        if !port.open {
            continue;
        }
        for (lane, notes) in p.lane_notes() {
            out.entry(lane).or_default().push(LaneSource {
                source: port.name.clone(),
                inputs: notes.iter().map(u8::to_string).collect(),
            });
        }
    }
    let mut keys: BTreeMap<DrumLane, Vec<String>> = BTreeMap::new();
    for (code, lane) in keyboard_map(config) {
        keys.entry(lane).or_default().push(key_label(code));
    }
    for (lane, inputs) in keys {
        out.entry(lane).or_default().push(LaneSource {
            source: KEYBOARD.to_owned(),
            inputs,
        });
    }
    out
}

/// どこまで前の入力を設定の画面に出すか
const MONITOR_FOR: Duration = Duration::from_secs(10);

/// 届いた MIDI を見張り、設定の画面に出す最後の打撃と、ログに残す内容を決める。
/// ログは、ポートから最初に届いたものと、割り当ての無いノートなど「効かなかった入力」を種類ごとに 1 回だけ書く
#[derive(Debug, Default)]
pub struct MidiMonitor {
    /// ポートごとの最後のノートオン（説明と届いた時刻）
    last: BTreeMap<usize, (String, Instant)>,
    seen_ports: HashSet<usize>,
    /// ログに書いたもの（ポート、種類、チャンネル、番号）
    logged: HashSet<(usize, u8, u8, u8)>,
}

impl MidiMonitor {
    /// ポートを開き直したら呼ぶ（ポートの番号が変わる）
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 1 つのメッセージを見る。`profile` はそのポートの割り当て
    pub fn observe(
        &mut self,
        port: usize,
        port_name: &str,
        msg: &MidiMessage,
        profile: Option<&MidiProfile>,
    ) {
        let desc = msg.describe();
        if self.seen_ports.insert(port) {
            log::info!("MIDI from {port_name}: {desc}");
        }
        let accepted = profile.is_some_and(|p| p.channel.is_none_or(|c| c == msg.channel()));
        match *msg {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } => {
                let lane = profile.and_then(|p| p.note_lane(note));
                let soft = profile.is_some_and(|p| velocity < p.min_velocity);
                let result = match (accepted, lane) {
                    (false, _) => "ignored (other channel)",
                    (true, Some(_)) if soft => "ignored (too soft)",
                    (true, Some(l)) => l.name(),
                    (true, None) => "not assigned",
                };
                // 設定の画面に出す文は日本語で（ログは英語のまま）
                let shown = match (accepted, lane) {
                    (false, _) => "ほかのチャンネルなので捨てた",
                    (true, Some(_)) if soft => "弱いので捨てた",
                    (true, Some(l)) => lane_label(l),
                    (true, None) => "割り当てなし",
                };
                self.last.insert(
                    port,
                    (
                        format!(
                            "ch{} ノート {note} ベロシティ {velocity} → {shown}",
                            channel + 1
                        ),
                        Instant::now(),
                    ),
                );
                let worked = accepted && lane.is_some() && !soft;
                if !worked && self.logged.insert((port, 0x90, channel, note)) {
                    log::info!("MIDI {port_name}: {desc}: {result}");
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller,
                ..
            } => {
                if self.logged.insert((port, 0xB0, channel, controller)) {
                    log::info!("MIDI {port_name}: {desc} (first CC{controller})");
                }
            }
            MidiMessage::PolyAftertouch { channel, .. } => {
                if self.logged.insert((port, 0xA0, channel, 0)) {
                    log::info!("MIDI {port_name}: {desc} (first aftertouch)");
                }
            }
            MidiMessage::NoteOff { .. } => {}
        }
    }

    /// ポートから最後に届いた打撃（しばらく前のものは出さない）
    pub fn last(&self, port: usize) -> Option<&str> {
        self.last
            .get(&port)
            .filter(|(_, at)| at.elapsed() < MONITOR_FOR)
            .map(|(s, _)| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, id: &str) -> PortInfo {
        PortInfo {
            name: name.to_owned(),
            id: id.to_owned(),
            open: true,
            error: None,
        }
    }

    fn dev(name: &str, id: &str) -> MidiDevice {
        MidiDevice {
            name: name.to_owned(),
            id: id.to_owned(),
            preset: "gm".to_owned(),
            ..MidiDevice::default()
        }
    }

    #[test]
    fn same_model_twice_is_told_apart_by_id() {
        let devices = [
            dev("Finger Drum Pad", "usb-a"),
            dev("Finger Drum Pad", "usb-b"),
        ];
        // 挿した順が逆でも ID で当てる（Windows は 2 台目の名前に「2- 」が付く）
        let ports = [
            ("Finger Drum Pad", "usb-b"),
            ("2- Finger Drum Pad", "usb-a"),
        ];
        assert_eq!(match_devices(&devices, &ports), vec![Some(1), Some(0)]);
        // ID が変わったら（別の口に挿した）名前で当てる。今ある ID を持つ設定はそのポートに残す
        let ports = [
            ("Finger Drum Pad", "usb-c"),
            ("2- Finger Drum Pad", "usb-a"),
        ];
        assert_eq!(match_devices(&devices, &ports), vec![Some(1), Some(0)]);
        // 知らない機器は設定なし
        let ports = [("TD-27", "x")];
        assert_eq!(match_devices(&devices, &ports), vec![None]);
    }

    #[test]
    fn each_device_keeps_its_own_notes() {
        let mut c = Config::default();
        let ports = [port("Finger Drum Pad", "fgdp"), port("V51", "v51")];
        // 同じノート 38 を、FGDP は LP に、V51 は SD のまま
        assign_note(&mut c, &ports, 0, 38, DrumLane::LP);
        let p = profiles(&c, &ports);
        assert_eq!(p[0].note_lane(38), Some(DrumLane::LP));
        assert_eq!(p[1].note_lane(38), Some(DrumLane::SD));
        // 出発点は名前から：V51 は Roland（リムの 40 は SD のリム）、FGDP は GM
        assert_eq!(c.midi_devices.len(), 1);
        assert_eq!(c.midi_devices[0].preset, "gm");
        assert_eq!(preset_for(None, "V51"), Preset::Roland);
        // 保存して読み直しても同じ
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(profiles(&back, &ports)[0].note_lane(38), Some(DrumLane::LP));
    }

    #[test]
    fn channel_and_disabled_per_device() {
        let mut c = Config::default();
        let ports = [port("Finger Drum Pad", "fgdp"), port("V51", "v51")];
        device_mut(&mut c, &ports, 0).unwrap().channel = 10;
        device_mut(&mut c, &ports, 0).unwrap().min_velocity = 8;
        device_mut(&mut c, &ports, 1).unwrap().disabled = true;
        let p = profiles(&c, &ports);
        assert_eq!(p[0].channel, Some(9));
        assert_eq!(p[0].min_velocity, 8);
        assert_eq!(p[1].min_velocity, 0);
        assert_eq!(p[1].channel, None);
        assert_eq!(ports_to_open(&c, &port_keys(&ports)), vec![true, false]);
    }

    #[test]
    fn old_pad_notes_are_copied_into_a_new_device() {
        let mut c = Config {
            pad_notes: vec![PadNote {
                note: 3,
                lane: "FT".to_owned(),
            }],
            ..Config::default()
        };
        let ports = [port("Finger Drum Pad", "fgdp"), port("V51", "v51")];
        // 設定の無い機器にも前の割り当てが効く
        assert_eq!(profiles(&c, &ports)[1].note_lane(3), Some(DrumLane::FT));
        assign_note(&mut c, &ports, 0, 7, DrumLane::LT);
        let p = profiles(&c, &ports);
        assert_eq!(p[0].note_lane(3), Some(DrumLane::FT));
        assert_eq!(p[0].note_lane(7), Some(DrumLane::LT));
        // 消すと前の割り当ても消える
        reset_device(&mut c, &ports, 0);
        assert_eq!(profiles(&c, &ports)[0].note_lane(3), None);
    }

    #[test]
    fn clear_lane_removes_it_from_every_input() {
        let mut c = Config::default();
        let ports = [port("Finger Drum Pad", "fgdp"), port("V51", "v51")];
        assign_note(&mut c, &ports, 0, 3, DrumLane::LP);
        set_key(&mut c, KeyCode::KeyX, Some(DrumLane::LP));
        clear_lane(&mut c, &ports, DrumLane::LP);
        let p = profiles(&c, &ports);
        assert!(
            p.iter()
                .all(|p| !p.lane_notes().contains_key(&DrumLane::LP))
        );
        assert!(keyboard_map(&c).iter().all(|(_, l)| *l != DrumLane::LP));
        // ほかのレーンはそのまま
        assert_eq!(p[1].note_lane(38), Some(DrumLane::SD));
    }

    #[test]
    fn keyboard_assignments_layer_on_the_defaults() {
        let mut c = Config::default();
        assert!(keyboard_map(&c).contains(&(KeyCode::KeyZ, DrumLane::LP)));
        // LP を X に足し、Z は外す
        set_key(&mut c, KeyCode::KeyX, Some(DrumLane::LP));
        set_key(&mut c, KeyCode::KeyZ, None);
        let map = keyboard_map(&c);
        assert!(map.contains(&(KeyCode::KeyX, DrumLane::LP)));
        assert!(!map.iter().any(|(k, _)| *k == KeyCode::KeyZ));
        // 既定に戻すと書かない
        set_key(&mut c, KeyCode::KeyZ, Some(DrumLane::LP));
        assert_eq!(c.keys.len(), 1);
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(keyboard_map(&back), keyboard_map(&c));
        // 演奏中の操作のキーは割り当てられない
        assert!(!assignable(KeyCode::KeyR));
        assert!(!assignable(KeyCode::F2));
        assert!(!assignable(KeyCode::Escape));
        assert!(assignable(KeyCode::Space) == ASSIGNABLE.contains(&KeyCode::Space));
        assert_eq!(key_label(KeyCode::KeyZ), "Z");
        assert_eq!(key_label(KeyCode::Numpad1), "Num1");
        assert_eq!(parse_key("KeyX"), Some(KeyCode::KeyX));
        assert_eq!(parse_key("KeyR"), None);
    }

    #[test]
    fn lane_sources_list_each_input() {
        let mut c = Config::default();
        let mut ports = vec![port("Finger Drum Pad", "fgdp"), port("V51", "v51")];
        assign_note(&mut c, &ports, 0, 3, DrumLane::LP);
        let s = lane_sources(&c, &ports);
        let lp = &s[&DrumLane::LP];
        assert_eq!(lp[0].source, "Finger Drum Pad");
        assert!(lp[0].inputs.contains(&"3".to_owned()));
        assert_eq!(lp[1].source, "V51");
        assert_eq!(lp.last().unwrap().source, KEYBOARD);
        assert_eq!(lp.last().unwrap().inputs, vec!["Z".to_owned()]);
        // 開いていない機器は出さない
        ports[0].open = false;
        let s = lane_sources(&c, &ports);
        assert!(
            s[&DrumLane::LP]
                .iter()
                .all(|l| l.source != "Finger Drum Pad")
        );
    }

    #[test]
    fn monitor_shows_the_last_hit_and_its_lane() {
        let mut m = MidiMonitor::default();
        let p = MidiProfile::general_midi();
        let on = |note| MidiMessage::NoteOn {
            channel: 9,
            note,
            velocity: 100,
        };
        m.observe(0, "Pad", &on(38), Some(&p));
        assert_eq!(m.last(0), Some("ch10 ノート 38 ベロシティ 100 → スネア"));
        m.observe(0, "Pad", &on(3), Some(&p));
        assert_eq!(
            m.last(0),
            Some("ch10 ノート 3 ベロシティ 100 → 割り当てなし")
        );
        let mut only1 = p.clone();
        only1.channel = Some(0);
        m.observe(1, "Pad2", &on(38), Some(&only1));
        assert_eq!(
            m.last(1),
            Some("ch10 ノート 38 ベロシティ 100 → ほかのチャンネルなので捨てた")
        );
        let mut strong = p.clone();
        strong.min_velocity = 110;
        m.observe(1, "Pad2", &on(38), Some(&strong));
        assert_eq!(
            m.last(1),
            Some("ch10 ノート 38 ベロシティ 100 → 弱いので捨てた")
        );
        assert_eq!(m.last(2), None);
    }
}
