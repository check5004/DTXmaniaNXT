//! 譜面の音の鳴らし方。BGM のほかに自動で鳴らすチップ（効果音、ドラムで遊ぶときのギターとベース）、
//! 音ごとの音量と左右（`#VOLUME` / `#PAN`）、空打ちの音（0xB1〜）と見えない音符（0x31〜）。
//! 上流 DTXManiaNX（`CStagePerfCommonScreen` と `CDTX`）のふるまいに合わせている。

use crate::lane::DrumLane;
use crate::parse::{Chart, to_base36x2};
use crate::timing::TempoMap;

/// 自動で鳴らすチップの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutoChannel {
    /// BGM（0x01）
    Bgm,
    /// 効果音 SE01〜SE32（0x61〜0x92 のうち 0x84〜0x89 を除く）。値はチャンネル番号。
    /// 0x61〜0x65（SE01〜SE05）は、同じチャンネルで次の音が鳴ると前の音を止める
    Se(u8),
    /// 0x84〜0x87。ハイハット・シンバル・ライド・左シンバルの音を自動で鳴らす
    Drum(u8),
    /// ギターのチップと 0x88。ドラムで遊ぶときは自動で鳴らし、次の音で前の音を止める
    Guitar,
    /// ベースのチップと 0x89。ギターと同じく、次の音で前の音を止める
    Bass,
}

impl AutoChannel {
    /// チップのチャンネルが自動で鳴らす音なら、その種類
    pub fn of(channel: u8) -> Option<Self> {
        Some(match channel {
            0x01 => Self::Bgm,
            0x61..=0x69 | 0x70..=0x79 | 0x80..=0x83 | 0x90..=0x92 => Self::Se(channel),
            0x84..=0x87 => Self::Drum(channel),
            0x88 => Self::Guitar,
            0x89 => Self::Bass,
            c if is_guitar(c) => Self::Guitar,
            c if is_bass(c) => Self::Bass,
            _ => return None,
        })
    }

    /// 次の音が鳴ると前の音を止める組。同じ組どうしで止める（止めないなら None）
    pub fn mute_group(self) -> Option<Self> {
        match self {
            Self::Se(0x61..=0x65) | Self::Guitar | Self::Bass => Some(self),
            _ => None,
        }
    }
}

/// ギターのチップ（上流 `EChannel.Guitar_*` のうち音の鳴るもの。ウェイリングとロングノートは除く）
fn is_guitar(c: u8) -> bool {
    matches!(c, 0x20..=0x27 | 0x93..=0x9F | 0xA9..=0xAF | 0xD0..=0xD3)
}

/// ベースのチップ（上流 `EChannel.Bass_*` のうち音の鳴るもの）
fn is_bass(c: u8) -> bool {
    matches!(
        c,
        0xA0..=0xA7 | 0xC5..=0xC6 | 0xC8..=0xCF | 0xDA..=0xDF | 0xE1..=0xE8
    )
}

/// 自動で鳴らす音 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct AutoSound {
    pub ms: f64,
    /// `#WAVzz` の番号
    pub wav: u16,
    pub channel: AutoChannel,
    /// 同じ組の次の音が鳴る時刻。この音はそこで止める
    pub cut_ms: Option<f64>,
}

/// レーンに結びついた音（空打ちの音、見えない音符）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaneSound {
    pub ms: f64,
    pub lane: DrumLane,
    pub wav: u16,
}

impl Chart {
    /// `#WAVzz` の音量（0〜100、上流 `#VOLUMEzz` / `#WAVVOLzz`）。書いていなければ 100
    pub fn wav_volume(&self, id: u16) -> u8 {
        let zz = to_base36x2(id);
        ["VOLUME", "WAVVOL"]
            .iter()
            .find_map(|k| self.field(&format!("{k}{zz}"))?.trim().parse::<i64>().ok())
            .map_or(100, |v| v.clamp(0, 100) as u8)
    }

    /// `#WAVzz` の左右（-100 が左、100 が右。上流 `#PANzz` / `#WAVPANzz`）。書いていなければ 0
    pub fn wav_pan(&self, id: u16) -> i8 {
        let zz = to_base36x2(id);
        ["PAN", "WAVPAN"]
            .iter()
            .find_map(|k| self.field(&format!("{k}{zz}"))?.trim().parse::<i64>().ok())
            .map_or(0, |v| v.clamp(-100, 100) as i8)
    }
}

impl TempoMap {
    /// 自動で鳴らす音を時刻の順に並べる（同じ時刻は譜面の並び順）。止める組の音には、次の音の時刻を入れる
    pub fn auto_sounds(&self, chart: &Chart) -> Vec<AutoSound> {
        let mut out: Vec<AutoSound> = chart
            .chips
            .iter()
            .filter_map(|c| {
                AutoChannel::of(c.channel).map(|channel| AutoSound {
                    ms: self.chip_ms(c),
                    wav: c.value,
                    channel,
                    cut_ms: None,
                })
            })
            .collect();
        out.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        let mut next: Vec<(AutoChannel, f64)> = Vec::new();
        for s in out.iter_mut().rev() {
            let Some(group) = s.channel.mute_group() else {
                continue;
            };
            match next.iter_mut().find(|(g, _)| *g == group) {
                Some((_, ms)) => {
                    s.cut_ms = Some(*ms);
                    *ms = s.ms;
                }
                None => next.push((group, s.ms)),
            }
        }
        out
    }

    /// 空打ちの音（0xB1〜0xB9、0xBC〜0xBE）。その時刻から先、そのレーンを音符の無い所で叩くとこの音を鳴らす
    pub fn empty_hit_sounds(&self, chart: &Chart) -> Vec<LaneSound> {
        self.lane_sounds(chart, |c| match c {
            0xB1..=0xB9 => DrumLane::from_channel(c - 0xB1 + 0x11),
            0xBC..=0xBE => DrumLane::from_channel(c - 0xBC + 0x1A),
            _ => None,
        })
    }

    /// 見えない音符（0x31〜0x3C）。判定はしないが、空打ちでそのレーンの近くの音を探すときに使う
    pub fn hidden_sounds(&self, chart: &Chart) -> Vec<LaneSound> {
        self.lane_sounds(chart, |c| match c {
            0x31..=0x3C => DrumLane::from_channel(c - 0x20),
            _ => None,
        })
    }

    fn lane_sounds(&self, chart: &Chart, lane: impl Fn(u8) -> Option<DrumLane>) -> Vec<LaneSound> {
        let mut out: Vec<LaneSound> = chart
            .chips
            .iter()
            .filter_map(|c| {
                lane(c.channel).map(|lane| LaneSound {
                    ms: self.chip_ms(c),
                    lane,
                    wav: c.value,
                })
            })
            .collect();
        out.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn channels() {
        assert_eq!(AutoChannel::of(0x01), Some(AutoChannel::Bgm));
        assert_eq!(AutoChannel::of(0x61), Some(AutoChannel::Se(0x61)));
        assert_eq!(AutoChannel::of(0x92), Some(AutoChannel::Se(0x92)));
        assert_eq!(AutoChannel::of(0x6A), None);
        assert_eq!(AutoChannel::of(0x84), Some(AutoChannel::Drum(0x84)));
        assert_eq!(AutoChannel::of(0x88), Some(AutoChannel::Guitar));
        assert_eq!(AutoChannel::of(0x20), Some(AutoChannel::Guitar));
        assert_eq!(AutoChannel::of(0xD3), Some(AutoChannel::Guitar));
        assert_eq!(AutoChannel::of(0xA0), Some(AutoChannel::Bass));
        assert_eq!(AutoChannel::of(0xE8), Some(AutoChannel::Bass));
        // ウェイリング、ロングノート、ドラム、見えない音符は自動では鳴らさない
        for c in [0x28, 0xA8, 0x2C, 0x2D, 0x11, 0x31, 0xB1, 0xC4] {
            assert_eq!(AutoChannel::of(c), None, "{c:#x}");
        }
        assert!(AutoChannel::Se(0x65).mute_group().is_some());
        assert!(AutoChannel::Se(0x66).mute_group().is_none());
        assert!(AutoChannel::Bgm.mute_group().is_none());
    }

    #[test]
    fn volume_and_pan() {
        let c = parse("#VOLUME01: 60\n#WAVVOL02 150\n#PAN01 -30\n#WAVPAN02: 200\n#PANEL abc\n");
        assert_eq!(c.wav_volume(1), 60);
        assert_eq!(c.wav_volume(2), 100);
        assert_eq!(c.wav_volume(3), 100);
        assert_eq!(c.wav_pan(1), -30);
        assert_eq!(c.wav_pan(2), 100);
        assert_eq!(c.wav_pan(3), 0);
    }

    #[test]
    fn muting_channels_cut_at_the_next_sound() {
        // BPM 120 で 1 小節 2000ms。SE01 は 4 分ごと、ギターは 2 分ごと、SE06 は止めない
        let c = parse("#BPM 120\n#00161: 01020304\n#00120: 0505\n#00166: 0606\n");
        let s = TempoMap::build(&c).auto_sounds(&c);
        let cut = |wav: u16| s.iter().find(|a| a.wav == wav).unwrap().cut_ms;
        assert_eq!(cut(1), Some(2500.0));
        assert_eq!(cut(3), Some(3500.0));
        assert_eq!(cut(4), None);
        let gt: Vec<_> = s.iter().filter(|a| a.wav == 5).map(|a| a.cut_ms).collect();
        assert_eq!(gt, vec![Some(3000.0), None]);
        assert!(s.iter().filter(|a| a.wav == 6).all(|a| a.cut_ms.is_none()));
    }

    #[test]
    fn lane_sounds() {
        let c = parse("#BPM 120\n#001B1: 07\n#001BC: 0008\n#00131: 09\n");
        let t = TempoMap::build(&c);
        let e = t.empty_hit_sounds(&c);
        assert_eq!(
            e,
            vec![
                LaneSound {
                    ms: 2000.0,
                    lane: DrumLane::HH,
                    wav: 7
                },
                LaneSound {
                    ms: 3000.0,
                    lane: DrumLane::LC,
                    wav: 8
                }
            ]
        );
        let h = t.hidden_sounds(&c);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].lane, DrumLane::HH);
    }
}
