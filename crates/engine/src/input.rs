//! 電子ドラムの入力。MIDI のメッセージを読み、パッドと叩き方、譜面のレーンにそろえる。
//!
//! 上流は MIDI のノートオンとベロシティしか読まないが、ここでは次も扱う。
//! - ハイハットの開き具合（CC4）。叩いた瞬間の値を打撃に添え、開閉の決め方にも使える
//! - ヘッド・リム・クロススティック、シンバルのボウ・エッジ・ベル
//! - シンバルのチョーク（ポリフォニック・アフタータッチ）
//! - 弱すぎる入力（クロストーク）と、同じパッドの短すぎる連打（二重トリガー）を捨てる
//!
//! 割り当て（[`MidiProfile`]）は入力機器（MIDI のポート）ごとに 1 つ持つ。チャンネルは既定ではすべて受け、
//! 機器ごとに 1 つのチャンネルに絞ることもできる。同じチャンネルで送る機器どうしも、ポートで見分ける。

use std::collections::BTreeMap;

use dtx::DrumLane;

/// MIDI のメッセージ（ドラムに要るものだけ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    PolyAftertouch {
        channel: u8,
        note: u8,
        pressure: u8,
    },
}

impl MidiMessage {
    /// 1 つのメッセージのバイト列を読む。ドラムに関係ないものは `None`
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let (&status, data) = bytes.split_first()?;
        let channel = status & 0x0F;
        let d = |i: usize| data.get(i).copied().filter(|b| *b < 0x80);
        match status & 0xF0 {
            0x90 => {
                let (note, velocity) = (d(0)?, d(1)?);
                // ベロシティ 0 のノートオンはノートオフ
                Some(if velocity == 0 {
                    Self::NoteOff { channel, note }
                } else {
                    Self::NoteOn {
                        channel,
                        note,
                        velocity,
                    }
                })
            }
            0x80 => Some(Self::NoteOff {
                channel,
                note: d(0)?,
            }),
            0xA0 => Some(Self::PolyAftertouch {
                channel,
                note: d(0)?,
                pressure: d(1)?,
            }),
            0xB0 => Some(Self::ControlChange {
                channel,
                controller: d(0)?,
                value: d(1)?,
            }),
            _ => None,
        }
    }

    /// バイト列に入っているメッセージをすべて読む。OS によっては 1 回に複数のメッセージが届き、
    /// ランニングステータス（2 つめから状態のバイトを省く書き方）も使われる。
    /// クロックなどのリアルタイムのメッセージ（0xF8〜）とシステムエクスクルーシブは読み飛ばす
    pub fn parse_all(bytes: &[u8]) -> Vec<Self> {
        let mut out = Vec::new();
        let mut running: Option<u8> = None;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b >= 0xF8 {
                i += 1;
                continue;
            }
            if b >= 0xF0 {
                // システムのメッセージはランニングステータスを打ち切る。続くデータのバイトは読み飛ばす
                running = None;
                i += 1;
                while i < bytes.len() && bytes[i] < 0x80 {
                    i += 1;
                }
                continue;
            }
            let (status, start) = if b >= 0x80 {
                (b, i + 1)
            } else if let Some(s) = running {
                (s, i)
            } else {
                // 状態のわからないデータのバイト
                i += 1;
                continue;
            };
            let len = match status & 0xF0 {
                0xC0 | 0xD0 => 1,
                _ => 2,
            };
            let data: Vec<u8> = bytes[start..]
                .iter()
                .copied()
                .filter(|&d| d < 0xF8)
                .take(len)
                .collect();
            if data.len() < len {
                break;
            }
            if let Some(at) = data.iter().position(|&d| d >= 0x80) {
                // 途中で次の状態のバイトが来た（壊れたメッセージ）。そこから読み直す
                running = None;
                i = start + at;
                continue;
            }
            let mut msg = vec![status];
            msg.extend_from_slice(&data);
            if let Some(m) = Self::parse(&msg) {
                out.push(m);
            }
            running = Some(status);
            // 間にリアルタイムのバイトがあれば、その分も進める
            let mut taken = 0;
            i = start;
            while taken < len {
                if bytes[i] < 0xF8 {
                    taken += 1;
                }
                i += 1;
            }
        }
        out
    }

    /// チャンネル（0〜15。画面に出すときは 1 を足す）
    pub fn channel(&self) -> u8 {
        match *self {
            Self::NoteOn { channel, .. }
            | Self::NoteOff { channel, .. }
            | Self::ControlChange { channel, .. }
            | Self::PolyAftertouch { channel, .. } => channel,
        }
    }

    /// ログと設定の画面に出す短い説明（チャンネルは 1〜16）
    pub fn describe(&self) -> String {
        match *self {
            Self::NoteOn {
                channel,
                note,
                velocity,
            } => format!("ch{} note {note} vel {velocity}", channel + 1),
            Self::NoteOff { channel, note } => format!("ch{} note {note} off", channel + 1),
            Self::ControlChange {
                channel,
                controller,
                value,
            } => format!("ch{} CC{controller} = {value}", channel + 1),
            Self::PolyAftertouch {
                channel,
                note,
                pressure,
            } => format!("ch{} aftertouch {note} = {pressure}", channel + 1),
        }
    }
}

/// 割り当ての出発点（機器ごとに選べる）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Roland V-Drums の工場出荷時のノート番号（リム・エッジ・ベルを打ち分ける）
    Roland,
    /// General MIDI のドラムの番号（多くの機器の既定）
    #[default]
    GeneralMidi,
    /// 何も割り当てない（すべて自分で割り当てる）
    Empty,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Self::Roland, Self::GeneralMidi, Self::Empty];

    /// 設定のファイルに書く名前
    pub fn name(self) -> &'static str {
        match self {
            Self::Roland => "roland",
            Self::GeneralMidi => "gm",
            Self::Empty => "empty",
        }
    }

    /// [`Preset::name`] の逆
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// 画面に出す名前
    pub fn label(self) -> &'static str {
        match self {
            Self::Roland => "Roland",
            Self::GeneralMidi => "GM",
            Self::Empty => "なし",
        }
    }

    /// 次のもの（設定の画面で Enter を押すたびに替える）
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// ポートの名前から出発点を選ぶ。Roland の電子ドラムらしい名前なら Roland、ほかは GM
    pub fn detect(port_name: &str) -> Self {
        let lower = port_name.to_lowercase();
        let tokens: Vec<&str> = lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect();
        let roland = lower.contains("roland")
            || lower.contains("v-drums")
            || lower.contains("td-")
            || tokens.iter().any(|t| {
                matches!(*t, "v51" | "v71" | "vdrums")
                    || t.starts_with("vad")
                    || t.starts_with("spd")
                    || (t.len() > 2
                        && t.starts_with("td")
                        && t[2..].bytes().all(|b| b.is_ascii_digit()))
            });
        if roland {
            Self::Roland
        } else {
            Self::GeneralMidi
        }
    }

    pub fn profile(self) -> MidiProfile {
        match self {
            Self::Roland => MidiProfile::roland_vdrums(),
            Self::GeneralMidi => MidiProfile::general_midi(),
            Self::Empty => MidiProfile::empty(),
        }
    }
}

/// 物理的なパッド
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pad {
    Kick,
    /// 左のキック（ツインペダルの左を別のパッドで受けるときなど。左バスドラム）
    LeftKick,
    Snare,
    Tom1,
    Tom2,
    Tom3,
    Tom4,
    HiHat,
    HiHatPedal,
    Crash1,
    Crash2,
    Ride,
    Aux,
}

/// 叩き方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Articulation {
    Head,
    Rim,
    CrossStick,
    /// シンバルの面（ハイハットはボウ）
    Bow,
    Edge,
    Bell,
}

impl Articulation {
    pub const ALL: [Articulation; 6] = [
        Self::Head,
        Self::Rim,
        Self::CrossStick,
        Self::Bow,
        Self::Edge,
        Self::Bell,
    ];

    /// 記録に書く名前
    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Rim => "rim",
            Self::CrossStick => "cross_stick",
            Self::Bow => "bow",
            Self::Edge => "edge",
            Self::Bell => "bell",
        }
    }

    /// [`Articulation::name`] の逆
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// ハイハットの開閉の決め方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HiHatMode {
    /// モジュールが送るノート番号（オープン / クローズ）で決める
    #[default]
    Note,
    /// 叩いた瞬間のペダルの値（CC4）で決める
    Pedal { closed_at: u8 },
}

/// ノート番号 1 つの意味
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteMapping {
    pub pad: Pad,
    pub articulation: Articulation,
    /// ハイハットのノートが「オープン」を表すか（ノートで開閉を決めるとき使う）
    pub hh_open: bool,
}

/// 電子ドラム 1 台ぶんの設定
#[derive(Debug, Clone, PartialEq)]
pub struct MidiProfile {
    pub name: String,
    pub notes: BTreeMap<u8, NoteMapping>,
    pub hi_hat: HiHatMode,
    /// これより弱い打撃は捨てる（クロストーク対策）。0 なら捨てない
    pub min_velocity: u8,
    /// 同じパッドがこれより短い間隔で鳴ったら捨てる（二重トリガー対策）。`None` なら捨てない
    pub retrigger_guard_us: Option<i64>,
    /// ハイハットの開き具合を表すコントロールチェンジの番号
    pub hh_pedal_cc: u8,
    /// このチャンネル（0〜15）だけ受ける。`None` ならすべて受ける
    pub channel: Option<u8>,
    /// シンバルのポリフォニック・アフタータッチをチョーク（握って止めた）と読む。
    /// Roland はチョークにだけ送るが、叩くたびに減っていく圧を送り続ける機器（YAMAHA FGDP など）もある
    pub choke_by_aftertouch: bool,
}

impl MidiProfile {
    /// Roland V-Drums の工場出荷時のノート番号（TD-17 / TD-27 / TD-50 系）。
    /// 機種やキットの設定で変わることがあるので、実機の MIDI モニターで確かめて直せるようにする
    pub fn roland_vdrums() -> Self {
        use Articulation::*;
        use Pad::*;
        let mut notes = BTreeMap::new();
        let mut put = |note: u8, pad: Pad, articulation: Articulation, hh_open: bool| {
            notes.insert(
                note,
                NoteMapping {
                    pad,
                    articulation,
                    hh_open,
                },
            );
        };
        put(36, Kick, Head, false);
        put(38, Snare, Head, false);
        put(40, Snare, Rim, false);
        put(37, Snare, CrossStick, false);
        put(48, Tom1, Head, false);
        put(50, Tom1, Rim, false);
        put(45, Tom2, Head, false);
        put(47, Tom2, Rim, false);
        put(43, Tom3, Head, false);
        put(58, Tom3, Rim, false);
        put(41, Tom4, Head, false);
        put(39, Tom4, Rim, false);
        put(46, HiHat, Bow, true);
        put(26, HiHat, Edge, true);
        put(42, HiHat, Bow, false);
        put(22, HiHat, Edge, false);
        put(44, HiHatPedal, Head, false);
        put(49, Crash1, Bow, false);
        put(55, Crash1, Edge, false);
        put(57, Crash2, Bow, false);
        put(52, Crash2, Edge, false);
        put(51, Ride, Bow, false);
        put(59, Ride, Edge, false);
        put(53, Ride, Bell, false);
        put(27, Aux, Head, false);
        put(28, Aux, Rim, false);
        Self {
            name: "Roland V-Drums".to_owned(),
            notes,
            hi_hat: HiHatMode::Note,
            min_velocity: 0,
            retrigger_guard_us: None,
            hh_pedal_cc: 4,
            channel: None,
            choke_by_aftertouch: true,
        }
    }

    /// 何も割り当てない
    pub fn empty() -> Self {
        Self {
            name: "None".to_owned(),
            notes: BTreeMap::new(),
            hi_hat: HiHatMode::Note,
            min_velocity: 0,
            retrigger_guard_us: None,
            hh_pedal_cc: 4,
            channel: None,
            choke_by_aftertouch: false,
        }
    }

    /// General MIDI のドラムの番号（GM の打楽器の表のうち、ドラムセットのもの）
    pub fn general_midi() -> Self {
        use Articulation::*;
        use Pad::*;
        let mut p = Self::empty();
        p.name = "General MIDI".to_owned();
        for (note, pad, articulation, hh_open) in [
            (35, Kick, Head, false),
            (36, Kick, Head, false),
            (37, Snare, CrossStick, false),
            (38, Snare, Head, false),
            (40, Snare, Rim, false),
            (41, Tom4, Head, false),
            (43, Tom3, Head, false),
            (45, Tom2, Head, false),
            (47, Tom2, Head, false),
            (48, Tom1, Head, false),
            (50, Tom1, Head, false),
            (42, HiHat, Bow, false),
            (46, HiHat, Bow, true),
            (44, HiHatPedal, Head, false),
            (49, Crash1, Bow, false),
            (55, Crash1, Bow, false),
            (57, Crash2, Bow, false),
            (52, Crash2, Bow, false),
            (51, Ride, Bow, false),
            (59, Ride, Edge, false),
            (53, Ride, Bell, false),
        ] {
            p.notes.insert(
                note,
                NoteMapping {
                    pad,
                    articulation,
                    hh_open,
                },
            );
        }
        p
    }
}

impl MidiProfile {
    /// DTXManiaNX の Config.ini のパッドの割り当て（ノート番号 → レーン）を重ねる。
    /// NX で遊べていた割り当てをそのまま使えるようにする。
    /// すでに同じレーンになるノートはそのまま（リムやエッジ、ハイハットの開き具合の区別を残す）。
    /// 違うレーンになるノートと、知らないノートは NX に合わせる
    pub fn apply_lanes(&mut self, notes: &[(u8, DrumLane)]) {
        use Articulation::*;
        for &(note, lane) in notes {
            if let Some(m) = self.notes.get(&note) {
                let same = match m.pad {
                    Pad::HiHat => matches!(lane, DrumLane::HH | DrumLane::HHO),
                    pad => pad_lane(pad) == Some(lane),
                };
                if same {
                    continue;
                }
            }
            let (pad, articulation, hh_open) = match lane {
                DrumLane::HH => (Pad::HiHat, Bow, false),
                DrumLane::HHO => (Pad::HiHat, Bow, true),
                DrumLane::SD => (Pad::Snare, Head, false),
                DrumLane::BD => (Pad::Kick, Head, false),
                DrumLane::HT => (Pad::Tom1, Head, false),
                DrumLane::LT => (Pad::Tom2, Head, false),
                DrumLane::FT => (Pad::Tom3, Head, false),
                DrumLane::CY => (Pad::Crash2, Bow, false),
                DrumLane::RD => (Pad::Ride, Bow, false),
                DrumLane::LC => (Pad::Crash1, Bow, false),
                DrumLane::LP => (Pad::HiHatPedal, Head, false),
                DrumLane::LBD => (Pad::LeftKick, Head, false),
            };
            self.notes.insert(
                note,
                NoteMapping {
                    pad,
                    articulation,
                    hh_open,
                },
            );
        }
    }
}

impl MidiProfile {
    /// 設定で変えた割り当てを順に重ねる（`None` は外す）
    pub fn apply_overrides(&mut self, overrides: &[(u8, Option<DrumLane>)]) {
        for &(note, lane) in overrides {
            match lane {
                Some(lane) => self.apply_lanes(&[(note, lane)]),
                None => self.unassign([note]),
            }
        }
    }

    /// ノートの割り当てを外す（叩いても何も起きなくなる）
    pub fn unassign(&mut self, notes: impl IntoIterator<Item = u8>) {
        for note in notes {
            self.notes.remove(&note);
        }
    }

    /// ノートのレーン（割り当てが無いか、レーンの無いパッドなら `None`）。ハイハットはノートの開閉で決める
    pub fn note_lane(&self, note: u8) -> Option<DrumLane> {
        let m = self.notes.get(&note)?;
        match m.pad {
            Pad::HiHat if m.hh_open => Some(DrumLane::HHO),
            Pad::HiHat => Some(DrumLane::HH),
            pad => pad_lane(pad),
        }
    }

    /// レーンごとのノート番号（設定の画面に出す）。ハイハットは「オープン」を表すノートを HHO にする
    pub fn lane_notes(&self) -> BTreeMap<DrumLane, Vec<u8>> {
        let mut out: BTreeMap<DrumLane, Vec<u8>> = BTreeMap::new();
        for &note in self.notes.keys() {
            if let Some(lane) = self.note_lane(note) {
                out.entry(lane).or_default().push(note);
            }
        }
        out
    }
}

/// ハイハット以外のパッドのレーン
fn pad_lane(pad: Pad) -> Option<DrumLane> {
    Some(match pad {
        Pad::Kick => DrumLane::BD,
        Pad::LeftKick => DrumLane::LBD,
        Pad::Snare => DrumLane::SD,
        Pad::Tom1 => DrumLane::HT,
        Pad::Tom2 => DrumLane::LT,
        Pad::Tom3 | Pad::Tom4 => DrumLane::FT,
        Pad::HiHatPedal => DrumLane::LP,
        Pad::Crash1 => DrumLane::LC,
        Pad::Crash2 => DrumLane::CY,
        Pad::Ride => DrumLane::RD,
        Pad::HiHat | Pad::Aux => return None,
    })
}

/// そろえた入力
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// 叩いた
    Hit {
        host_us: i64,
        /// 譜面のレーン（割り当てが無いパッドは `None`。記録には残せる）
        lane: Option<DrumLane>,
        pad: Pad,
        articulation: Articulation,
        velocity: u8,
        /// 叩いた瞬間のハイハットの開き具合（0 = 開き切り、127 = 閉じ切り）。まだ届いていなければ `None`
        hh_pedal: Option<u8>,
    },
    /// シンバルを握って止めた
    Choke { host_us: i64, pad: Pad },
    /// ハイハットのペダルが動いた
    HiHatPedal { host_us: i64, value: u8 },
}

/// MIDI のメッセージを順に受け取り、[`InputEvent`] にする
#[derive(Debug, Clone)]
pub struct Normalizer {
    profile: MidiProfile,
    hh_pedal: Option<u8>,
    last_hit_us: BTreeMap<Pad, i64>,
}

impl Normalizer {
    pub fn new(profile: MidiProfile) -> Self {
        Self {
            profile,
            hh_pedal: None,
            last_hit_us: BTreeMap::new(),
        }
    }

    pub fn profile(&self) -> &MidiProfile {
        &self.profile
    }

    /// `host_us` は叩いた瞬間のホスト時刻（`clock::MidiTimestampMapper` で直したもの）
    pub fn feed(&mut self, msg: MidiMessage, host_us: i64) -> Option<InputEvent> {
        if !self.accepts(&msg) {
            return None;
        }
        match msg {
            MidiMessage::ControlChange {
                controller, value, ..
            } if controller == self.profile.hh_pedal_cc => {
                self.hh_pedal = Some(value);
                Some(InputEvent::HiHatPedal { host_us, value })
            }
            MidiMessage::PolyAftertouch { note, pressure, .. } => {
                if !self.profile.choke_by_aftertouch {
                    return None;
                }
                let m = self.profile.notes.get(&note)?;
                let is_cymbal = matches!(m.pad, Pad::Crash1 | Pad::Crash2 | Pad::Ride | Pad::HiHat);
                (is_cymbal && pressure > 0).then_some(InputEvent::Choke {
                    host_us,
                    pad: m.pad,
                })
            }
            MidiMessage::NoteOn { note, velocity, .. } => {
                let m = *self.profile.notes.get(&note)?;
                if velocity < self.profile.min_velocity {
                    return None;
                }
                if let Some(guard) = self.profile.retrigger_guard_us
                    && self
                        .last_hit_us
                        .get(&m.pad)
                        .is_some_and(|&last| host_us - last < guard)
                {
                    return None;
                }
                self.last_hit_us.insert(m.pad, host_us);
                Some(InputEvent::Hit {
                    host_us,
                    lane: self.lane_for(m),
                    pad: m.pad,
                    articulation: m.articulation,
                    velocity,
                    hh_pedal: self.hh_pedal,
                })
            }
            _ => None,
        }
    }

    /// この機器のチャンネルの絞り込みに合うか
    pub fn accepts(&self, msg: &MidiMessage) -> bool {
        self.profile.channel.is_none_or(|ch| ch == msg.channel())
    }

    fn lane_for(&self, m: NoteMapping) -> Option<DrumLane> {
        Some(match m.pad {
            Pad::HiHat => {
                let open = match self.profile.hi_hat {
                    HiHatMode::Note => m.hh_open,
                    // ペダルの値がまだ届いていなければ、ノート番号に従う
                    HiHatMode::Pedal { closed_at } => {
                        self.hh_pedal.map_or(m.hh_open, |v| v < closed_at)
                    }
                };
                if open { DrumLane::HHO } else { DrumLane::HH }
            }
            pad => return pad_lane(pad),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nx_lanes_override_only_what_differs() {
        let mut p = MidiProfile::roland_vdrums();
        p.apply_lanes(&[
            (36, DrumLane::BD),
            (40, DrumLane::SD),
            (46, DrumLane::HH),
            (49, DrumLane::CY),
            (60, DrumLane::SD),
            (35, DrumLane::LBD),
        ]);
        // 同じレーンになるものは元のまま（スネアのリム、ハイハットの開き）
        assert_eq!(p.notes[&40].articulation, Articulation::Rim);
        assert!(p.notes[&46].hh_open);
        // 違うレーンは NX に合わせる
        assert_eq!(p.notes[&49].pad, Pad::Crash2);
        // 知らないノートは足す
        assert_eq!(p.notes[&60].pad, Pad::Snare);
        // 左バスドラムは左のキックとして足す
        assert_eq!(p.notes[&35].pad, Pad::LeftKick);
        assert_eq!(p.note_lane(35), Some(DrumLane::LBD));
    }

    fn on(note: u8, velocity: u8) -> MidiMessage {
        MidiMessage::NoteOn {
            channel: 9,
            note,
            velocity,
        }
    }

    fn hit_lane(e: Option<InputEvent>) -> Option<DrumLane> {
        match e {
            Some(InputEvent::Hit { lane, .. }) => lane,
            _ => None,
        }
    }

    #[test]
    fn parse_messages() {
        assert_eq!(MidiMessage::parse(&[0x99, 38, 100]), Some(on(38, 100)));
        assert_eq!(
            MidiMessage::parse(&[0x99, 38, 0]),
            Some(MidiMessage::NoteOff {
                channel: 9,
                note: 38
            })
        );
        assert_eq!(
            MidiMessage::parse(&[0xB9, 4, 90]),
            Some(MidiMessage::ControlChange {
                channel: 9,
                controller: 4,
                value: 90
            })
        );
        assert_eq!(
            MidiMessage::parse(&[0xA9, 49, 127]),
            Some(MidiMessage::PolyAftertouch {
                channel: 9,
                note: 49,
                pressure: 127
            })
        );
        assert_eq!(MidiMessage::parse(&[0xF8]), None);
        assert_eq!(MidiMessage::parse(&[0x99, 38]), None);
    }

    #[test]
    fn roland_kit_maps_to_dtx_lanes() {
        let mut n = Normalizer::new(MidiProfile::roland_vdrums());
        let cases = [
            (36, DrumLane::BD),
            (38, DrumLane::SD),
            (40, DrumLane::SD),
            (37, DrumLane::SD),
            (48, DrumLane::HT),
            (45, DrumLane::LT),
            (43, DrumLane::FT),
            (42, DrumLane::HH),
            (46, DrumLane::HHO),
            (44, DrumLane::LP),
            (49, DrumLane::LC),
            (57, DrumLane::CY),
            (51, DrumLane::RD),
            (53, DrumLane::RD),
        ];
        for (i, (note, lane)) in cases.into_iter().enumerate() {
            assert_eq!(
                hit_lane(n.feed(on(note, 100), i as i64 * 100_000)),
                Some(lane),
                "note {note}"
            );
        }
    }

    #[test]
    fn rim_is_kept_as_articulation() {
        let mut n = Normalizer::new(MidiProfile::roland_vdrums());
        let Some(InputEvent::Hit {
            articulation, lane, ..
        }) = n.feed(on(40, 90), 0)
        else {
            panic!()
        };
        assert_eq!(
            (lane, articulation),
            (Some(DrumLane::SD), Articulation::Rim)
        );
    }

    #[test]
    fn hi_hat_by_pedal_value() {
        let mut p = MidiProfile::roland_vdrums();
        p.hi_hat = HiHatMode::Pedal { closed_at: 100 };
        let mut n = Normalizer::new(p);
        let cc = |v| MidiMessage::ControlChange {
            channel: 9,
            controller: 4,
            value: v,
        };
        assert_eq!(
            n.feed(cc(127), 0),
            Some(InputEvent::HiHatPedal {
                host_us: 0,
                value: 127
            })
        );
        // モジュールがオープンのノートを送っても、ペダルが閉じていればクローズ
        assert_eq!(hit_lane(n.feed(on(46, 100), 10)), Some(DrumLane::HH));
        n.feed(cc(40), 20);
        let e = n.feed(on(42, 100), 30);
        assert_eq!(hit_lane(e), Some(DrumLane::HHO));
        let Some(InputEvent::Hit { hh_pedal, .. }) = e else {
            panic!()
        };
        assert_eq!(hh_pedal, Some(40));
    }

    #[test]
    fn choke_on_cymbals_only() {
        let mut n = Normalizer::new(MidiProfile::roland_vdrums());
        let at = |note, pressure| MidiMessage::PolyAftertouch {
            channel: 9,
            note,
            pressure,
        };
        assert_eq!(
            n.feed(at(49, 127), 5),
            Some(InputEvent::Choke {
                host_us: 5,
                pad: Pad::Crash1
            })
        );
        assert_eq!(n.feed(at(49, 0), 6), None);
        assert_eq!(n.feed(at(38, 127), 7), None);
    }

    #[test]
    fn aftertouch_after_each_hit_is_not_a_choke() {
        // FGDP は叩くたびにアフタータッチで減っていく圧を送る。GM ではチョークと読まない
        let mut n = Normalizer::new(MidiProfile::general_midi());
        let at = MidiMessage::PolyAftertouch {
            channel: 9,
            note: 49,
            pressure: 120,
        };
        assert!(n.feed(on(49, 127), 0).is_some());
        assert_eq!(n.feed(at, 20_000), None);
    }

    #[test]
    fn crosstalk_and_retrigger_filters() {
        let mut p = MidiProfile::roland_vdrums();
        p.min_velocity = 10;
        p.retrigger_guard_us = Some(15_000);
        let mut n = Normalizer::new(p);
        assert_eq!(n.feed(on(38, 5), 0), None);
        assert!(n.feed(on(38, 80), 1_000).is_some());
        // 10ms 後の同じパッドは二重トリガーとして捨てる
        assert_eq!(n.feed(on(38, 60), 11_000), None);
        // 別のパッドは捨てない
        assert!(n.feed(on(36, 60), 11_000).is_some());
        assert!(n.feed(on(38, 60), 20_000).is_some());
    }

    #[test]
    fn parse_all_reads_running_status_and_skips_clock() {
        // ランニングステータスで 2 つ、間にクロック、最後にベロシティ 0（ノートオフ）
        let bytes = [0x99, 36, 100, 0xF8, 38, 90, 0xF8, 0xF8, 0x89, 36, 0, 38, 0];
        assert_eq!(
            MidiMessage::parse_all(&bytes),
            vec![
                on(36, 100),
                on(38, 90),
                MidiMessage::NoteOff {
                    channel: 9,
                    note: 36
                },
                MidiMessage::NoteOff {
                    channel: 9,
                    note: 38
                },
            ]
        );
        // データのバイトの途中で次の状態のバイトが来たら、そこから読み直す
        assert_eq!(
            MidiMessage::parse_all(&[0x99, 36, 0x99, 38, 80]),
            vec![on(38, 80)]
        );
        // システムエクスクルーシブは飛ばす。足りないメッセージは捨てる
        assert_eq!(
            MidiMessage::parse_all(&[0xF0, 0x41, 0x10, 0xF7, 0x99, 42, 70, 0x99, 44]),
            vec![on(42, 70)]
        );
        // クロックだけなら何も無い
        assert!(MidiMessage::parse_all(&[0xF8, 0xFE]).is_empty());
    }

    #[test]
    fn channel_filter_is_per_profile() {
        let mut p = MidiProfile::general_midi();
        p.channel = Some(0);
        let mut n = Normalizer::new(p);
        // 10ch のノートは受けない、1ch は受ける
        assert_eq!(n.feed(on(38, 100), 0), None);
        let ch1 = MidiMessage::NoteOn {
            channel: 0,
            note: 38,
            velocity: 100,
        };
        assert_eq!(hit_lane(n.feed(ch1, 1)), Some(DrumLane::SD));
        // 絞らなければどのチャンネルでも受ける
        let mut n = Normalizer::new(MidiProfile::general_midi());
        assert_eq!(hit_lane(n.feed(ch1, 0)), Some(DrumLane::SD));
        assert_eq!(hit_lane(n.feed(on(38, 100), 100_000)), Some(DrumLane::SD));
    }

    #[test]
    fn presets_by_port_name() {
        for name in [
            "TD-27",
            "Roland TD-17",
            "V51",
            "2- V71",
            "V-Drums",
            "VAD706",
            "SPD-SX PRO",
            "TD07",
        ] {
            assert_eq!(Preset::detect(name), Preset::Roland, "{name}");
        }
        for name in [
            "Finger Drum Pad",
            "DTX-PRO",
            "Alesis Strike",
            "USB MIDI Interface",
            "V5 Port",
        ] {
            assert_eq!(Preset::detect(name), Preset::GeneralMidi, "{name}");
        }
        assert_eq!(Preset::Roland.next(), Preset::GeneralMidi);
        assert_eq!(Preset::Empty.next(), Preset::Roland);
        for p in Preset::ALL {
            assert_eq!(Preset::from_name(p.name()), Some(p));
        }
        assert!(Preset::Empty.profile().notes.is_empty());
    }

    #[test]
    fn general_midi_kit_lanes() {
        let lanes = MidiProfile::general_midi().lane_notes();
        assert_eq!(lanes[&DrumLane::BD], vec![35, 36]);
        assert_eq!(lanes[&DrumLane::SD], vec![37, 38, 40]);
        assert_eq!(lanes[&DrumLane::HH], vec![42]);
        assert_eq!(lanes[&DrumLane::HHO], vec![46]);
        assert_eq!(lanes[&DrumLane::LP], vec![44]);
        assert_eq!(lanes[&DrumLane::HT], vec![48, 50]);
        assert_eq!(lanes[&DrumLane::LT], vec![45, 47]);
        assert_eq!(lanes[&DrumLane::FT], vec![41, 43]);
        assert_eq!(lanes[&DrumLane::LC], vec![49, 55]);
        assert_eq!(lanes[&DrumLane::CY], vec![52, 57]);
        assert_eq!(lanes[&DrumLane::RD], vec![51, 53, 59]);
    }

    #[test]
    fn overrides_apply_in_order() {
        let mut p = MidiProfile::general_midi();
        p.apply_overrides(&[
            (3, Some(DrumLane::FT)),
            (38, None),
            (3, Some(DrumLane::LT)),
            (40, None),
            (40, Some(DrumLane::LP)),
        ]);
        assert_eq!(p.note_lane(3), Some(DrumLane::LT));
        assert_eq!(p.note_lane(38), None);
        assert_eq!(p.note_lane(40), Some(DrumLane::LP));
    }

    #[test]
    fn unassign_removes_notes() {
        let mut p = MidiProfile::roland_vdrums();
        p.unassign([38, 40]);
        assert_eq!(p.note_lane(38), None);
        assert_eq!(p.note_lane(37), Some(DrumLane::SD));
        let mut n = Normalizer::new(p);
        assert_eq!(n.feed(on(38, 100), 0), None);
    }

    #[test]
    fn unknown_notes_are_ignored() {
        let mut n = Normalizer::new(MidiProfile::roland_vdrums());
        assert_eq!(n.feed(on(100, 100), 0), None);
    }

    #[test]
    fn lane_notes_group_notes_by_lane() {
        let mut p = MidiProfile::roland_vdrums();
        p.apply_lanes(&[(60, DrumLane::HT)]);
        let lanes = p.lane_notes();
        assert_eq!(lanes[&DrumLane::SD], vec![37, 38, 40]);
        assert_eq!(lanes[&DrumLane::HH], vec![22, 42]);
        assert_eq!(lanes[&DrumLane::HHO], vec![26, 46]);
        assert_eq!(lanes[&DrumLane::HT], vec![48, 50, 60]);
        // 割り当ての無いパッド（Aux）は出さない
        assert!(lanes.values().all(|n| !n.contains(&27)));
    }
}
