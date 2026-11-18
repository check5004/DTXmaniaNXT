//! ドラムのレーン（チャンネル 0x11〜0x1C）。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DrumLane {
    /// ハイハット（クローズ）0x11
    HH,
    /// スネア 0x12
    SD,
    /// バスドラム 0x13
    BD,
    /// ハイタム 0x14
    HT,
    /// ロータム 0x15
    LT,
    /// 右シンバル 0x16
    CY,
    /// フロアタム 0x17
    FT,
    /// ハイハット（オープン）0x18
    HHO,
    /// ライド 0x19
    RD,
    /// 左シンバル 0x1A
    LC,
    /// 左ペダル 0x1B
    LP,
    /// 左バスドラム 0x1C
    LBD,
}

impl DrumLane {
    pub const ALL: [DrumLane; 12] = [
        Self::HH,
        Self::SD,
        Self::BD,
        Self::HT,
        Self::LT,
        Self::CY,
        Self::FT,
        Self::HHO,
        Self::RD,
        Self::LC,
        Self::LP,
        Self::LBD,
    ];

    pub fn from_channel(ch: u8) -> Option<Self> {
        match ch {
            0x11..=0x1C => Some(Self::ALL[usize::from(ch - 0x11)]),
            _ => None,
        }
    }

    pub fn channel(self) -> u8 {
        0x11 + self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::HH => "HH",
            Self::SD => "SD",
            Self::BD => "BD",
            Self::HT => "HT",
            Self::LT => "LT",
            Self::CY => "CY",
            Self::FT => "FT",
            Self::HHO => "HHO",
            Self::RD => "RD",
            Self::LC => "LC",
            Self::LP => "LP",
            Self::LBD => "LBD",
        }
    }

    /// [`DrumLane::name`] の逆
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for lane in DrumLane::ALL {
            assert_eq!(DrumLane::from_channel(lane.channel()), Some(lane));
        }
        assert_eq!(DrumLane::from_channel(0x1A), Some(DrumLane::LC));
        assert_eq!(DrumLane::from_channel(0x1D), None);
        for lane in DrumLane::ALL {
            assert_eq!(DrumLane::from_name(lane.name()), Some(lane));
        }
        assert_eq!(DrumLane::from_name("XX"), None);
    }
}
