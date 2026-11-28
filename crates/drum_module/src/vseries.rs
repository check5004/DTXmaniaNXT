//! Roland V71・V51・V31 の番地表のうち、このアプリが使う所（3 機種で同じ）。
//!
//! 元は 3 機種の「MIDI Implementation」（V71 は Version 1.12、V51・V31 は Version 2.00）の
//! 「3. Parameter Address Map」。見比べて、次が 3 機種で同じことを確かめた。
//!
//! - SysEx の Model ID（`01 06 01`）と、Identity Reply の Device family code（`01 06`）
//! - Current・Kit 1〜200 の番地と、KitCommon・KitMidi・KitUnitCommon・KitUnitLayer の中の並びと大きさ
//!
//! 違うのは Identity Reply の Device family number code（[`Model::number`]）と、つなげるパッドの数
//! （V31 の AUX は 1 つ。番地表の AUX2〜4 はどの機種にもある）。音色の一覧は見比べていない。
//!
//! 本体は RQ1 で求める範囲を塊ごとに決めている（「決まった開始番地と大きさで求めること」）ので、
//! 読むときは [`Block`] ごとに求め、その中から値を取り出す。書くときは値の番地だけ DT1 で送ってよい。

use crate::nibble;
use crate::roland::{Address, Identity, ROLAND_ID, off};

/// 3 機種で同じ Model ID
pub const MODEL_ID: [u8; 3] = [0x01, 0x06, 0x01];
/// Identity Reply の Device family code（3 機種で同じ）
pub const FAMILY: [u8; 2] = [0x01, 0x06];
/// 本体の設定を変えていなければこの Device ID（17）
pub const DEFAULT_DEVICE_ID: u8 = 0x10;
/// キットの数（3 機種で同じ）
pub const KIT_COUNT: u16 = 200;

/// 機種。違いは名前と Identity の番号だけで、番地表は同じ
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Model {
    /// 画面と記録に出す名前（`V51`）
    pub name: &'static str,
    /// 曲ごとのキットなどを記録に残すときの名前（`roland-v51`）
    pub key: &'static str,
    /// Identity Reply の Device family number code
    pub number: [u8; 2],
}

pub const V71: Model = Model {
    name: "V71",
    key: "roland-v71",
    number: [0x01, 0x00],
};
pub const V51: Model = Model {
    name: "V51",
    key: "roland-v51",
    number: [0x02, 0x00],
};
pub const V31: Model = Model {
    name: "V31",
    key: "roland-v31",
    number: [0x03, 0x00],
};
/// 対応している機種
pub const MODELS: [Model; 3] = [V71, V51, V31];

/// Identity Reply の機種（対応していなければ None）
pub fn model(id: &Identity) -> Option<Model> {
    (id.manufacturer == ROLAND_ID && id.family == FAMILY)
        .then(|| MODELS.into_iter().find(|m| m.number == id.number))
        .flatten()
}

/// MIDI のポートの名前から、その機種らしいか（`V51 Generic Mode` など）
pub fn model_named(port: &str) -> Option<Model> {
    let port = port.to_ascii_lowercase();
    MODELS
        .into_iter()
        .find(|m| port.contains(&m.name.to_ascii_lowercase()))
}

/// 名前から機種（記録に残した名前を読むとき）
pub fn model_by_name(name: &str) -> Option<Model> {
    MODELS
        .into_iter()
        .find(|m| m.name.eq_ignore_ascii_case(name))
}

/// RQ1 で一度に求める塊（開始番地と大きさ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub addr: Address,
    pub size: u32,
}

impl Block {
    /// 塊の中の `offset` の番地
    pub fn at(self, offset: u32) -> Address {
        self.addr.offset(offset)
    }
}

/// 値の形
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 1 バイトに 7 ビットまで（`0aaa aaaa` など）
    Byte,
    /// 4 ビットずつ `len` バイト。符号なし
    Unsigned,
    /// 4 ビットずつ `len` バイト。負の数もある（2 の補数と見ている。[`crate::nibble`]）
    Signed,
}

/// 塊の中の 1 つの値
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// 塊の頭からの位置
    pub offset: u32,
    /// バイト数
    pub len: usize,
    pub kind: Kind,
}

impl Field {
    const fn byte(offset: u32) -> Self {
        Self {
            offset,
            len: 1,
            kind: Kind::Byte,
        }
    }

    const fn unsigned(offset: u32) -> Self {
        Self {
            offset,
            len: 4,
            kind: Kind::Unsigned,
        }
    }

    const fn signed(offset: u32) -> Self {
        Self {
            offset,
            len: 4,
            kind: Kind::Signed,
        }
    }

    /// 塊のバイト列から生のバイトを取り出す
    pub fn raw<'a>(&self, block: &'a [u8]) -> Option<&'a [u8]> {
        block.get(self.offset as usize..self.offset as usize + self.len)
    }

    /// 塊のバイト列から値を読む
    pub fn read(&self, block: &[u8]) -> Option<i32> {
        let raw = self.raw(block)?;
        Some(match self.kind {
            Kind::Byte => i32::from(raw[0] & 0x7F),
            Kind::Unsigned => nibble::decode_unsigned(raw) as i32,
            Kind::Signed => nibble::decode_signed(raw),
        })
    }

    /// 値を DT1 で送るバイト列にする
    pub fn encode(&self, value: i32) -> Vec<u8> {
        match self.kind {
            Kind::Byte => vec![(value as u8) & 0x7F],
            Kind::Unsigned | Kind::Signed => nibble::encode(value, self.len),
        }
    }
}

/// パッドの音の単位（[KitUnitCommon] と [KitUnitLayer] の番号、1〜28）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Unit {
    Kick = 1,
    SnareHead,
    SnareRim,
    Tom1Head,
    Tom1Rim,
    Tom2Head,
    Tom2Rim,
    Tom3Head,
    Tom3Rim,
    Tom4Head,
    Tom4Rim,
    HiHatHead,
    HiHatRim,
    Crash1Head,
    Crash1Rim,
    Crash2Head,
    Crash2Rim,
    RideHead,
    RideEdge,
    RideBell,
    Aux1Head,
    Aux1Rim,
    Aux2Head,
    Aux2Rim,
    Aux3Head,
    Aux3Rim,
    Aux4Head,
    Aux4Rim,
}

impl Unit {
    pub const ALL: [Unit; 28] = [
        Unit::Kick,
        Unit::SnareHead,
        Unit::SnareRim,
        Unit::Tom1Head,
        Unit::Tom1Rim,
        Unit::Tom2Head,
        Unit::Tom2Rim,
        Unit::Tom3Head,
        Unit::Tom3Rim,
        Unit::Tom4Head,
        Unit::Tom4Rim,
        Unit::HiHatHead,
        Unit::HiHatRim,
        Unit::Crash1Head,
        Unit::Crash1Rim,
        Unit::Crash2Head,
        Unit::Crash2Rim,
        Unit::RideHead,
        Unit::RideEdge,
        Unit::RideBell,
        Unit::Aux1Head,
        Unit::Aux1Rim,
        Unit::Aux2Head,
        Unit::Aux2Rim,
        Unit::Aux3Head,
        Unit::Aux3Rim,
        Unit::Aux4Head,
        Unit::Aux4Rim,
    ];

    /// 番地表の番号（1〜28）
    pub fn number(self) -> u32 {
        self as u32
    }

    /// 本体の表示に合わせた名前（記録用）
    pub fn label(self) -> &'static str {
        match self {
            Unit::Kick => "KICK",
            Unit::SnareHead => "SNARE HEAD",
            Unit::SnareRim => "SNARE RIM",
            Unit::Tom1Head => "TOM1 HEAD",
            Unit::Tom1Rim => "TOM1 RIM",
            Unit::Tom2Head => "TOM2 HEAD",
            Unit::Tom2Rim => "TOM2 RIM",
            Unit::Tom3Head => "TOM3 HEAD",
            Unit::Tom3Rim => "TOM3 RIM",
            Unit::Tom4Head => "TOM4 HEAD",
            Unit::Tom4Rim => "TOM4 RIM",
            Unit::HiHatHead => "HI-HAT HEAD",
            Unit::HiHatRim => "HI-HAT RIM",
            Unit::Crash1Head => "CRASH1 HEAD",
            Unit::Crash1Rim => "CRASH1 RIM",
            Unit::Crash2Head => "CRASH2 HEAD",
            Unit::Crash2Rim => "CRASH2 RIM",
            Unit::RideHead => "RIDE HEAD",
            Unit::RideEdge => "RIDE EDGE",
            Unit::RideBell => "RIDE BELL",
            Unit::Aux1Head => "AUX1 HEAD",
            Unit::Aux1Rim => "AUX1 RIM",
            Unit::Aux2Head => "AUX2 HEAD",
            Unit::Aux2Rim => "AUX2 RIM",
            Unit::Aux3Head => "AUX3 HEAD",
            Unit::Aux3Rim => "AUX3 RIM",
            Unit::Aux4Head => "AUX4 HEAD",
            Unit::Aux4Rim => "AUX4 RIM",
        }
    }

    /// [KitMidi] の中のノート番号の位置。ハイハットはオープンのもの、ライドのエッジは「Ride Rim」
    pub fn note_field(self) -> Field {
        let at = match self {
            Unit::Kick => off(0, 0x00, 0x00),
            Unit::SnareHead => off(0, 0x00, 0x04),
            Unit::SnareRim => off(0, 0x00, 0x08),
            Unit::Tom1Head => off(0, 0x00, 0x2C),
            Unit::Tom1Rim => off(0, 0x00, 0x30),
            Unit::Tom2Head => off(0, 0x00, 0x34),
            Unit::Tom2Rim => off(0, 0x00, 0x38),
            Unit::Tom3Head => off(0, 0x00, 0x3C),
            Unit::Tom3Rim => off(0, 0x00, 0x40),
            Unit::Tom4Head => off(0, 0x00, 0x44),
            Unit::Tom4Rim => off(0, 0x00, 0x48),
            Unit::HiHatHead => off(0, 0x00, 0x4C),
            Unit::HiHatRim => off(0, 0x00, 0x50),
            Unit::Crash1Head => off(0, 0x00, 0x60),
            Unit::Crash1Rim => off(0, 0x00, 0x64),
            Unit::Crash2Head => off(0, 0x00, 0x68),
            Unit::Crash2Rim => off(0, 0x00, 0x6C),
            Unit::RideHead => off(0, 0x00, 0x70),
            Unit::RideEdge => off(0, 0x00, 0x74),
            Unit::RideBell => off(0, 0x00, 0x78),
            Unit::Aux1Head => off(0, 0x00, 0x7C),
            Unit::Aux1Rim => off(0, 0x01, 0x00),
            Unit::Aux2Head => off(0, 0x01, 0x04),
            Unit::Aux2Rim => off(0, 0x01, 0x08),
            Unit::Aux3Head => off(0, 0x01, 0x0C),
            Unit::Aux3Rim => off(0, 0x01, 0x10),
            Unit::Aux4Head => off(0, 0x01, 0x14),
            Unit::Aux4Rim => off(0, 0x01, 0x18),
        };
        Field::unsigned(at)
    }

    /// [KitMidi] の中のチャンネルの位置（0〜15 が 1〜16 ch、16 は GLOBAL）
    pub fn channel_field(self) -> Field {
        Field::byte(off(0, 0x01, 0x38) + self.number() - 1)
    }
}

/// [KitMidi] のノート番号の「OFF」
pub const NOTE_OFF: i32 = 128;
/// [KitMidi] のチャンネルの「GLOBAL」（本体の SYSTEM MIDI の Global MIDI Channel を使う）
pub const CHANNEL_GLOBAL: i32 = 16;
/// チャンネルが GLOBAL のときに送るチャンネル（工場出荷の Global MIDI Channel、10 ch）
pub const GLOBAL_CHANNEL_DEFAULT: u8 = 9;
/// [KitMidi] の中の、ハイハットを閉じて叩いたときのノート番号（Note Hi-Hat Close Head）
pub const HH_CLOSE_NOTE: Field = Field::unsigned(off(0, 0x00, 0x54));
/// [KitMidi] の中の、ハイハットのペダルを踏んだときのノート番号（Note Hi-Hat Closed Pedal）
pub const HH_PEDAL_NOTE: Field = Field::unsigned(off(0, 0x00, 0x5C));
/// 工場出荷の設定で Hi-Hat Pedal CC になっている CC（Foot Controller）。0 で開き、127 で閉じる
pub const HH_PEDAL_CC: u8 = 4;

/// レイヤー
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    A,
    B,
    C,
}

impl Layer {
    fn base(self) -> u32 {
        match self {
            Layer::A => off(0, 0x50, 0),
            Layer::B => off(0x01, 0x10, 0),
            Layer::C => off(0x01, 0x50, 0),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layer::A => "A",
            Layer::B => "B",
            Layer::C => "C",
        }
    }
}

/// [Current]。今のキット
pub fn current() -> Block {
    Block {
        addr: Address::new(0, 0, 0, 0),
        size: 4,
    }
}

/// [Current] の KitNum（0〜199 が キット 1〜200）
pub const KIT_NUM: Field = Field::unsigned(0);

/// キット `kit`（1〜200）の頭の番地
pub fn kit_base(kit: u16) -> Address {
    let kit = u32::from(kit.clamp(1, KIT_COUNT));
    Address::new(0x04, 0, 0, 0).offset((kit - 1) * off(0x04, 0, 0))
}

/// [KitCommon]。キットの名前など
pub fn kit_common(kit: u16) -> Block {
    Block {
        addr: kit_base(kit),
        size: off(0, 0x01, 0x41),
    }
}

/// [KitCommon] のキットの名前（16 文字）
pub const KIT_NAME: Field = Field {
    offset: 0,
    len: 16,
    kind: Kind::Byte,
};

/// キットの名前を読む（後ろの空白は除く）
pub fn kit_name(block: &[u8]) -> Option<String> {
    let raw = KIT_NAME.raw(block)?;
    let s: String = raw
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect();
    Some(s.trim_end().to_owned())
}

/// [KitMidi]。パッドごとのノート番号とチャンネル
pub fn kit_midi(kit: u16) -> Block {
    Block {
        addr: kit_base(kit).offset(off(0, 0x02, 0)),
        size: off(0, 0x01, 0x54),
    }
}

/// [KitUnitCommon]。パッドの音の単位ごとの音量など
pub fn unit_common(kit: u16, unit: Unit) -> Block {
    Block {
        addr: kit_base(kit).offset(off(0, 0x20, 0) + (unit.number() - 1) * off(0, 0x01, 0)),
        size: 0x34,
    }
}

/// [KitUnitCommon] の音量（-601 が -INF、-600〜60 が -60.0〜+6.0 dB）
pub const UNIT_VOLUME: Field = Field::signed(0x02);

/// [KitUnitLayer]。レイヤーごとの音色と音量
pub fn unit_layer(kit: u16, unit: Unit, layer: Layer) -> Block {
    Block {
        addr: kit_base(kit).offset(layer.base() + (unit.number() - 1) * off(0, 0x02, 0)),
        size: off(0, 0x01, 0x51),
    }
}

/// [KitUnitLayer] の Inst Switch（0 が OFF、1 が ON）
pub const LAYER_SWITCH: Field = Field::byte(0x00);
/// [KitUnitLayer] の Instrument（音色の番号）
pub const LAYER_INSTRUMENT: Field = Field::unsigned(0x01);
/// [KitUnitLayer] の Inst Bank
pub const LAYER_BANK: Field = Field::unsigned(0x05);
/// [KitUnitLayer] の音量（[`UNIT_VOLUME`] と同じ形）
pub const LAYER_VOLUME: Field = Field::signed(0x09);

/// 音量の -INF
pub const VOLUME_OFF: i32 = -601;
/// 音量の上限（+6.0 dB）
pub const VOLUME_MAX: i32 = 60;

/// 音量の値を dB にする（-INF は `None`）
pub fn volume_db(value: i32) -> Option<f32> {
    (value > VOLUME_OFF).then_some(value as f32 / 10.0)
}

/// 音量の値を記録用の文字にする（`-INF`、`+0.0 dB` など）
pub fn volume_label(value: i32) -> String {
    match volume_db(value) {
        Some(db) => format!("{db:+.1} dB"),
        None => "-INF".to_owned(),
    }
}

/// 読んだ音量が番地表の範囲に入っているか（負の数の表し方の推測が合っているかの目安にもなる）
pub fn volume_in_range(value: i32) -> bool {
    (VOLUME_OFF..=VOLUME_MAX).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roland::{dt1, rq1};

    #[test]
    fn kit_addresses_match_the_map() {
        assert_eq!(kit_base(1), Address::new(0x04, 0, 0, 0));
        assert_eq!(kit_base(2), Address::new(0x04, 0x04, 0, 0));
        assert_eq!(kit_base(200), Address::new(0x0A, 0x1C, 0, 0));
        assert_eq!(kit_midi(1).addr, Address::new(0x04, 0, 0x02, 0));
        assert_eq!(kit_midi(1).size, 0xD4);
        assert_eq!(kit_common(1).size, 0xC1);
    }

    #[test]
    fn tom1_head_addresses() {
        // キット 1 の TOM1 HEAD の音量は 04 00 23 02〜05
        let common = unit_common(1, Unit::Tom1Head);
        assert_eq!(common.addr, Address::new(0x04, 0, 0x23, 0));
        assert_eq!(
            common.at(UNIT_VOLUME.offset),
            Address::new(0x04, 0, 0x23, 0x02)
        );
        // レイヤー A は 04 00 56 00、音色は 01〜04
        let layer = unit_layer(1, Unit::Tom1Head, Layer::A);
        assert_eq!(layer.addr, Address::new(0x04, 0, 0x56, 0));
        assert_eq!(layer.size, 0xD1);
        // 28 番目のレイヤーはそれぞれ 01 06 00、01 46 00、02 06 00
        assert_eq!(
            unit_layer(1, Unit::Aux4Rim, Layer::A).addr,
            Address::new(0x04, 0x01, 0x06, 0)
        );
        assert_eq!(
            unit_layer(1, Unit::Aux4Rim, Layer::B).addr,
            Address::new(0x04, 0x01, 0x46, 0)
        );
        assert_eq!(
            unit_layer(1, Unit::Aux4Rim, Layer::C).addr,
            Address::new(0x04, 0x02, 0x06, 0)
        );
        assert_eq!(
            unit_common(1, Unit::Aux4Rim).addr,
            Address::new(0x04, 0, 0x3B, 0)
        );
    }

    #[test]
    fn manual_example_layer_eq_matches_the_layer_address() {
        // 番地表の例 1 は SNARE HEAD のレイヤー A の +21
        let layer = unit_layer(1, Unit::SnareHead, Layer::A);
        assert_eq!(
            dt1(0x10, &MODEL_ID, layer.at(0x21), &[1]),
            vec![
                0xF0, 0x41, 0x10, 0x01, 0x06, 0x01, 0x12, 0x04, 0x00, 0x52, 0x21, 0x01, 0x08, 0xF7
            ]
        );
    }

    #[test]
    fn midi_fields() {
        assert_eq!(Unit::Kick.channel_field().offset, off(0, 0x01, 0x38));
        assert_eq!(Unit::Tom1Head.channel_field().offset, off(0, 0x01, 0x3B));
        assert_eq!(Unit::RideHead.channel_field().offset, off(0, 0x01, 0x49));
        assert_eq!(Unit::Aux4Rim.channel_field().offset, off(0, 0x01, 0x53));
        // 最後のチャンネルは塊の終わり（01 54）のすぐ前
        assert_eq!(Unit::Aux4Rim.channel_field().offset + 1, kit_midi(1).size);
        let mut block = vec![0u8; kit_midi(1).size as usize];
        // TOM1 HEAD のノート 48（00 00 03 00）と 10 ch（09）
        block[0x2C..0x30].copy_from_slice(&[0, 0, 3, 0]);
        block[Unit::Tom1Head.channel_field().offset as usize] = 9;
        assert_eq!(Unit::Tom1Head.note_field().read(&block), Some(48));
        assert_eq!(Unit::Tom1Head.channel_field().read(&block), Some(9));
        assert_eq!(Unit::Aux4Rim.note_field().offset + 4, off(0, 0x01, 0x1C));
        // ハイハットはオープン（00 4C）、クローズ（00 54）、ペダル（00 5C）の順
        assert_eq!(Unit::HiHatHead.note_field().offset, off(0, 0x00, 0x4C));
        assert_eq!(HH_CLOSE_NOTE.offset, off(0, 0x00, 0x54));
        assert_eq!(HH_PEDAL_NOTE.offset, off(0, 0x00, 0x5C));
        assert_eq!(
            HH_PEDAL_NOTE.offset + 4,
            Unit::Crash1Head.note_field().offset
        );
    }

    #[test]
    fn volume_values() {
        let mut block = vec![0u8; 0x34];
        block[2..6].copy_from_slice(&UNIT_VOLUME.encode(VOLUME_OFF));
        assert_eq!(UNIT_VOLUME.read(&block), Some(VOLUME_OFF));
        assert_eq!(volume_label(VOLUME_OFF), "-INF");
        assert_eq!(volume_label(0), "+0.0 dB");
        assert_eq!(volume_label(-125), "-12.5 dB");
        assert!(volume_in_range(60) && !volume_in_range(61));
        assert_eq!(LAYER_SWITCH.encode(1), vec![1]);
    }

    #[test]
    fn reading_the_current_kit() {
        assert_eq!(
            rq1(0x10, &MODEL_ID, current().addr, current().size),
            vec![
                0xF0, 0x41, 0x10, 0x01, 0x06, 0x01, 0x11, 0, 0, 0, 0, 0, 0, 0, 4, 0x7C, 0xF7
            ]
        );
        assert_eq!(KIT_NUM.read(&[0, 0, 0x0C, 0x07]), Some(199));
        let mut name = b"Studio          ".to_vec();
        name.resize(0xC1, 0);
        assert_eq!(kit_name(&name).as_deref(), Some("Studio"));
    }

    #[test]
    fn identifies_the_models() {
        let id = Identity {
            device: 0x10,
            manufacturer: ROLAND_ID,
            family: FAMILY,
            number: [0x02, 0x00],
            revision: [0, 2, 0, 0],
        };
        assert_eq!(model(&id), Some(V51));
        // MIDI Implementation の Identity Reply: V71 は 01 00、V31 は 03 00
        assert_eq!(
            model(&Identity {
                number: [0x01, 0x00],
                ..id
            }),
            Some(V71)
        );
        assert_eq!(
            model(&Identity {
                number: [0x03, 0x00],
                ..id
            }),
            Some(V31)
        );
        assert_eq!(
            model(&Identity {
                number: [0x04, 0x00],
                ..id
            }),
            None
        );
        assert_eq!(
            model(&Identity {
                family: [0x01, 0x05],
                ..id
            }),
            None
        );
        assert_eq!(
            model(&Identity {
                manufacturer: 0x43,
                ..id
            }),
            None
        );
    }

    #[test]
    fn models_by_port_name() {
        assert_eq!(model_named("V51 Generic Mode"), Some(V51));
        assert_eq!(model_named("MIDIIN2 (V71)"), Some(V71));
        assert_eq!(model_named("v31"), Some(V31));
        assert_eq!(model_named("FGDP-30"), None);
        assert_eq!(model_by_name("v51"), Some(V51));
        assert_eq!(model_by_name(""), None);
    }
}
