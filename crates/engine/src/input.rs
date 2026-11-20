//! 電子ドラムの入力。MIDI のメッセージを読み、パッドと叩き方、譜面のレーンにそろえる。
//!
//! 上流は MIDI のノートオンとベロシティしか読まないが、ここでは次も扱う。
//! - ハイハットの開き具合（CC4）。叩いた瞬間の値を打撃に添え、開閉の決め方にも使える
//! - ヘッド・リム・クロススティック、シンバルのボウ・エッジ・ベル
//! - シンバルのチョーク（ポリフォニック・アフタータッチ）
//! - 弱すぎる入力（クロストーク）と、同じパッドの短すぎる連打（二重トリガー）を捨てる

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
}

/// 物理的なパッド
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pad {
    Kick,
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
        }
    }
}

impl MidiProfile {
    /// DTXManiaNX の Config.ini のパッドの割り当て（ノート番号 → レーン）を重ねる。
    /// NX で遊べていた割り当てをそのまま使えるようにする。
    /// すでに同じレーンになるノートはそのまま（リムやエッジ、ハイハットの開き具合の区別を残す）。
    /// 違うレーンになるノートと、知らないノートは NX に合わせる。左バスドラム（LBD）はまだ扱わない
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
                DrumLane::LBD => continue,
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
    /// レーンごとのノート番号（設定の画面に出す）。ハイハットは「オープン」を表すノートを HHO にする
    pub fn lane_notes(&self) -> BTreeMap<DrumLane, Vec<u8>> {
        let mut out: BTreeMap<DrumLane, Vec<u8>> = BTreeMap::new();
        for (&note, m) in &self.notes {
            let lane = match m.pad {
                Pad::HiHat if m.hh_open => Some(DrumLane::HHO),
                Pad::HiHat => Some(DrumLane::HH),
                pad => pad_lane(pad),
            };
            if let Some(lane) = lane {
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
        match msg {
            MidiMessage::ControlChange {
                controller, value, ..
            } if controller == self.profile.hh_pedal_cc => {
                self.hh_pedal = Some(value);
                Some(InputEvent::HiHatPedal { host_us, value })
            }
            MidiMessage::PolyAftertouch { note, pressure, .. } => {
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
        assert!(!p.notes.contains_key(&35));
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
