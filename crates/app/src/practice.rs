//! 練習（区間ループ）。区間を選ぶ画面の中身と、練習中の周ごとの集計。
//!
//! 周の並べ方と速度の決め方は `engine::practice` が決め、ここではそれを判定・音・記録につなぐ。
//! 判定の音符は周ごとに後ろへ足していくので、判定の音符の添字から「何周目か」を引けるようにしておく。
//!
//! 周の合否で次の周の速度を決めるので、周は判定と表示に要るぶんだけ先に並べる。
//! つなぐときは次の周がもう落ちてきているので、速度が変わったら、まだ始まっていない次の周を取り下げ、
//! 新しい速度の助走から並べ直す（間に合わなければ次の次の周から）。区切るときは合否が出てから次の周を並べる。

use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dtx::{DrumLane, DrumNote, TempoMap};
use engine::analysis::mean_and_sd;
use engine::judge::{JudgeEvent, Judgement, Tally};
use engine::practice::{
    Attempt, BeatGrid, BeatSlot, Lap, LoopStyle, Looper, Next, PassRule, SPEED_MIN, SPEED_STEP,
    STEP_LAPS, STEP_SPEED, Section, SectionTimes, SpeedPlan, SpotTally, next_step, snap_speed,
    speed_percent,
};
use records::{ClearRecord, StrayHit};

use crate::game::WeakMap;
use crate::history::History;
use crate::stretch::Pitch;

/// 練習の決めごと
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PracticeOptions {
    pub section: Section,
    pub style: LoopStyle,
    pub rule: PassRule,
    pub speed: SpeedPlan,
    pub pitch: Pitch,
    /// 周の中も拍ごとにクリックを鳴らす（助走のカウントはいつも鳴らす）
    pub metronome: bool,
    /// 1 段の周の数
    pub step_laps: u32,
}

/// 区間のほかの決めごと（設定に残し、次の練習でも使う）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PracticePrefs {
    pub style: LoopStyle,
    pub rule: PassRule,
    pub speed: SpeedPlan,
    pub pitch: Pitch,
    pub metronome: bool,
    pub step_laps: u32,
}

impl Default for PracticePrefs {
    fn default() -> Self {
        Self {
            style: LoopStyle::default(),
            rule: PassRule::default(),
            speed: SpeedPlan::default(),
            pitch: Pitch::default(),
            metronome: true,
            step_laps: STEP_LAPS,
        }
    }
}

/// 区間を選ぶ画面
#[derive(Debug, Clone)]
pub struct PracticeSetup {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub notes_per_measure: Vec<usize>,
    /// 苦手な小節（記録が無ければ全部「記録なし」）
    pub weak: WeakMap,
    /// 練習でクリアした区間（古い順）
    pub clears: Vec<ClearRecord>,
    pub section: Option<Section>,
    pub style: LoopStyle,
    pub rule: PassRule,
    pub speed: SpeedPlan,
    pub pitch: Pitch,
    pub metronome: bool,
    pub step_laps: u32,
}

/// 苦手フレーズも記録も無いときに選んでおく長さ（小節）
const DEFAULT_MEASURES: u32 = 4;

impl PracticeSetup {
    /// 譜面を読んで小節の数と音符の数を出し、記録から苦手な小節を出す。
    /// `previous` があればその区間と決めごとを、無ければ `prefs` を選んでおく
    pub fn load(
        path: &Path,
        history: &History,
        previous: Option<PracticeOptions>,
        prefs: PracticePrefs,
    ) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let chart = dtx::load(&bytes);
        let tempo = TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        let notes_per_measure = count_per_measure(&notes, tempo.measure_count());
        let weak = history.weak_map(path, &notes_per_measure, None);
        let clears = history.clears(path);
        let section = previous
            .map(|p| p.section.clamped(notes_per_measure.len()))
            .or_else(|| default_section(&weak, &notes_per_measure));
        Ok(Self {
            path: path.to_owned(),
            title: chart.title().to_owned(),
            artist: chart.artist().to_owned(),
            notes_per_measure,
            weak,
            clears,
            section,
            style: previous.map_or(prefs.style, |p| p.style),
            rule: previous.map_or(prefs.rule, |p| p.rule),
            speed: previous.map_or(prefs.speed, |p| p.speed),
            pitch: previous.map_or(prefs.pitch, |p| p.pitch),
            metronome: previous.map_or(prefs.metronome, |p| p.metronome),
            step_laps: previous.map_or(prefs.step_laps, |p| p.step_laps),
        })
    }

    /// 今の決めごと（設定に残す）
    pub fn prefs(&self) -> PracticePrefs {
        PracticePrefs {
            style: self.style,
            rule: self.rule,
            speed: self.speed,
            pitch: self.pitch,
            metronome: self.metronome,
            step_laps: self.step_laps,
        }
    }

    pub fn measure_count(&self) -> usize {
        self.notes_per_measure.len()
    }

    /// 区間の音符の数
    pub fn notes_in(&self, s: Section) -> usize {
        (s.first..=s.last)
            .filter_map(|m| self.notes_per_measure.get(m as usize))
            .sum()
    }

    /// 始められるなら練習の決めごと（音符の無い区間は練習できない）
    pub fn options(&self) -> Option<PracticeOptions> {
        let section = self.section.filter(|&s| self.notes_in(s) > 0)?;
        Some(PracticeOptions {
            section,
            style: self.style,
            rule: self.rule,
            speed: self.speed,
            pitch: self.pitch,
            metronome: self.metronome,
            step_laps: self.step_laps,
        })
    }

    /// 段階アップと固定を切り替える。速度（段階アップなら始めの速度）はそのまま
    pub fn toggle_ladder(&mut self) {
        self.speed = match self.speed {
            SpeedPlan::Fixed(s) => SpeedPlan::Ladder {
                start: s,
                target: SpeedPlan::DEFAULT_TARGET.max(s),
            },
            SpeedPlan::Ladder { start, .. } => SpeedPlan::Fixed(start),
        };
    }

    /// 速度（段階アップなら始めの速度）を `steps` 刻み動かす
    pub fn nudge_speed(&mut self, steps: i64) {
        let moved = |s: u32| snap_speed(i64::from(s) + steps * i64::from(SPEED_STEP));
        self.speed = match self.speed {
            SpeedPlan::Fixed(s) => SpeedPlan::Fixed(moved(s)),
            SpeedPlan::Ladder { start, target } => SpeedPlan::Ladder {
                start: moved(start),
                target,
            },
        };
    }

    /// 段階アップの目標の速度を決める
    pub fn set_target(&mut self, percent: u32) {
        if let SpeedPlan::Ladder { start, .. } = self.speed {
            self.speed = SpeedPlan::Ladder {
                start,
                target: percent,
            };
        }
    }

    /// クリアした区間ごとの、一番速い速度（%）と克服したか。区間の順
    pub fn cleared_sections(&self) -> Vec<(Section, u32, bool)> {
        let mut out: Vec<(Section, u32, bool)> = Vec::new();
        for c in &self.clears {
            let s = Section::new(c.first, c.last);
            let speed = speed_percent(c.speed);
            match out.iter_mut().find(|o| o.0 == s) {
                Some(o) => {
                    o.1 = o.1.max(speed);
                    o.2 |= c.overcome.is_some();
                }
                None => out.push((s, speed, c.overcome.is_some())),
            }
        }
        out.sort_by_key(|o| (o.0.first, o.0.last));
        out
    }

    /// 区間を `delta` 小節ずらす
    pub fn shift(&mut self, delta: i64) {
        if let Some(s) = self.section {
            self.section = Some(s.shifted(delta, self.measure_count()));
        }
    }

    /// 区間の終わりを `delta` 小節のばす（縮める）
    pub fn resize(&mut self, delta: i64) {
        if let Some(s) = self.section {
            let max = (self.measure_count() as i64 - 1).max(i64::from(s.first));
            let last = (i64::from(s.last) + delta).clamp(i64::from(s.first), max);
            self.section = Some(Section::new(s.first, last as u32));
        }
    }
}

/// 小節ごとの音符の数
pub fn count_per_measure(notes: &[DrumNote], measures: usize) -> Vec<usize> {
    let mut n = vec![0; measures];
    for note in notes {
        if let Some(c) = n.get_mut(note.at.measure as usize) {
            *c += 1;
        }
    }
    n
}

/// はじめに選んでおく区間。一番の苦手フレーズ、無ければ音符のある最初の小節から 4 小節
fn default_section(weak: &WeakMap, notes_per_measure: &[usize]) -> Option<Section> {
    if let Some(p) = weak.phrases.first() {
        return Some(Section::new(p.first, p.last));
    }
    let first = notes_per_measure.iter().position(|&n| n > 0)? as u32;
    let last = (first + DEFAULT_MEASURES - 1).min(notes_per_measure.len() as u32 - 1);
    Some(Section::new(first, last))
}

/// 周の頭の助走のあいだに、レーンの真ん中で知らせること
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// 練習の始まり（最初の段）
    Start,
    /// 段の挑戦のあと。`from` は前の挑戦の速度、`next` は決まったこと（上げる・もう一度・下げる）
    Step { from: u32, next: Next },
    /// 手で速度を変えた
    Manual { from: u32 },
}

/// 練習中の 1 周
#[derive(Debug, Clone)]
pub struct LapState {
    pub plan: Lap,
    /// 判定の音符の添字の範囲
    pub notes: Range<usize>,
    pub tally: Tally,
    offsets: Vec<f64>,
    pub strays: Vec<StrayHit>,
    /// 判定の音符ごとの、区間の中の拍（区切るときの着地は None）とレーン
    slots: Vec<(Option<BeatSlot>, DrumLane)>,
    /// 落とした音符（POOR と MISS）の拍とレーン
    pub misses: Vec<(BeatSlot, DrumLane)>,
    /// 判定が済んだ音符の数
    pub judged: usize,
    /// 助走のあいだに知らせること（助走の無い周や、知らせることの無い周は None）
    pub cue: Option<Cue>,
    /// 音符をすべて判定し終えた実時間（周の終わりを少しのあいだ見せる）
    pub settled_at_ms: Option<f64>,
    /// 何回目の段の挑戦の周か（0 から）と、その挑戦の何周目か（1 から。手で速度を変えたらそこから数え直す）
    pub attempt: usize,
    pub in_attempt: usize,
}

impl LapState {
    pub fn complete(&self) -> bool {
        self.judged >= self.notes.len()
    }

    /// 今、助走のあいだか（`now` は実時間）
    pub fn in_run_up(&self, now: f64) -> bool {
        self.plan.has_run_up() && now >= self.plan.audio.at_ms && now < self.plan.start_ms
    }

    /// ノーミス（POOR と MISS が 0）
    pub fn clean(&self) -> bool {
        self.tally.poor == 0 && self.tally.miss == 0
    }

    pub fn summary(&self) -> LapSummary {
        let (mean, sd) = mean_and_sd(&self.offsets);
        let completed = self.complete();
        LapSummary {
            index: self.plan.index,
            speed: self.plan.speed,
            notes: self.notes.len(),
            tally: self.tally.clone(),
            offset_mean_ms: mean,
            offset_sd_ms: sd,
            completed,
            passed: completed.then_some(self.clean()),
            misses: self.misses.clone(),
            attempt: self.attempt,
        }
    }
}

/// 1 周の成績（画面と記録に出す）
#[derive(Debug, Clone, PartialEq)]
pub struct LapSummary {
    pub index: usize,
    pub speed: f64,
    pub notes: usize,
    pub tally: Tally,
    pub offset_mean_ms: Option<f64>,
    pub offset_sd_ms: Option<f64>,
    /// 区間の音符をすべて判定した（途中でやめた周は false）
    pub completed: bool,
    /// ノーミス（POOR と MISS が 0）か（記録の passed）。終わっていない周は None
    pub passed: Option<bool>,
    /// 落とした音符の拍とレーン
    pub misses: Vec<(BeatSlot, DrumLane)>,
    /// 何回目の段の挑戦の周か（0 から）
    pub attempt: usize,
}

/// 空打ちの多いパッド（パッドが弱い振動まで拾っていないかを結果の画面で知らせる）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrayHint {
    /// 空打ちの数（すべてのパッド）
    pub total: usize,
    pub lane: DrumLane,
    pub count: usize,
    /// そのうち弱い打（強さが [`WEAK_VELOCITY`] 未満）
    pub weak: usize,
}

/// 空打ちのうち「弱い」とみなす強さ（MIDI のベロシティ）
pub const WEAK_VELOCITY: u8 = 30;
/// 空打ちがこれより少なければ知らせない
const STRAY_HINT_MIN: usize = 10;

/// 取り下げた周（判定の音符と、鳴らす予定だった音を取り除く）
#[derive(Debug, Clone, PartialEq)]
pub struct Withdrawn {
    /// 判定の音符の添字（ここから後ろを取り除く）
    pub notes_from: usize,
    /// この実時間から後に鳴らす予定だった音を取りやめる
    pub audio_from_ms: f64,
}

/// クリアしてから結果の画面へ進むまで（ミリ秒）。クリアを見せる間
pub const CLEAR_HOLD_MS: f64 = 3000.0;

/// 練習 1 回の周の並びと集計
pub struct Practice {
    pub options: PracticeOptions,
    tempo: TempoMap,
    looper: Looper,
    /// 速度の決め方（固定の速度を手で変えたらその速度）
    plan: SpeedPlan,
    /// 次に並べる周の速度（%）
    speed: u32,
    /// 今の段の挑戦（0 から）と、その最初の周の添字
    attempt: usize,
    attempt_start: usize,
    /// 今の段で続けて「もう一度」になった回数
    again: u32,
    /// 今の挑戦で、同じ拍のミスが重なった（この周で挑戦を終え、もう一度になる）
    decided: bool,
    /// 終わった段の挑戦
    pub done: Vec<Attempt>,
    /// クリアした実時間（それからは周を並べない）
    pub cleared_at_ms: Option<f64>,
    /// 音符をすべて判定し終えた周の数（周は順に終わる）
    settled: usize,
    /// 次に並べる周を助走から入れる・そのときに知らせること
    run_up_next: bool,
    next_cue: Option<Cue>,
    /// 次の挑戦の最初の周を、並べるときの実時間より後に置く
    resume_pending: bool,
    /// 曲全体の音符（曲の時刻）
    chart_notes: Vec<DrumNote>,
    pub laps: Vec<LapState>,
    /// 判定の音符の添字 → 周
    lap_of: Vec<usize>,
    /// 判定しない音符（助走など。実時間、レーン）。薄く見せる
    pub faint: Vec<(f64, DrumLane)>,
    /// 曲全体の小節の帯（苦手度）。区間の場所を見せる。始めてから記録を読んで入れる
    pub band: Vec<Option<f64>>,
    /// 区間の拍（落とした所の地図）
    pub grid: BeatGrid,
    /// 次に並べる周の、判定の音符ごとの拍とレーン（`next_lap` が作り、`push_lap` が周に移す）
    next_slots: Vec<(Option<BeatSlot>, DrumLane)>,
}

/// 取り下げるには、周の音が鳴り始めるまでこれだけ余裕が要る（ミリ秒。音の出力の遅れのぶん）
pub const WITHDRAW_MARGIN_MS: f64 = 60.0;

impl Practice {
    pub fn new(options: PracticeOptions, tempo: &TempoMap, chart_notes: Vec<DrumNote>) -> Self {
        let times = SectionTimes::new(tempo, options.section);
        Self {
            options,
            tempo: tempo.clone(),
            looper: Looper::new(times, options.style),
            plan: options.speed,
            speed: snap_speed(i64::from(options.speed.first_speed())),
            attempt: 0,
            attempt_start: 0,
            again: 0,
            decided: false,
            done: Vec::new(),
            cleared_at_ms: None,
            settled: 0,
            run_up_next: false,
            next_cue: None,
            resume_pending: false,
            chart_notes,
            laps: Vec::new(),
            lap_of: Vec::new(),
            faint: Vec::new(),
            band: Vec::new(),
            grid: BeatGrid::new(tempo, options.section),
            next_slots: Vec::new(),
        }
    }

    /// 練習する区間の曲の時刻（区間の音はこの範囲で作る）
    pub fn times(&self) -> SectionTimes {
        SectionTimes::new(&self.tempo, self.options.section)
    }

    /// 次に並べる周の速度（%）
    pub fn speed(&self) -> u32 {
        self.speed
    }

    /// 速度の決め方（固定の速度を手で変えたらその速度）
    pub fn plan(&self) -> SpeedPlan {
        self.plan
    }

    /// 今の段の挑戦（0 から）
    pub fn attempt_index(&self) -> usize {
        self.attempt
    }

    /// 1 段の周の数
    pub fn step_laps(&self) -> usize {
        self.options.step_laps.max(1) as usize
    }

    /// 今の挑戦の周（並べた順）
    pub fn attempt_laps(&self) -> &[LapState] {
        &self.laps[self.attempt_start.min(self.laps.len())..]
    }

    /// 今の段で続けて「もう一度」になった回数
    pub fn again_in_row(&self) -> u32 {
        self.again
    }

    pub fn first_audio_ms(&self) -> f64 {
        self.looper.first_audio_ms(f64::from(self.speed) / 100.0)
    }

    /// 次の挑戦の最初の周を並べるのを待っている（決まった速度の音ができるまで並べない）
    pub fn resume_pending(&self) -> bool {
        self.resume_pending
    }

    /// 次の周を並べる時か。`now_ms` は今の実時間、`ahead_ms` は落ちてくる音符が見え始めるまでの時間。
    /// つなぐときは次の周の音が `ahead_ms` 先に迫ったら、区切るときは前の周の音符をすべて判定したら並べる。
    /// 段の周を並べ終えたら（同じ拍のミスが重なったら、その周で）、挑戦が終わるまで並べない。クリアしたら並べない
    pub fn wants_lap(&self, now_ms: f64, ahead_ms: f64) -> bool {
        if self.cleared_at_ms.is_some() {
            return false;
        }
        if self.laps.is_empty() {
            return true;
        }
        if self.decided || self.laps.len() - self.attempt_start >= self.step_laps() {
            return false;
        }
        if self.resume_pending {
            return true;
        }
        match self.options.style {
            LoopStyle::Connect => self.looper.next_audio_ms(1.0) <= now_ms + ahead_ms,
            LoopStyle::Restart => self.settled >= self.laps.len(),
        }
    }

    /// 最後に並べた周の速度（%）
    pub fn last_speed(&self) -> Option<u32> {
        self.laps.last().map(|l| speed_percent(l.plan.speed))
    }

    /// 次の周を速度 `percent` で並べる。区切るときと挑戦の始めは、助走を実時間 `earliest_ms` より前には
    /// 始めない。返すのは周と、判定に足す音符
    pub fn next_lap(&mut self, percent: u32, earliest_ms: f64) -> (Lap, Vec<DrumNote>) {
        if self.resume_pending {
            self.looper.resume_at(earliest_ms);
            self.resume_pending = false;
        }
        self.looper.delay_to(earliest_ms);
        let lap = self
            .looper
            .next_lap(f64::from(percent) / 100.0, self.run_up_next);
        self.run_up_next = false;
        let mut judged = Vec::new();
        self.next_slots.clear();
        for n in lap.notes(&self.chart_notes) {
            let note = &self.chart_notes[n.note];
            if n.judged {
                // 区切るときの着地（区間の外）は地図に入れない
                self.next_slots.push((self.grid.slot(note), note.lane));
                judged.push(DrumNote {
                    ms: n.ms,
                    ..note.clone()
                });
            } else {
                self.faint.push((n.ms, note.lane));
            }
        }
        (lap, judged)
    }

    /// 音符をすべて判定し終えた周を数える。挑戦の周を叩き終えたら（同じ拍のミスが重なったら、並べた周を
    /// 叩き終えたら）挑戦を終え、次の段を決める
    pub fn settle(&mut self, now_ms: f64) {
        while let Some(lap) = self.laps.get(self.settled) {
            if !lap.complete() {
                break;
            }
            self.laps[self.settled].settled_at_ms = Some(now_ms);
            self.settled += 1;
        }
        let laid = self.laps.len() - self.attempt_start.min(self.laps.len());
        if self.cleared_at_ms.is_none()
            && laid > 0
            && self.settled == self.laps.len()
            && (self.decided || laid >= self.step_laps())
        {
            self.close_attempt(now_ms);
        }
    }

    /// 今の挑戦のまとめ（終わっていなければ合否は None）
    pub fn report(&self) -> Attempt {
        let laps: Vec<&LapState> = self
            .attempt_laps()
            .iter()
            .filter(|l| l.complete() && !l.notes.is_empty())
            .collect();
        let mut spots = SpotTally::default();
        for l in &laps {
            spots.add_lap(&l.misses);
        }
        Attempt {
            index: self.attempt,
            speed: self
                .laps
                .get(self.attempt_start)
                .map_or(self.speed, |l| speed_percent(l.plan.speed)),
            laps: laps.len() as u32,
            clean_laps: laps.iter().filter(|l| l.clean()).count() as u32,
            notes: laps.iter().map(|l| l.notes.len() as u32).sum(),
            missed: laps.iter().map(|l| l.tally.poor + l.tally.miss).sum(),
            spots,
            weak_times: self.options.rule.weak_times(),
            passed: None,
        }
    }

    /// 終わった挑戦と、途中でやめた挑戦（最後まで叩いた周があれば）（結果の画面に出す）
    pub fn attempts(&self) -> Vec<Attempt> {
        let mut v = self.done.clone();
        let r = self.report();
        if r.laps > 0 && v.last().is_none_or(|l| l.index != r.index) {
            v.push(r);
        }
        v
    }

    /// 挑戦を終える。合否から次の段を決め、次の挑戦を助走から並べるようにする（クリアなら並べない）
    fn close_attempt(&mut self, now_ms: f64) {
        let mut report = self.report();
        let passed = !self.decided && report.weak().is_empty();
        report.passed = Some(passed);
        self.again = if passed { 0 } else { self.again + 1 };
        let next = next_step(self.plan, report.speed, passed, self.again);
        log::info!(
            "attempt {} at {}%: {} laps, {} clean, {} weak -> {next:?}",
            report.index + 1,
            report.speed,
            report.laps,
            report.clean_laps,
            report.weak().len()
        );
        let from = report.speed;
        self.done.push(report);
        self.attempt += 1;
        self.attempt_start = self.laps.len();
        self.decided = false;
        match next {
            Next::Clear => {
                self.cleared_at_ms = Some(now_ms);
                return;
            }
            Next::Up(s) | Next::Down(s) => {
                self.speed = s;
                self.again = 0;
            }
            Next::Again => {}
        }
        self.run_up_next = true;
        self.resume_pending = true;
        self.next_cue = Some(Cue::Step { from, next });
    }

    /// 並べたがまだ始まっていない最後の周を、要らなくなったら取り下げる（手で決め直した速度と違う周、
    /// 同じ拍のミスが重なって終える挑戦の続きの周）。取り下げた周の音符と音は呼んだ側が取り除く。
    /// `now_ms` は今の実時間
    pub fn withdraw_stale(&mut self, now_ms: f64) -> Option<Withdrawn> {
        let lap = self.laps.last()?;
        let index = self.laps.len() - 1;
        let stale = speed_percent(lap.plan.speed) != self.speed
            || (self.decided && lap.attempt == self.attempt);
        if !stale
            || index < self.settled
            || index == 0
            || lap.judged > 0
            || !lap.strays.is_empty()
            || lap.plan.audio.at_ms < now_ms + WITHDRAW_MARGIN_MS
        {
            return None;
        }
        if !self.looper.unlay(&lap.plan) {
            return None;
        }
        let lap = self.laps.pop()?;
        self.lap_of.truncate(lap.notes.start);
        self.faint.retain(|f| f.0 < lap.plan.audio.at_ms);
        if lap.cue.is_some() {
            self.next_cue = self.next_cue.or(lap.cue);
            self.run_up_next |= lap.plan.has_run_up();
        }
        if index == self.attempt_start && matches!(lap.cue, Some(Cue::Step { .. })) {
            self.resume_pending = true;
        }
        log::info!(
            "lap {} withdrawn ({}% -> {}%)",
            lap.plan.index + 1,
            speed_percent(lap.plan.speed),
            self.speed
        );
        Some(Withdrawn {
            notes_from: lap.notes.start,
            audio_from_ms: lap.plan.audio.at_ms,
        })
    }

    /// 手で速度を `steps` 刻み動かす（次に並べる周から、助走のあと）。段階アップなら 1 段ずつ、固定なら 5% ずつ。
    /// 今の段はその速度の周から数え直す。クリアしたあとは動かさない
    pub fn nudge_speed(&mut self, steps: i64) {
        if self.cleared_at_ms.is_some() {
            return;
        }
        let from = self.speed;
        let s = match self.plan {
            SpeedPlan::Ladder { target, .. } => {
                let s = i64::from(from) + steps * i64::from(STEP_SPEED);
                s.clamp(i64::from(SPEED_MIN), i64::from(target.max(from))) as u32
            }
            SpeedPlan::Fixed(_) => snap_speed(i64::from(from) + steps * i64::from(SPEED_STEP)),
        };
        if s == from {
            return;
        }
        self.speed = s;
        self.again = 0;
        if let SpeedPlan::Fixed(_) = self.plan {
            self.plan = SpeedPlan::Fixed(s);
        }
        self.next_cue = Some(match self.next_cue {
            Some(Cue::Start) => Cue::Start,
            _ => Cue::Manual { from },
        });
    }

    /// 始め直すときの決めごと。固定の速度を手で変えていたらその速度で（段階アップは始めの速度から）
    pub fn restart_options(&self) -> PracticeOptions {
        PracticeOptions {
            speed: self.plan,
            ..self.options
        }
    }

    /// 等速以上で越えた段の、一番速い速度（%）（記録のクリア）
    pub fn clear_speed(&self) -> Option<u32> {
        self.done
            .iter()
            .filter(|r| r.cleared())
            .map(|r| r.speed)
            .max()
    }

    /// 目標の段を越えた（固定の速度なら、その段を越えた）
    pub fn cleared(&self) -> bool {
        self.cleared_at_ms.is_some()
    }

    /// 越えた段の速度（下から）
    pub fn passed_speeds(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .done
            .iter()
            .filter(|r| r.passed == Some(true))
            .map(|r| r.speed)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// 段（下から）。下げて叩いた段も入れる
    pub fn stairs(&self) -> Vec<u32> {
        let lowest = self
            .done
            .iter()
            .map(|r| r.speed)
            .chain(std::iter::once(self.speed))
            .min()
            .unwrap_or(self.speed);
        let mut v = self.plan.stairs(lowest);
        // 手で段の間の速度にしたら、その段も入れる
        for r in self.done.iter().map(|r| r.speed).chain([self.speed]) {
            if !v.contains(&r) {
                v.push(r);
            }
        }
        v.sort_unstable();
        v
    }

    /// 目標の段まで、あと何段か（今の段を入れる）。クリアしたら 0
    pub fn steps_left(&self) -> usize {
        if self.cleared() {
            return 0;
        }
        let goal = self.plan.goal().max(self.speed);
        self.stairs()
            .iter()
            .filter(|&&s| s >= self.speed && s <= goal)
            .count()
    }

    /// 光らせる拍：前の挑戦が「もう一度」なら、そのとき重なった拍とレーン（多い順）。越えたら無し
    pub fn focus_beats(&self) -> Vec<(BeatSlot, Vec<DrumLane>)> {
        match self.done.last() {
            Some(r) if r.passed == Some(false) && self.cleared_at_ms.is_none() => r
                .weak()
                .into_iter()
                .map(|(b, s)| (b, s.lanes.clone()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// 先に作っておく速度（%）。今の速度と、1 段上・下（固定なら 1 刻み上・下）
    pub fn speeds_to_prepare(&self) -> Vec<u32> {
        let s = self.speed;
        let mut v = vec![s];
        let mut add = |x: u32| {
            if !v.contains(&x) {
                v.push(x);
            }
        };
        match self.plan {
            SpeedPlan::Ladder { target, .. } => {
                add((s + STEP_SPEED).min(target.max(s)));
                add(s.saturating_sub(STEP_SPEED).max(SPEED_MIN));
            }
            SpeedPlan::Fixed(_) => {
                add(snap_speed(i64::from(s) + i64::from(SPEED_STEP)));
                add(snap_speed(i64::from(s) - i64::from(SPEED_STEP)));
            }
        }
        v
    }

    /// 並べた周を覚える。`notes` は判定に足した音符の添字の範囲
    pub fn push_lap(&mut self, plan: Lap, notes: Range<usize>) {
        let index = self.laps.len();
        let cue = if index == 0 {
            self.next_cue = None;
            Some(Cue::Start)
        } else if plan.has_run_up() {
            self.next_cue.take()
        } else {
            None
        };
        // 手で速度を変えたら、その段は新しい速度の周から数え直す
        if matches!(cue, Some(Cue::Manual { .. })) {
            self.attempt_start = index;
            self.decided = false;
        }
        self.lap_of.resize(notes.end, index);
        self.laps.push(LapState {
            plan,
            notes,
            tally: Tally::default(),
            offsets: Vec::new(),
            strays: Vec::new(),
            slots: std::mem::take(&mut self.next_slots),
            misses: Vec::new(),
            judged: 0,
            cue,
            settled_at_ms: None,
            attempt: self.attempt,
            in_attempt: index - self.attempt_start.min(index) + 1,
        });
    }

    /// 判定の結果を周ごとに数える。今の挑戦で同じ拍のミスが重なったら、その周で挑戦を終える
    pub fn on_events(&mut self, events: &[JudgeEvent]) {
        let mut missed = false;
        for e in events {
            let Some(&i) = self.lap_of.get(e.note) else {
                continue;
            };
            let lap = &mut self.laps[i];
            lap.tally.add(e.judgement);
            lap.judged += 1;
            if let Some(o) = e.offset_ms {
                lap.offsets.push(o);
            }
            if matches!(e.judgement, Judgement::Poor | Judgement::Miss)
                && let Some(&(Some(slot), lane)) = lap.slots.get(e.note - lap.notes.start)
            {
                lap.misses.push((slot, lane));
                missed |= i >= self.attempt_start;
            }
        }
        if missed
            && !self.decided
            && !self
                .attempt_spots()
                .weak(self.options.rule.weak_times())
                .is_empty()
        {
            self.decided = true;
            log::info!("attempt {} decided: a beat missed again", self.attempt + 1);
        }
    }

    /// 拍 `slot` の曲の時刻（拍の頭と終わり。ミリ秒）
    pub fn beat_ms(&self, slot: BeatSlot) -> (f64, f64) {
        let m = slot.measure as usize;
        let a = self.tempo.measure_start_beat(m) + f64::from(slot.beat);
        let b = (a + 1.0).min(self.tempo.measure_start_beat(m + 1));
        (self.tempo.beat_to_ms(a), self.tempo.beat_to_ms(b))
    }

    /// 今の挑戦で落とした拍の数え（叩いている途中の周も入れる）
    pub fn attempt_spots(&self) -> SpotTally {
        let mut spots = SpotTally::default();
        for l in self.attempt_laps() {
            if l.judged > 0 {
                spots.add_lap(&l.misses);
            }
        }
        spots
    }

    /// 空打ちを、そのとき叩いていた周に数える。助走の空打ちは数えない
    pub fn on_stray(&mut self, stray: StrayHit, poor_ms: f64) {
        if let Some(lap) = self
            .laps
            .iter_mut()
            .rev()
            .find(|l| l.plan.owns_stray(stray.at_ms, poor_ms))
        {
            lap.tally.stray += 1;
            lap.strays.push(stray);
        }
    }

    /// 終わった周と今の周の成績（並べただけでまだ始まっていない周は入れない）
    pub fn summaries(&self, now_ms: f64) -> Vec<LapSummary> {
        self.laps
            .iter()
            .filter(|l| l.judged > 0 || !l.strays.is_empty() || l.plan.start_ms <= now_ms)
            .map(LapState::summary)
            .collect()
    }

    /// 今叩いている周の番号（0 から）。1 周目の助走の間は None
    pub fn current_lap(&self, now_ms: f64) -> Option<usize> {
        self.laps
            .iter()
            .rposition(|l| l.plan.audio.at_ms <= now_ms)
            .filter(|&i| i > 0 || now_ms >= self.laps[0].plan.start_ms - 1.0)
    }

    /// 空打ちが一つのパッドに偏っていて、その多くが弱い打なら、そのパッド（拾いすぎの疑い）
    pub fn stray_hint(&self) -> Option<StrayHint> {
        let mut lanes: Vec<(DrumLane, usize, usize)> = Vec::new();
        let mut total = 0;
        for st in self.laps.iter().flat_map(|l| &l.strays) {
            total += 1;
            let weak = usize::from(st.velocity < WEAK_VELOCITY);
            match lanes.iter_mut().find(|e| e.0 == st.lane) {
                Some(e) => {
                    e.1 += 1;
                    e.2 += weak;
                }
                None => lanes.push((st.lane, 1, weak)),
            }
        }
        let &(lane, count, weak) = lanes.iter().max_by_key(|e| e.1)?;
        (total >= STRAY_HINT_MIN && count * 2 >= total && weak * 10 >= count * 6).then_some(
            StrayHint {
                total,
                lane,
                count,
                weak,
            },
        )
    }

    /// 最後まで叩いた周の、ずれの平均（ミリ秒。- は早い）と打った数
    pub fn offset_mean(&self) -> Option<(f64, usize)> {
        let all: Vec<f64> = self
            .laps
            .iter()
            .filter(|l| l.complete())
            .flat_map(|l| l.offsets.iter().copied())
            .collect();
        let (mean, _) = mean_and_sd(&all);
        Some((mean?, all.len()))
    }

    /// 古い薄い音符を捨てる
    pub fn forget_before(&mut self, ms: f64) {
        self.faint.retain(|f| f.0 >= ms);
    }
}

/// つなぎ方の名前
pub fn style_label(s: LoopStyle) -> &'static str {
    match s {
        LoopStyle::Connect => "切れ目なく",
        LoopStyle::Restart => "毎回区切る",
    }
}

/// 段の越え方（「同じ拍で 2 回ミスしなければ、次の段へ」）
pub fn pass_line(r: PassRule) -> String {
    match r.weak_times() {
        1 => "ミスしなければ、次の段へ".to_owned(),
        n => format!("同じ拍で {n} 回ミスしなければ、次の段へ"),
    }
}

/// 区間の名前（「小節 3〜6」）
pub fn section_label(s: Section) -> String {
    if s.first == s.last {
        format!("小節 {}", s.first)
    } else {
        format!("小節 {}〜{}", s.first, s.last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::judge::{HitWindows, Judge, Judgement, LaneGroups};

    /// BPM 120 で小節 0〜3 に 4 分の SD
    fn chart() -> (TempoMap, Vec<DrumNote>) {
        let c = dtx::parse(
            "#BPM 120\n#00012: 01010101\n#00112: 01010101\n#00212: 01010101\n#00312: 01010101\n",
        );
        let t = TempoMap::build(&c);
        let n = t.drum_notes(&c);
        (t, n)
    }

    fn practice(style: LoopStyle) -> Practice {
        practice_at(style, SpeedPlan::Fixed(100))
    }

    fn practice_at(style: LoopStyle, speed: SpeedPlan) -> Practice {
        practice_with(style, speed, Section::new(1, 1), STEP_LAPS)
    }

    fn practice_with(style: LoopStyle, speed: SpeedPlan, section: Section, laps: u32) -> Practice {
        let (tempo, notes) = chart();
        Practice::new(
            PracticeOptions {
                section,
                style,
                rule: PassRule::Normal,
                speed,
                pitch: Pitch::Keep,
                metronome: true,
                step_laps: laps,
            },
            &tempo,
            notes,
        )
    }

    #[test]
    fn laps_feed_the_judge_and_count_per_lap() {
        let mut p = practice(LoopStyle::Connect);
        let mut judge =
            Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default());
        // 助走は小節 0（-2000〜0）
        assert!((p.first_audio_ms() + 2000.0).abs() < 1e-6);
        assert!(p.wants_lap(-3000.0, 2000.0));
        for _ in 0..2 {
            let (lap, notes) = p.next_lap(100, f64::MIN);
            let range = judge.extend(notes);
            p.push_lap(lap, range);
        }
        // 1 周 4 音符、助走の 4 音符は薄く見せる
        assert_eq!(judge.notes().len(), 8);
        assert_eq!(p.faint.len(), 4);
        assert!((judge.notes()[4].ms - 2000.0).abs() < 1e-6);

        // 1 周目は全部ぴったり、2 周目は 1 つ見逃す
        for i in 0..4 {
            let ev = judge.hit(engine::judge::Hit {
                ms: 500.0 * i as f64,
                lane: DrumLane::SD,
                velocity: 100,
            });
            p.on_events(&ev);
        }
        for i in 0..3 {
            let ev = judge.hit(engine::judge::Hit {
                ms: 2000.0 + 500.0 * i as f64 + 50.0,
                lane: DrumLane::SD,
                velocity: 100,
            });
            p.on_events(&ev);
        }
        let ev = judge.advance(4000.0);
        p.on_events(&ev);
        let s = p.summaries(4000.0);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].tally.perfect, s[0].passed), (4, Some(true)));
        assert_eq!((s[1].tally.great, s[1].tally.miss), (3, 1));
        assert_eq!(s[1].passed, Some(false));
        assert_eq!(s[1].offset_mean_ms, Some(50.0));
        // 落とした拍は小節 1 の 4 拍目
        assert_eq!(
            s[1].misses,
            vec![(
                BeatSlot {
                    measure: 1,
                    beat: 3
                },
                DrumLane::SD
            )]
        );
    }

    #[test]
    fn missed_notes_are_counted_per_beat() {
        let mut p = practice(LoopStyle::Restart);
        let mut judge =
            Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default());
        for _ in 0..2 {
            let (lap, notes) = p.next_lap(100, f64::MIN);
            let range = judge.extend(notes);
            p.push_lap(lap, range);
        }
        // 1 周目は 3 拍目だけ見逃す（着地も見逃すが、区間の外なので地図には入れない）
        for i in [0, 1, 3] {
            let ev = judge.hit(engine::judge::Hit {
                ms: 500.0 * i as f64,
                lane: DrumLane::SD,
                velocity: 100,
            });
            p.on_events(&ev);
        }
        let start2 = p.laps[1].plan.start_ms;
        let ev = judge.advance(start2 - 300.0);
        p.on_events(&ev);
        assert_eq!(p.laps[0].tally.miss, 2);
        let slot = |b| BeatSlot {
            measure: 1,
            beat: b,
        };
        assert_eq!(p.laps[0].misses, vec![(slot(2), DrumLane::SD)]);
        // 2 周目も 3 拍目を見逃すと、その拍は 2 周ぶん（苦手）
        for i in [0, 1, 3, 4] {
            let ev = judge.hit(engine::judge::Hit {
                ms: start2 + 500.0 * i as f64,
                lane: DrumLane::SD,
                velocity: 100,
            });
            p.on_events(&ev);
        }
        let ev = judge.advance(start2 + 5000.0);
        p.on_events(&ev);
        let r = p.report();
        assert_eq!((r.laps, r.clean_laps), (2, 0));
        let t = r.spots;
        assert_eq!(t.laps, 2);
        assert_eq!(t.count(slot(2)), 2);
        assert_eq!(t.weak(PassRule::Normal.weak_times())[0].0, slot(2));
    }

    #[test]
    fn strays_go_to_the_lap_being_played() {
        let mut p = practice(LoopStyle::Connect);
        let mut judge =
            Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default());
        let (lap, notes) = p.next_lap(100, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        let stray = |at_ms| StrayHit {
            at_ms,
            lane: DrumLane::BD,
            velocity: 80,
            articulation: None,
            hh_pedal: None,
        };
        // 助走の空打ちは数えない
        p.on_stray(stray(-1000.0), 117.0);
        p.on_stray(stray(1000.0), 117.0);
        assert_eq!(p.laps[0].tally.stray, 1);
        assert_eq!(p.laps[0].strays[0].at_ms, 1000.0);
    }

    #[test]
    fn current_lap_and_streak() {
        let mut p = practice(LoopStyle::Restart);
        let mut judge =
            Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default());
        for _ in 0..2 {
            let (lap, notes) = p.next_lap(100, f64::MIN);
            let range = judge.extend(notes);
            p.push_lap(lap, range);
        }
        assert_eq!(p.current_lap(-500.0), None);
        assert_eq!(p.current_lap(100.0), Some(0));
        // 区切るときは、着地（小節 2 の頭の SD）も判定するので 1 周 5 音符
        assert_eq!(p.laps[0].notes.len(), 5);
        let first: Vec<(f64, DrumLane)> =
            judge.notes()[..5].iter().map(|n| (n.ms, n.lane)).collect();
        for (ms, lane) in first {
            let ev = judge.hit(engine::judge::Hit {
                ms,
                lane,
                velocity: 100,
            });
            assert_eq!(ev[0].judgement, Judgement::Perfect);
            p.on_events(&ev);
        }
        assert_eq!(p.laps[0].summary().passed, Some(true));
        // 2 周目の助走は 1 周目の音の終わり（着地の 2 拍あと）から
        assert_eq!(p.current_lap(p.laps[1].plan.audio.at_ms + 1.0), Some(1));
    }

    #[test]
    fn setup_defaults_and_moves() {
        let weak = WeakMap {
            weakness: vec![None; 10],
            phrases: Vec::new(),
            plays: 0,
            practice_laps: 0,
        };
        let npm = vec![0, 0, 3, 4, 4, 4, 4, 4, 0, 0];
        assert_eq!(default_section(&weak, &npm), Some(Section::new(2, 5)));
        let mut s = PracticeSetup {
            path: PathBuf::new(),
            title: String::new(),
            artist: String::new(),
            notes_per_measure: npm,
            weak,
            clears: Vec::new(),
            section: Some(Section::new(2, 5)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
        };
        assert_eq!(s.notes_in(Section::new(2, 5)), 15);
        s.shift(5);
        assert_eq!(s.section, Some(Section::new(6, 9)));
        s.resize(-3);
        assert_eq!(s.section, Some(Section::new(6, 6)));
        s.resize(-1);
        assert_eq!(s.section, Some(Section::new(6, 6)));
        s.shift(2);
        // 音符の無い区間は始められない
        assert_eq!(s.section, Some(Section::new(8, 8)));
        assert!(s.options().is_none());
    }

    #[test]
    fn setup_speed_choices() {
        let mut s = PracticeSetup {
            path: PathBuf::new(),
            title: String::new(),
            artist: String::new(),
            notes_per_measure: vec![4],
            weak: WeakMap {
                weakness: vec![None],
                phrases: Vec::new(),
                plays: 0,
                practice_laps: 0,
            },
            clears: Vec::new(),
            section: Some(Section::new(0, 0)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
        };
        s.nudge_speed(-1);
        assert_eq!(
            s.speed,
            SpeedPlan::Ladder {
                start: 65,
                target: 100
            }
        );
        s.set_target(110);
        s.toggle_ladder();
        assert_eq!(s.speed, SpeedPlan::Fixed(65));
        // 固定のときは目標を変えない
        s.set_target(105);
        s.nudge_speed(100);
        assert_eq!(s.speed, SpeedPlan::Fixed(150));
        s.toggle_ladder();
        assert_eq!(
            s.speed,
            SpeedPlan::Ladder {
                start: 150,
                target: 150
            }
        );
        assert_eq!(s.options().map(|o| o.speed), Some(s.speed));
    }

    /// 周を並べ、その周の判定する音符をすべてぴったり（`hit`）か見逃しで終える
    fn play_lap(p: &mut Practice, judge: &mut Judge, speed: u32, hit: bool) -> usize {
        let n = p.options.section.measures() as usize * 4 + 1;
        let miss: Vec<usize> = if hit { Vec::new() } else { (0..n).collect() };
        play_lap_missing(p, judge, speed, &miss)
    }

    /// 周を並べ、周の中で `miss` 番目（0 から）の音符だけ見逃し、ほかはぴったり叩く。終えたら数える
    fn play_lap_missing(p: &mut Practice, judge: &mut Judge, speed: u32, miss: &[usize]) -> usize {
        let (lap, notes) = p.next_lap(speed, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range.clone());
        let notes: Vec<(f64, DrumLane)> = judge.notes()[range.clone()]
            .iter()
            .map(|n| (n.ms, n.lane))
            .collect();
        for (k, (ms, lane)) in notes.into_iter().enumerate() {
            let ev = if miss.contains(&k) {
                judge.advance(ms + 200.0)
            } else {
                judge.hit(engine::judge::Hit {
                    ms,
                    lane,
                    velocity: 100,
                })
            };
            p.on_events(&ev);
        }
        let i = p.laps.len() - 1;
        // 空打ちは数えないので、最後の音符を判定したらすぐ数える
        let start = p.laps[i].plan.start_ms;
        p.settle(start);
        i
    }

    fn judge() -> Judge {
        Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default())
    }

    const LADDER: SpeedPlan = SpeedPlan::Ladder {
        start: 70,
        target: 100,
    };

    #[test]
    fn a_step_goes_up_after_its_laps_without_stopping_to_ask() {
        let mut p = practice_with(LoopStyle::Connect, LADDER, Section::new(1, 1), 4);
        let mut judge = judge();
        // 70% なら助走（1 小節 = 2000ms）は 2857ms
        assert!((p.first_audio_ms() + 2000.0 / 0.7).abs() < 1e-6);
        // 先に作る音は、今の速度と 1 段上・下
        assert_eq!(p.speeds_to_prepare(), vec![70, 80, 60]);
        assert_eq!(p.stairs(), vec![70, 80, 90, 100]);
        assert_eq!(p.steps_left(), 4);
        for k in 0..4 {
            assert!(p.wants_lap(0.0, 1e12), "lap {k}");
            assert_eq!(p.attempt_index(), 0);
            play_lap(&mut p, &mut judge, 70, true);
        }
        // 4 周で段を越え、止まらずに次の段（80%）を助走から並べる
        let a = p.done.last().expect("the step is over").clone();
        assert_eq!((a.laps, a.clean_laps, a.passed), (4, 4, Some(true)));
        assert_eq!((p.speed(), p.attempt_index()), (80, 1));
        assert_eq!(p.passed_speeds(), vec![70]);
        assert_eq!(p.steps_left(), 3);
        assert!(p.focus_beats().is_empty());
        assert!(p.resume_pending());
        assert!(p.wants_lap(0.0, 0.0));
        let earliest = p.laps[3].plan.end_ms + 5000.0;
        let (lap, notes) = p.next_lap(80, earliest);
        assert!(lap.has_run_up());
        assert!(lap.audio.at_ms >= earliest - 1e-6);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        assert_eq!(
            p.laps[4].cue,
            Some(Cue::Step {
                from: 70,
                next: Next::Up(80)
            })
        );
        assert_eq!((p.laps[4].attempt, p.laps[4].in_attempt), (1, 1));
        assert_eq!(p.attempt_laps().len(), 1);
    }

    #[test]
    fn a_beat_missed_twice_ends_the_step_and_twice_again_steps_down() {
        let section = Section::new(1, 2);
        let mut p = practice_with(LoopStyle::Connect, LADDER, section, 4);
        let mut judge = judge();
        // 1 周は小節 1〜2 の 8 音符。6 番目（小節 2 の 3 拍目）を 2 周続けて落とす。1 周目は 1 番目も
        play_lap_missing(&mut p, &mut judge, 70, &[0, 6]);
        assert!(!p.decided);
        assert!(p.wants_lap(0.0, 1e12));
        play_lap_missing(&mut p, &mut judge, 70, &[6]);
        // 同じ拍が 2 回になったので、4 周を待たずにその周で終えて、もう一度
        let spot = BeatSlot {
            measure: 2,
            beat: 2,
        };
        let a = p.done.last().expect("the step ends early").clone();
        assert_eq!((a.laps, a.passed), (2, Some(false)));
        let weak: Vec<BeatSlot> = a.weak().iter().map(|w| w.0).collect();
        assert_eq!(weak, vec![spot]);
        // 1 回きりの小節 1 の 1 拍目は数えない
        assert_eq!(a.slips(), 1);
        assert_eq!((p.speed(), p.again_in_row()), (70, 1));
        assert_eq!(p.focus_beats(), vec![(spot, vec![DrumLane::SD])]);
        let (lap, notes) = p.next_lap(70, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        assert_eq!(
            p.laps[2].cue,
            Some(Cue::Step {
                from: 70,
                next: Next::Again
            })
        );
        // もう一度も同じ拍を 2 回落としたら、1 段下げる（始めの速度より下にも）
        let i = p.laps.len() - 1;
        let notes: Vec<usize> = p.laps[i].notes.clone().collect();
        for (k, n) in notes.into_iter().enumerate() {
            let ms = judge.notes()[n].ms;
            let ev = if k == 6 {
                judge.advance(ms + 200.0)
            } else {
                judge.hit(engine::judge::Hit {
                    ms,
                    lane: DrumLane::SD,
                    velocity: 100,
                })
            };
            p.on_events(&ev);
        }
        p.settle(0.0);
        play_lap_missing(&mut p, &mut judge, 70, &[6]);
        assert_eq!((p.speed(), p.again_in_row()), (60, 0));
        assert_eq!(p.stairs(), vec![60, 70, 80, 90, 100]);
        assert_eq!(p.done.len(), 2);
        assert!(!p.cleared());
    }

    #[test]
    fn a_decided_step_takes_back_the_lap_laid_ahead() {
        // つなぐときは次の周が先に並んでいる。重なった周で終えるので、まだ始まっていない次の周は取り下げる
        let mut p = practice_with(LoopStyle::Connect, LADDER, Section::new(1, 1), 4);
        let mut judge = judge();
        play_lap_missing(&mut p, &mut judge, 70, &[2]);
        let (lap, notes) = p.next_lap(70, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        let (lap, notes) = p.next_lap(70, f64::MIN);
        let ahead_from = judge.extend(notes).start;
        p.push_lap(lap, ahead_from..judge.notes().len());
        // 2 周目の 3 拍目を落として決まる（3 周目は並んだまま、まだ始まっていない）
        let second: Vec<f64> = judge.notes()[p.laps[1].notes.clone()]
            .iter()
            .map(|n| n.ms)
            .collect();
        for (k, ms) in second.iter().enumerate() {
            let ev = if k == 2 {
                judge.advance(ms + 200.0)
            } else {
                judge.hit(engine::judge::Hit {
                    ms: *ms,
                    lane: DrumLane::SD,
                    velocity: 100,
                })
            };
            p.on_events(&ev);
        }
        assert!(p.decided);
        assert!(!p.wants_lap(0.0, 1e12));
        let now = p.laps[1].plan.end_ms - 300.0;
        let w = p
            .withdraw_stale(now)
            .expect("the lap laid ahead is taken back");
        assert_eq!(w.notes_from, ahead_from);
        p.settle(now);
        assert_eq!(p.done.last().map(|a| a.passed), Some(Some(false)));
        assert!(p.wants_lap(now, 0.0));
    }

    #[test]
    fn a_missed_landing_is_not_counted() {
        // 毎回区切るときは次の小節の頭（着地）まで判定する。着地は区間の外なので、落としても数えない
        let mut p = practice_with(
            LoopStyle::Restart,
            SpeedPlan::Fixed(100),
            Section::new(1, 1),
            4,
        );
        let mut judge = judge();
        for _ in 0..4 {
            play_lap_missing(&mut p, &mut judge, 100, &[4]);
        }
        let a = p.done.last().expect("the step is over");
        assert!(a.weak().is_empty());
        assert_eq!(a.passed, Some(true));
        assert!(p.cleared());
    }

    #[test]
    fn the_goal_step_clears_and_nothing_more_is_laid() {
        let mut judge = judge();
        // 違う拍を 1 回ずつ落としても越える。固定はその段を越えたらクリア
        let mut p = practice_with(
            LoopStyle::Connect,
            SpeedPlan::Fixed(100),
            Section::new(1, 1),
            4,
        );
        play_lap_missing(&mut p, &mut judge, 100, &[1]);
        play_lap(&mut p, &mut judge, 100, true);
        play_lap_missing(&mut p, &mut judge, 100, &[3]);
        assert_eq!(p.clear_speed(), None);
        // 途中の挑戦も結果の画面に出す
        assert_eq!(p.attempts().len(), 1);
        assert_eq!((p.attempts()[0].laps, p.attempts()[0].passed), (3, None));
        play_lap(&mut p, &mut judge, 100, true);
        assert!(p.cleared());
        assert!(p.cleared_at_ms.is_some());
        assert_eq!(p.clear_speed(), Some(100));
        assert_eq!(p.attempts().len(), 1);
        assert_eq!(p.steps_left(), 0);
        assert!(!p.wants_lap(0.0, 1e12));
        // クリアしたら速度は変えない
        p.nudge_speed(1);
        assert_eq!(p.speed(), 100);
        // 等速より遅い段のクリアは、記録のクリアではない
        let mut judge = judge_new();
        let mut p = practice_with(
            LoopStyle::Connect,
            SpeedPlan::Fixed(95),
            Section::new(1, 1),
            4,
        );
        for _ in 0..4 {
            play_lap(&mut p, &mut judge, 95, true);
        }
        assert_eq!((p.clear_speed(), p.cleared()), (None, true));
        // 段階アップは目標の段まで
        let mut judge = judge_new();
        let mut p = practice_with(
            LoopStyle::Connect,
            SpeedPlan::Ladder {
                start: 100,
                target: 110,
            },
            Section::new(1, 1),
            4,
        );
        for _ in 0..4 {
            play_lap(&mut p, &mut judge, 100, true);
        }
        assert_eq!(
            (p.clear_speed(), p.cleared(), p.speed()),
            (Some(100), false, 110)
        );
    }

    fn judge_new() -> Judge {
        judge()
    }

    #[test]
    fn restart_waits_for_the_verdict_before_the_next_lap() {
        let mut p = practice(LoopStyle::Restart);
        let mut judge = judge();
        assert!(p.wants_lap(-10_000.0, 2000.0));
        let (lap, notes) = p.next_lap(100, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        assert!(!p.wants_lap(0.0, 2000.0));
        let ev = judge.advance(10_000.0);
        p.on_events(&ev);
        let end = p.laps[0].plan.end_ms;
        p.settle(end + 117.0);
        // 全部落としても、どの拍もまだ 1 回なので段は続ける
        assert!(p.done.is_empty());
        assert!(p.wants_lap(end + 117.0, 2000.0));
        // 助走は合否が出てから 2000ms 先（音の終わりの 3000 より後）
        let (lap, _) = p.next_lap(100, end + 117.0 + 2000.0);
        assert!((lap.audio.at_ms - 4117.0).abs() < 1e-6);
    }

    #[test]
    fn a_manual_speed_change_takes_back_the_next_lap_and_counts_the_step_again() {
        let mut p = practice_at(LoopStyle::Connect, LADDER);
        let mut judge = judge();
        play_lap(&mut p, &mut judge, 70, true);
        play_lap(&mut p, &mut judge, 70, true);
        // 3 周目は前もって 70% で並べてある（まだ始まっていない）
        let (lap, notes) = p.next_lap(70, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        assert_eq!(p.laps[2].cue, None);
        let next_audio = p.laps[2].plan.audio.at_ms;
        let now = p.laps[1].plan.end_ms - 500.0;
        // 段階アップは 1 段ずつ
        p.nudge_speed(1);
        assert_eq!(p.speed(), 80);
        // 音が鳴り始める間際の周は取り下げない
        assert!(p.withdraw_stale(next_audio - 10.0).is_none());
        let w = p.withdraw_stale(now).expect("the next lap is taken back");
        assert!(judge.truncate(w.notes_from));
        assert_eq!(judge.notes().len(), w.notes_from);
        assert_eq!(w.audio_from_ms, next_audio);
        assert_eq!(p.laps.len(), 2);
        // 並べ直す周は 80% の 1 小節（2500ms）の助走から
        let (lap, notes) = p.next_lap(80, f64::MIN);
        assert!(lap.has_run_up());
        assert!((lap.audio.at_ms - next_audio).abs() < 1e-6);
        assert!((lap.start_ms - lap.audio.at_ms - 2500.0).abs() < 1e-6);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        assert_eq!(p.laps[2].cue, Some(Cue::Manual { from: 70 }));
        // 段は新しい速度の周から数え直す
        assert_eq!((p.attempt_laps().len(), p.laps[2].in_attempt), (1, 1));
        // 一度取り下げたら、同じ速度のあいだはもう取り下げない
        assert!(p.withdraw_stale(now).is_none());
        // 始め直すときは段階アップの始めの速度から、固定なら手で変えた速度で
        assert_eq!(p.restart_options(), p.options);
        let mut f = practice_at(LoopStyle::Connect, SpeedPlan::Fixed(80));
        f.nudge_speed(1);
        assert_eq!(f.restart_options().speed, SpeedPlan::Fixed(85));
    }

    #[test]
    fn weak_strays_on_one_pad_are_pointed_out() {
        let mut p = practice(LoopStyle::Connect);
        let mut judge =
            Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default());
        let (lap, notes) = p.next_lap(100, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range);
        let stray = |lane, velocity| StrayHit {
            at_ms: 1000.0,
            lane,
            velocity,
            articulation: None,
            hh_pedal: None,
        };
        for _ in 0..8 {
            p.on_stray(stray(DrumLane::HHO, 12), 117.0);
        }
        assert_eq!(p.stray_hint(), None);
        for _ in 0..4 {
            p.on_stray(stray(DrumLane::BD, 100), 117.0);
        }
        assert_eq!(
            p.stray_hint(),
            Some(StrayHint {
                total: 12,
                lane: DrumLane::HHO,
                count: 8,
                weak: 8,
            })
        );
        // 強く叩いた空打ちなら、拾いすぎではない
        for _ in 0..12 {
            p.on_stray(stray(DrumLane::SD, 100), 117.0);
        }
        assert_eq!(p.stray_hint(), None);
    }

    #[test]
    fn cleared_sections_are_merged() {
        let clear = |first, last, speed, overcome: bool| ClearRecord {
            first,
            last,
            speed,
            cleared_at: 0,
            overcome: overcome.then_some((1, None, None)),
        };
        let mut s = PracticeSetup {
            path: PathBuf::new(),
            title: String::new(),
            artist: String::new(),
            notes_per_measure: vec![4; 10],
            weak: WeakMap {
                weakness: vec![None; 10],
                phrases: Vec::new(),
                plays: 0,
                practice_laps: 0,
            },
            clears: Vec::new(),
            section: None,
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
        };
        s.clears = vec![
            clear(5, 6, 1.0, false),
            clear(1, 2, 1.0, false),
            clear(5, 6, 1.1, true),
        ];
        assert_eq!(
            s.cleared_sections(),
            vec![
                (Section::new(1, 2), 100, false),
                (Section::new(5, 6), 110, true)
            ]
        );
    }
}
