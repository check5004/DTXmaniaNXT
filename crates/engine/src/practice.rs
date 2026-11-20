//! 練習（区間ループ）。曲の区間を周回ぶん後ろへ並べた 1 本の時間軸を作る。
//!
//! 時間軸は「練習を始めてからの実時間」で、巻き戻らない。区間の頭を A、終わり（次の小節の頭）を B、
//! 周 k の区間の頭の時刻を T_k、速度を s_k とすると、曲の時刻 x は `T_k + (x − A) / s_k` に置かれる。
//! 1 周目の頭が 0 で、その前に助走（区間の 1 小節前からの BGM と 4 拍のカウント）が入る。
//!
//! 判定の幅とずれは実時間のミリ秒のまま使うので、遅くしても判定は甘くならない。
//!
//! 周の速度は `Ladder` が周の合否から決める（段階アップ）。速度を変えるのは周の切れ目だけで、
//! 決めた速度は次に並べる周から使う。

use dtx::{DrumNote, TempoMap};

use crate::judge::Tally;

/// 練習する区間。小節の範囲（両端を含む）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub first: u32,
    pub last: u32,
}

impl Section {
    /// 2 つの小節から作る（順は問わない）
    pub fn new(a: u32, b: u32) -> Self {
        Self {
            first: a.min(b),
            last: a.max(b),
        }
    }

    pub fn measures(&self) -> u32 {
        self.last - self.first + 1
    }

    pub fn contains(&self, measure: u32) -> bool {
        (self.first..=self.last).contains(&measure)
    }

    /// 曲の小節の数 `measure_count` に収める（譜面を直して小節が減ったときなど）
    pub fn clamped(&self, measure_count: usize) -> Self {
        let max = measure_count.saturating_sub(1) as u32;
        Self::new(self.first.min(max), self.last.min(max))
    }

    /// `delta` 小節ずらす。曲の外（0 より前、`measure_count` 以降）にははみ出さない
    pub fn shifted(&self, delta: i64, measure_count: usize) -> Self {
        let len = i64::from(self.last - self.first);
        let max_first = (measure_count as i64 - 1 - len).max(0);
        let first = (i64::from(self.first) + delta).clamp(0, max_first);
        Self {
            first: first as u32,
            last: (first + len) as u32,
        }
    }
}

/// 周のつなぎ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopStyle {
    /// 区間の終わりから切れ目なく区間の頭へ戻る（ビート、繰り返すパターン向き）
    #[default]
    Connect,
    /// 次の小節の頭の 1 打（着地）まで判定し、助走からやり直す（フィル、曲の変わり目向き）
    Restart,
}

impl LoopStyle {
    pub const ALL: [LoopStyle; 2] = [Self::Connect, Self::Restart];

    /// 記録や設定に書く名前
    pub fn name(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Restart => "restart",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// 区切るときに、着地のあと BGM を鳴らし続ける長さ（拍）
const RESTART_TAIL_BEATS: f64 = 2.0;

/// 助走でカウントする拍の数
pub const COUNT_IN_BEATS: usize = 4;

/// 区間を曲の時刻で表したもの
#[derive(Debug, Clone, PartialEq)]
pub struct SectionTimes {
    pub section: Section,
    /// 区間の頭（A）
    pub start_ms: f64,
    /// 区間の終わり = 次の小節の頭（B）
    pub end_ms: f64,
    /// 助走の始まり。区間の 1 小節前と 4 拍前の早いほう（曲頭より前ならテンポを延ばして数える）
    pub run_up_ms: f64,
    /// カウントのクリックの時刻（区間の頭の 4 拍前から 1 拍ずつ）
    pub count_in_ms: [f64; COUNT_IN_BEATS],
    /// 区切るときに鳴らし終える時刻（着地の 2 拍あと）
    pub tail_ms: f64,
}

impl SectionTimes {
    pub fn new(tempo: &TempoMap, section: Section) -> Self {
        let a = tempo.measure_start_beat(section.first as usize);
        let b = tempo.measure_start_beat(section.last as usize + 1);
        let prev = if section.first > 0 {
            tempo.measure_start_beat(section.first as usize - 1)
        } else {
            a
        };
        let count_from = a - COUNT_IN_BEATS as f64;
        let run_up = prev.min(count_from);
        Self {
            section,
            start_ms: tempo.beat_to_ms(a),
            end_ms: tempo.beat_to_ms(b),
            run_up_ms: tempo.beat_to_ms(run_up),
            count_in_ms: std::array::from_fn(|i| tempo.beat_to_ms(count_from + i as f64)),
            tail_ms: tempo.beat_to_ms(b + RESTART_TAIL_BEATS),
        }
    }
}

/// 曲の時刻 `[from_ms, to_ms)` を、実時間 `at_ms` から鳴らす
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub from_ms: f64,
    pub to_ms: f64,
    pub at_ms: f64,
}

/// 並べた 1 周
#[derive(Debug, Clone, PartialEq)]
pub struct Lap {
    /// 0 から数える
    pub index: usize,
    pub speed: f64,
    pub style: LoopStyle,
    /// 区間の頭（A）の実時間（T_k）
    pub start_ms: f64,
    /// 判定する部分の終わり（B）の実時間
    pub end_ms: f64,
    /// 区間の頭（A）の曲の時刻
    song_start_ms: f64,
    song_end_ms: f64,
    /// この周で鳴らす曲の範囲（助走や着地のあとを含む）
    pub audio: Span,
    /// カウントのクリックの実時間
    pub clicks: Vec<f64>,
}

impl Lap {
    /// 曲の時刻 → この周での実時間
    pub fn real_ms(&self, song_ms: f64) -> f64 {
        self.start_ms + (song_ms - self.song_start_ms) / self.speed
    }

    /// この周での実時間 → 曲の時刻
    pub fn song_ms(&self, real_ms: f64) -> f64 {
        self.song_start_ms + (real_ms - self.start_ms) * self.speed
    }

    /// 曲の時刻 `song_ms` の音符を、この周で判定するか。区間の中の音符と、区切るときは着地の音符
    pub fn judges(&self, song_ms: f64) -> bool {
        const EPS: f64 = 1e-6;
        let inside = song_ms >= self.song_start_ms - EPS && song_ms < self.song_end_ms - EPS;
        let landing = self.style == LoopStyle::Restart && (song_ms - self.song_end_ms).abs() <= EPS;
        inside || landing
    }

    /// 実時間 `ms` の空打ちを、この周のものとして数えるか。助走や着地のあとの空打ちは数えない。
    /// `poor_ms` は判定の幅の一番外（音符に届かない早さ・遅さの空打ちもその周に入れる）
    pub fn owns_stray(&self, ms: f64, poor_ms: f64) -> bool {
        ms >= self.start_ms - poor_ms && ms < self.stray_until(poor_ms)
    }

    /// この周の空打ちを数え終える実時間。これを過ぎて音符もすべて判定し終えたら、周の成績は動かない
    pub fn stray_until(&self, poor_ms: f64) -> f64 {
        match self.style {
            // つなぐときは次の周の頭の少し前まで（それより後は次の周の早打ち）
            LoopStyle::Connect => self.end_ms - poor_ms,
            LoopStyle::Restart => self.end_ms + poor_ms,
        }
    }

    /// この周に並べる音符。曲の音符 `notes` のうち、この周で鳴らす範囲にあるもの
    pub fn notes(&self, notes: &[DrumNote]) -> Vec<LapNote> {
        let from = notes.partition_point(|n| n.ms < self.audio.from_ms - 1e-6);
        let to = notes.partition_point(|n| n.ms < self.audio.to_ms - 1e-6);
        // 区切るときは、着地の音符（B ちょうど）が鳴らす範囲の外になることはない（tail > B）
        (from..to)
            .map(|i| LapNote {
                note: i,
                ms: self.real_ms(notes[i].ms),
                judged: self.judges(notes[i].ms),
            })
            .collect()
    }
}

/// 周に並べた音符 1 つ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LapNote {
    /// 曲の音符の添字
    pub note: usize,
    /// 実時間
    pub ms: f64,
    /// 判定するか（助走や着地のあとの音符は見せるだけ）
    pub judged: bool,
}

/// 周を順に並べる
#[derive(Debug, Clone)]
pub struct Looper {
    times: SectionTimes,
    style: LoopStyle,
    next_index: usize,
    /// 次の周の音を鳴らし始められる実時間（前の周の音の終わり）
    cursor_ms: f64,
}

impl Looper {
    pub fn new(times: SectionTimes, style: LoopStyle) -> Self {
        Self {
            times,
            style,
            next_index: 0,
            cursor_ms: 0.0,
        }
    }

    pub fn times(&self) -> &SectionTimes {
        &self.times
    }

    pub fn style(&self) -> LoopStyle {
        self.style
    }

    /// 1 周目の助走が始まる実時間（`speed` は 1 周目の速度）
    pub fn first_audio_ms(&self, speed: f64) -> f64 {
        -(self.times.start_ms - self.times.run_up_ms) / speed
    }

    /// 次の周の音が始まる実時間（まだ並べていない周）
    pub fn next_audio_ms(&self, speed: f64) -> f64 {
        if self.next_index == 0 {
            self.first_audio_ms(speed)
        } else {
            self.cursor_ms
        }
    }

    /// 次の周の音を実時間 `audio_ms` より前には始めない。区切るときだけ効く（つなぐときは切れ目を作らない）。
    /// 区切るときは前の周の合否で次の周の速度を決めてから並べるので、そのぶん待つのに使う
    pub fn delay_to(&mut self, audio_ms: f64) {
        if self.style == LoopStyle::Restart && self.next_index > 0 {
            self.cursor_ms = self.cursor_ms.max(audio_ms);
        }
    }

    /// 次の周を速度 `speed` で並べる
    pub fn next_lap(&mut self, speed: f64) -> Lap {
        let speed = speed.max(0.01);
        let t = &self.times;
        let run_up = (t.start_ms - t.run_up_ms) / speed;
        let index = self.next_index;
        let start_ms = match (index, self.style) {
            (0, _) => 0.0,
            (_, LoopStyle::Connect) => self.cursor_ms,
            (_, LoopStyle::Restart) => self.cursor_ms + run_up,
        };
        let with_run_up = index == 0 || self.style == LoopStyle::Restart;
        let (from_ms, to_ms) = match self.style {
            LoopStyle::Connect => (if with_run_up { t.run_up_ms } else { t.start_ms }, t.end_ms),
            LoopStyle::Restart => (t.run_up_ms, t.tail_ms),
        };
        let mut lap = Lap {
            index,
            speed,
            style: self.style,
            start_ms,
            end_ms: 0.0,
            song_start_ms: t.start_ms,
            song_end_ms: t.end_ms,
            audio: Span {
                from_ms,
                to_ms,
                at_ms: 0.0,
            },
            clicks: Vec::new(),
        };
        lap.end_ms = lap.real_ms(t.end_ms);
        lap.audio.at_ms = lap.real_ms(from_ms);
        if with_run_up {
            lap.clicks = t.count_in_ms.iter().map(|&c| lap.real_ms(c)).collect();
        }
        self.cursor_ms = lap.real_ms(to_ms);
        self.next_index += 1;
        lap
    }
}

/// 音 1 つを、周の音の範囲に合わせて置いたもの
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// 鳴らし始める実時間
    pub at_ms: f64,
    /// 音の頭から飛ばす長さ（途中から鳴らすとき）
    pub skip_ms: f64,
    /// 鳴らす長さ
    pub len_ms: f64,
    /// 途中から鳴らす（頭に短いフェードをかける）
    pub cut_start: bool,
    /// 途中で切る（終わりに短いフェードをかける）
    pub cut_end: bool,
}

/// 曲の時刻 `chip_ms` に鳴り始める長さ `sound_ms`（等速での長さ）の音を、周の音の範囲 `span` に置く。
/// 音は速度 `speed` に合わせて伸ばしてあるものとし、置いた位置と長さは実時間で返す。
/// 範囲にかからなければ `None`。範囲より前から鳴っている音は途中から鳴らし、範囲の終わりで切る
pub fn place(chip_ms: f64, sound_ms: f64, span: Span, speed: f64) -> Option<Placement> {
    let end = chip_ms + sound_ms;
    if chip_ms >= span.to_ms || end <= span.from_ms || sound_ms <= 0.0 {
        return None;
    }
    let from = chip_ms.max(span.from_ms);
    let to = end.min(span.to_ms);
    Some(Placement {
        at_ms: span.at_ms + (from - span.from_ms) / speed,
        skip_ms: (from - chip_ms) / speed,
        len_ms: (to - from) / speed,
        cut_start: from > chip_ms,
        cut_end: to < end,
    })
}

/// 周の合格の基準
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassRule {
    /// MISS と POOR が 0
    Easy,
    /// MISS と POOR が 0、PERFECT が 8 割以上
    #[default]
    Normal,
    /// MISS と POOR が 0、PERFECT が 9 割以上、空打ち 0、ずれの標準偏差 15ms 以下
    Strict,
}

impl PassRule {
    pub const ALL: [PassRule; 3] = [Self::Easy, Self::Normal, Self::Strict];

    pub fn name(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Normal => "normal",
            Self::Strict => "strict",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }

    /// 1 周の成績が合格か。`notes` はその周で判定した音符の数
    pub fn passes(self, tally: &Tally, notes: usize, offset_sd_ms: Option<f64>) -> bool {
        let clean = tally.miss == 0 && tally.poor == 0;
        let perfect_rate = |rate: f64| f64::from(tally.perfect) >= rate * notes as f64 - 1e-9;
        match self {
            Self::Easy => clean,
            Self::Normal => clean && perfect_rate(0.8),
            Self::Strict => {
                clean
                    && perfect_rate(0.9)
                    && tally.stray == 0
                    && offset_sd_ms.is_none_or(|sd| sd <= 15.0)
            }
        }
    }
}

/// 速度（%）の下限・上限・刻み
pub const SPEED_MIN: u32 = 25;
pub const SPEED_MAX: u32 = 150;
pub const SPEED_STEP: u32 = 5;

/// 速度（%）を刻みにそろえ、下限・上限に収める
pub fn snap_speed(percent: i64) -> u32 {
    let step = i64::from(SPEED_STEP);
    let snapped = (percent + step / 2).div_euclid(step) * step;
    snapped.clamp(i64::from(SPEED_MIN), i64::from(SPEED_MAX)) as u32
}

/// 周の速度（0〜1.5 の倍率）を % にする
pub fn speed_percent(speed: f64) -> u32 {
    (speed * 100.0).round().max(0.0) as u32
}

/// 速度の決め方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedPlan {
    /// 決めた速度のまま回る
    Fixed(u32),
    /// 合格が続いたら上げ、不合格が続いたら下げる。目標の速度で続けて合格したらクリア
    Ladder { start: u32, target: u32 },
}

impl Default for SpeedPlan {
    fn default() -> Self {
        Self::Ladder {
            start: Self::DEFAULT_START,
            target: Self::DEFAULT_TARGET,
        }
    }
}

impl SpeedPlan {
    pub const DEFAULT_START: u32 = 70;
    pub const DEFAULT_TARGET: u32 = 100;
    /// 選べる目標の速度（100% より上は余裕を作るため）
    pub const TARGETS: [u32; 3] = [100, 105, 110];

    /// 1 周目の速度（%）
    pub fn first_speed(self) -> u32 {
        match self {
            Self::Fixed(s) => s,
            Self::Ladder { start, .. } => start,
        }
    }
}

/// 段階アップで、上げるまでの連続合格
pub const PASSES_TO_UP: u32 = 2;
/// 段階アップで、下げるまでの連続不合格
pub const FAILS_TO_DOWN: u32 = 3;

/// 周の結果で速度がどう動いたか
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedChange {
    Stay,
    Up,
    Down,
    /// 目標の速度で続けて合格した
    Cleared,
}

/// 次に並べる周の速度を決める
#[derive(Debug, Clone, PartialEq)]
pub struct Ladder {
    plan: SpeedPlan,
    speed: u32,
    passes: u32,
    fails: u32,
    cleared: bool,
}

impl Ladder {
    pub fn new(plan: SpeedPlan) -> Self {
        Self {
            plan,
            speed: snap_speed(i64::from(plan.first_speed())),
            passes: 0,
            fails: 0,
            cleared: false,
        }
    }

    pub fn plan(&self) -> SpeedPlan {
        self.plan
    }

    /// 次に並べる周の速度（%）
    pub fn speed(&self) -> u32 {
        self.speed
    }

    /// 今の速度での連続合格・連続不合格
    pub fn streaks(&self) -> (u32, u32) {
        (self.passes, self.fails)
    }

    /// 目標の速度で続けて合格した（段階アップのときだけ）
    pub fn cleared(&self) -> bool {
        self.cleared
    }

    /// 終わった周の合否を受ける。`lap_speed` はその周の速度（%）。
    /// 速度を変えたあとに、前の速度のまま並べてあった周は数えない
    pub fn record(&mut self, lap_speed: u32, passed: bool) -> SpeedChange {
        if lap_speed != self.speed {
            return SpeedChange::Stay;
        }
        if passed {
            self.passes += 1;
            self.fails = 0;
        } else {
            self.fails += 1;
            self.passes = 0;
        }
        let SpeedPlan::Ladder { start, target } = self.plan else {
            return SpeedChange::Stay;
        };
        if self.passes >= PASSES_TO_UP {
            self.passes = 0;
            if self.speed >= target {
                if self.cleared {
                    return SpeedChange::Stay;
                }
                self.cleared = true;
                return SpeedChange::Cleared;
            }
            self.speed = (self.speed + SPEED_STEP).min(target);
            return SpeedChange::Up;
        }
        if self.fails >= FAILS_TO_DOWN {
            self.fails = 0;
            // 始めの速度（手で下げていればその速度）より下げない
            let floor = start.min(self.speed);
            let down = self.speed.saturating_sub(SPEED_STEP).max(floor);
            if down < self.speed {
                self.speed = down;
                return SpeedChange::Down;
            }
        }
        SpeedChange::Stay
    }

    /// 手で速度を変える（次に並べる周から）。連続の数え直しになる
    pub fn set_speed(&mut self, percent: u32) {
        let s = snap_speed(i64::from(percent));
        if s != self.speed {
            self.speed = s;
            self.passes = 0;
            self.fails = 0;
        }
        if let SpeedPlan::Fixed(_) = self.plan {
            self.plan = SpeedPlan::Fixed(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judge::Judgement;
    use dtx::{DrumLane, NotePos};

    /// BPM 120（1 拍 500ms、1 小節 2000ms）の 8 小節
    fn tempo() -> TempoMap {
        TempoMap::build(&dtx::parse("#BPM 120\n#00711: 01\n"))
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn note(ms: f64) -> DrumNote {
        DrumNote {
            ms,
            lane: DrumLane::SD,
            wav: 1,
            at: NotePos::new(0, 0, 1),
            beat: 0.0,
        }
    }

    #[test]
    fn section_basics() {
        let s = Section::new(5, 3);
        assert_eq!((s.first, s.last, s.measures()), (3, 5, 3));
        assert!(s.contains(4) && !s.contains(6));
        assert_eq!(s.shifted(1, 8), Section::new(4, 6));
        // 曲の外にははみ出さない
        assert_eq!(s.shifted(-10, 8), Section::new(0, 2));
        assert_eq!(s.shifted(10, 8), Section::new(5, 7));
        assert_eq!(s.clamped(5), Section::new(3, 4));
        assert_eq!(s.clamped(2), Section::new(1, 1));
    }

    #[test]
    fn section_times() {
        let t = SectionTimes::new(&tempo(), Section::new(2, 3));
        assert!(close(t.start_ms, 4000.0));
        assert!(close(t.end_ms, 8000.0));
        // 1 小節前（= 4 拍前）から助走
        assert!(close(t.run_up_ms, 2000.0));
        assert_eq!(t.count_in_ms, [2000.0, 2500.0, 3000.0, 3500.0]);
        // 着地の 2 拍あと
        assert!(close(t.tail_ms, 9000.0));
    }

    #[test]
    fn run_up_before_the_first_measure_extends_the_tempo() {
        let t = SectionTimes::new(&tempo(), Section::new(0, 0));
        assert!(close(t.run_up_ms, -2000.0));
        assert!(close(t.count_in_ms[0], -2000.0));
    }

    #[test]
    fn run_up_covers_four_beats_after_a_short_measure() {
        // 小節 1 が 2 拍（小節長 0.5）。区間 2 の助走は 4 拍前 = 小節 0 の途中から
        let tempo = TempoMap::build(&dtx::parse("#BPM 120\n#00102: 0.5\n#00311: 01\n"));
        let t = SectionTimes::new(&tempo, Section::new(2, 2));
        assert!(close(t.start_ms, 3000.0));
        assert!(close(t.run_up_ms, 1000.0));
        assert!(close(t.count_in_ms[0], 1000.0));
    }

    #[test]
    fn connected_laps_follow_each_other() {
        let mut l = Looper::new(
            SectionTimes::new(&tempo(), Section::new(2, 3)),
            LoopStyle::Connect,
        );
        assert!(close(l.first_audio_ms(1.0), -2000.0));
        let a = l.next_lap(1.0);
        // 1 周目は助走（小節 1）から鳴らし、区間の頭が 0
        assert_eq!(
            a.audio,
            Span {
                from_ms: 2000.0,
                to_ms: 8000.0,
                at_ms: -2000.0
            }
        );
        assert!(close(a.start_ms, 0.0) && close(a.end_ms, 4000.0));
        assert_eq!(a.clicks, vec![-2000.0, -1500.0, -1000.0, -500.0]);
        // 実時間と曲の時刻は行き来できる
        assert!(close(a.song_ms(1000.0), 5000.0));
        assert!(close(a.song_ms(a.real_ms(6500.0)), 6500.0));
        assert!(close(l.next_audio_ms(1.0), 4000.0));
        // 2 周目は区間だけ、切れ目なく
        let b = l.next_lap(1.0);
        assert_eq!(
            b.audio,
            Span {
                from_ms: 4000.0,
                to_ms: 8000.0,
                at_ms: 4000.0
            }
        );
        assert!(b.clicks.is_empty());
        // 区間の中の音符は T_k + (x − A)
        assert!(close(b.real_ms(5000.0), 5000.0));
        assert!(close(l.next_lap(1.0).start_ms, 8000.0));
    }

    #[test]
    fn slower_laps_stretch_the_timeline() {
        let mut l = Looper::new(
            SectionTimes::new(&tempo(), Section::new(2, 3)),
            LoopStyle::Connect,
        );
        // 50% なら助走も区間も 2 倍の長さ
        assert!(close(l.first_audio_ms(0.5), -4000.0));
        let a = l.next_lap(0.5);
        assert!(close(a.audio.at_ms, -4000.0));
        assert!(close(a.end_ms, 8000.0));
        assert!(close(a.real_ms(5000.0), 2000.0));
        // 次の周は等速: T_1 = 8000、区間の 1000ms 目は 9000
        let b = l.next_lap(1.0);
        assert!(close(b.start_ms, 8000.0));
        assert!(close(b.real_ms(5000.0), 9000.0));
    }

    #[test]
    fn restarted_laps_include_landing_tail_and_run_up() {
        let mut l = Looper::new(
            SectionTimes::new(&tempo(), Section::new(2, 3)),
            LoopStyle::Restart,
        );
        let a = l.next_lap(1.0);
        // 助走（2000〜4000）+ 区間（4000〜8000）+ 着地のあと 2 拍（〜9000）
        assert_eq!(
            a.audio,
            Span {
                from_ms: 2000.0,
                to_ms: 9000.0,
                at_ms: -2000.0
            }
        );
        // 次の周は 5000（音の終わり）から助走、区間の頭は 7000
        let b = l.next_lap(1.0);
        assert!(close(b.audio.at_ms, 5000.0));
        assert!(close(b.start_ms, 7000.0));
        assert_eq!(b.clicks, vec![5000.0, 5500.0, 6000.0, 6500.0]);
    }

    #[test]
    fn which_notes_are_judged() {
        let notes: Vec<DrumNote> = [
            1000.0, 2000.0, 3500.0, 4000.0, 7999.0, 8000.0, 8500.0, 9000.0,
        ]
        .into_iter()
        .map(note)
        .collect();
        let times = SectionTimes::new(&tempo(), Section::new(2, 3));
        let mut connect = Looper::new(times.clone(), LoopStyle::Connect);
        let lap = connect.next_lap(1.0);
        let got: Vec<(usize, bool)> = lap
            .notes(&notes)
            .iter()
            .map(|n| (n.note, n.judged))
            .collect();
        // 助走の音符（2000、3500）は見せるだけ。8000 は次の周の頭なので入らない
        assert_eq!(got, vec![(1, false), (2, false), (3, true), (4, true)]);
        let lap = connect.next_lap(1.0);
        let got: Vec<(usize, bool)> = lap
            .notes(&notes)
            .iter()
            .map(|n| (n.note, n.judged))
            .collect();
        assert_eq!(got, vec![(3, true), (4, true)]);

        let mut restart = Looper::new(times, LoopStyle::Restart);
        let lap = restart.next_lap(1.0);
        let got: Vec<(usize, bool)> = lap
            .notes(&notes)
            .iter()
            .map(|n| (n.note, n.judged))
            .collect();
        // 着地（8000）は判定し、そのあと（8500）は見せるだけ。9000 は鳴らす範囲の外
        assert_eq!(
            got,
            vec![
                (1, false),
                (2, false),
                (3, true),
                (4, true),
                (5, true),
                (6, false)
            ]
        );
        assert!(close(lap.notes(&notes)[4].ms, 4000.0));
    }

    #[test]
    fn strays_belong_to_the_lap_being_played() {
        let times = SectionTimes::new(&tempo(), Section::new(2, 3));
        let mut connect = Looper::new(times.clone(), LoopStyle::Connect);
        let a = connect.next_lap(1.0);
        let b = connect.next_lap(1.0);
        // 助走の空打ちはどの周にも入れない
        assert!(!a.owns_stray(-1000.0, 117.0));
        assert!(a.owns_stray(-100.0, 117.0));
        assert!(a.owns_stray(3800.0, 117.0));
        // 次の周の頭の少し前は次の周
        assert!(!a.owns_stray(3900.0, 117.0));
        assert!(b.owns_stray(3900.0, 117.0));

        let mut restart = Looper::new(times, LoopStyle::Restart);
        let a = restart.next_lap(1.0);
        // 着地の少し後までは入れ、その後の助走は入れない
        assert!(a.owns_stray(4100.0, 117.0));
        assert!(!a.owns_stray(4500.0, 117.0));
    }

    #[test]
    fn placing_sounds_in_a_span() {
        let span = Span {
            from_ms: 4000.0,
            to_ms: 8000.0,
            at_ms: 10_000.0,
        };
        // 範囲の中で鳴り終わる音はそのまま
        assert_eq!(
            place(5000.0, 500.0, span, 1.0),
            Some(Placement {
                at_ms: 11_000.0,
                skip_ms: 0.0,
                len_ms: 500.0,
                cut_start: false,
                cut_end: false
            })
        );
        // 曲頭からの長い BGM は途中から鳴らし、範囲の終わりで切る
        assert_eq!(
            place(0.0, 60_000.0, span, 1.0),
            Some(Placement {
                at_ms: 10_000.0,
                skip_ms: 4000.0,
                len_ms: 4000.0,
                cut_start: true,
                cut_end: true
            })
        );
        // 範囲にかからない音
        assert_eq!(place(8000.0, 100.0, span, 1.0), None);
        assert_eq!(place(1000.0, 3000.0, span, 1.0), None);
        // 50% に伸ばした区間の音（曲の 2000ms から 9000ms 分）。範囲の 4000〜8000 は実時間で 2 倍
        assert_eq!(
            place(2000.0, 9000.0, span, 0.5),
            Some(Placement {
                at_ms: 10_000.0,
                skip_ms: 4000.0,
                len_ms: 8000.0,
                cut_start: true,
                cut_end: true
            })
        );
    }

    #[test]
    fn restart_waits_when_asked() {
        let times = SectionTimes::new(&tempo(), Section::new(2, 3));
        let mut restart = Looper::new(times.clone(), LoopStyle::Restart);
        // 1 周目の前には効かない
        restart.delay_to(100_000.0);
        assert!(close(restart.next_lap(1.0).audio.at_ms, -2000.0));
        // 音の終わり（5000）より後まで待つ
        restart.delay_to(6000.0);
        assert!(close(restart.next_audio_ms(1.0), 6000.0));
        let b = restart.next_lap(1.0);
        assert!(close(b.audio.at_ms, 6000.0) && close(b.start_ms, 8000.0));
        // 早い時刻を渡しても前へは詰めない（2 周目の音の終わりは 8000 + 5000）
        restart.delay_to(0.0);
        assert!(close(restart.next_audio_ms(1.0), 13_000.0));
        // つなぐときは切れ目を作らない
        let mut connect = Looper::new(times, LoopStyle::Connect);
        connect.next_lap(1.0);
        connect.delay_to(10_000.0);
        assert!(close(connect.next_audio_ms(1.0), 4000.0));
    }

    #[test]
    fn speeds_snap_to_steps() {
        assert_eq!(snap_speed(72), 70);
        assert_eq!(snap_speed(73), 75);
        assert_eq!(snap_speed(10), SPEED_MIN);
        assert_eq!(snap_speed(500), SPEED_MAX);
        assert_eq!(speed_percent(0.7), 70);
        assert_eq!(speed_percent(1.05), 105);
    }

    fn ladder(start: u32, target: u32) -> Ladder {
        Ladder::new(SpeedPlan::Ladder { start, target })
    }

    #[test]
    fn ladder_goes_up_after_two_passes() {
        let mut l = ladder(70, 100);
        assert_eq!(l.speed(), 70);
        assert_eq!(l.record(70, true), SpeedChange::Stay);
        assert_eq!(l.streaks(), (1, 0));
        // 不合格で連続合格は数え直し
        assert_eq!(l.record(70, false), SpeedChange::Stay);
        assert_eq!(l.record(70, true), SpeedChange::Stay);
        assert_eq!(l.record(70, true), SpeedChange::Up);
        assert_eq!((l.speed(), l.streaks()), (75, (0, 0)));
        // 決める前に 70% で並べてあった周は数えない
        assert_eq!(l.record(70, true), SpeedChange::Stay);
        assert_eq!(l.streaks(), (0, 0));
    }

    #[test]
    fn ladder_goes_down_after_three_fails_but_not_below_start() {
        let mut l = ladder(70, 100);
        for _ in 0..2 {
            l.record(70, true);
        }
        assert_eq!(l.speed(), 75);
        assert_eq!(l.record(75, false), SpeedChange::Stay);
        assert_eq!(l.record(75, false), SpeedChange::Stay);
        assert_eq!(l.record(75, false), SpeedChange::Down);
        assert_eq!(l.speed(), 70);
        // 始めの速度より下には下げない
        for _ in 0..3 {
            assert_eq!(l.record(70, false), SpeedChange::Stay);
        }
        assert_eq!(l.speed(), 70);
    }

    #[test]
    fn ladder_clears_at_the_target() {
        let mut l = ladder(95, 100);
        l.record(95, true);
        assert_eq!(l.record(95, true), SpeedChange::Up);
        assert_eq!(l.speed(), 100);
        assert!(!l.cleared());
        l.record(100, true);
        assert_eq!(l.record(100, true), SpeedChange::Cleared);
        assert!(l.cleared());
        // クリアしたあとも目標の速度のまま
        l.record(100, true);
        assert_eq!(l.record(100, true), SpeedChange::Stay);
        assert_eq!(l.speed(), 100);
        // 目標を 110% にすれば 100% の先も上げる
        let mut l = ladder(100, 110);
        l.record(100, true);
        assert_eq!(l.record(100, true), SpeedChange::Up);
        assert_eq!(l.speed(), 105);
    }

    #[test]
    fn fixed_speed_and_manual_changes() {
        let mut f = Ladder::new(SpeedPlan::Fixed(80));
        for _ in 0..5 {
            assert_eq!(f.record(80, true), SpeedChange::Stay);
        }
        assert_eq!(f.speed(), 80);
        f.set_speed(86);
        assert_eq!((f.speed(), f.plan()), (85, SpeedPlan::Fixed(85)));
        // 手で始めの速度より下げたら、そこより下には自動で下げない
        let mut l = ladder(70, 100);
        l.record(70, true);
        l.set_speed(60);
        assert_eq!((l.speed(), l.streaks()), (60, (0, 0)));
        for _ in 0..3 {
            l.record(60, false);
        }
        assert_eq!(l.speed(), 60);
        for _ in 0..2 {
            l.record(60, true);
        }
        assert_eq!(l.speed(), 65);
    }

    fn tally(perfect: u32, great: u32, poor: u32, miss: u32, stray: u32) -> Tally {
        let mut t = Tally::default();
        for (j, n) in [
            (Judgement::Perfect, perfect),
            (Judgement::Great, great),
            (Judgement::Poor, poor),
            (Judgement::Miss, miss),
        ] {
            for _ in 0..n {
                t.add(j);
            }
        }
        t.stray = stray;
        t
    }

    #[test]
    fn pass_rules() {
        // 10 音符で PERFECT 8、GREAT 2
        let t = tally(8, 2, 0, 0, 1);
        assert!(PassRule::Easy.passes(&t, 10, Some(20.0)));
        assert!(PassRule::Normal.passes(&t, 10, Some(20.0)));
        // きびしめは PERFECT 9 割、空打ち 0、ばらつき 15ms 以下
        assert!(!PassRule::Strict.passes(&t, 10, Some(10.0)));
        assert!(PassRule::Strict.passes(&tally(9, 1, 0, 0, 0), 10, Some(15.0)));
        assert!(!PassRule::Strict.passes(&tally(9, 1, 0, 0, 0), 10, Some(15.1)));
        // PERFECT 7 割はふつうでは不合格
        assert!(!PassRule::Normal.passes(&tally(7, 3, 0, 0, 0), 10, None));
        // MISS か POOR があればどれも不合格
        for r in PassRule::ALL {
            assert!(!r.passes(&tally(9, 0, 1, 0, 0), 10, None));
            assert!(!r.passes(&tally(9, 0, 0, 1, 0), 10, None));
        }
    }

    #[test]
    fn names_round_trip() {
        for s in LoopStyle::ALL {
            assert_eq!(LoopStyle::from_name(s.name()), Some(s));
        }
        for r in PassRule::ALL {
            assert_eq!(PassRule::from_name(r.name()), Some(r));
        }
    }
}
