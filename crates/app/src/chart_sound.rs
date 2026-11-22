//! 譜面の音の鳴らし方（上流 DTXManiaNX と同じに）。自動で鳴らす音（BGM・効果音・ドラムで遊ぶときのギターとベース）、
//! 音ごとの音量と左右（`#VOLUME` / `#PAN`）、空打ちの音（0xB1〜）と見えない音符（0x31〜）をまとめる。
//! 譜面の読み方そのものは `dtx`（`AutoChannel` など）。

use std::collections::HashMap;

use dtx::{Chart, DrumLane, LaneSound, TempoMap};

/// 自動で鳴らす音 1 つ（鳴らす形にしたもの）
#[derive(Debug, Clone, PartialEq)]
pub struct AutoVoice {
    pub ms: f64,
    /// `#WAVzz` の番号
    pub wav: u16,
    /// 音量（0〜1、`#VOLUME`）
    pub gain: f32,
    /// 左右（-1〜1、`#PAN`）
    pub pan: f32,
    /// 鳴らす長さ。同じ組の次の音が鳴るときはそこで止める（効果音 SE01〜SE05、ギター、ベース）
    pub len_ms: Option<f64>,
}

/// 1 曲の譜面の音
#[derive(Debug, Clone, Default)]
pub struct ChartSounds {
    /// 自動で鳴らす音（時刻の順）
    pub auto: Vec<AutoVoice>,
    empty_hits: Vec<LaneSound>,
    hidden: Vec<LaneSound>,
    /// 音ごとの音量と左右（どちらも既定なら入れない）
    mix: HashMap<u16, (f32, f32)>,
}

impl ChartSounds {
    pub fn new(chart: &Chart, tempo: &TempoMap) -> Self {
        let mut mix = HashMap::new();
        for c in &chart.chips {
            if mix.contains_key(&c.value) {
                continue;
            }
            let (volume, pan) = (chart.wav_volume(c.value), chart.wav_pan(c.value));
            if volume != 100 || pan != 0 {
                mix.insert(c.value, (f32::from(volume) / 100.0, f32::from(pan) / 100.0));
            }
        }
        let mut sounds = Self {
            auto: Vec::new(),
            empty_hits: tempo.empty_hit_sounds(chart),
            hidden: tempo.hidden_sounds(chart),
            mix,
        };
        sounds.auto = tempo
            .auto_sounds(chart)
            .into_iter()
            .map(|a| {
                let (gain, pan) = sounds.gain_pan(a.wav);
                AutoVoice {
                    ms: a.ms,
                    wav: a.wav,
                    gain,
                    pan,
                    len_ms: a.cut_ms.map(|cut| cut - a.ms),
                }
            })
            .collect();
        sounds
    }

    /// 自動で叩くレーンの音符（時刻、音）を、自動で鳴らす音に足す（音量と左右はその音のもの）
    pub fn add_auto_notes(&mut self, notes: impl IntoIterator<Item = (f64, u16)>) {
        let added: Vec<AutoVoice> = notes
            .into_iter()
            .map(|(ms, wav)| {
                let (gain, pan) = self.gain_pan(wav);
                AutoVoice {
                    ms,
                    wav,
                    gain,
                    pan,
                    len_ms: None,
                }
            })
            .collect();
        if added.is_empty() {
            return;
        }
        self.auto.extend(added);
        self.auto.sort_by(|a, b| a.ms.total_cmp(&b.ms));
    }

    /// 読んでおく音（自動で鳴らす音、空打ちの音、見えない音符）
    pub fn wavs(&self) -> impl Iterator<Item = u16> + '_ {
        self.auto
            .iter()
            .map(|a| a.wav)
            .chain(self.empty_hits.iter().map(|s| s.wav))
            .chain(self.hidden.iter().map(|s| s.wav))
    }

    /// 音 `wav` の音量（0〜1）と左右（-1〜1）
    pub fn gain_pan(&self, wav: u16) -> (f32, f32) {
        self.mix.get(&wav).copied().unwrap_or((1.0, 0.0))
    }

    /// 曲の時刻 `ms` にレーン `lane` を音符の無い所で叩いたときの音。空打ちの音（0xB1〜）を決めてあればそれ
    pub fn empty_hit(&self, lane: DrumLane, ms: f64) -> Option<u16> {
        self.empty_hits
            .iter()
            .take_while(|s| s.ms <= ms)
            .filter(|s| s.lane == lane)
            .last()
            .map(|s| s.wav)
    }

    /// レーン `lane` の見えない音符のうち、曲の時刻 `ms` に一番近いもの（時刻と音）
    pub fn nearest_hidden(&self, lane: DrumLane, ms: f64) -> Option<(f64, u16)> {
        self.hidden
            .iter()
            .filter(|s| s.lane == lane)
            .min_by(|a, b| (a.ms - ms).abs().total_cmp(&(b.ms - ms).abs()))
            .map(|s| (s.ms, s.wav))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sounds(text: &str) -> ChartSounds {
        let chart = dtx::parse(text);
        ChartSounds::new(&chart, &TempoMap::build(&chart))
    }

    #[test]
    fn auto_voices_carry_volume_pan_and_cut() {
        // BPM 120 で小節 1 は 2000ms から。SE01 は次の音で止める
        let s = sounds("#BPM 120\n#VOLUME02 60\n#PAN02 -50\n#00101: 01\n#00161: 0202\n");
        assert_eq!(
            s.auto,
            vec![
                AutoVoice {
                    ms: 2000.0,
                    wav: 1,
                    gain: 1.0,
                    pan: 0.0,
                    len_ms: None
                },
                AutoVoice {
                    ms: 2000.0,
                    wav: 2,
                    gain: 0.6,
                    pan: -0.5,
                    len_ms: Some(1000.0)
                },
                AutoVoice {
                    ms: 3000.0,
                    wav: 2,
                    gain: 0.6,
                    pan: -0.5,
                    len_ms: None
                },
            ]
        );
        assert_eq!(s.gain_pan(9), (1.0, 0.0));
    }

    #[test]
    fn auto_notes_join_in_time_order() {
        let mut s = sounds("#BPM 120\n#VOLUME05 50\n#00101: 01\n#00201: 02\n#00112: 0005\n");
        s.add_auto_notes([(3000.0, 5)]);
        let order: Vec<(f64, u16, f32)> = s.auto.iter().map(|a| (a.ms, a.wav, a.gain)).collect();
        assert_eq!(
            order,
            vec![(2000.0, 1, 1.0), (3000.0, 5, 0.5), (4000.0, 2, 1.0)]
        );
    }

    #[test]
    fn empty_hit_sound_takes_effect_from_its_time() {
        // 空打ちの音は小節 1 の頭（2000ms）に SD を 07、2 拍目（3000ms）に 08 にする
        let s = sounds("#BPM 120\n#001B2: 0708\n#00132: 0009\n");
        assert_eq!(s.empty_hit(DrumLane::SD, 1999.0), None);
        assert_eq!(s.empty_hit(DrumLane::SD, 2000.0), Some(7));
        assert_eq!(s.empty_hit(DrumLane::SD, 3500.0), Some(8));
        assert_eq!(s.empty_hit(DrumLane::HH, 3500.0), None);
        assert_eq!(s.nearest_hidden(DrumLane::SD, 0.0), Some((3000.0, 9)));
        assert_eq!(s.nearest_hidden(DrumLane::BD, 0.0), None);
        let mut w: Vec<u16> = s.wavs().collect();
        w.sort();
        assert_eq!(w, vec![7, 8, 9]);
    }
}
