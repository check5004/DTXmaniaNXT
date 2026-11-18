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

    /// 叩く音符を時刻順に並べる
    pub fn drum_notes(&self, chart: &Chart) -> Vec<DrumNote> {
        let mut notes: Vec<DrumNote> = chart
            .chips
            .iter()
            .filter_map(|c| {
                DrumLane::from_channel(c.channel).map(|lane| DrumNote {
                    ms: self.chip_ms(c),
                    lane,
                    wav: c.value,
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
}
