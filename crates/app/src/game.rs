//! 1 曲を遊ぶ。譜面・音・判定・入力をまとめ、曲の時刻で動かす。
//! 練習（区間ループ）も同じ形で動かす。練習の時計は「始めてからの実時間」で、周ごとに音符と BGM を後ろへ足す。

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dtx::{Chart, DrumLane, TempoMap};
use engine::analysis::{NoteResult, WeakPhrase, mean_and_sd};
use engine::clock::MidiTimestampMapper;
use engine::input::{Articulation, InputEvent, MidiMessage, MidiProfile, Normalizer, Pad};
use engine::judge::{Hit, HitWindows, Judge, JudgeEvent, Judgement, LaneGroups, Tally};
use engine::practice::place;
use records::{Lap, NoteHit, Session, SessionKind, StrayHit};

use crate::audio::{AudioOut, Sound, click, load_sound};
use crate::midi::RawMidi;
use crate::practice::{LapSummary, Practice, PracticeOptions, count_per_measure};
use crate::time::host_us;

/// 曲が始まる前の空白（ミリ秒）
const LEAD_IN_MS: f64 = 2000.0;

/// 練習で、助走の前に置く空白（ミリ秒）
const PRACTICE_LEAD_IN_MS: f64 = 1000.0;

/// カウントのクリックの音量
const CLICK_GAIN: f32 = 0.8;

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
    /// 小節ごとの音符の数（長さが譜面の小節の数）
    pub notes_per_measure: Vec<usize>,
    /// 今回の、音符ごとの判定（苦手の帯に使う。記録が読めないときもこれで出す）
    pub results: Vec<NoteResult>,
    /// 苦手な小節（記録を読んでから入れる）
    pub weak: Option<WeakMap>,
    /// 練習なら、その決めごとと周ごとの成績
    pub practice: Option<PracticeOutcome>,
}

/// 練習 1 回の成績
#[derive(Debug, Clone)]
pub struct PracticeOutcome {
    pub options: PracticeOptions,
    pub laps: Vec<LapSummary>,
}

/// 苦手な小節の地図（結果の画面に出す）
#[derive(Debug, Clone)]
pub struct WeakMap {
    /// 小節ごとの苦手度（0〜1）。記録の無い小節は None
    pub weakness: Vec<Option<f64>>,
    pub phrases: Vec<WeakPhrase>,
    /// 何回ぶんの普通のプレイの記録から出したか
    pub plays: usize,
    /// 何周ぶんの練習の記録から出したか
    pub practice_laps: usize,
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
    /// 譜面のファイルの中身のハッシュ（記録で譜面を見分ける）
    pub chart_hash: u64,
    /// 始めた時刻（UNIX 秒）
    pub started_at: i64,
    notes_per_measure: Vec<usize>,
    /// 練習なら周の並びと集計
    pub practice: Option<Practice>,
    /// カウントのクリック（1 拍目、ほか）
    clicks: Option<(Sound, Sound)>,
    /// 音符ごとの結果（判定したものだけ埋まる）
    note_hits: Vec<Option<NoteHit>>,
    strays: Vec<StrayHit>,
    sounds: HashMap<u16, Sound>,
    normalizer: Normalizer,
    mappers: HashMap<String, MidiTimestampMapper>,
    /// 音が出ないときの時計の起点（ホスト時刻）。この時刻が時間軸の `start_ms`
    fallback_origin_us: i64,
    start_ms: f64,
}

impl Play {
    /// 譜面を読んで始める。`input_offset_ms` と `drum_sounds` は設定から
    pub fn load(path: &Path, input_offset_ms: f64, drum_sounds: bool) -> Result<Self> {
        Self::open(path, input_offset_ms, drum_sounds, None)
    }

    /// 区間を練習する
    pub fn load_practice(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        options: PracticeOptions,
    ) -> Result<Self> {
        Self::open(path, input_offset_ms, drum_sounds, Some(options))
    }

    fn open(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        practice: Option<PracticeOptions>,
    ) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let chart = dtx::load(&bytes);
        for w in &chart.warnings {
            log::warn!("{}:{}: {}", path.display(), w.line, w.message);
        }
        let tempo = TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        let notes_per_measure = count_per_measure(&notes, tempo.measure_count());
        let end_ms = chart
            .chips
            .iter()
            .map(|c| tempo.chip_ms(c))
            .fold(0.0, f64::max)
            + 2000.0;
        let bgm: Vec<(f64, u16)> = chart
            .chips
            .iter()
            .filter(|c| c.channel == dtx::ch::BGM)
            .map(|c| (tempo.chip_ms(c), c.value))
            .collect();
        // 叩き分けの寄せ方は曲全体の音符で決める（練習の区間によって判定が変わらないように）
        let groups = LaneGroups::default().effective_for(&notes);
        let (judge, practice) = match practice {
            None => (
                Judge::with_groups(notes, HitWindows::default(), groups),
                None,
            ),
            Some(options) => (
                Judge::with_groups(Vec::new(), HitWindows::default(), groups),
                Some(Practice::new(options, &tempo, notes, bgm.clone())),
            ),
        };
        let start_ms = practice
            .as_ref()
            .map_or(-LEAD_IN_MS, |p| p.first_audio_ms() - PRACTICE_LEAD_IN_MS);

        let mut audio = match AudioOut::open(start_ms) {
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
        if let Some(a) = audio.as_mut()
            && practice.is_none()
        {
            // BGM（ch01）は決まった時刻に鳴らす
            for (ms, wav) in &bgm {
                if let Some(s) = sounds.get(wav) {
                    a.play_at(s, *ms, 1.0);
                }
            }
        }
        let clicks = audio
            .as_ref()
            .map(|a| (click(a.rate(), true), click(a.rate(), false)));

        log::info!(
            "{}: {} notes, {:.1}s{}",
            path.display(),
            judge.notes().len(),
            end_ms / 1000.0,
            practice.as_ref().map_or(String::new(), |p| format!(
                ", practice measures {}-{}",
                p.options.section.first, p.options.section.last
            ))
        );
        let note_count = judge.notes().len();
        let mut play = Self {
            path: path.to_owned(),
            title: chart.title().to_owned(),
            artist: chart.artist().to_owned(),
            level: chart.drum_level(),
            judge,
            audio,
            end_ms,
            input_offset_ms,
            drum_sounds,
            flashes: HashMap::new(),
            last_input: None,
            hh_pedal: None,
            hit_offsets: Vec::new(),
            chart_hash: records::hash_bytes(&bytes),
            started_at: unix_now(),
            notes_per_measure,
            practice,
            clicks,
            note_hits: vec![None; note_count],
            strays: Vec::new(),
            sounds,
            normalizer: Normalizer::new(MidiProfile::roland_vdrums()),
            mappers: HashMap::new(),
            fallback_origin_us: host_us(),
            start_ms,
        };
        play.lay_laps(start_ms);
        Ok(play)
    }

    /// 練習なら、`now_ms` から少し先までの周を並べる（判定の音符・BGM・クリック）
    fn lay_laps(&mut self, now_ms: f64) {
        let Some(p) = self.practice.as_mut() else {
            return;
        };
        while p.wants_lap(now_ms) {
            let (lap, notes) = p.next_lap();
            let range = self.judge.extend(notes);
            self.note_hits.resize(self.judge.notes().len(), None);
            if let Some(a) = self.audio.as_mut() {
                let rate = a.rate();
                for (ms, wav) in &p.bgm {
                    if let Some(s) = self.sounds.get(wav)
                        && let Some(at) = place(*ms, s.ms(rate), lap.audio)
                    {
                        a.play_placed(s, &at, 1.0);
                    }
                }
                if let Some((accent, plain)) = &self.clicks {
                    for (i, &c) in lap.clicks.iter().enumerate() {
                        a.play_at(if i == 0 { accent } else { plain }, c, CLICK_GAIN);
                    }
                }
            }
            p.push_lap(lap, range);
        }
        p.forget_before(now_ms - 2000.0);
    }

    /// 小節ごとの音符の数（曲全体）
    pub fn notes_per_measure(&self) -> &[usize] {
        &self.notes_per_measure
    }

    /// ホスト時刻 `host_us` の曲の時刻
    pub fn song_ms_at(&self, at_us: i64) -> f64 {
        self.audio
            .as_ref()
            .and_then(|a| a.song_ms_at(at_us))
            .unwrap_or_else(|| (at_us - self.fallback_origin_us) as f64 / 1000.0 + self.start_ms)
    }

    pub fn now_ms(&self) -> f64 {
        self.song_ms_at(host_us())
    }

    /// 曲が終わった（練習は自分でやめるまで終わらない）
    pub fn finished(&self) -> bool {
        self.practice.is_none() && self.now_ms() > self.end_ms
    }

    /// 時間を進め、見逃しを判定する
    pub fn update(&mut self) {
        let now = self.now_ms();
        self.lay_laps(now);
        let misses = self.judge.advance(now);
        self.record(&misses, None, None);
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
                    self.hit(lane, velocity, host_us, Some(articulation), hh_pedal);
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
        self.hit(lane, 100, at_us, None, None);
    }

    fn hit(
        &mut self,
        lane: DrumLane,
        velocity: u8,
        at_us: i64,
        articulation: Option<Articulation>,
        hh_pedal: Option<u8>,
    ) {
        let ms = self.song_ms_at(at_us) - self.input_offset_ms;
        let events = self.judge.hit(Hit { ms, lane, velocity });
        if events.is_empty() {
            let stray = StrayHit {
                at_ms: ms,
                lane,
                velocity,
                articulation,
                hh_pedal,
            };
            match self.practice.as_mut() {
                Some(p) => p.on_stray(stray, self.judge.windows().poor),
                None => self.strays.push(stray),
            }
        }
        self.record(&events, articulation, hh_pedal);
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
        let (mean, sd) = mean_and_sd(&self.hit_offsets);
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
            notes_per_measure: self.notes_per_measure.clone(),
            results: self
                .note_hits
                .iter()
                .flatten()
                .map(|h| NoteResult {
                    measure: h.at.measure,
                    judgement: h.judgement,
                })
                .collect(),
            weak: None,
            practice: self.practice.as_ref().map(|p| PracticeOutcome {
                options: p.options,
                laps: p.summaries(self.now_ms()),
            }),
        }
    }

    /// 判定の結果を音符ごとの記録に書く
    fn record(
        &mut self,
        events: &[JudgeEvent],
        articulation: Option<Articulation>,
        hh_pedal: Option<u8>,
    ) {
        if let Some(p) = self.practice.as_mut() {
            p.on_events(events);
        }
        for e in events {
            let note = &self.judge.notes()[e.note];
            self.note_hits[e.note] = Some(NoteHit {
                at: note.at,
                lane: note.lane,
                judgement: e.judgement,
                offset_ms: e.offset_ms,
                velocity: e.velocity,
                articulation,
                hh_pedal,
            });
        }
    }

    /// 記録に残す形にする。何も叩かず何も過ぎていなければ None（すぐやめたときは残さない）
    pub fn session(&self) -> Option<Session> {
        let w = self.judge.windows();
        // 設定は JSON で残す（数は有限なので Display のままで JSON の数になる）
        let mut settings = format!(
            "{{\"input_offset_ms\":{},\"windows_ms\":[{},{},{},{}],\"drum_sounds\":{}",
            self.input_offset_ms, w.perfect, w.great, w.good, w.poor, self.drum_sounds
        );
        if let Some(p) = &self.practice {
            let o = p.options;
            settings += &format!(
                ",\"section\":[{},{}],\"style\":\"{}\",\"rule\":\"{}\"",
                o.section.first,
                o.section.last,
                o.style.name(),
                o.rule.name()
            );
        }
        settings.push('}');
        let laps = match &self.practice {
            None => {
                let hits: Vec<NoteHit> = self.note_hits.iter().flatten().cloned().collect();
                if hits.is_empty() && self.strays.is_empty() {
                    return None;
                }
                let o = self.outcome();
                vec![Lap {
                    speed: 1.0,
                    completed: o.cleared,
                    passed: None,
                    tally: o.tally,
                    offset_mean_ms: o.offset_mean_ms,
                    offset_sd_ms: o.offset_sd_ms,
                    hits,
                    strays: self.strays.clone(),
                }]
            }
            // 練習は最後まで叩いた周だけを残す（途中でやめた周の見逃しは苦手のせいではないので）
            Some(p) => p
                .laps
                .iter()
                .filter(|l| l.complete() && !l.notes.is_empty())
                .map(|l| {
                    let s = l.summary(p.options.rule);
                    Lap {
                        speed: s.speed,
                        completed: s.completed,
                        passed: s.passed,
                        tally: s.tally,
                        offset_mean_ms: s.offset_mean_ms,
                        offset_sd_ms: s.offset_sd_ms,
                        hits: self.note_hits[l.notes.clone()]
                            .iter()
                            .flatten()
                            .cloned()
                            .collect(),
                        strays: l.strays.clone(),
                    }
                })
                .collect(),
        };
        if laps.is_empty() {
            return None;
        }
        Some(Session {
            kind: if self.practice.is_some() {
                SessionKind::Practice
            } else {
                SessionKind::Play
            },
            started_at: self.started_at,
            settings,
            laps,
        })
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

/// 今の UNIX 秒
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
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
