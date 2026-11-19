//! 練習（区間ループ）。区間を選ぶ画面の中身と、練習中の周ごとの集計。
//!
//! 周の並べ方と速度の決め方は `engine::practice` が決め、ここではそれを判定・音・記録につなぐ。
//! 判定の音符は周ごとに後ろへ足していくので、判定の音符の添字から「何周目か」を引けるようにしておく。
//!
//! 周の合否で次の周の速度を決めるので、周は判定と表示に要るぶんだけ先に並べる。
//! つなぐときは次の周がもう落ちてきているので、決めた速度は次の次の周から。区切るときは合否が出てから次の周を並べる。

use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dtx::{DrumLane, DrumNote, TempoMap};
use engine::analysis::mean_and_sd;
use engine::judge::{JudgeEvent, Tally};
use engine::practice::{
    Ladder, Lap, LoopStyle, Looper, PassRule, SPEED_STEP, Section, SectionTimes, SpeedChange,
    SpeedPlan, snap_speed, speed_percent,
};
use records::StrayHit;

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
}

/// 区間のほかの決めごと（設定に残し、次の練習でも使う）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PracticePrefs {
    pub style: LoopStyle,
    pub rule: PassRule,
    pub speed: SpeedPlan,
    pub pitch: Pitch,
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
    pub section: Option<Section>,
    pub style: LoopStyle,
    pub rule: PassRule,
    pub speed: SpeedPlan,
    pub pitch: Pitch,
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
        let section = previous
            .map(|p| p.section.clamped(notes_per_measure.len()))
            .or_else(|| default_section(&weak, &notes_per_measure));
        Ok(Self {
            path: path.to_owned(),
            title: chart.title().to_owned(),
            artist: chart.artist().to_owned(),
            notes_per_measure,
            weak,
            section,
            style: previous.map_or(prefs.style, |p| p.style),
            rule: previous.map_or(prefs.rule, |p| p.rule),
            speed: previous.map_or(prefs.speed, |p| p.speed),
            pitch: previous.map_or(prefs.pitch, |p| p.pitch),
        })
    }

    /// 今の決めごと（設定に残す）
    pub fn prefs(&self) -> PracticePrefs {
        PracticePrefs {
            style: self.style,
            rule: self.rule,
            speed: self.speed,
            pitch: self.pitch,
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

/// 練習中の 1 周
#[derive(Debug, Clone)]
pub struct LapState {
    pub plan: Lap,
    /// 判定の音符の添字の範囲
    pub notes: Range<usize>,
    pub tally: Tally,
    offsets: Vec<f64>,
    pub strays: Vec<StrayHit>,
    /// 判定が済んだ音符の数
    pub judged: usize,
}

impl LapState {
    pub fn complete(&self) -> bool {
        self.judged >= self.notes.len()
    }

    pub fn summary(&self, rule: PassRule) -> LapSummary {
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
            passed: completed.then(|| rule.passes(&self.tally, self.notes.len(), sd)),
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
    /// 合格か。終わっていない周は None
    pub passed: Option<bool>,
}

impl LapSummary {
    /// PERFECT の割合（0〜1）
    pub fn perfect_rate(&self) -> f64 {
        if self.notes == 0 {
            0.0
        } else {
            f64::from(self.tally.perfect) / self.notes as f64
        }
    }
}

/// 速度が動いたこと（練習中の画面に少しのあいだ出す）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedNote {
    pub change: SpeedChange,
    /// 動いたあとの速度（%）
    pub speed: u32,
    /// 決めた実時間
    pub at_ms: f64,
    /// その速度で並べる最初の周（0 から）
    pub from_lap: usize,
}

/// 練習 1 回の周の並びと集計
pub struct Practice {
    pub options: PracticeOptions,
    looper: Looper,
    ladder: Ladder,
    /// 合否を速度の決めに使い終えた周の数（周は順に終わる）
    settled: usize,
    /// 最後に速度が動いたこと
    pub last_change: Option<SpeedNote>,
    /// 曲全体の音符（曲の時刻）
    chart_notes: Vec<DrumNote>,
    pub laps: Vec<LapState>,
    /// 判定の音符の添字 → 周
    lap_of: Vec<usize>,
    /// 判定しない音符（助走など。実時間、レーン）。薄く見せる
    pub faint: Vec<(f64, DrumLane)>,
    /// 曲全体の小節の帯（苦手度）。区間の場所を見せる。始めてから記録を読んで入れる
    pub band: Vec<Option<f64>>,
}

impl Practice {
    pub fn new(options: PracticeOptions, tempo: &TempoMap, chart_notes: Vec<DrumNote>) -> Self {
        let times = SectionTimes::new(tempo, options.section);
        Self {
            options,
            looper: Looper::new(times, options.style),
            ladder: Ladder::new(options.speed),
            settled: 0,
            last_change: None,
            chart_notes,
            laps: Vec::new(),
            lap_of: Vec::new(),
            faint: Vec::new(),
            band: Vec::new(),
        }
    }

    /// 区間の曲の時刻
    pub fn times(&self) -> &SectionTimes {
        self.looper.times()
    }

    pub fn ladder(&self) -> &Ladder {
        &self.ladder
    }

    /// 1 周目の助走が始まる実時間
    pub fn first_audio_ms(&self) -> f64 {
        self.looper
            .first_audio_ms(f64::from(self.ladder.speed()) / 100.0)
    }

    /// 次の周を並べる時か。`now_ms` は今の実時間、`ahead_ms` は落ちてくる音符が見え始めるまでの時間。
    /// つなぐときは次の周の音が `ahead_ms` 先に迫ったら、区切るときは前の周の合否が出たら並べる
    pub fn wants_lap(&self, now_ms: f64, ahead_ms: f64) -> bool {
        if self.laps.is_empty() {
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

    /// 次の周を速度 `percent` で並べる。区切るときは助走を実時間 `earliest_ms` より前には始めない。
    /// 返すのは周と、判定に足す音符
    pub fn next_lap(&mut self, percent: u32, earliest_ms: f64) -> (Lap, Vec<DrumNote>) {
        self.looper.delay_to(earliest_ms);
        let lap = self.looper.next_lap(f64::from(percent) / 100.0);
        let mut judged = Vec::new();
        for n in lap.notes(&self.chart_notes) {
            let note = &self.chart_notes[n.note];
            if n.judged {
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

    /// 成績が動かなくなった周の合否で、次の速度を決める。`poor_ms` は判定の幅の一番外
    pub fn settle(&mut self, now_ms: f64, poor_ms: f64) {
        while let Some(lap) = self.laps.get(self.settled) {
            if !lap.complete() || now_ms < lap.plan.stray_until(poor_ms) {
                break;
            }
            let passed = lap.summary(self.options.rule).passed == Some(true);
            let change = self.ladder.record(speed_percent(lap.plan.speed), passed);
            if change != SpeedChange::Stay {
                self.last_change = Some(SpeedNote {
                    change,
                    speed: self.ladder.speed(),
                    at_ms: now_ms,
                    from_lap: self.laps.len(),
                });
            }
            self.settled += 1;
        }
    }

    /// 手で速度を `steps` 刻み動かす（次に並べる周から）
    pub fn nudge_speed(&mut self, steps: i64) {
        let s = i64::from(self.ladder.speed()) + steps * i64::from(SPEED_STEP);
        self.ladder.set_speed(snap_speed(s));
    }

    /// 始め直すときの決めごと。固定の速度を手で変えていたらその速度で（段階アップは始めの速度から）
    pub fn restart_options(&self) -> PracticeOptions {
        PracticeOptions {
            speed: self.ladder.plan(),
            ..self.options
        }
    }

    /// 先に作っておく速度（%）。次の速度と、その上下
    pub fn speeds_to_prepare(&self) -> Vec<u32> {
        let s = self.ladder.speed();
        let mut v = vec![s];
        for d in [1, -1] {
            let n = snap_speed(i64::from(s) + d * i64::from(SPEED_STEP));
            if !v.contains(&n) {
                v.push(n);
            }
        }
        v
    }

    /// 並べた周を覚える。`notes` は判定に足した音符の添字の範囲
    pub fn push_lap(&mut self, plan: Lap, notes: Range<usize>) {
        let index = self.laps.len();
        self.lap_of.resize(notes.end, index);
        self.laps.push(LapState {
            plan,
            notes,
            tally: Tally::default(),
            offsets: Vec::new(),
            strays: Vec::new(),
            judged: 0,
        });
    }

    /// 判定の結果を周ごとに数える
    pub fn on_events(&mut self, events: &[JudgeEvent]) {
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
        }
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
            .map(|l| l.summary(self.options.rule))
            .collect()
    }

    /// 今叩いている周の番号（0 から）。助走の間は None
    pub fn current_lap(&self, now_ms: f64) -> Option<usize> {
        self.laps
            .iter()
            .rposition(|l| l.plan.audio.at_ms <= now_ms)
            .filter(|&i| i > 0 || now_ms >= self.laps[0].plan.start_ms - 1.0)
    }

    /// 連続で合格した周の数（最後に終わった周から数える）
    pub fn pass_streak(&self) -> usize {
        self.laps
            .iter()
            .filter(|l| l.complete())
            .rev()
            .take_while(|l| l.summary(self.options.rule).passed == Some(true))
            .count()
    }

    /// 速度が動いたことを今も出しておくか。決めてから少しのあいだと、その速度の周が始まって少しのあいだ
    pub fn speed_note(&self, now_ms: f64) -> Option<SpeedNote> {
        const SHOW_MS: f64 = 3000.0;
        self.last_change.filter(|c| {
            now_ms - c.at_ms < SHOW_MS
                || self
                    .laps
                    .get(c.from_lap)
                    .is_none_or(|l| now_ms < l.plan.start_ms + SHOW_MS)
        })
    }

    /// 古い薄い音符を捨てる
    pub fn forget_before(&mut self, ms: f64) {
        self.faint.retain(|f| f.0 >= ms);
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
        let (tempo, notes) = chart();
        Practice::new(
            PracticeOptions {
                section: Section::new(1, 1),
                style,
                rule: PassRule::Normal,
                speed,
                pitch: Pitch::Keep,
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
        assert_eq!(p.pass_streak(), 0);
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
        assert_eq!(p.pass_streak(), 1);
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
            section: Some(Section::new(2, 5)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
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
            section: Some(Section::new(0, 0)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
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
        let (lap, notes) = p.next_lap(speed, f64::MIN);
        let range = judge.extend(notes);
        p.push_lap(lap, range.clone());
        let notes: Vec<(f64, DrumLane)> = judge.notes()[range.clone()]
            .iter()
            .map(|n| (n.ms, n.lane))
            .collect();
        for (ms, lane) in notes {
            let ev = if hit {
                judge.hit(engine::judge::Hit {
                    ms,
                    lane,
                    velocity: 100,
                })
            } else {
                judge.advance(ms + 200.0)
            };
            p.on_events(&ev);
        }
        p.laps.len() - 1
    }

    #[test]
    fn passing_laps_raise_the_speed_of_later_laps() {
        let mut p = practice_at(
            LoopStyle::Connect,
            SpeedPlan::Ladder {
                start: 70,
                target: 100,
            },
        );
        let mut judge =
            Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default());
        // 70% なら助走（1 小節 = 2000ms）は 2857ms
        assert!((p.first_audio_ms() + 2000.0 / 0.7).abs() < 1e-6);
        assert_eq!(p.speeds_to_prepare(), vec![70, 75, 65]);
        for _ in 0..2 {
            let speed = p.ladder().speed();
            let i = play_lap(&mut p, &mut judge, speed, true);
            let end = p.laps[i].plan.end_ms;
            // 周の終わりまでは決めない（空打ちを数え終えるまで）
            p.settle(end - 200.0, 117.0);
            p.settle(end, 117.0);
        }
        assert_eq!(p.ladder().speed(), 75);
        assert_eq!(
            p.last_change.map(|c| (c.change, c.speed)),
            Some((SpeedChange::Up, 75))
        );
        let i = play_lap(&mut p, &mut judge, 75, true);
        assert!((p.laps[i].plan.speed - 0.75).abs() < 1e-9);
        // 3 周目は 75%: 区間（2000ms）は 2667ms
        let l = &p.laps[i].plan;
        assert!((l.end_ms - l.start_ms - 2000.0 / 0.75).abs() < 1e-6);
        // 手で下げる。始め直すときは始めの速度から
        p.nudge_speed(-2);
        assert_eq!(p.ladder().speed(), 65);
        assert_eq!(p.restart_options(), p.options);
        // 固定なら手で変えた速度で始め直す
        let mut f = practice_at(LoopStyle::Connect, SpeedPlan::Fixed(80));
        f.nudge_speed(1);
        assert_eq!(f.restart_options().speed, SpeedPlan::Fixed(85));
    }

    #[test]
    fn restart_waits_for_the_verdict_before_the_next_lap() {
        let mut p = practice(LoopStyle::Restart);
        let mut judge =
            Judge::with_groups(Vec::new(), HitWindows::default(), LaneGroups::default());
        assert!(p.wants_lap(-10_000.0, 2000.0));
        let i = play_lap(&mut p, &mut judge, 100, false);
        assert!(!p.wants_lap(0.0, 2000.0));
        let end = p.laps[i].plan.end_ms;
        p.settle(end + 117.0, 117.0);
        assert!(p.wants_lap(end + 117.0, 2000.0));
        // 助走は合否が出てから 2000ms 先（音の終わりの 3000 より後）
        let (lap, _) = p.next_lap(100, end + 117.0 + 2000.0);
        assert!((lap.audio.at_ms - 4117.0).abs() < 1e-6);
    }
}
