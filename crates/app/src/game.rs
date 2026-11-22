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
use crate::chart_sound::{AutoMix, ChartSounds, METRONOME_BAR, METRONOME_BEAT};
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

/// 自動で鳴らす音（BGM・効果音など）を、今よりこれだけ先の分まで出力に渡しておく（ミリ秒）
const AUTO_AHEAD_MS: f64 = 1500.0;

/// 自動で叩いたときのレーンの光の強さ（ベロシティ）
const AUTO_VELOCITY: u8 = 100;

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
    /// 音符の無いところを叩いて MISS にした（上流の Tight）。`lane` は叩いたパッドのレーン
    StrayMiss { lane: DrumLane },
}

/// 判定の決まり（設定から）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    /// 自動で叩くレーン（上流の AutoPlay）
    pub auto: Vec<DrumLane>,
    /// 判定の幅（ペダル以外）
    pub windows: HitWindows,
    /// バスドラム・左ペダル・左バスドラムの判定の幅
    pub pedal_windows: HitWindows,
    /// 音符の無いところを叩くと MISS（上流の Tight。練習では使わない）
    pub tight: bool,
    pub gauge: GaugeRule,
    /// 叩き分け（譜面に無いレーンは共通に寄せてから使う）
    pub groups: LaneGroups,
    pub hit_sounds: HitSoundRule,
    /// 自動で鳴らす音の音量・BGM の有無・ずらし
    pub auto_mix: AutoMix,
    /// メトロノームを鳴らす
    pub metronome: bool,
}

/// 叩いた音の鳴らし方（上流の HH / FT / CY Priority と Muting LP）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitSoundRule {
    /// 共通にしたレーンで別のレーンの音符を取ったとき、叩いたパッドのレーンの音を鳴らす
    /// （ハイハットと左シンバル、ロータムとフロアタム、シンバルとライドの順）
    pub pad_first: [bool; 3],
    /// 左ペダルを踏んだら、叩いて鳴っているハイハットの音を止める
    pub muting_lp: bool,
    /// 叩いた音の音量にかける数（上流の ChipVolume）
    pub volume: f32,
}

impl Default for HitSoundRule {
    fn default() -> Self {
        Self {
            pad_first: [false; 3],
            muting_lp: true,
            volume: 1.0,
        }
    }
}

/// 叩いたハイハットの音に付ける目印（左ペダルで止める）
const HH_TAG: u8 = 1;

/// ゲージの増え方と減り方（上流 `CActPerfCommonGauge`）。演奏の画面がこれでゲージを動かす
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaugeRule {
    /// MISS の減りにかける数（上流 `fDamageLevelFactor`。ふつうは 0.5）
    pub miss_factor: f64,
    /// ミスの許容数（上流の Risky。0 なら使わない）
    pub risky: u32,
    /// GREAT と GOOD でも MISS と同じだけ減る（上流の HAZARD）
    pub hazard: bool,
}

impl Default for GaugeRule {
    fn default() -> Self {
        Self {
            miss_factor: 0.5,
            risky: 0,
            hazard: false,
        }
    }
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
    /// 譜面の音の鳴らし方（自動で鳴らす音、音ごとの音量と左右、空打ちの音）
    chart_sounds: ChartSounds,
    /// 自動で鳴らす音のうち、次に出力へ渡すもの（練習では使わない）
    auto_next: usize,
    /// MIDI のポートごとの入力の読み手と、ポートの名前（[`crate::midi::MidiIn::ports`] と同じ並び）
    inputs: Vec<(String, Normalizer)>,
    mappers: HashMap<usize, MidiTimestampMapper>,
    /// 音が出ないときの時計の起点（ホスト時刻）。この時刻が時間軸の `start_ms`
    fallback_origin_us: i64,
    start_ms: f64,
    /// 一時停止しているなら、止めた曲の時刻
    paused_ms: Option<f64>,
    /// 早送りか巻き戻しをした（上流の練習扱い。記録に残さず、STAGE FAILED にもしない）
    skipped: bool,
    /// 空打ちを MISS にする（練習では使わない）
    tight: bool,
    /// ゲージの決まり（演奏の画面が使う）
    pub gauge: GaugeRule,
    hit_sounds: HitSoundRule,
}

impl Play {
    /// 譜面を読んで始める。`input_offset_ms` と `drum_sounds` は設定から
    /// MIDI のポートごとの割り当て（ポートの名前と割り当て。[`crate::midi::MidiIn::ports`] と同じ並び）
    pub fn set_inputs(&mut self, inputs: Vec<(String, MidiProfile)>) {
        self.inputs = inputs
            .into_iter()
            .map(|(name, p)| (name, Normalizer::new(p)))
            .collect();
    }

    /// `rules` は自動で叩くレーンや判定の幅
    pub fn load(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
    ) -> Result<Self> {
        Self::open(path, input_offset_ms, drum_sounds, rules, None)
    }

    /// 区間を練習する
    pub fn load_practice(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
        options: PracticeOptions,
    ) -> Result<Self> {
        Self::open(path, input_offset_ms, drum_sounds, rules, Some(options))
    }

    fn open(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
        practice: Option<PracticeOptions>,
    ) -> Result<Self> {
        let auto = rules.auto.as_slice();
        let bytes =
            std::fs::read(path).with_context(|| format!("{} を読めません", path.display()))?;
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
        let mut chart_sounds = ChartSounds::mixed(&chart, &tempo, rules.auto_mix);
        // 自動で叩くレーンの音は、BGM と同じく時刻を決めて鳴らす（叩いた音の設定に関わらず）
        chart_sounds.add_auto_notes(
            notes
                .iter()
                .filter(|n| auto.contains(&n.lane))
                .map(|n| (n.ms, n.wav)),
        );
        let lines = beat_lines(&tempo);
        if rules.metronome {
            chart_sounds.add_metronome(lines.iter().map(|l| (l.ms, l.bar)));
        }
        // 叩き分けの寄せ方は曲全体の音符で決める（練習の区間によって判定が変わらないように）
        let groups = rules.groups.effective_for(&notes);
        let (mut judge, practice) = match practice {
            None => (Judge::with_groups(notes, rules.windows, groups), None),
            Some(options) => (
                Judge::with_groups(Vec::new(), rules.windows, groups),
                Some(Practice::new(options, &tempo, notes)),
            ),
        };
        judge.set_auto(auto);
        judge.set_pedal_windows(rules.pedal_windows);
        let tight = rules.tight && practice.is_none();
        let start_ms = practice
            .as_ref()
            .map_or(-LEAD_IN_MS, |p| p.first_audio_ms() - PRACTICE_LEAD_IN_MS);

        let audio = match AudioOut::open(start_ms) {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("audio is off: {e:#}");
                None
            }
        };
        let mut sounds = match &audio {
            Some(a) => load_sounds(
                &chart,
                &chart_sounds,
                path.parent().unwrap_or(Path::new(".")),
                a.rate(),
            ),
            None => HashMap::new(),
        };
        // メトロノームの音は NX のスキンに無いので、数え始めと同じ作った音を使う
        if let (true, Some(a)) = (rules.metronome, &audio) {
            sounds.insert(METRONOME_BAR, click(a.rate(), true));
            sounds.insert(METRONOME_BEAT, click(a.rate(), false));
        }
        let mut section_audio = None;
        // 練習でなければ、自動で鳴らす音（BGM・効果音など）は時刻が近づいたものから出力に渡す（`lay_auto`）。
        // 練習は区間の自動で鳴らす音を 1 本にまとめ、周の速度に伸ばして鳴らす
        if let (Some(a), Some(p)) = (audio.as_ref(), &practice) {
            let t = p.times();
            let from = t.run_up_ms - SECTION_MARGIN_MS;
            let to = t.tail_ms.max(t.end_ms) + SECTION_MARGIN_MS;
            let started = std::time::Instant::now();
            let first = p.ladder().speed();
            section_audio = Some(SectionAudio::new(
                mix_bgm(&chart_sounds.auto, &sounds, a.rate(), from, to),
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
            lines,
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
            chart_sounds,
            auto_next: 0,
            inputs: Vec::new(),
            mappers: HashMap::new(),
            fallback_origin_us: host_us(),
            start_ms,
            paused_ms: None,
            skipped: false,
            tight,
            gauge: rules.gauge,
            hit_sounds: rules.hit_sounds,
        };
        play.lay_laps(start_ms);
        play.lay_auto(start_ms);
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
        p.settle(now_ms, self.judge.widest_poor());
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

    /// 練習でなければ、自動で鳴らす音（BGM・効果音・ギターとベースなど）のうち `now_ms` から少し先までのものを出力に渡す。
    /// 何千もある曲でも、出力へ渡す口があふれないように少しずつ渡す
    fn lay_auto(&mut self, now_ms: f64) {
        if self.practice.is_some() {
            return;
        }
        let Some(a) = self.audio.as_mut() else {
            return;
        };
        let until = now_ms + AUTO_AHEAD_MS;
        while let Some(v) = self.chart_sounds.auto.get(self.auto_next) {
            if v.ms >= until {
                break;
            }
            self.auto_next += 1;
            if let Some(s) = self.sounds.get(&v.wav) {
                a.play_chip(s, v.ms, v.len_ms, v.gain, v.pan);
            }
        }
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

    /// ホスト時刻 `host_us` の曲の時刻（一時停止しているあいだは止めた時刻）
    pub fn song_ms_at(&self, at_us: i64) -> f64 {
        if let Some(ms) = self.paused_ms {
            return ms;
        }
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

    /// 一時停止する・つづける（上流 Pause）。止めているあいだは音も曲の時刻も止まり、叩いても数えない。
    /// 止めたら true
    pub fn toggle_pause(&mut self) -> bool {
        match self.paused_ms.take() {
            Some(ms) => {
                if let Some(a) = &self.audio {
                    a.set_paused(false);
                }
                // 音が出ないときの時計は、止めていたぶん起点を後ろへずらす
                self.fallback_origin_us = host_us() - ((ms - self.start_ms) * 1000.0) as i64;
                false
            }
            None => {
                let ms = self.now_ms();
                if let Some(a) = &self.audio {
                    a.set_paused(true);
                }
                self.paused_ms = Some(ms);
                true
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.paused_ms.is_some()
    }

    /// 早送りか巻き戻しをした
    pub fn skipped(&self) -> bool {
        self.skipped
    }

    /// 曲の時刻を `delta_ms` だけ先へ（マイナスなら前へ）飛ぶ（上流 SkipForward / SkipBackward）。
    /// 練習では使わない（None）。飛んだプレイは上流と同じく練習扱いで、記録に残さない。飛んだ先の時刻を返す
    pub fn seek(&mut self, delta_ms: f64) -> Option<f64> {
        if self.practice.is_some() {
            return None;
        }
        let now = self.now_ms();
        let to = (now + delta_ms).clamp(self.start_ms, self.end_ms.max(self.start_ms));
        let d = to - now;
        self.skipped = true;
        if d == 0.0 {
            return Some(to);
        }
        if let Some(a) = self.audio.as_mut() {
            a.seek(d);
        }
        self.fallback_origin_us -= (d * 1000.0) as i64;
        if self.paused_ms.is_some() {
            self.paused_ms = Some(to);
        }
        self.judge.jump_to(to);
        // 飛んだ先でまだ鳴っているはずの自動の音（長い BGM など）は途中から鳴らし直し、その先は `lay_auto` に任せる
        self.auto_next = self.chart_sounds.auto.partition_point(|v| v.ms < to);
        if let Some(a) = self.audio.as_mut() {
            let rate = f64::from(a.rate());
            for v in &self.chart_sounds.auto[..self.auto_next] {
                let Some(s) = self.sounds.get(&v.wav) else {
                    continue;
                };
                let len =
                    (s.frames() as f64 / rate * 1000.0).min(v.len_ms.unwrap_or(f64::INFINITY));
                if v.ms + len > to {
                    a.play_chip(s, v.ms, v.len_ms, v.gain, v.pan);
                }
            }
        }
        self.lay_auto(to);
        log::info!("jumped from {now:.0}ms to {to:.0}ms");
        Some(to)
    }

    /// 時間を進め、見逃しを判定する
    pub fn update(&mut self) {
        if self.paused() {
            return;
        }
        let now = self.now_ms();
        self.lay_laps(now);
        self.lay_auto(now);
        let events = self.judge.advance(now);
        // 自動で叩いた音符は、叩いたときと同じくレーンを光らせる（音は時刻を決めて鳴らしてある）
        for e in &events {
            let lane = self.judge.notes()[e.note].lane;
            if e.offset_ms.is_none() && self.judge.is_auto(lane) {
                self.push_fx(Fx::Hit {
                    lane,
                    velocity: AUTO_VELOCITY,
                });
            }
        }
        self.record(&events, None, None);
        self.show(&events);
    }

    /// ポートから届いた MIDI（`msgs` は `raw.bytes` を読んだもの）
    pub fn on_midi(&mut self, raw: &RawMidi, msgs: &[MidiMessage]) {
        let Some((name, normalizer)) = self.inputs.get_mut(raw.port) else {
            return;
        };
        let mapper = self.mappers.entry(raw.port).or_default();
        let at_us = mapper.map(Some(raw.device_us), raw.arrival_us);
        let mut hits = Vec::new();
        for &msg in msgs {
            match normalizer.feed(msg, at_us) {
                Some(InputEvent::Hit {
                    host_us,
                    lane,
                    pad,
                    articulation,
                    velocity,
                    hh_pedal,
                }) => {
                    self.last_input = Some(LastInput {
                        source: name.clone(),
                        pad: Some(pad),
                        articulation: Some(articulation),
                        lane,
                        velocity,
                        hh_pedal,
                    });
                    if let Some(lane) = lane {
                        hits.push((lane, velocity, host_us, articulation, hh_pedal));
                    }
                }
                Some(InputEvent::HiHatPedal { value, .. }) => self.hh_pedal = Some(value),
                Some(InputEvent::Choke { .. }) | None => {}
            }
        }
        for (lane, velocity, host_us, articulation, hh_pedal) in hits {
            self.hit(lane, velocity, host_us, Some(articulation), hh_pedal);
        }
    }

    /// キーボードで叩いた（電子ドラムが無いときの確認用）
    /// 音を止める（STAGE FAILED）。時計はシステム時刻になるが、もう譜面は進めない
    pub fn silence(&mut self) {
        self.audio = None;
        self.section_audio = None;
    }

    pub fn on_key(&mut self, lane: DrumLane, at_us: i64) {
        self.last_input = Some(LastInput {
            source: crate::inputs::KEYBOARD.into(),
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
        // 上流と同じく、止めているあいだは叩いても何もしない
        if self.paused() {
            return;
        }
        let ms = self.song_ms_at(at_us) - self.input_offset_ms;
        self.push_fx(Fx::Hit { lane, velocity });
        let events = self.judge.hit(Hit { ms, lane, velocity });
        // 自動で叩くレーンを叩いても空打ちには数えない
        if events.is_empty() && !self.judge.is_auto(lane) {
            let stray = StrayHit {
                at_ms: ms,
                lane,
                velocity,
                articulation,
                hh_pedal,
            };
            match self.practice.as_mut() {
                Some(p) => p.on_stray(stray, self.judge.widest_poor()),
                None => self.strays.push(stray),
            }
            // 上流の Tight: MISS を出し、コンボを切り、ゲージを減らす（MISS の数には入れない）
            if self.tight {
                self.judge.break_combo();
                self.push_fx(Fx::StrayMiss { lane });
            }
        }
        self.record(&events, articulation, hh_pedal);
        if self.drum_sounds {
            // 取った音符の音を鳴らす。空打ちなら空打ちの音、無ければそのレーンの一番近い音符の音
            let wav = events
                .first()
                .map(|e| self.hit_wav(lane, &self.judge.notes()[e.note], ms))
                .or_else(|| self.stray_wav(lane, ms));
            // 上流の Muting LP: 左ペダルで、叩いて鳴っているハイハットを止める
            let muting = self.hit_sounds.muting_lp && lane == DrumLane::LP;
            let tag = if matches!(lane, DrumLane::HH | DrumLane::HHO) {
                HH_TAG
            } else {
                0
            };
            if let Some(a) = self.audio.as_mut() {
                if muting {
                    a.stop(HH_TAG);
                }
                if let Some(w) = wav
                    && let Some(s) = self.sounds.get(&w)
                {
                    let (gain, pan) = self.chart_sounds.gain_pan(w);
                    let gain = gain * self.hit_sounds.volume * f32::from(velocity) / 127.0;
                    a.play_now_panned(s, gain, pan, tag);
                }
            }
        }
        self.show(&events);
    }

    /// 音符の無い所でレーン `lane` を叩いたときの音。上流と同じく、空打ちの音（0xB1〜）を決めてあればそれ、
    /// 無ければそのレーンの一番近い音符（見えない音符 0x31〜 も含む）の音。
    /// 練習は周ごとに時刻を並べ直すので、並べた音符のうち一番近いものの音にする
    fn stray_wav(&self, lane: DrumLane, ms: f64) -> Option<u16> {
        if self.practice.is_none()
            && let Some(w) = self.chart_sounds.empty_hit(lane, ms)
        {
            return Some(w);
        }
        self.lane_wav(lane, ms)
    }

    /// 叩いて音符 `note` を取ったときの音。叩いたパッドのレーン `lane` と音符のレーンが違い（叩き分けを共通にした）、
    /// パッドの音を先にする設定なら、そのパッドのレーンの一番近い音符の音（上流 `tProcessDrumHit` の P>C）。
    /// 譜面に無いレーンは、上流と同じく開いたハイハットと左シンバルを閉じたハイハット、ライドを右シンバルとして探す
    fn hit_wav(&self, lane: DrumLane, note: &dtx::DrumNote, ms: f64) -> u16 {
        use DrumLane::*;
        let family = match lane {
            HH | HHO | LC => 0,
            LT | FT => 1,
            CY | RD => 2,
            _ => return note.wav,
        };
        if note.lane == lane || !self.hit_sounds.pad_first[family] {
            return note.wav;
        }
        let has = |l: DrumLane| self.judge.notes().iter().any(|n| n.lane == l);
        let pad = match lane {
            HHO | LC if !has(lane) => HH,
            RD if !has(RD) => CY,
            l => l,
        };
        self.lane_wav(pad, ms).unwrap_or(note.wav)
    }

    /// レーン `lane` の音符（見えない音符 0x31〜 も含む）のうち、曲の時刻 `ms` に一番近いものの音。
    /// 練習は周ごとに時刻を並べ直すので、並べた音符のうち一番近いものの音にする
    fn lane_wav(&self, lane: DrumLane, ms: f64) -> Option<u16> {
        let nearest = self
            .judge
            .notes()
            .iter()
            .filter(|n| n.lane == lane)
            .min_by(|a, b| (a.ms - ms).abs().total_cmp(&(b.ms - ms).abs()))
            .map(|n| (n.ms, n.wav));
        if self.practice.is_some() {
            return nearest.map(|(_, w)| w);
        }
        match (nearest, self.chart_sounds.nearest_hidden(lane, ms)) {
            (Some(n), Some(h)) => Some(if (h.0 - ms).abs() < (n.0 - ms).abs() {
                h.1
            } else {
                n.1
            }),
            (n, h) => n.or(h).map(|(_, w)| w),
        }
    }

    /// 自動で叩くレーンがある（ベストの記録には入れない）
    pub fn has_auto(&self) -> bool {
        self.judge.has_auto()
    }

    /// 譜面の音符をすべて自動で叩く
    fn all_auto(&self) -> bool {
        self.judge.has_auto()
            && self
                .judge
                .notes()
                .iter()
                .all(|n| self.judge.is_auto(n.lane))
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
            // 自動で叩いた音符は、苦手やくせを出す記録に入れない
            if self.judge.is_auto(note.lane) {
                continue;
            }
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

    /// 記録に残す形にする。何も叩かず何も過ぎていなければ None（すぐやめたときは残さない）。
    /// 音符をすべて自動で叩いたときと、早送りか巻き戻しをしたときも残さない（上流と同じ）
    pub fn session(&self) -> Option<Session> {
        if self.all_auto() || self.skipped {
            return None;
        }
        let w = self.judge.windows();
        // 設定は JSON で残す（数は有限なので Display のままで JSON の数になる）
        let mut settings = format!(
            "{{\"input_offset_ms\":{},\"windows_ms\":[{},{},{},{}],\"drum_sounds\":{}",
            self.input_offset_ms, w.perfect, w.great, w.good, w.poor, self.drum_sounds
        );
        let pw = self.judge.pedal_windows();
        if pw != w {
            settings += &format!(
                ",\"pedal_windows_ms\":[{},{},{},{}]",
                pw.perfect, pw.great, pw.good, pw.poor
            );
        }
        if self.tight {
            settings += ",\"tight\":true";
        }
        let groups = self.judge.groups();
        if groups != LaneGroups::default() {
            settings += &format!(",\"groups\":\"{groups:?}\"");
        }
        let g = self.gauge;
        if g != GaugeRule::default() {
            settings += &format!(
                ",\"gauge\":{{\"miss_factor\":{},\"risky\":{},\"hazard\":{}}}",
                g.miss_factor, g.risky, g.hazard
            );
        }
        let auto: Vec<String> = DrumLane::ALL
            .into_iter()
            .filter(|&l| self.judge.is_auto(l))
            .map(|l| format!("\"{}\"", l.name()))
            .collect();
        if !auto.is_empty() {
            settings += &format!(",\"auto\":[{}]", auto.join(","));
        }
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
            // スコアは演奏の画面が数えている（History::save で入れる）
            score: None,
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

/// 譜面が使う音（叩く音符、自動で鳴らす音、空打ちの音、見えない音符）をすべて読む。読めない音は飛ばす
fn load_sounds(
    chart: &Chart,
    chart_sounds: &ChartSounds,
    dir: &Path,
    rate: u32,
) -> HashMap<u16, Sound> {
    let used: BTreeSet<u16> = chart
        .chips
        .iter()
        .filter(|c| DrumLane::from_channel(c.channel).is_some())
        .map(|c| c.value)
        .chain(chart_sounds.wavs())
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

    /// 譜面 `text` を一時ファイルに書いて読む（音は鳴らさない）
    fn load_text(name: &str, text: &str, rules: &Rules) -> Play {
        let path = std::env::temp_dir().join(format!("nxt-{name}-{}.dtx", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let play = Play::load(&path, 0.0, false, rules);
        std::fs::remove_file(&path).unwrap();
        play.unwrap()
    }

    #[test]
    fn pad_sound_first_plays_the_pad_lane() {
        // BPM 60 で 1 小節 4000ms。HH（wav 01）は 4000ms と 12000ms、LC（wav 02）は 7500ms
        let text = "#BPM 60\n#00111: 01\n#0011A: 0000000000000002\n#00311: 01\n";
        let common = LaneGroups {
            hh: engine::judge::HhGroup::Common,
            ..Default::default()
        };
        let chip_first = load_text(
            "chip-first",
            text,
            &Rules {
                groups: common,
                ..Default::default()
            },
        );
        let hh = chip_first.judge.notes()[0].clone();
        assert_eq!(hh.lane, DrumLane::HH);
        // 左シンバルで閉じたハイハットの音符を取っても、ふだん（C>P）は譜面の音
        assert_eq!(chip_first.hit_wav(DrumLane::LC, &hh, 4000.0), 1);
        let pad_first = load_text(
            "pad-first",
            text,
            &Rules {
                groups: common,
                hit_sounds: HitSoundRule {
                    pad_first: [true, false, false],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        // P>C なら叩いた左シンバルのレーンの音
        assert_eq!(pad_first.hit_wav(DrumLane::LC, &hh, 4000.0), 2);
        // 譜面に開いたハイハットが無ければ、閉じたハイハットの音
        assert_eq!(pad_first.hit_wav(DrumLane::HHO, &hh, 4000.0), 1);
        // 同じレーンなら譜面の音のまま
        assert_eq!(pad_first.hit_wav(DrumLane::HH, &hh, 4000.0), 1);
    }

    #[test]
    fn metronome_and_bgm_follow_the_rules() {
        // BPM 120（1 小節 2000ms）。BGM（wav 01）は小節 1 の頭、HH（wav 02）は小節 2 の頭
        let text = "#BPM 120\n#00101: 01\n#00211: 02\n";
        let wavs = |p: &Play| -> Vec<(f64, u16)> {
            p.chart_sounds.auto.iter().map(|v| (v.ms, v.wav)).collect()
        };
        let plain = load_text("metronome-off", text, &Rules::default());
        assert_eq!(wavs(&plain), vec![(2000.0, 1)]);
        let on = load_text(
            "metronome-on",
            text,
            &Rules {
                metronome: true,
                auto_mix: AutoMix {
                    bgm: false,
                    shift_ms: 30.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let all = wavs(&on);
        // BGM は鳴らさず、メトロノームは BGMAdjust でずらさない
        assert!(all.iter().all(|&(_, w)| w != 1));
        assert_eq!(all[..2], [(0.0, METRONOME_BAR), (500.0, METRONOME_BEAT)]);
        assert!(all.contains(&(4000.0, METRONOME_BAR)));
    }

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
