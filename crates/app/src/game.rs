//! 1 曲を遊ぶ。譜面・音・判定・入力をまとめ、曲の時刻で動かす。
//! 練習（区間ループ）も同じ形で動かす。練習の時計は「始めてからの実時間」で、周ごとに音符と BGM を後ろへ足す。

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dtx::{Chart, DrumLane, TempoMap};
use engine::analysis::{NoteResult, Tendency, WeakPhrase, mean_and_sd};
use engine::clock::MidiTimestampMapper;
use engine::input::{Articulation, InputEvent, MidiMessage, MidiProfile, Normalizer, Pad};
use engine::judge::{Hit, HitWindows, Judge, JudgeEvent, Judgement, LaneGroups, Tally};
use engine::practice::{SpeedPlan, place};
use records::{Lap, NoteHit, SectionClear, Session, SessionKind, StrayHit};

use crate::audio::{AudioOut, Sound, click, load_sound};
use crate::history::OvercomeCheck;
use crate::midi::RawMidi;
use crate::practice::{LapSummary, Practice, PracticeOptions, count_per_measure};
use crate::stretch::{SectionAudio, mix_bgm};
use crate::time::host_us;

/// 曲が始まる前の空白（ミリ秒）
const LEAD_IN_MS: f64 = 2000.0;

/// 練習で、助走の前に置く空白（ミリ秒）
const PRACTICE_LEAD_IN_MS: f64 = 1000.0;

/// カウントのクリックの音量
const CLICK_GAIN: f32 = 0.8;

/// 練習の区間の BGM をまとめるとき、助走の前と着地のあとに足す余白（曲のミリ秒）。伸ばした音の端のにじみを周に入れないため
const SECTION_MARGIN_MS: f64 = 1000.0;

/// 落ちてくる音符が見え始めるより、これだけ前に周を並べる（ミリ秒）
const LAY_MARGIN_MS: f64 = 500.0;

/// 画面の効果に使う出来事。演奏画面が毎フレーム取り出す
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fx {
    /// 叩いた（判定に関わらず。レーンの光とパッドの動き）。`lane` は叩いたパッドのレーン
    Hit { lane: DrumLane, velocity: u8 },
    /// 音符を判定した（見逃しも）。`lane` と `note_ms` は音符のもの
    Judged {
        lane: DrumLane,
        judgement: Judgement,
        offset_ms: Option<f64>,
        note_ms: f64,
    },
}

/// 取り出されないまま溜められる効果の数（画面を描いていないあいだに増え続けないように）
const MAX_FX: usize = 1024;

/// 小節線と拍線（曲の時刻）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatLine {
    pub ms: f64,
    /// 小節の頭（false なら拍の線）
    pub bar: bool,
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
    /// 練習でクリアした区間を、今回のプレイで克服したか（記録を読んでから入れる）
    pub overcome: Vec<OvercomeCheck>,
    /// すべての曲を通した叩き方のくせ（記録を読んでから入れる）
    pub tendencies: Vec<Tendency>,
    /// 始めた時刻（UNIX 秒）
    pub started_at: i64,
    /// 練習なら、その決めごとと周ごとの成績
    pub practice: Option<PracticeOutcome>,
}

/// 練習 1 回の成績
#[derive(Debug, Clone)]
pub struct PracticeOutcome {
    pub options: PracticeOptions,
    pub laps: Vec<LapSummary>,
    /// 段階アップで目標の速度まで合格した
    pub cleared: bool,
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
    /// 画面の効果（演奏画面が取り出す）
    fx: Vec<Fx>,
    /// 小節線と拍線（時刻の順）
    pub lines: Vec<BeatLine>,
    /// 曲の頭のテンポ
    pub bpm: f64,
    /// 譜面の最後のチップの時刻（進み具合のバーに使う）
    pub last_chip_ms: f64,
    /// ジャケットの画像（`#PREIMAGE`。無ければ None）
    pub preimage: Option<PathBuf>,
    pub last_input: Option<LastInput>,
    pub hh_pedal: Option<u8>,
    /// 叩けた音符のずれ（ミリ秒）
    hit_offsets: Vec<f64>,
    /// 譜面のファイルの中身のハッシュ（記録で譜面を見分ける）
    pub chart_hash: u64,
    /// 始めた時刻（UNIX 秒）
    pub started_at: i64,
    notes_per_measure: Vec<usize>,
    /// 小節の頭の拍（曲頭から）。音符の拍の中の位置を出すのに使う
    measure_beats: Vec<f64>,
    /// 練習なら周の並びと集計
    pub practice: Option<Practice>,
    /// 練習の区間の BGM（速度ごとに伸ばしたもの）。音が出ないときは None
    section_audio: Option<SectionAudio>,
    /// 判定の線より上に見えている時間（ミリ秒）。練習の周をこれより前に並べる。画面の高さと落ちる速さから決める
    pub view_ahead_ms: f64,
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
    /// 電子ドラムのノート番号の割り当てを変える（NX の Config.ini の割り当てを重ねたもの）
    pub fn set_midi_profile(&mut self, profile: MidiProfile) {
        self.normalizer = Normalizer::new(profile);
    }

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
        let last_chip_ms = chart
            .chips
            .iter()
            .map(|c| tempo.chip_ms(c))
            .fold(0.0, f64::max);
        let end_ms = last_chip_ms + 2000.0;
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
                Some(Practice::new(options, &tempo, notes)),
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
        let mut section_audio = None;
        if let Some(a) = audio.as_mut() {
            match &practice {
                // BGM（ch01）は決まった時刻に鳴らす
                None => {
                    for (ms, wav) in &bgm {
                        if let Some(s) = sounds.get(wav) {
                            a.play_at(s, *ms, 1.0);
                        }
                    }
                }
                // 練習は区間の BGM を 1 本にまとめ、周の速度に伸ばして鳴らす
                Some(p) => {
                    let t = p.times();
                    let from = t.run_up_ms - SECTION_MARGIN_MS;
                    let to = t.tail_ms.max(t.end_ms) + SECTION_MARGIN_MS;
                    let started = std::time::Instant::now();
                    let first = p.ladder().speed();
                    section_audio = Some(SectionAudio::new(
                        mix_bgm(&bgm, &sounds, a.rate(), from, to),
                        from,
                        a.rate(),
                        p.options.pitch,
                        first,
                    ));
                    log::info!(
                        "section audio at {first}%: {:.0}ms",
                        started.elapsed().as_secs_f64() * 1000.0
                    );
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
                ", practice measures {}-{} from {start_ms:.0}ms",
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
            fx: Vec::new(),
            lines: beat_lines(&tempo),
            bpm: 60_000.0 / (tempo.beat_to_ms(1.0) - tempo.beat_to_ms(0.0)).max(1.0),
            last_chip_ms,
            preimage: preimage_path(&chart, path),
            last_input: None,
            hh_pedal: None,
            hit_offsets: Vec::new(),
            chart_hash: records::hash_bytes(&bytes),
            started_at: unix_now(),
            notes_per_measure,
            measure_beats: (0..=tempo.measure_count())
                .map(|m| tempo.measure_start_beat(m))
                .collect(),
            practice,
            section_audio,
            view_ahead_ms: 2000.0,
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

    /// 鳴らし始める（曲の時計を動かす）。読み込んでから画面を切り替えるときに呼ぶ
    pub fn begin(&mut self) {
        if let Some(a) = &self.audio
            && let Err(e) = a.start()
        {
            log::warn!("audio is off: {e:#}");
            self.audio = None;
            self.section_audio = None;
        }
        self.fallback_origin_us = host_us();
    }

    /// たまった画面の効果を取り出す
    pub fn take_fx(&mut self) -> Vec<Fx> {
        std::mem::take(&mut self.fx)
    }

    fn push_fx(&mut self, fx: Fx) {
        if self.fx.len() >= MAX_FX {
            self.fx.drain(..MAX_FX / 2);
        }
        self.fx.push(fx);
    }

    /// 練習なら、終わった周の合否で速度を決め、`now_ms` から少し先までの周を並べる（判定の音符・BGM・クリック）
    fn lay_laps(&mut self, now_ms: f64) {
        let Some(p) = self.practice.as_mut() else {
            return;
        };
        if let Some(sa) = self.section_audio.as_mut() {
            sa.poll();
        }
        p.settle(now_ms, self.judge.windows().poor);
        let ahead = self.view_ahead_ms + LAY_MARGIN_MS;
        while p.wants_lap(now_ms, ahead) {
            // 決めた速度の音がまだできていなければ、できるまで前の周の速度で回す
            let want = p.ladder().speed();
            let speed = match &self.section_audio {
                Some(sa) if sa.get(want).is_none() => p.last_speed().unwrap_or(want),
                _ => want,
            };
            let (lap, notes) = p.next_lap(speed, now_ms + ahead);
            log::info!(
                "lap {} at {speed}%: {:.0}-{:.0}ms",
                lap.index + 1,
                lap.start_ms,
                lap.end_ms
            );
            let range = self.judge.extend(notes);
            self.note_hits.resize(self.judge.notes().len(), None);
            if let Some(a) = self.audio.as_mut() {
                if let Some(sa) = &self.section_audio
                    && let Some(s) = sa.get(speed)
                    && let Some(at) = place(sa.from_ms, sa.song_len_ms, lap.audio, lap.speed)
                {
                    a.play_placed(s, &at, 1.0);
                }
                if let Some((accent, plain)) = &self.clicks {
                    for (i, &c) in lap.clicks.iter().enumerate() {
                        a.play_at(if i == 0 { accent } else { plain }, c, CLICK_GAIN);
                    }
                }
            }
            p.push_lap(lap, range);
        }
        if let Some(sa) = self.section_audio.as_mut() {
            for s in p.speeds_to_prepare() {
                sa.request(s);
            }
        }
        p.forget_before(now_ms - 2000.0);
    }

    /// 練習の速度を手で `steps` 刻み動かす（次に並べる周から）
    pub fn nudge_speed(&mut self, steps: i64) {
        if let Some(p) = self.practice.as_mut() {
            p.nudge_speed(steps);
        }
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
        self.show(&misses);
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
    /// 時刻の来た音符を自動で叩く（画面を確かめるための開発用。`DTXMANIANXT_AUTOPLAY=1` で起動）
    pub fn autoplay(&mut self) {
        let at = host_us();
        let now = self.song_ms_at(at) - self.input_offset_ms;
        let notes = self.judge.notes();
        let from = notes.partition_point(|n| n.ms < now - 100.0);
        let due: Vec<DrumLane> = notes[from..]
            .iter()
            .enumerate()
            .take_while(|(_, n)| n.ms <= now)
            .filter(|&(i, _)| !self.judge.is_done(from + i))
            .map(|(_, n)| n.lane)
            .collect();
        for lane in due {
            self.hit(lane, 100, at, None, None);
        }
    }

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
        self.push_fx(Fx::Hit { lane, velocity });
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
        self.show(&events);
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
            overcome: Vec::new(),
            tendencies: Vec::new(),
            started_at: self.started_at,
            practice: self.practice.as_ref().map(|p| PracticeOutcome {
                options: p.options,
                laps: p.summaries(self.now_ms()),
                cleared: p.ladder().cleared(),
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
            let beat_pos = self
                .measure_beats
                .get(note.at.measure as usize)
                .map(|start| (note.beat - start).rem_euclid(1.0));
            self.note_hits[e.note] = Some(NoteHit {
                at: note.at,
                lane: note.lane,
                judgement: e.judgement,
                offset_ms: e.offset_ms,
                velocity: e.velocity,
                articulation,
                hh_pedal,
                beat_pos,
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
            let speed = match o.speed {
                SpeedPlan::Fixed(s) => format!("{{\"plan\":\"fixed\",\"speed\":{s}}}"),
                SpeedPlan::Ladder { start, target } => {
                    format!("{{\"plan\":\"ladder\",\"start\":{start},\"target\":{target}}}")
                }
            };
            settings += &format!(
                ",\"section\":[{},{}],\"style\":\"{}\",\"rule\":\"{}\",\"speed\":{},\"pitch\":\"{}\",\"cleared\":{}",
                o.section.first,
                o.section.last,
                o.style.name(),
                o.rule.name(),
                speed,
                o.pitch.name(),
                p.ladder().cleared()
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
            // 等速以上で続けて合格したら、その区間をクリアとして残す（普通のプレイで克服したかを見る）
            clear: self.practice.as_ref().and_then(|p| {
                p.clear_speed().map(|speed| SectionClear {
                    first: p.options.section.first,
                    last: p.options.section.last,
                    speed: f64::from(speed) / 100.0,
                })
            }),
        })
    }

    fn show(&mut self, events: &[JudgeEvent]) {
        for e in events {
            if let Some(o) = e.offset_ms {
                self.hit_offsets.push(o);
            }
            let note = &self.judge.notes()[e.note];
            let fx = Fx::Judged {
                lane: note.lane,
                judgement: e.judgement,
                offset_ms: e.offset_ms,
                note_ms: note.ms,
            };
            self.push_fx(fx);
        }
    }
}

/// 小節線と拍線を並べる（上流 `CDTX` の「拍子_拍線の挿入」と同じく、曲の終わりの小節まで小節線を引き、
/// 小節の中は 1 拍ごとに拍線を引く。拍線のずらし（C1）と表示の切り替え（C2）はまだ読まない）
fn beat_lines(tempo: &TempoMap) -> Vec<BeatLine> {
    let measures = tempo.measure_count();
    let mut out = Vec::new();
    for m in 0..=measures {
        let start = tempo.measure_start_beat(m);
        out.push(BeatLine {
            ms: tempo.beat_to_ms(start),
            bar: true,
        });
        if m == measures {
            break;
        }
        let len = tempo.measure_start_beat(m + 1) - start;
        let mut i = 1.0;
        while i < len - 1e-9 {
            out.push(BeatLine {
                ms: tempo.beat_to_ms(start + i),
                bar: false,
            });
            i += 1.0;
        }
    }
    out
}

/// 譜面の `#PREIMAGE` の画像のファイル（大文字小文字の違いは許す）
fn preimage_path(chart: &Chart, path: &Path) -> Option<PathBuf> {
    let name = chart.field("PREIMAGE")?.trim();
    if name.is_empty() {
        return None;
    }
    nx::find_file(path.parent()?, name).filter(|p| p.is_file())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_lines_follow_bar_lengths() {
        // BPM 120（1 拍 500ms）。小節の長さは次に変えるまで続くので、小節 2 で 4/4 に戻す
        let chart = dtx::parse("#BPM 120\n#00102: 0.75\n#00202: 1\n#00211: 01\n");
        let lines = beat_lines(&TempoMap::build(&chart));
        let bars: Vec<f64> = lines.iter().filter(|l| l.bar).map(|l| l.ms).collect();
        let beats: Vec<f64> = lines.iter().filter(|l| !l.bar).map(|l| l.ms).collect();
        // 小節 0（4 拍）、小節 1（3 拍）、小節 2（4 拍）、曲の終わり
        assert_eq!(bars, vec![0.0, 2000.0, 3500.0, 5500.0]);
        assert_eq!(
            beats,
            vec![
                500.0, 1000.0, 1500.0, 2500.0, 3000.0, 4000.0, 4500.0, 5000.0
            ]
        );
    }
}
