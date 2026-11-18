//! 小節・拍・時刻の変換。1 小節 = 4 拍 × 小節長。BPM は ch03（`#BASEBPM` + 16 進値）と
//! ch08（`#BASEBPM` + `#BPMzz`）で変わる。

use crate::ch;
use crate::lane::DrumLane;
use crate::number::parse_nx_number;
use crate::parse::{Chart, Chip, to_base36x2};

#[derive(Debug, Clone, Copy)]
struct BpmEvent {
    beat: f64,
    bpm: f64,
    ms: f64,
}

/// テンポの地図。拍から時刻（ミリ秒）を求める
#[derive(Debug, Clone)]
pub struct TempoMap {
    /// 各小節の先頭の拍（`measure_start[m]`）。最後の要素は「最後の小節の終わり」
    measure_start: Vec<f64>,
    bar_len: Vec<f64>,
    events: Vec<BpmEvent>,
}

/// 叩く音符 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct DrumNote {
    pub ms: f64,
    pub lane: DrumLane,
    /// `#WAVzz` の番号
    pub wav: u16,
    /// 譜面上の位置（小節と、小節の中の位置）。記録で音符を見分けるのに使う
    pub at: NotePos,
    /// 曲頭からの拍
    pub beat: f64,
}

/// 譜面上の位置。小節 `measure` の `num / den`（約分済み、0 以上 1 未満）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NotePos {
    pub measure: u32,
    pub num: u32,
    pub den: u32,
}

impl NotePos {
    /// チップの位置を約分して持つ
    pub fn of(chip: &Chip) -> Self {
        Self::new(chip.measure, chip.pos, chip.div)
    }

    pub fn new(measure: u32, num: u32, den: u32) -> Self {
        let den = den.max(1);
        let g = gcd(num, den);
        Self {
            measure,
            num: num / g,
            den: den / g,
        }
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

fn initial_bpm(chart: &Chart) -> f64 {
    chart
        .field("BPM")
        .or_else(|| chart.field("BPM00"))
        .and_then(parse_nx_number)
        .filter(|v| *v > 0.0)
        .unwrap_or(120.0)
}

fn chip_bpm(chip: &Chip, chart: &Chart, base: f64) -> Option<f64> {
    match chip.channel {
        ch::BPM => Some(base + f64::from(chip.value)),
        ch::BPM_EX => chart
            .field(&format!("BPM{}", to_base36x2(chip.value)))
            .and_then(parse_nx_number)
            .map(|v| base + v),
        _ => None,
    }
}

impl TempoMap {
    pub fn build(chart: &Chart) -> Self {
        let last = chart
            .chips
            .iter()
            .map(|c| c.measure)
            .chain(chart.bar_lengths.keys().copied())
            .max()
            .unwrap_or(0) as usize;
        let mut bar_len = Vec::with_capacity(last + 1);
        let mut measure_start = Vec::with_capacity(last + 2);
        let mut cur = 1.0;
        let mut beat = 0.0;
        for m in 0..=last {
            if let Some(&v) = chart.bar_lengths.get(&(m as u32)) {
                cur = v;
            }
            bar_len.push(cur);
            measure_start.push(beat);
            beat += 4.0 * cur;
        }
        measure_start.push(beat);

        let mut map = TempoMap {
            measure_start,
            bar_len,
            events: Vec::new(),
        };
        let base = chart
            .field("BASEBPM")
            .and_then(parse_nx_number)
            .filter(|v| *v > 0.0)
            .unwrap_or(0.0);
        let mut raw: Vec<(f64, f64)> = chart
            .chips
            .iter()
            .filter_map(|c| {
                chip_bpm(c, chart, base)
                    .filter(|b| *b > 0.0)
                    .map(|b| (map.beat_of(c), b))
            })
            .collect();
        // 同じ位置は譜面の並び順で後のものが有効（安定ソート）
        raw.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut events = vec![BpmEvent {
            beat: 0.0,
            bpm: initial_bpm(chart),
            ms: 0.0,
        }];
        for (beat, bpm) in raw {
            let last = *events.last().expect("events is never empty");
            if (beat - last.beat).abs() < 1e-9 {
                events.last_mut().expect("events is never empty").bpm = bpm;
                continue;
            }
            let ms = last.ms + (beat - last.beat) * 60_000.0 / last.bpm;
            events.push(BpmEvent { beat, bpm, ms });
        }
        map.events = events;
        map
    }

    /// チップの位置（曲頭からの拍）
    pub fn beat_of(&self, chip: &Chip) -> f64 {
        let m = (chip.measure as usize).min(self.bar_len.len().saturating_sub(1));
        let frac = f64::from(chip.pos) / f64::from(chip.div.max(1));
        self.measure_start.get(m).copied().unwrap_or(0.0)
            + frac * 4.0 * self.bar_len.get(m).copied().unwrap_or(1.0)
    }

    /// 拍 → 曲頭からのミリ秒
    pub fn beat_to_ms(&self, beat: f64) -> f64 {
        let i = self
            .events
            .partition_point(|e| e.beat <= beat)
            .saturating_sub(1);
        let e = self.events[i];
        e.ms + (beat - e.beat) * 60_000.0 / e.bpm
    }

    pub fn chip_ms(&self, chip: &Chip) -> f64 {
        self.beat_to_ms(self.beat_of(chip))
    }

    /// 小節の数（チップか小節長の変更がある最後の小節まで。小節 0 を含む）
    pub fn measure_count(&self) -> usize {
        self.bar_len.len()
    }

    /// 小節 `m` の頭の拍。最後の小節の次（`measure_count()`）を渡すと曲の終わりの拍
    pub fn measure_start_beat(&self, m: usize) -> f64 {
        let last = self.measure_start.len() - 1;
        self.measure_start[m.min(last)]
    }

    /// 小節 `m` の頭の時刻（ミリ秒）
    pub fn measure_start_ms(&self, m: usize) -> f64 {
        self.beat_to_ms(self.measure_start_beat(m))
    }

    /// 叩く音符を時刻順に並べる
    pub fn drum_notes(&self, chart: &Chart) -> Vec<DrumNote> {
        let mut notes: Vec<DrumNote> = chart
            .chips
            .iter()
            .filter_map(|c| {
                DrumLane::from_channel(c.channel).map(|lane| {
                    let beat = self.beat_of(c);
                    DrumNote {
                        ms: self.beat_to_ms(beat),
                        lane,
                        wav: c.value,
                        at: NotePos::of(c),
                        beat,
                    }
                })
            })
            .collect();
        notes.sort_by(|a, b| a.ms.total_cmp(&b.ms).then(a.lane.cmp(&b.lane)));
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn base_bpm_adds_to_changes() {
        // BASEBPM 100 + ch03 の 0x14(20) = 120
        let c = parse("#BPM 60\n#BASEBPM 100\n#00103: 14\n#00113: 0101\n");
        let t = TempoMap::build(&c);
        let notes = t.drum_notes(&c);
        // 小節 0 は BPM 60 で 4 拍 = 4000ms、小節 1 は BPM 120
        assert!((notes[0].ms - 4000.0).abs() < 1e-6);
        assert!((notes[1].ms - 5000.0).abs() < 1e-6);
    }

    #[test]
    fn note_positions_are_reduced() {
        // 小節 2 の 16 分割の 4 番目と 0 番目 = 1/4 と 0
        let c = parse("#BPM 120\n#00211: 00000000010000000000000000000000\n#00212: 01\n");
        let t = TempoMap::build(&c);
        let notes = t.drum_notes(&c);
        assert_eq!(notes[0].at, NotePos::new(2, 0, 1));
        assert_eq!(notes[1].at, NotePos::new(2, 1, 4));
        assert_eq!((notes[1].at.num, notes[1].at.den), (1, 4));
        // 小節 2 の頭は 8 拍目、1/4 は 9 拍目
        assert!((notes[1].beat - 9.0).abs() < 1e-9);
        assert_eq!(t.measure_count(), 3);
        assert!((t.measure_start_ms(2) - 4000.0).abs() < 1e-6);
        // 最後の小節の次は曲の終わり
        assert!((t.measure_start_ms(3) - 6000.0).abs() < 1e-6);
    }
}
