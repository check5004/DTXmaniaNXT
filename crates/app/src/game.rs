//! 1 曲を遊ぶ。譜面・音・判定・入力をまとめ、曲の時刻で動かす。

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dtx::{Chart, DrumLane, TempoMap};
use engine::clock::MidiTimestampMapper;
use engine::input::{Articulation, InputEvent, MidiMessage, MidiProfile, Normalizer, Pad};
use engine::judge::{Hit, HitWindows, Judge, JudgeEvent, Judgement, LaneGroups, Tally};

use crate::audio::{AudioOut, Sound, load_sound};
use crate::midi::RawMidi;
use crate::time::host_us;

/// 曲が始まる前の空白（ミリ秒）
const LEAD_IN_MS: f64 = 2000.0;

/// レーンごとの最後の判定（画面に出す）
#[derive(Debug, Clone, Copy)]
pub struct Flash {
    pub judgement: Judgement,
    pub offset_ms: Option<f64>,
    pub song_ms: f64,
}

/// 最後に届いた入力（電子ドラムの確認用に画面に出す）
#[derive(Debug, Clone)]
pub struct LastInput {
    pub source: String,
    pub pad: Option<Pad>,
    pub articulation: Option<Articulation>,
    pub lane: Option<DrumLane>,
    pub velocity: u8,
    pub hh_pedal: Option<u8>,
}

/// 1 曲を遊び終えたときの成績
#[derive(Debug, Clone)]
pub struct Outcome {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub level: Option<f64>,
    pub notes: usize,
    pub tally: Tally,
    /// 叩けた音符のずれ（ミリ秒、マイナスは早い）の平均と標準偏差
    pub offset_mean_ms: Option<f64>,
    pub offset_sd_ms: Option<f64>,
    /// 最後まで遊んだ（途中でやめたら false）
    pub cleared: bool,
}

pub struct Play {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub level: Option<f64>,
    pub judge: Judge,
    pub audio: Option<AudioOut>,
    pub end_ms: f64,
    /// 入力の遅れの補正。叩いた時刻からこの分を引く
    pub input_offset_ms: f64,
    /// 叩いたときに譜面の音を鳴らすか（モジュールの音を使うなら切る）
    pub drum_sounds: bool,
    pub flashes: HashMap<DrumLane, Flash>,
    pub last_input: Option<LastInput>,
    pub hh_pedal: Option<u8>,
    /// 叩けた音符のずれ（ミリ秒）
    hit_offsets: Vec<f64>,
    sounds: HashMap<u16, Sound>,
    normalizer: Normalizer,
    mappers: HashMap<String, MidiTimestampMapper>,
    /// 音が出ないときの時計の起点（ホスト時刻）
    fallback_origin_us: i64,
}

impl Play {
    /// 譜面を読んで始める。`input_offset_ms` と `drum_sounds` は設定から
    pub fn load(path: &Path, input_offset_ms: f64, drum_sounds: bool) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let chart = dtx::load(&bytes);
        for w in &chart.warnings {
            log::warn!("{}:{}: {}", path.display(), w.line, w.message);
        }
        let tempo = TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        let end_ms = chart
            .chips
            .iter()
            .map(|c| tempo.chip_ms(c))
            .fold(0.0, f64::max)
            + 2000.0;

        let mut audio = match AudioOut::open(-LEAD_IN_MS) {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("audio is off: {e:#}");
                None
            }
        };
        let sounds = match &audio {
            Some(a) => load_sounds(&chart, path.parent().unwrap_or(Path::new(".")), a.rate()),
            None => HashMap::new(),
        };
        if let Some(a) = audio.as_mut() {
            // BGM（ch01）は決まった時刻に鳴らす
            for chip in chart.chips.iter().filter(|c| c.channel == dtx::ch::BGM) {
                if let Some(s) = sounds.get(&chip.value) {
                    a.play_at(s, tempo.chip_ms(chip), 1.0);
                }
            }
        }

        log::info!(
            "{}: {} notes, {:.1}s",
            path.display(),
            notes.len(),
            end_ms / 1000.0
        );
        Ok(Self {
            path: path.to_owned(),
            title: chart.title().to_owned(),
            artist: chart.artist().to_owned(),
            level: chart.drum_level(),
            judge: Judge::new(notes, HitWindows::default(), LaneGroups::default()),
            audio,
            end_ms,
            input_offset_ms,
            drum_sounds,
            flashes: HashMap::new(),
            last_input: None,
            hh_pedal: None,
            hit_offsets: Vec::new(),
            sounds,
            normalizer: Normalizer::new(MidiProfile::roland_vdrums()),
            mappers: HashMap::new(),
            fallback_origin_us: host_us(),
        })
    }

    /// ホスト時刻 `host_us` の曲の時刻
    pub fn song_ms_at(&self, at_us: i64) -> f64 {
        self.audio
            .as_ref()
            .and_then(|a| a.song_ms_at(at_us))
            .unwrap_or_else(|| (at_us - self.fallback_origin_us) as f64 / 1000.0 - LEAD_IN_MS)
    }

    pub fn now_ms(&self) -> f64 {
        self.song_ms_at(host_us())
    }

    pub fn finished(&self) -> bool {
        self.now_ms() > self.end_ms
    }

    /// 時間を進め、見逃しを判定する
    pub fn update(&mut self) {
        let now = self.now_ms();
        let misses = self.judge.advance(now);
        self.show(&misses, now);
    }

    pub fn on_midi(&mut self, raw: RawMidi) {
        let Some(msg) = MidiMessage::parse(&raw.bytes) else {
            return;
        };
        let mapper = self.mappers.entry(raw.port.clone()).or_default();
        let at_us = mapper.map(Some(raw.device_us), raw.arrival_us);
        match self.normalizer.feed(msg, at_us) {
            Some(InputEvent::Hit {
                host_us,
                lane,
                pad,
                articulation,
                velocity,
                hh_pedal,
            }) => {
                self.last_input = Some(LastInput {
                    source: raw.port,
                    pad: Some(pad),
                    articulation: Some(articulation),
                    lane,
                    velocity,
                    hh_pedal,
                });
                if let Some(lane) = lane {
                    self.hit(lane, velocity, host_us);
                }
            }
            Some(InputEvent::HiHatPedal { value, .. }) => self.hh_pedal = Some(value),
            Some(InputEvent::Choke { .. }) | None => {}
        }
    }

    /// キーボードで叩いた（電子ドラムが無いときの確認用）
    pub fn on_key(&mut self, lane: DrumLane, at_us: i64) {
        self.last_input = Some(LastInput {
            source: "keyboard".into(),
            pad: None,
            articulation: None,
            lane: Some(lane),
            velocity: 100,
            hh_pedal: None,
        });
        self.hit(lane, 100, at_us);
    }

    fn hit(&mut self, lane: DrumLane, velocity: u8, at_us: i64) {
        let ms = self.song_ms_at(at_us) - self.input_offset_ms;
        let events = self.judge.hit(Hit { ms, lane, velocity });
        if self.drum_sounds {
            // 取った音符の音を鳴らす。空打ちなら、そのレーンの一番近い音符の音
            let wav = events
                .first()
                .map(|e| self.judge.notes()[e.note].wav)
                .or_else(|| self.nearest_wav(lane, ms));
            if let (Some(a), Some(s)) = (self.audio.as_mut(), wav.and_then(|w| self.sounds.get(&w)))
            {
                a.play_now(s, f32::from(velocity) / 127.0);
            }
        }
        self.show(&events, ms);
    }

    fn nearest_wav(&self, lane: DrumLane, ms: f64) -> Option<u16> {
        self.judge
            .notes()
            .iter()
            .filter(|n| n.lane == lane)
            .min_by(|a, b| (a.ms - ms).abs().total_cmp(&(b.ms - ms).abs()))
            .map(|n| n.wav)
    }

    /// 今までの成績
    pub fn outcome(&self) -> Outcome {
        let n = self.hit_offsets.len();
        let mean = (n > 0).then(|| self.hit_offsets.iter().sum::<f64>() / n as f64);
        let sd = mean.filter(|_| n > 1).map(|m| {
            (self
                .hit_offsets
                .iter()
                .map(|o| (o - m).powi(2))
                .sum::<f64>()
                / (n - 1) as f64)
                .sqrt()
        });
        Outcome {
            path: self.path.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            level: self.level,
            notes: self.judge.notes().len(),
            tally: self.judge.tally().clone(),
            offset_mean_ms: mean,
            offset_sd_ms: sd,
            cleared: self.finished(),
        }
    }

    fn show(&mut self, events: &[JudgeEvent], song_ms: f64) {
        for e in events {
            if let Some(o) = e.offset_ms {
                self.hit_offsets.push(o);
            }
            let lane = self.judge.notes()[e.note].lane;
            self.flashes.insert(
                lane,
                Flash {
                    judgement: e.judgement,
                    offset_ms: e.offset_ms,
                    song_ms,
                },
            );
        }
    }
}

/// 譜面が使う音をすべて読む。読めない音は飛ばす
fn load_sounds(chart: &Chart, dir: &Path, rate: u32) -> HashMap<u16, Sound> {
    let used: BTreeSet<u16> = chart
        .chips
        .iter()
        .filter(|c| c.channel == dtx::ch::BGM || DrumLane::from_channel(c.channel).is_some())
        .map(|c| c.value)
        .collect();
    let mut out = HashMap::new();
    for id in used {
        let Some(name) = chart.wav(id) else { continue };
        let path = dir.join(name.replace('\\', "/"));
        match load_sound(&path, rate) {
            Ok(s) => {
                out.insert(id, s);
            }
            Err(e) => log::warn!("sound {}: {e:#}", path.display()),
        }
    }
    out
}
