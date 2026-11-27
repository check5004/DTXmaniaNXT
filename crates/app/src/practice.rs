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
use dtx::{DrumLane, DrumNote, NotePos, TempoMap};
use engine::analysis::mean_and_sd;
use engine::judge::{JudgeEvent, Judgement, Tally};
use engine::practice::{
    Attempt, BeatGrid, BeatSlot, Lap, LoopStyle, Looper, Next, PassRule, SPEED_MIN, SPEED_STEP,
    STEP_LAPS, STEP_SPEED, Section, SectionTimes, SpeedPlan, SpotTally, next_step, snap_speed,
    speed_percent,
};
use engine::practice_way::{
    Build, MAX_STOPS, Mark, Part, Stops, TOUR_LAPS, Way, break_muted, build_lengths, build_section,
    fade_rank, fade_share, find_fills, find_seams, lane_mask, shuffle,
};
use i18n::{count, t, tf};
use records::{ClearRecord, StrayHit};

use crate::chart_picture::{ChartPicture, Marks};
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
    /// 時間の区切り（秒。今日のメニュー）。過ぎたら、その段の挑戦の終わりで結果へ。設定には残さない
    pub time_box_s: Option<u32>,
    /// やり方と、パート練習で叩くパート・積み上げの向き
    pub way: Way,
    pub part: Part,
    pub build: Build,
    /// めぐるやり方でめぐる所（めぐる順）。ほかのやり方では空
    pub stops: Stops,
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
    pub way: Way,
    pub part: Part,
    pub build: Build,
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
            way: Way::default(),
            part: Part::default(),
            build: Build::default(),
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
    /// 小節ごとの、音符のあるレーン（`DrumLane as usize` のビット）
    pub lanes_per_measure: Vec<u16>,
    /// 苦手な小節（記録が無ければ全部「記録なし」）
    pub weak: WeakMap,
    /// 練習でクリアした区間（古い順）
    pub clears: Vec<ClearRecord>,
    /// 譜面の絵と、記録でよくミスする音
    pub picture: ChartPicture,
    pub marks: Marks,
    pub section: Option<Section>,
    pub style: LoopStyle,
    pub rule: PassRule,
    pub speed: SpeedPlan,
    pub pitch: Pitch,
    pub metronome: bool,
    pub step_laps: u32,
    pub way: Way,
    pub part: Part,
    pub build: Build,
    /// 譜面から探したフィルと、形が変わる所（フィルだけ・つなぎ目でめぐる所）
    pub fills: Vec<Section>,
    pub seams: Vec<Section>,
    /// 苦手めぐりでめぐる所（苦手な所を、開くたびにまぜた順）
    pub weak_tour: Vec<Section>,
    /// 小節の頭の曲の時刻（ミリ秒。めぐる時間の見積もりに使う）
    pub measure_ms: Vec<f64>,
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
        let lanes_per_measure = lanes_per_measure(&notes, tempo.measure_count());
        let weak = history.weak_map(path, &notes_per_measure, None);
        let clears = history.clears(path);
        let picture = ChartPicture::new(&notes, &tempo);
        let marks = history.note_marks(path);
        let section = previous
            .map(|p| p.section.clamped(notes_per_measure.len()))
            .or_else(|| default_section(&weak, &notes_per_measure));
        let fills = find_fills(&notes, tempo.measure_count());
        let seams = find_seams(&notes, tempo.measure_count());
        let spots: Vec<Section> = weak
            .phrases
            .iter()
            .map(|p| Section::new(p.first, p.last))
            .collect();
        let mut setup = Self {
            path: path.to_owned(),
            title: chart.title().to_owned(),
            artist: chart.artist().to_owned(),
            notes_per_measure,
            lanes_per_measure,
            weak,
            clears,
            picture,
            marks,
            section,
            style: previous.map_or(prefs.style, |p| p.style),
            rule: previous.map_or(prefs.rule, |p| p.rule),
            speed: previous.map_or(prefs.speed, |p| p.speed),
            pitch: previous.map_or(prefs.pitch, |p| p.pitch),
            metronome: previous.map_or(prefs.metronome, |p| p.metronome),
            step_laps: previous.map_or(prefs.step_laps, |p| p.step_laps),
            way: previous.map_or(prefs.way, |p| p.way),
            part: previous.map_or(prefs.part, |p| p.part),
            build: previous.map_or(prefs.build, |p| p.build),
            fills,
            seams,
            weak_tour: Vec::new(),
            measure_ms: (0..=tempo.measure_count())
                .map(|m| tempo.measure_start_ms(m))
                .collect(),
        };
        setup.set_weak_tour(&spots, tour_seed());
        Ok(setup)
    }

    /// 苦手めぐりでめぐる所を、苦手な所 `spots`（苦手な順）から決める。苦手な方から [`MAX_STOPS`] か所を、
    /// `seed` で決まる順にまぜる（同じ所を続けるより、まぜた方が覚えが残りやすい）
    pub fn set_weak_tour(&mut self, spots: &[Section], seed: u64) {
        let mut v: Vec<Section> = spots.iter().copied().take(MAX_STOPS).collect();
        shuffle(&mut v, seed);
        self.weak_tour = v;
    }

    /// 苦手めぐりの順を、苦手な所 `spots` からまぜ直す（練習メニューを開くたび）
    pub fn reshuffle_weak_tour(&mut self, spots: &[Section]) {
        self.set_weak_tour(spots, tour_seed());
    }

    /// 今のやり方でめぐり終えるまでのおよその時間（秒）。所ごとに 1 小節の助走と区間と着地を
    /// [`TOUR_LAPS`] 周、今の速度で
    pub fn tour_seconds(&self) -> f64 {
        let ms = |m: u32| self.measure_ms.get(m as usize).copied();
        let speed = f64::from(self.speed.first_speed().max(1)) / 100.0;
        self.stops()
            .iter()
            .map(|s| {
                let (Some(a), Some(b)) = (ms(s.first), ms(s.last + 1)) else {
                    return 0.0;
                };
                let run_up = a - s.first.checked_sub(1).and_then(ms).unwrap_or(a - 2000.0);
                let lap = (run_up + (b - a) + 600.0) / speed;
                lap * f64::from(TOUR_LAPS) / 1000.0
            })
            .sum()
    }

    /// 今のやり方でめぐる所（めぐるやり方でなければ空）
    pub fn stops(&self) -> Vec<Section> {
        match self.way {
            Way::Tour => self.weak_tour.clone(),
            Way::Fills => self.fills.clone(),
            Way::Seams => self.seams.clone(),
            _ => Vec::new(),
        }
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
            way: self.way,
            part: self.part,
            build: self.build,
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

    /// 区間 `s` に、パート `part` の音符があるか
    pub fn part_in(&self, s: Section, part: Part) -> bool {
        (s.first..=s.last)
            .filter_map(|m| self.lanes_per_measure.get(m as usize))
            .any(|&l| l & part.mask() != 0)
    }

    /// 曲に音符のあるパート（パート練習で選べるもの）。手と足はいつも出す
    pub fn parts(&self) -> Vec<Part> {
        let all = self.lanes_per_measure.iter().fold(0, |a, &l| a | l);
        Part::ALL
            .into_iter()
            .filter(|p| matches!(p, Part::Hands | Part::Feet) || all & p.mask() != 0)
            .collect()
    }

    /// 区間 `s` を今のやり方で練習できるか（音符の無い区間、パート練習でそのパートの音符が無い区間はできない）。
    /// めぐるやり方は、区間に関わらずめぐる所があればできる
    pub fn playable(&self, s: Section) -> bool {
        if self.way.tours() {
            return !self.stops().is_empty();
        }
        self.notes_in(s) > 0 && (self.way != Way::Part || self.part_in(s, self.part))
    }

    /// 始められるなら練習の決めごと。めぐるやり方の区間は、選んでいた区間のまま（めぐる所は `stops`）
    pub fn options(&self) -> Option<PracticeOptions> {
        let stops = self.stops();
        let section = if self.way.tours() {
            self.section.or(stops.first().copied())
        } else {
            self.section
        }
        .filter(|&s| self.playable(s))?;
        Some(PracticeOptions {
            section,
            style: self.style,
            rule: self.rule,
            speed: self.speed,
            pitch: self.pitch,
            metronome: self.metronome,
            step_laps: self.step_laps,
            time_box_s: None,
            way: self.way,
            part: self.part,
            build: self.build,
            stops: Stops::new(&stops),
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

/// 小節ごとの、音符のあるレーン（`DrumLane as usize` のビット）
pub fn lanes_per_measure(notes: &[DrumNote], measures: usize) -> Vec<u16> {
    let mut v = vec![0u16; measures];
    for note in notes {
        if let Some(m) = v.get_mut(note.at.measure as usize) {
            *m |= lane_mask([note.lane]);
        }
    }
    v
}

/// 苦手めぐりの順をまぜる種（開くたびに変わる）
fn tour_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
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
    /// 積み上げで、区間を `from` 小節から `to` 小節に伸ばした
    Grow { from: u32, to: u32 },
    /// めぐるやり方で、次の所（0 から数えて `index` か所目）へ移った
    Stop { index: usize },
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
    /// 判定の音符ごとの、区間の中の拍（区切るときの着地は None）とレーン、譜面の位置
    slots: Vec<(Option<BeatSlot>, DrumLane, NotePos)>,
    /// 落とした音符（POOR と MISS）の拍とレーン
    pub misses: Vec<(BeatSlot, DrumLane)>,
    /// 落とした音符の譜面の位置とレーン（譜面の絵に印を付ける）
    pub missed_notes: Vec<(NotePos, DrumLane)>,
    /// 判定が済んだ音符の数
    pub judged: usize,
    /// 助走のあいだに知らせること（助走の無い周や、知らせることの無い周は None）
    pub cue: Option<Cue>,
    /// 音符をすべて判定し終えた実時間（周の終わりを少しのあいだ見せる）
    pub settled_at_ms: Option<f64>,
    /// 叩かずに休んだ周（成績にも記録にも入れない）
    pub rest: bool,
    /// 何回目の段の挑戦の周か（0 から）と、その挑戦の何周目か（1 から。手で速度を変えたらそこから数え直す）
    pub attempt: usize,
    pub in_attempt: usize,
    /// お手本の周（自動で叩くのを見て聞くだけ。判定の音符は無い）
    pub demo: bool,
    /// 叩いた区間（積み上げでは区間の一部）
    pub section: Section,
    /// 判定の音符ごとの、曲のブレイクで音の消えている所か
    muted: Vec<bool>,
    /// 曲のブレイクで、音の消えている所を叩いたずれ（ミリ秒）
    pub break_offsets: Vec<f64>,
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
            missed_notes: self.missed_notes.clone(),
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
    /// 落とした音符の譜面の位置とレーン
    pub missed_notes: Vec<(NotePos, DrumLane)>,
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
/// 時間の区切りを過ぎたら、「時間です」を見せてから結果の画面へ（ミリ秒）
pub const TIME_UP_HOLD_MS: f64 = 2000.0;

/// 1 周（助走を入れて）叩かなければ休みとする。1 周がこれより短くても、これだけは待つ（ミリ秒）
pub const REST_MIN_MS: f64 = 4000.0;

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
    /// 時間の区切りを過ぎて終えた実時間（それからは周を並べない）
    pub timed_out_at_ms: Option<f64>,
    /// めぐるやり方で、最後の所まで叩き終えた実時間（それからは周を並べない）
    pub toured_at_ms: Option<f64>,
    /// 練習する小節の範囲（めぐるやり方は、めぐる所をすべて含む範囲。ほかは選んだ区間）
    span: Section,
    /// めぐるやり方で、今叩いている所（めぐる順の添字）
    stop: usize,
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
    /// 判定しない音符（助走など。実時間、レーン、`#WAVzz` の番号）。薄く見せる
    pub faint: Vec<(f64, DrumLane, u16)>,
    /// 曲全体の小節の帯（苦手度）。区間の場所を見せる。始めてから記録を読んで入れる
    pub band: Vec<Option<f64>>,
    /// 区間の拍（落とした所の地図）
    pub grid: BeatGrid,
    /// 譜面の絵（落とした音に印を付ける）
    pub picture: ChartPicture,
    /// 次に並べる周の、判定の音符ごとの拍とレーン（`next_lap` が作り、`push_lap` が周に移す）
    next_slots: Vec<(Option<BeatSlot>, DrumLane, NotePos)>,
    /// 叩くのをやめたら休みにするか（区間の音符がすべて自動で叩くレーンなら見ない）
    rest_watch: bool,
    /// 最後にパッドを叩いた実時間と、叩かない時間を数え始めた実時間（休みのあと）
    last_hit_ms: f64,
    listen_from_ms: f64,
    /// 休んでいる（並べた周を叩き終えるまで次の周を並べず、終えたら助走からその段を数え直す）
    resting: bool,
    /// 積み上げの長さ（小節）と、今の長さの添字（積み上げでなければ None）
    build: Option<(Vec<u32>, usize)>,
    /// 終わった段の挑戦ごとに、区間ぜんぶを叩いたか（積み上げの途中はクリアに数えない）
    full_attempts: Vec<bool>,
    /// お手本の周の音符（実時間、レーン、`#WAVzz`）。見せるだけで判定しない
    pub demo: Vec<(f64, DrumLane, u16)>,
    /// 並べたお手本の周の、鳴らす音符（実時間。[`Practice::take_demo`] で取り出す）
    pending_demo: Vec<DrumNote>,
    /// 次に並べる周がお手本か、判定の音符ごとに曲のブレイクで音の消える所か
    /// （`next_lap` が作り、`push_lap` が周に移す）
    next_demo: bool,
    next_muted: Vec<bool>,
}

/// 取り下げるには、周の音が鳴り始めるまでこれだけ余裕が要る（ミリ秒。音の出力の遅れのぶん）
pub const WITHDRAW_MARGIN_MS: f64 = 60.0;

impl Practice {
    pub fn new(options: PracticeOptions, tempo: &TempoMap, chart_notes: Vec<DrumNote>) -> Self {
        let mut options = options;
        // めぐるやり方は、めぐる所が無ければ選んだ区間だけをめぐる
        if options.way.tours() && options.stops.is_empty() {
            options.stops = Stops::new(&[options.section]);
        }
        // 積み上げは 1 小節から（区間が 1 小節なら、ふつうに段をのぼる）
        let build = (options.way == Way::Build)
            .then(|| build_lengths(options.section.measures()))
            .filter(|v| v.len() > 1)
            .map(|v| (v, 0));
        let first = match &build {
            Some((v, i)) => build_section(options.section, v[*i], options.build),
            None if options.way.tours() => options.stops.as_slice()[0],
            None => options.section,
        };
        let span = match options.stops.cover() {
            Some(c) if options.way.tours() => c,
            _ => options.section,
        };
        // めぐるやり方は毎回区切る（所ごとに 1 小節前から入り、次の小節の頭の 1 打まで叩く）
        let style = if options.way.tours() {
            LoopStyle::Restart
        } else {
            options.style
        };
        let times = SectionTimes::new(tempo, first);
        let picture = ChartPicture::new(&chart_notes, tempo);
        Self {
            options,
            tempo: tempo.clone(),
            looper: Looper::new(times, style),
            plan: options.speed,
            speed: snap_speed(i64::from(options.speed.first_speed())),
            attempt: 0,
            attempt_start: 0,
            again: 0,
            decided: false,
            done: Vec::new(),
            cleared_at_ms: None,
            timed_out_at_ms: None,
            toured_at_ms: None,
            span,
            stop: 0,
            settled: 0,
            run_up_next: false,
            next_cue: None,
            resume_pending: false,
            chart_notes,
            laps: Vec::new(),
            lap_of: Vec::new(),
            faint: Vec::new(),
            band: Vec::new(),
            grid: BeatGrid::new(tempo, span),
            picture,
            next_slots: Vec::new(),
            rest_watch: true,
            last_hit_ms: f64::NEG_INFINITY,
            listen_from_ms: f64::NEG_INFINITY,
            resting: false,
            build,
            full_attempts: Vec::new(),
            demo: Vec::new(),
            pending_demo: Vec::new(),
            next_demo: false,
            next_muted: Vec::new(),
        }
    }

    /// 今叩いている区間（積み上げでは区間の一部）
    pub fn current_section(&self) -> Section {
        self.looper.times().section
    }

    /// 積み上げの長さ（小節）と、今の長さの添字（積み上げでなければ None）
    pub fn build_stage(&self) -> Option<(&[u32], usize)> {
        self.build.as_ref().map(|(v, i)| (v.as_slice(), *i))
    }

    /// 積み上げの途中（区間ぜんぶにまだ届いていない）
    fn building(&self) -> bool {
        self.build.as_ref().is_some_and(|(v, i)| i + 1 < v.len())
    }

    /// 区間の音を作る区間（めぐるやり方は所ごと。ほかは選んだ区間）
    pub fn audio_sections(&self) -> Vec<Section> {
        if self.options.way.tours() {
            self.options.stops.as_slice().to_vec()
        } else {
            vec![self.options.section]
        }
    }

    /// 譜面の絵と進み具合のバーに出す区間（めぐるやり方は今の所。ほかは選んだ区間）
    pub fn shown_section(&self) -> Section {
        if self.options.way.tours() {
            self.current_section()
        } else {
            self.options.section
        }
    }

    /// [`Practice::shown_section`] の曲の時刻
    pub fn times(&self) -> SectionTimes {
        SectionTimes::new(&self.tempo, self.shown_section())
    }

    /// めぐる所（めぐる順。めぐるやり方でなければ空）
    pub fn stops(&self) -> &[Section] {
        if self.options.way.tours() {
            self.options.stops.as_slice()
        } else {
            &[]
        }
    }

    /// めぐるやり方で、今の所（めぐる順の添字）
    pub fn stop_index(&self) -> Option<usize> {
        self.options.way.tours().then_some(self.stop)
    }

    /// めぐり終えた所ごとの出来（めぐる順。叩いている途中の所とまだの所は None）
    pub fn stop_marks(&self) -> Vec<Option<Mark>> {
        (0..self.stops().len())
            .map(|i| self.done.get(i).and_then(Mark::of))
            .collect()
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

    /// 1 段の周の数（お手本と交互は、お手本の周を入れて偶数）
    pub fn step_laps(&self) -> usize {
        self.options.way.laps_per_step(self.options.step_laps) as usize
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
        if self.over() || self.resting {
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
        match self.looper.style() {
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
        // 段の何周目か（手で速度を変えた周は 1 周目から数え直す）
        let manual = lap.has_run_up() && matches!(self.next_cue, Some(Cue::Manual { .. }));
        let k = if manual || self.laps.is_empty() {
            1
        } else {
            self.laps.len() - self.attempt_start.min(self.laps.len()) + 1
        };
        self.next_demo = self.options.way.demo_lap(k);
        let muted = self.break_windows(&lap);
        let mut judged = Vec::new();
        self.next_slots.clear();
        self.next_muted.clear();
        for n in lap.notes(&self.chart_notes) {
            let note = &self.chart_notes[n.note];
            let placed = DrumNote {
                ms: n.ms,
                ..note.clone()
            };
            if n.judged && self.next_demo {
                // お手本の周は、判定せずに自動で叩く
                self.demo.push((n.ms, note.lane, note.wav));
                self.pending_demo.push(placed);
            } else if n.judged {
                // 区切るときの着地（区間の外。めぐるやり方は今の所の外）は地図に入れない
                let inside =
                    !self.options.way.tours() || self.current_section().contains(note.at.measure);
                let slot = self.grid.slot(note).filter(|_| inside);
                self.next_slots.push((slot, note.lane, note.at));
                self.next_muted
                    .push(muted.iter().any(|&(a, b)| note.ms >= a && note.ms < b));
                judged.push(placed);
            } else {
                self.faint.push((n.ms, note.lane, note.wav));
            }
        }
        (lap, judged)
    }

    /// 並べたお手本の周の、鳴らす音符（実時間）を取り出す
    pub fn take_demo(&mut self) -> Vec<DrumNote> {
        std::mem::take(&mut self.pending_demo)
    }

    /// 曲のブレイクで、周 `lap` の中で音を消す所（曲の時刻の範囲）。練習を始めてからの小節を数え、
    /// 2 小節鳴らして 2 小節消す。助走と、区切るときの着地のあとは消さない
    pub fn break_windows(&self, lap: &Lap) -> Vec<(f64, f64)> {
        if self.options.way != Way::Break {
            return Vec::new();
        }
        let s = self.current_section();
        let n = s.measures();
        let mut out: Vec<(f64, f64)> = Vec::new();
        for i in 0..n {
            let bar = lap.index as u32 * n + i;
            if !break_muted(bar) {
                continue;
            }
            let m = (s.first + i) as usize;
            let (a, b) = (
                self.tempo.measure_start_ms(m),
                self.tempo.measure_start_ms(m + 1),
            );
            match out.last_mut() {
                Some(last) if (last.1 - a).abs() < 1e-6 => last.1 = b,
                _ => out.push((a, b)),
            }
        }
        out
    }

    /// 曲のブレイクで、今（実時間 `now_ms`）音が消えているか
    pub fn in_break(&self, now_ms: f64) -> bool {
        let Some(i) = self.current_lap(now_ms) else {
            return false;
        };
        let lap = &self.laps[i].plan;
        let song = lap.song_ms(now_ms);
        self.break_windows(lap)
            .iter()
            .any(|&(a, b)| song >= a && song < b)
    }

    /// 曲のブレイクで、音の消えている所を叩いたずれの平均（ミリ秒。- は早い）と打った数
    pub fn break_offset(&self) -> Option<(f64, usize)> {
        let all: Vec<f64> = self
            .laps
            .iter()
            .filter(|l| !l.rest)
            .flat_map(|l| l.break_offsets.iter().copied())
            .collect();
        let (mean, _) = mean_and_sd(&all);
        Some((mean?, all.len()))
    }

    /// 消える譜面で、判定の音符 `note` を見せる濃さ（0 か 1）。段の周を重ねるごとに見せる音符を減らし、
    /// 同じ段の前の周で落とした音は見せる。ほかのやり方はいつも 1
    pub fn note_alpha(&self, note: usize) -> f32 {
        if self.options.way != Way::Fade {
            return 1.0;
        }
        let Some(lap) = self.lap_of.get(note).and_then(|&i| self.laps.get(i)) else {
            return 1.0;
        };
        let Some(&(_, lane, at)) = lap.slots.get(note - lap.notes.start) else {
            return 1.0;
        };
        let share = fade_share(lap.in_attempt, self.step_laps());
        if fade_rank(at, lane) < share {
            return 1.0;
        }
        let missed_before = self.laps.iter().any(|l| {
            l.attempt == lap.attempt
                && l.plan.index < lap.plan.index
                && l.missed_notes.contains(&(at, lane))
        });
        if missed_before { 1.0 } else { 0.0 }
    }

    /// 消える譜面で、周 `lap`（添字）に見せる音符の割合（0〜1）
    pub fn fade_share_of(&self, lap: usize) -> Option<f64> {
        let l = self.laps.get(lap)?;
        (self.options.way == Way::Fade).then(|| fade_share(l.in_attempt, self.step_laps()))
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
        // 休みの前に並べた周を叩き終えたら、助走からその段を数え直す
        if self.resting && self.settled == self.laps.len() {
            self.resting = false;
            self.resume_pending = true;
            self.run_up_next = true;
            self.next_cue = Some(Cue::Start);
            self.listen_from_ms = now_ms;
        }
        let laid = self.laps.len() - self.attempt_start.min(self.laps.len());
        if !self.over()
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
        if self.options.way.tours() {
            self.next_stop(now_ms);
            return;
        }
        let mut report = self.report();
        let passed = !self.decided && report.weak().is_empty();
        report.passed = Some(passed);
        self.again = if passed { 0 } else { self.again + 1 };
        // 積み上げの途中で越えたら、速度はそのままで区間を伸ばす
        let grow = passed && self.building();
        let next = if grow {
            Next::Again
        } else {
            next_step(self.plan, report.speed, passed, self.again)
        };
        log::info!(
            "attempt {} at {}% ({}): {} laps, {} clean, {} weak -> {}",
            report.index + 1,
            report.speed,
            section_label(self.current_section()),
            report.laps,
            report.clean_laps,
            report.weak().len(),
            if grow {
                "grow".to_owned()
            } else {
                format!("{next:?}")
            }
        );
        let from = report.speed;
        self.done.push(report);
        self.full_attempts.push(!self.building());
        self.attempt += 1;
        self.attempt_start = self.laps.len();
        self.decided = false;
        if grow && let Some((lengths, stage)) = self.build.as_mut() {
            let from = lengths[*stage];
            *stage += 1;
            let to = lengths[*stage];
            let s = build_section(self.options.section, to, self.options.build);
            self.looper.set_times(SectionTimes::new(&self.tempo, s));
            if self.time_left_ms(now_ms).is_some_and(|left| left <= 0.0) {
                self.timed_out_at_ms = Some(now_ms);
                return;
            }
            self.run_up_next = true;
            self.resume_pending = true;
            self.next_cue = Some(Cue::Grow { from, to });
            return;
        }
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
        // 時間の区切りを過ぎたら、次の挑戦は並べずに終える
        if self.time_left_ms(now_ms).is_some_and(|left| left <= 0.0) {
            log::info!("time is up after attempt {}", self.attempt);
            self.timed_out_at_ms = Some(now_ms);
            return;
        }
        self.run_up_next = true;
        self.resume_pending = true;
        self.next_cue = Some(Cue::Step { from, next });
    }

    /// めぐるやり方で、今の所を終えて次の所へ移る（最後の所なら終える）。速度は変えない。
    /// 合否は ◎○ なら越えた、× ならもう一度（記録と効果音のため。めぐるやり方はやり直さない）
    fn next_stop(&mut self, now_ms: f64) {
        let mut report = self.report();
        let mark = Mark::of(&report);
        report.passed = Some(mark != Some(Mark::Weak));
        log::info!(
            "stop {} of {} ({}): {} laps, {} missed -> {:?}",
            self.stop + 1,
            self.stops().len(),
            section_label(self.current_section()),
            report.laps,
            report.missed,
            mark
        );
        self.done.push(report);
        self.full_attempts.push(false);
        self.attempt += 1;
        self.attempt_start = self.laps.len();
        self.decided = false;
        self.again = 0;
        if self.stop + 1 >= self.stops().len() {
            self.toured_at_ms = Some(now_ms);
            return;
        }
        self.stop += 1;
        let s = self.stops()[self.stop];
        self.looper.set_times(SectionTimes::new(&self.tempo, s));
        self.run_up_next = true;
        self.resume_pending = true;
        self.next_cue = Some(Cue::Stop { index: self.stop });
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
        self.demo.retain(|f| f.0 < lap.plan.audio.at_ms);
        if lap.cue.is_some() {
            self.next_cue = self.next_cue.or(lap.cue);
            self.run_up_next |= lap.plan.has_run_up();
        }
        if index == self.attempt_start && matches!(lap.cue, Some(Cue::Step { .. } | Cue::Start)) {
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

    /// 区間の音符がすべて自動で叩くレーンなら、叩くのをやめても休みにしない（`auto` は自動で叩くレーン）
    pub fn set_auto_lanes(&mut self, auto: &[DrumLane]) {
        let s = self.span;
        self.rest_watch = self
            .chart_notes
            .iter()
            .any(|n| n.at.measure >= s.first && n.at.measure <= s.last && !auto.contains(&n.lane));
    }

    /// パッドを叩いた（判定に関わらず。`at_ms` は実時間）
    pub fn on_hit(&mut self, at_ms: f64) {
        self.last_hit_ms = self.last_hit_ms.max(at_ms);
    }

    /// 1 周（助走を入れて。[`REST_MIN_MS`] より短ければそれだけ）叩いていなければ休みにする。
    /// 叩かなかった周は成績にも記録にも入れず、今の段の挑戦は取りやめる（もう一度にも数えない）。
    /// 並べた周を叩き終えたら、同じ段を助走から数え直す。休みにしたら true（呼んだ側は一時停止の板を出す）
    pub fn check_rest(&mut self, now_ms: f64) -> bool {
        if !self.rest_watch || self.resting || self.over() {
            return false;
        }
        let (Some(first), Some(last)) = (self.laps.first(), self.laps.last()) else {
            return false;
        };
        // お手本の周は叩かないので、そのあいだとその前は数えない
        let demo_until = self
            .laps
            .iter()
            .filter(|l| l.demo && l.plan.audio.at_ms <= now_ms)
            .map(|l| l.plan.end_ms)
            .fold(f64::NEG_INFINITY, f64::max);
        if demo_until > now_ms {
            return false;
        }
        // 1 周目の助走のあいだは数えない
        let since = self
            .last_hit_ms
            .max(self.listen_from_ms)
            .max(first.plan.start_ms)
            .max(demo_until);
        let lap_ms = (last.plan.end_ms - last.plan.audio.at_ms).max(0.0);
        if now_ms - since < lap_ms.max(REST_MIN_MS) {
            return false;
        }
        let last_hit = self.last_hit_ms;
        for l in &mut self.laps {
            if !l.complete() || l.plan.start_ms > last_hit {
                l.rest = true;
            }
        }
        self.resting = true;
        self.decided = false;
        self.attempt_start = self.laps.len();
        log::info!(
            "resting: no hits for {:.0}ms (attempt {} at {}%)",
            now_ms - since,
            self.attempt + 1,
            self.speed
        );
        true
    }

    /// 結果に出す判定の数と音符の数から、休んだ周のぶんを除く（`total` と `notes` は判定ぜんぶ）
    pub fn without_rest(&self, total: &Tally, notes: usize) -> (Tally, usize) {
        let mut t = total.clone();
        let mut n = notes;
        for l in self.laps.iter().filter(|l| l.rest) {
            t.perfect = t.perfect.saturating_sub(l.tally.perfect);
            t.great = t.great.saturating_sub(l.tally.great);
            t.good = t.good.saturating_sub(l.tally.good);
            t.poor = t.poor.saturating_sub(l.tally.poor);
            t.miss = t.miss.saturating_sub(l.tally.miss);
            t.stray = t.stray.saturating_sub(l.tally.stray);
            n = n.saturating_sub(l.notes.len());
        }
        (t, n)
    }

    /// 手で速度を `steps` 刻み動かす（次に並べる周から、助走のあと）。段階アップなら 1 段ずつ、固定なら 5% ずつ。
    /// 今の段はその速度の周から数え直す。クリアしたあとは動かさない
    pub fn nudge_speed(&mut self, steps: i64) {
        if self.over() {
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
    /// パート練習（ほかのパートは自動）と、積み上げの途中の段はクリアに数えない
    pub fn clear_speed(&self) -> Option<u32> {
        if self.options.way == Way::Part || self.options.way.tours() {
            return None;
        }
        self.done
            .iter()
            .zip(&self.full_attempts)
            .filter(|(r, full)| **full && r.cleared())
            .map(|(r, _)| r.speed)
            .max()
    }

    /// 目標の段を越えた（固定の速度なら、その段を越えた）
    pub fn cleared(&self) -> bool {
        self.cleared_at_ms.is_some()
    }

    /// クリアしたか、時間の区切りを過ぎたか、めぐり終えて、もう周を並べない
    pub fn over(&self) -> bool {
        self.cleared_at_ms.is_some() || self.timed_out_at_ms.is_some() || self.toured()
    }

    /// めぐるやり方で、最後の所まで叩き終えた
    pub fn toured(&self) -> bool {
        self.toured_at_ms.is_some()
    }

    /// 時間の区切りまでの残り（ミリ秒。区切りが無ければ None）。最初の周の区間の頭から数える
    pub fn time_left_ms(&self, now_ms: f64) -> Option<f64> {
        let limit = f64::from(self.options.time_box_s?) * 1000.0;
        let from = self.laps.first().map_or(now_ms, |l| l.plan.start_ms);
        Some(limit - (now_ms - from).max(0.0))
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

    /// 目標の段まで、あと何段か（今の段を入れる）。クリアしたら 0。
    /// めぐるやり方は、あと何か所か（今の所を入れる）
    pub fn steps_left(&self) -> usize {
        if self.cleared() || self.toured() {
            return 0;
        }
        if self.options.way.tours() {
            return self.stops().len() - self.stop;
        }
        let goal = self.plan.goal().max(self.speed);
        self.stairs()
            .iter()
            .filter(|&&s| s >= self.speed && s <= goal)
            .count()
    }

    /// 光らせる拍：前の挑戦が「もう一度」なら、そのとき重なった拍とレーン（多い順）。越えたら無し
    pub fn focus_beats(&self) -> Vec<(BeatSlot, Vec<DrumLane>)> {
        // めぐるやり方は次の所が別の所なので、前の所の拍は光らせない
        if self.options.way.tours() {
            return Vec::new();
        }
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
            missed_notes: Vec::new(),
            judged: 0,
            cue,
            settled_at_ms: None,
            rest: false,
            attempt: self.attempt,
            in_attempt: index - self.attempt_start.min(index) + 1,
            demo: std::mem::take(&mut self.next_demo),
            section: self.current_section(),
            muted: std::mem::take(&mut self.next_muted),
            break_offsets: Vec::new(),
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
                if lap.muted.get(e.note - lap.notes.start) == Some(&true) {
                    lap.break_offsets.push(o);
                }
            }
            if matches!(e.judgement, Judgement::Poor | Judgement::Miss)
                && let Some(&(Some(slot), lane, at)) = lap.slots.get(e.note - lap.notes.start)
            {
                lap.misses.push((slot, lane));
                lap.missed_notes.push((at, lane));
                missed |= i >= self.attempt_start;
            }
        }
        if missed
            && !self.options.way.tours()
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

    /// 今の挑戦で落とした音（叩いている途中の周も入れる）。譜面の絵の印
    pub fn attempt_marks(&self) -> Marks {
        marks_of(self.attempt_laps().iter().map(|l| &l.missed_notes))
    }

    /// 挑戦 `attempt`（0 から）で落とした音
    pub fn marks_of_attempt(&self, attempt: usize) -> Marks {
        marks_of(
            self.laps
                .iter()
                .filter(|l| l.attempt == attempt && !l.rest)
                .map(|l| &l.missed_notes),
        )
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
            .filter(|l| !l.rest && !l.demo)
            .filter(|l| l.judged > 0 || !l.strays.is_empty() || l.plan.start_ms <= now_ms)
            .map(LapState::summary)
            .collect()
    }

    /// 画面の時刻（周を並べた実時間）の、譜面の中の時刻。1 周目の助走の間は 1 周目の並びで
    pub fn chart_ms(&self, now_ms: f64) -> f64 {
        let i = self.current_lap(now_ms).unwrap_or(0);
        self.laps.get(i).map_or(now_ms, |l| l.plan.song_ms(now_ms))
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
            .filter(|l| l.complete() && !l.rest)
            .flat_map(|l| l.offsets.iter().copied())
            .collect();
        let (mean, _) = mean_and_sd(&all);
        Some((mean?, all.len()))
    }

    /// 古い薄い音符を捨てる
    pub fn forget_before(&mut self, ms: f64) {
        self.faint.retain(|f| f.0 >= ms);
        self.demo.retain(|f| f.0 >= ms);
    }
}

/// やり方の名前
pub fn way_label(w: Way) -> &'static str {
    match w {
        Way::Climb => t("Step Up", "段をのぼる"),
        Way::Part => t("Part Practice", "パート練習"),
        Way::Demo => t("Watch, Then Play", "お手本と交互"),
        Way::Build => t("Build Up", "積み上げ"),
        Way::Break => t("Song Break", "曲のブレイク"),
        Way::Fade => t("Vanishing Notes", "消える譜面"),
        Way::Tour => t("Weak Spot Tour", "苦手めぐり"),
        Way::Fills => t("Fills Only", "フィルだけ"),
        Way::Seams => t("Transitions", "つなぎ目"),
    }
}

/// パートの名前（「手だけ」）
pub fn part_label(p: Part) -> &'static str {
    match p {
        Part::Hands => t("Hands Only", "手だけ"),
        Part::Feet => t("Feet Only", "足だけ"),
        Part::HiHat => t("Hi-Hat Only", "ハイハットだけ"),
        Part::Snare => t("Snare Only", "スネアだけ"),
        Part::Toms => t("Toms Only", "タムだけ"),
        Part::Cymbals => t("Cymbals Only", "シンバルだけ"),
    }
}

/// 積み上げの向きの名前
pub fn build_label(b: Build) -> &'static str {
    match b {
        Build::Forward => t("From the Start", "前から"),
        Build::Backward => t("From the End", "後ろから"),
    }
}

/// やり方の説明（練習メニューの右の板）。`measures` は区間の長さ（積み上げの長さを出す）、
/// `stops` はめぐる所の数
pub fn way_help(way: Way, part: Part, build: Build, measures: u32, stops: usize) -> String {
    match way {
        Way::Climb => t(
            "Climbs from the start speed to the target speed, one step at a time.",
            "始めの速度から目標の速度まで、段をのぼります。",
        )
        .to_owned(),
        Way::Part => tf!(
            "{}: you play only that part and the rest plays itself. Learn the hard parts one at a time, then put them together.",
            "{}：そのパートだけを叩き、ほかのパートは自動で鳴ります。難しい所を分けて覚え、あとで合わせましょう。",
            part_label(part)
        ),
        Way::Demo => t(
            "Watch and listen to one loop played for you, then play the next loop yourself. Only your loops are scored.",
            "1 周目は自動で叩くのを見て聞き、2 周目に自分で叩きます。点を付けるのは自分の周だけです。",
        )
        .to_owned(),
        Way::Build => {
            let lengths: Vec<String> = build_lengths(measures)
                .iter()
                .map(u32::to_string)
                .collect();
            let path = lengths.join(" → ");
            tf!(
                "Start with 1 measure and make it longer each time you clear it, up to the whole section ({path} measures, {}). The speed stays put until then.",
                "1 小節から始め、クリアするたびに伸ばします（{path} 小節、{}）。区間ぜんぶまで速度は同じです。",
                build_label(build).to_lowercase()
            )
        }
        Way::Break => t(
            "The music and the click drop out every other 2 measures. Keep the beat, and see if you are still with the song when it comes back.",
            "曲の音とクリックが 2 小節ずつ消えます。消えているあいだもリズムを保ち、戻ったときに曲と合っているかを見ます。",
        )
        .to_owned(),
        Way::Fade => t(
            "Notes thin out with each loop of a step, and the last loop shows none. Notes you miss come back in the next loop.",
            "段の中で周を重ねるごとに音符が減り、最後の周は何も見えません。ミスした音は次の周で戻ってきます。",
        )
        .to_owned(),
        Way::Tour => {
            let n = count(stops, "spot", "spots", "か所");
            tf!(
                "Visits your weak spots ({n}) in a mixed-up order, {TOUR_LAPS} loops each, then shows how each went. Mixing them up helps them stick.",
                "苦手な所 {n}を、まぜた順に {TOUR_LAPS} 周ずつめぐり、所ごとの出来を出します。同じ所を続けるより、まぜた方が覚えが残ります。"
            )
        }
        Way::Fills => {
            let n = count(stops, "fill", "fills", "か所");
            tf!(
                "Found {n} in this song (fills with the same shape count once). Plays each one {TOUR_LAPS} times, in order, through the first hit after it.",
                "この曲のフィルを {n}見つけました（同じ形のフィルは 1 つ）。順に {TOUR_LAPS} 周ずつ、フィルのあとの 1 打まで叩きます。"
            )
        }
        Way::Seams => {
            let n = count(stops, "place", "places", "か所");
            tf!(
                "Found {n} where the beat changes (like verse to chorus). Plays the measure before and after each one {TOUR_LAPS} times, in order.",
                "形が変わる所（A メロからサビなど）を {n}見つけました。前後 1 小節ずつを、順に {TOUR_LAPS} 周ずつ叩きます。"
            )
        }
    }
}

/// めぐった所の出来の意味
pub fn mark_label(m: Mark) -> &'static str {
    match m {
        Mark::Clean => t("No misses", "ノーミス"),
        Mark::Fair => t("No repeat misses", "同じ所のミスなし"),
        Mark::Weak => t("Same spot missed twice", "同じ所を 2 周ともミス"),
    }
}

/// 出来の印の見方（1 行）
pub fn mark_line() -> String {
    t(
        "◎ No misses  ○ No repeats  × Same spot twice",
        "◎ ノーミス　○ 同じ所のミスなし　× 2 周とも",
    )
    .to_owned()
}

/// めぐる所が無くて始められないときの知らせ
pub fn no_stops_line(way: Way) -> &'static str {
    match way {
        Way::Tour => t(
            "No weak spots yet. Play the song through once and they show up.",
            "苦手な所がまだありません。1 回通して遊ぶと出てきます。",
        ),
        Way::Fills => t(
            "No fills were found in this song.",
            "この曲ではフィルが見つかりませんでした。",
        ),
        _ => t(
            "No places where the beat changes were found in this song.",
            "この曲では、形が変わる所が見つかりませんでした。",
        ),
    }
}

/// つなぎ方の名前
pub fn style_label(s: LoopStyle) -> &'static str {
    match s {
        LoopStyle::Connect => t("Seamless", "切れ目なく"),
        LoopStyle::Restart => t("Restart Each Loop", "毎回区切る"),
    }
}

/// 段の越え方（「同じ拍で 2 回ミスしなければ、次の段へ」）
pub fn pass_line(r: PassRule) -> String {
    match r.weak_times() {
        1 => t("Move up with no misses.", "ミスしなければ、次の段へ").to_owned(),
        2 => t(
            "Move up unless you miss the same beat twice.",
            "同じ拍で 2 回ミスしなければ、次の段へ",
        )
        .to_owned(),
        n => tf!(
            "Move up unless you miss the same beat {n} times.",
            "同じ拍で {n} 回ミスしなければ、次の段へ"
        ),
    }
}

/// 周ごとの落とした音を、音ごとに「何周で落としたか」に数える（同じ周で同じ音は 1 回）
pub fn marks_of<'a>(laps: impl Iterator<Item = &'a Vec<(NotePos, DrumLane)>>) -> Marks {
    let mut m = Marks::new();
    for lap in laps {
        let mut seen: Vec<&(NotePos, DrumLane)> = Vec::new();
        for k in lap {
            if !seen.contains(&k) {
                seen.push(k);
                *m.entry(*k).or_default() += 1;
            }
        }
    }
    m
}

/// 区間の名前（「小節 3〜6」）
pub fn section_label(s: Section) -> String {
    if s.first == s.last {
        tf!("Measure {}", "小節 {}", s.first)
    } else {
        tf!("Measures {}–{}", "小節 {}〜{}", s.first, s.last)
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
                time_box_s: None,
                way: Default::default(),
                part: Default::default(),
                build: Default::default(),
                stops: Default::default(),
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
            lanes_per_measure: Vec::new(),
            weak,
            clears: Vec::new(),
            picture: ChartPicture::default(),
            marks: Marks::new(),
            section: Some(Section::new(2, 5)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
            way: Default::default(),
            part: Default::default(),
            build: Default::default(),
            fills: Vec::new(),
            seams: Vec::new(),
            weak_tour: Vec::new(),
            measure_ms: Vec::new(),
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
            lanes_per_measure: Vec::new(),
            weak: WeakMap {
                weakness: vec![None],
                phrases: Vec::new(),
                plays: 0,
                practice_laps: 0,
            },
            clears: Vec::new(),
            picture: ChartPicture::default(),
            marks: Marks::new(),
            section: Some(Section::new(0, 0)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
            way: Default::default(),
            part: Default::default(),
            build: Default::default(),
            fills: Vec::new(),
            seams: Vec::new(),
            weak_tour: Vec::new(),
            measure_ms: Vec::new(),
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
                p.on_hit(ms);
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
    fn a_time_box_ends_at_the_end_of_the_step_in_play() {
        let mut p = practice_with(LoopStyle::Connect, LADDER, Section::new(1, 1), 4);
        p.options.time_box_s = Some(1);
        let mut jd = judge();
        assert_eq!(p.time_left_ms(0.0), Some(1000.0));
        for _ in 0..3 {
            play_lap(&mut p, &mut jd, 70, true);
        }
        // 時間を過ぎても、段の途中では止めない
        let last = p.laps.last().unwrap().plan.start_ms;
        assert!(p.time_left_ms(last).unwrap() < 0.0);
        assert!(p.timed_out_at_ms.is_none());
        assert!(p.wants_lap(0.0, 1e12));
        play_lap(&mut p, &mut jd, 70, true);
        // 段を越えても、次の段は並べずに終える（クリアではない）
        assert_eq!(p.done.len(), 1);
        assert_eq!(p.done[0].passed, Some(true));
        assert!(p.timed_out_at_ms.is_some());
        assert!(!p.cleared() && p.over());
        assert!(!p.wants_lap(0.0, 1e12));
        // 区切りの無い練習は続く
        let mut q = practice_with(LoopStyle::Connect, LADDER, Section::new(1, 1), 4);
        let mut jd = judge();
        assert_eq!(q.time_left_ms(0.0), None);
        for _ in 0..4 {
            play_lap(&mut q, &mut jd, 70, true);
        }
        assert!(!q.over());
    }

    #[test]
    fn not_hitting_for_a_lap_rests_and_counts_the_step_again() {
        let mut p = practice_with(LoopStyle::Connect, LADDER, Section::new(1, 2), 4);
        let mut judge = judge();
        play_lap(&mut p, &mut judge, 70, true);
        let end = p.laps[0].plan.end_ms;
        assert!(!p.check_rest(end));
        // 2 周目は何も叩かない。1 周ぶん叩かずに過ぎたら休み
        let i = play_lap(&mut p, &mut judge, 70, false);
        let end = p.laps[i].plan.end_ms;
        assert!(!p.check_rest(end - 3000.0));
        assert!(p.check_rest(end + 10.0));
        // 叩かなかった周は数えず、もう一度にもしない。休みのあいだは周を並べない
        assert!(p.laps[i].rest && !p.laps[0].rest);
        assert!(!p.wants_lap(end, 1e12));
        assert!(!p.check_rest(end + 5000.0));
        assert_eq!(p.summaries(end).len(), 1);
        let (t, n) = p.without_rest(judge.tally(), judge.notes().len());
        assert_eq!((t.miss, n), (0, p.laps[0].notes.len()));
        assert!(p.done.is_empty());
        assert_eq!(p.again_in_row(), 0);
        // 並べた周を叩き終えたら、同じ段を助走から数え直す
        p.settle(end + 20.0);
        assert!(p.wants_lap(end + 20.0, 0.0));
        let j = play_lap(&mut p, &mut judge, 70, true);
        assert_eq!(p.laps[j].cue, Some(Cue::Start));
        assert!(p.laps[j].plan.has_run_up());
        assert_eq!(p.laps[j].in_attempt, 1);
        assert_eq!(p.speed(), 70);
        // 数え直したあとは、そこから 1 周ぶん待つ
        assert!(!p.check_rest(end + 1000.0));
    }

    #[test]
    fn all_auto_lanes_never_rest() {
        let mut p = practice_with(LoopStyle::Connect, LADDER, Section::new(1, 2), 4);
        p.set_auto_lanes(&[DrumLane::SD]);
        let mut judge = judge();
        let i = play_lap(&mut p, &mut judge, 70, false);
        assert!(!p.check_rest(p.laps[i].plan.end_ms + 60_000.0));
        p.set_auto_lanes(&[DrumLane::BD]);
        assert!(p.check_rest(p.laps[i].plan.end_ms + 60_000.0));
    }

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
            lanes_per_measure: Vec::new(),
            weak: WeakMap {
                weakness: vec![None; 10],
                phrases: Vec::new(),
                plays: 0,
                practice_laps: 0,
            },
            clears: Vec::new(),
            picture: ChartPicture::default(),
            marks: Marks::new(),
            section: None,
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
            way: Default::default(),
            part: Default::default(),
            build: Default::default(),
            fills: Vec::new(),
            seams: Vec::new(),
            weak_tour: Vec::new(),
            measure_ms: Vec::new(),
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

    fn practice_way(way: Way, speed: SpeedPlan, section: Section, laps: u32) -> Practice {
        let mut p = practice_with(LoopStyle::Connect, speed, section, laps);
        p.options.way = way;
        // 積み上げの長さは作るときに決まるので、作り直す
        let (tempo, notes) = chart();
        Practice::new(p.options, &tempo, notes)
    }

    #[test]
    fn demo_loops_alternate_and_only_your_loops_count() {
        // 1 段 3 周は、お手本と自分の 2 組（4 周）にする
        let mut p = practice_way(Way::Demo, SpeedPlan::Fixed(100), Section::new(1, 1), 3);
        let mut judge = judge();
        assert_eq!(p.step_laps(), 4);
        for k in 0..4 {
            let i = play_lap(&mut p, &mut judge, 100, true);
            let demo = k % 2 == 0;
            assert_eq!(p.laps[i].demo, demo, "lap {k}");
            if demo {
                // お手本の周は判定せず、音符は自動で鳴らす
                assert!(p.laps[i].notes.is_empty());
                assert_eq!(p.take_demo().len(), 4);
            } else {
                assert_eq!(p.laps[i].notes.len(), 4);
                assert!(p.take_demo().is_empty());
            }
        }
        assert_eq!(p.demo.len(), 8);
        let a = p.done.last().expect("the step is over");
        assert_eq!((a.laps, a.passed), (2, Some(true)));
        assert!(p.cleared());
        assert_eq!(p.summaries(f64::MAX).len(), 2);
    }

    #[test]
    fn build_up_grows_from_one_measure_at_the_same_speed() {
        let section = Section::new(0, 3);
        let mut p = practice_way(Way::Build, SpeedPlan::Fixed(100), section, 2);
        let mut judge = judge();
        assert_eq!(p.current_section(), Section::new(0, 0));
        assert_eq!(p.build_stage(), Some((&[1u32, 2, 4][..], 0)));
        for _ in 0..2 {
            play_lap(&mut p, &mut judge, 100, true);
        }
        // 1 小節を越えたら、同じ速度で 2 小節へ（クリアには数えない）
        assert_eq!(p.current_section(), Section::new(0, 1));
        assert!(!p.cleared());
        assert_eq!(p.clear_speed(), None);
        assert_eq!(p.speed(), 100);
        let i = play_lap(&mut p, &mut judge, 100, true);
        assert_eq!(p.laps[i].cue, Some(Cue::Grow { from: 1, to: 2 }));
        assert_eq!(p.laps[i].notes.len(), 8);
        play_lap(&mut p, &mut judge, 100, true);
        assert_eq!(p.current_section(), section);
        for _ in 0..2 {
            play_lap(&mut p, &mut judge, 100, true);
        }
        // 区間ぜんぶを越えてクリア
        assert!(p.cleared());
        assert_eq!(p.clear_speed(), Some(100));
        // 後ろからは終わりの小節から
        let mut p = practice_way(Way::Build, SpeedPlan::Fixed(100), section, 2);
        p.options.build = Build::Backward;
        let (tempo, notes) = chart();
        let p = Practice::new(p.options, &tempo, notes);
        assert_eq!(p.current_section(), Section::new(3, 3));
    }

    #[test]
    fn song_break_mutes_two_measures_and_keeps_the_offsets_there() {
        let mut p = practice_way(Way::Break, SpeedPlan::Fixed(100), Section::new(0, 3), 4);
        let mut judge = judge();
        let i = play_lap(&mut p, &mut judge, 100, true);
        // BPM 120 で 1 小節 2000ms。小節 2〜3 が消える
        assert_eq!(p.break_windows(&p.laps[i].plan), vec![(4000.0, 8000.0)]);
        assert_eq!(p.laps[i].break_offsets.len(), 8);
        assert_eq!(p.break_offset(), Some((0.0, 8)));
        // 1 小節の区間は、周をまたいで 2 小節鳴らして 2 小節消す
        let mut p = practice_way(Way::Break, SpeedPlan::Fixed(100), Section::new(1, 1), 4);
        let mut judge = super::tests::judge();
        let muted: Vec<bool> = (0..4)
            .map(|_| {
                let i = play_lap(&mut p, &mut judge, 100, true);
                !p.break_windows(&p.laps[i].plan).is_empty()
            })
            .collect();
        assert_eq!(muted, [false, false, true, true]);
        // ほかのやり方では消さない
        let p = practice(LoopStyle::Connect);
        let lap = engine::practice::Looper::new(p.times(), LoopStyle::Connect).next_lap(1.0, false);
        assert!(p.break_windows(&lap).is_empty());
    }

    #[test]
    fn vanishing_notes_thin_out_and_misses_come_back() {
        let mut p = practice_way(Way::Fade, SpeedPlan::Fixed(100), Section::new(0, 3), 4);
        let mut judge = judge();
        let shown = |p: &Practice, i: usize| {
            p.laps[i]
                .notes
                .clone()
                .filter(|&n| p.note_alpha(n) > 0.0)
                .count()
        };
        let i = play_lap(&mut p, &mut judge, 100, true);
        assert_eq!(shown(&p, i), 16);
        let i = play_lap(&mut p, &mut judge, 100, true);
        let second = shown(&p, i);
        assert!((6..16).contains(&second), "{second}");
        // 3 周目で 6 番目の音符を落とすと、4 周目（何も見えない周）でもその音符だけ見える
        let i = play_lap_missing(&mut p, &mut judge, 100, &[5]);
        assert!(shown(&p, i) <= second);
        let (lap, notes) = p.next_lap(100, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range.clone());
        let visible: Vec<usize> = range.filter(|&n| p.note_alpha(n) > 0.0).collect();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0] - p.laps[3].notes.start, 5);
        assert_eq!(p.fade_share_of(3), Some(0.0));
    }

    #[test]
    fn part_practice_needs_notes_for_the_part_and_never_counts_as_a_clear() {
        use engine::practice_way::lane_mask;
        let mut s = PracticeSetup {
            path: PathBuf::new(),
            title: String::new(),
            artist: String::new(),
            notes_per_measure: vec![4; 4],
            lanes_per_measure: vec![
                lane_mask([DrumLane::SD]),
                lane_mask([DrumLane::HT, DrumLane::BD]),
                0,
                lane_mask([DrumLane::SD]),
            ],
            weak: WeakMap {
                weakness: vec![None; 4],
                phrases: Vec::new(),
                plays: 0,
                practice_laps: 0,
            },
            clears: Vec::new(),
            picture: ChartPicture::default(),
            marks: Marks::new(),
            section: Some(Section::new(0, 0)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
            way: Way::Part,
            part: Part::Toms,
            build: Build::Forward,
            fills: Vec::new(),
            seams: Vec::new(),
            weak_tour: Vec::new(),
            measure_ms: Vec::new(),
        };
        assert_eq!(
            s.parts(),
            vec![Part::Hands, Part::Feet, Part::Snare, Part::Toms]
        );
        // 小節 0 にタムは無いので始められない
        assert!(s.options().is_none());
        s.section = Some(Section::new(0, 1));
        let o = s.options().expect("measure 1 has toms");
        assert_eq!((o.way, o.part), (Way::Part, Part::Toms));
        // 自動で叩くパートがあるので、越えても区間のクリアには残さない
        let mut p = practice_way(Way::Part, SpeedPlan::Fixed(100), Section::new(1, 1), 2);
        let mut judge = judge();
        for _ in 0..2 {
            play_lap(&mut p, &mut judge, 100, true);
        }
        assert!(p.cleared());
        assert_eq!(p.clear_speed(), None);
    }

    #[test]
    fn a_tour_plays_each_stop_twice_and_marks_it() {
        let mut o = practice_with(
            LoopStyle::Connect,
            SpeedPlan::Ladder {
                start: 80,
                target: 100,
            },
            Section::new(0, 0),
            4,
        )
        .options;
        o.way = Way::Fills;
        o.stops = Stops::new(&[Section::new(3, 3), Section::new(1, 1)]);
        let (tempo, notes) = chart();
        let mut p = Practice::new(o, &tempo, notes);
        let mut judge = judge();
        // 1 か所 2 周、毎回区切る。区間の音は所ごと
        assert_eq!(p.step_laps(), 2);
        assert_eq!(p.looper.style(), LoopStyle::Restart);
        assert_eq!(p.current_section(), Section::new(3, 3));
        assert_eq!(p.shown_section(), Section::new(3, 3));
        assert_eq!(
            p.audio_sections(),
            vec![Section::new(3, 3), Section::new(1, 1)]
        );
        assert_eq!(p.steps_left(), 2);
        // 同じ拍を 2 周とも落としても、途中で打ち切らずに 2 周叩く
        play_lap_missing(&mut p, &mut judge, 80, &[0]);
        assert!(p.wants_lap(0.0, 0.0));
        play_lap_missing(&mut p, &mut judge, 80, &[0]);
        assert_eq!(p.done.len(), 1);
        assert_eq!(p.stop_index(), Some(1));
        assert_eq!(p.current_section(), Section::new(1, 1));
        assert_eq!(p.speed(), 80);
        assert!(p.focus_beats().is_empty());
        // 次の所は助走から。着地（所の外の 1 打）は 2 周とも落としても × にしない
        let i = play_lap_missing(&mut p, &mut judge, 80, &[4]);
        assert_eq!(p.laps[i].cue, Some(Cue::Stop { index: 1 }));
        assert_eq!(p.laps[i].notes.len(), 5);
        assert!(!p.toured());
        play_lap_missing(&mut p, &mut judge, 80, &[4]);
        assert!(p.toured() && p.over());
        assert!(!p.wants_lap(0.0, 0.0));
        assert_eq!(p.stop_marks(), vec![Some(Mark::Weak), Some(Mark::Fair)]);
        assert_eq!(p.steps_left(), 0);
        // めぐるやり方は区間のクリアに残さない
        assert!(!p.cleared());
        assert_eq!(p.clear_speed(), None);
    }

    #[test]
    fn the_setup_tours_weak_spots_fills_or_seams() {
        let mut s = PracticeSetup {
            path: PathBuf::new(),
            title: String::new(),
            artist: String::new(),
            notes_per_measure: vec![4; 8],
            lanes_per_measure: Vec::new(),
            weak: WeakMap {
                weakness: vec![None; 8],
                phrases: Vec::new(),
                plays: 0,
                practice_laps: 0,
            },
            clears: Vec::new(),
            picture: ChartPicture::default(),
            marks: Marks::new(),
            section: Some(Section::new(0, 1)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::Fixed(100),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: STEP_LAPS,
            way: Way::Tour,
            part: Part::Hands,
            build: Build::Forward,
            fills: vec![Section::new(3, 3), Section::new(7, 7)],
            seams: Vec::new(),
            weak_tour: Vec::new(),
            measure_ms: (0..=8).map(|m| f64::from(m) * 2000.0).collect(),
        };
        // 苦手な所が無ければ、苦手めぐりは始められない
        assert!(s.options().is_none());
        let spots: Vec<Section> = (0..7).map(|m| Section::new(m, m)).collect();
        s.set_weak_tour(&spots, 7);
        // 苦手な方から 5 か所を、まぜた順に
        let mut tour = s.stops();
        assert_eq!(tour.len(), MAX_STOPS);
        assert_ne!(tour, spots[..MAX_STOPS].to_vec());
        tour.sort_by_key(|s| s.first);
        assert_eq!(tour, spots[..MAX_STOPS].to_vec());
        let o = s.options().expect("weak spots to tour");
        assert_eq!(o.stops.len(), MAX_STOPS);
        // 練習する区間は選んでいたまま
        assert_eq!(o.section, Section::new(0, 1));
        s.way = Way::Fills;
        assert_eq!(
            s.options().map(|o| o.stops),
            Some(Stops::new(&[Section::new(3, 3), Section::new(7, 7)]))
        );
        // 1 か所は 1 小節の助走 + 1 小節 + 着地を 2 周（1 小節 2 秒）
        assert!(
            (s.tour_seconds() - 2.0 * 2.0 * 4.6).abs() < 1e-6,
            "{}",
            s.tour_seconds()
        );
        s.way = Way::Seams;
        assert!(s.options().is_none());
    }
}
