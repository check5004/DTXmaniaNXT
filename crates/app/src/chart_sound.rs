//! 譜面の音の鳴らし方（上流 DTXManiaNX と同じに）。自動で鳴らす音（BGM・効果音・ドラムで遊ぶときのギターとベース）、
//! 音ごとの音量と左右（`#VOLUME` / `#PAN`）、空打ちの音（0xB1〜）と見えない音符（0x31〜）をまとめる。
//! 譜面の読み方そのものは `dtx`（`AutoChannel` など）。

use std::collections::HashMap;

use dtx::{AutoChannel, Chart, DrumLane, LaneSound, TempoMap};

/// 自動で鳴らす音の鳴らし方（上流の BGM Sound、BGMAdjust）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoMix {
    /// BGM（0x01）を鳴らす
    pub bgm: bool,
    /// BGM と効果音（0x61〜0x92）をずらすミリ秒（プラスで遅く。上流 `t各自動再生音チップの再生時刻を変更する`）
    pub shift_ms: f64,
}

impl Default for AutoMix {
    /// そのまま鳴らす
    fn default() -> Self {
        Self {
            bgm: true,
            shift_ms: 0.0,
        }
    }
}

/// ハイハットの音の止め方（上流の Muting LP）での音の役目。
/// 上流 `tPlaySound` と同じく、閉じたハイハット（0x11、自動の 0x84）と左ペダル（0x1B）を鳴らすと、
/// それより前に鳴らしたハイハットの音（閉じた・開いた）を止める
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hat {
    /// 閉じたハイハット。前のハイハットの音を止め、自分も後の音で止められる
    Closed,
    /// 開いたハイハット。後の音で止められる
    Open,
    /// 左ペダル。前のハイハットの音を止める
    Pedal,
}

impl Hat {
    /// レーン `lane` の音の役目
    pub fn of_lane(lane: DrumLane) -> Option<Self> {
        match lane {
            DrumLane::HH => Some(Self::Closed),
            DrumLane::HHO => Some(Self::Open),
            DrumLane::LP => Some(Self::Pedal),
            _ => None,
        }
    }

    /// 鳴らすと、前に鳴らしたハイハットの音を止める
    pub fn mutes(self) -> bool {
        matches!(self, Self::Closed | Self::Pedal)
    }

    /// ハイハットの音（後の音で止められる）
    pub fn is_hat(self) -> bool {
        matches!(self, Self::Closed | Self::Open)
    }
}

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
    /// 鳴らす長さ。同じ組の次の音が鳴るときはそこで止める（効果音 SE01〜SE05、ギター、ベース、
    /// [`ChartSounds::mute_hats`] のあとのハイハット）
    pub len_ms: Option<f64>,
    /// ハイハットの音の止め方での役目（ハイハットと左ペダルの音だけ）
    pub hat: Option<Hat>,
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
    #[cfg(test)]
    pub fn new(chart: &Chart, tempo: &TempoMap) -> Self {
        Self::mixed(chart, tempo, AutoMix::default())
    }

    /// `auto` の BGM の有無・ずらしをかけて作る
    pub fn mixed(chart: &Chart, tempo: &TempoMap, auto: AutoMix) -> Self {
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
            .filter(|a| auto.bgm || a.channel != AutoChannel::Bgm)
            .map(|a| {
                let (gain, pan) = sounds.gain_pan(a.wav);
                let shift = match a.channel {
                    AutoChannel::Bgm | AutoChannel::Se(_) | AutoChannel::Drum(_) => auto.shift_ms,
                    AutoChannel::Guitar | AutoChannel::Bass => 0.0,
                };
                AutoVoice {
                    ms: a.ms + shift,
                    wav: a.wav,
                    gain,
                    pan,
                    len_ms: a.cut_ms.map(|cut| cut - a.ms),
                    // 0x84 は自動で鳴らす閉じたハイハット（上流は 0x11 と同じに扱う）
                    hat: (a.channel == AutoChannel::Drum(0x84)).then_some(Hat::Closed),
                }
            })
            .collect();
        sounds.auto.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        sounds
    }

    /// 自動で叩くレーンの音符（時刻、音、レーン）を、自動で鳴らす音に足す（音量と左右はその音のもの）
    pub fn add_auto_notes(&mut self, notes: impl IntoIterator<Item = (f64, u16, DrumLane)>) {
        let added: Vec<AutoVoice> = notes
            .into_iter()
            .map(|(ms, wav, lane)| {
                let (gain, pan) = self.gain_pan(wav);
                AutoVoice {
                    ms,
                    wav,
                    gain,
                    pan,
                    len_ms: None,
                    hat: Hat::of_lane(lane),
                }
            })
            .collect();
        if added.is_empty() {
            return;
        }
        self.auto.extend(added);
        self.auto.sort_by(|a, b| a.ms.total_cmp(&b.ms));
    }

    /// 上流の Muting LP: 自動で鳴らすハイハットの音を、後の閉じたハイハットか左ペダルの音の時刻で止める
    /// （同じ時刻の音どうしでは止めない）。自動で鳴らす音をすべて足してから呼ぶ
    pub fn mute_hats(&mut self) {
        // まだ止めていないハイハットの音（`auto` の添え字）
        let mut ringing: Vec<usize> = Vec::new();
        for i in 0..self.auto.len() {
            let (at, Some(hat)) = (self.auto[i].ms, self.auto[i].hat) else {
                continue;
            };
            if hat.mutes() {
                ringing.retain(|&j| {
                    let v = &mut self.auto[j];
                    if v.ms >= at {
                        return true;
                    }
                    let len = at - v.ms;
                    v.len_ms = Some(v.len_ms.map_or(len, |l| l.min(len)));
                    false
                });
            }
            if hat.is_hat() {
                ringing.push(i);
            }
        }
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
                    len_ms: None,
                    hat: None
                },
                AutoVoice {
                    ms: 2000.0,
                    wav: 2,
                    gain: 0.6,
                    pan: -0.5,
                    len_ms: Some(1000.0),
                    hat: None
                },
                AutoVoice {
                    ms: 3000.0,
                    wav: 2,
                    gain: 0.6,
                    pan: -0.5,
                    len_ms: None,
                    hat: None
                },
            ]
        );
        assert_eq!(s.gain_pan(9), (1.0, 0.0));
    }

    #[test]
    fn auto_notes_join_in_time_order() {
        let mut s = sounds("#BPM 120\n#VOLUME05 50\n#00101: 01\n#00201: 02\n#00112: 0005\n");
        s.add_auto_notes([(3000.0, 5, DrumLane::SD)]);
        let order: Vec<(f64, u16, f32)> = s.auto.iter().map(|a| (a.ms, a.wav, a.gain)).collect();
        assert_eq!(
            order,
            vec![(2000.0, 1, 1.0), (3000.0, 5, 0.5), (4000.0, 2, 1.0)]
        );
    }

    #[test]
    fn closed_hats_and_the_left_pedal_cut_the_hats_before_them() {
        use DrumLane::*;
        // 0x84（自動で鳴らす閉じたハイハット、音 09）は 3000ms
        let mut s = sounds("#BPM 120\n#00184: 0009\n");
        s.add_auto_notes([
            (2000.0, 1, HHO),
            (2500.0, 1, HHO),
            (3000.0, 1, HHO),
            (3500.0, 3, LP),
            (3500.0, 4, SD),
            (4000.0, 2, HH),
        ]);
        let lens = |s: &ChartSounds| -> Vec<(f64, u16, Option<f64>)> {
            s.auto.iter().map(|a| (a.ms, a.wav, a.len_ms)).collect()
        };
        // Muting LP が OFF なら止めない
        assert!(lens(&s).iter().all(|l| l.2.is_none()));
        s.mute_hats();
        let got = lens(&s);
        let len = |ms: f64, wav: u16| got.iter().find(|l| l.0 == ms && l.1 == wav).unwrap().2;
        // 開いたハイハットは開いたハイハットでは止めず、0x84 で止める
        assert_eq!(len(2000.0, 1), Some(1000.0));
        assert_eq!(len(2500.0, 1), Some(500.0));
        // 同じ時刻の音どうしは止めない。左ペダルで、開いたハイハットも 0x84 も止める
        assert_eq!(len(3000.0, 1), Some(500.0));
        assert_eq!(len(3000.0, 9), Some(500.0));
        // 左ペダル・スネア・最後のハイハットはそのまま
        assert_eq!(len(3500.0, 3), None);
        assert_eq!(len(3500.0, 4), None);
        assert_eq!(len(4000.0, 2), None);
    }

    #[test]
    fn auto_mix_scales_drops_and_shifts() {
        // BGM（01）とギター（03）は 2000ms、SE01（02）は 3000ms
        let text = "#BPM 120\n#00101: 01\n#00161: 0002\n#00120: 0300\n";
        let chart = dtx::parse(text);
        let s = ChartSounds::mixed(
            &chart,
            &TempoMap::build(&chart),
            AutoMix {
                bgm: false,
                shift_ms: 600.0,
            },
        );
        let got: Vec<(f64, u16, f32)> = s.auto.iter().map(|a| (a.ms, a.wav, a.gain)).collect();
        // BGM は鳴らさず、効果音だけ 600ms 遅らせる（ギターはずらさない）。音量は出力で系統ごとにかける
        assert_eq!(got, vec![(2000.0, 3, 1.0), (3600.0, 2, 1.0)]);
        let mut s = s;
        s.add_auto_notes([(2000.0, 4, DrumLane::SD)]);
        assert_eq!(s.auto[0].gain, 1.0);
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
