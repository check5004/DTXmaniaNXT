//! 練習（区間ループ）。曲の区間を周回ぶん後ろへ並べた 1 本の時間軸を作る。
//!
//! 時間軸は「練習を始めてからの実時間」で、巻き戻らない。区間の頭を A、終わり（次の小節の頭）を B、
//! 周 k の区間の頭の時刻を T_k、速度を s_k とすると、曲の時刻 x は `T_k + (x − A) / s_k` に置かれる。
//! 1 周目の頭が 0 で、その前に助走（区間の 1 小節前からの BGM と 4 拍のカウント）が入る。
//!
//! 判定の幅とずれは実時間のミリ秒のまま使うので、遅くしても判定は甘くならない。
//!
//! 練習は、始めの速度から目標の速度まで 10% ずつの段をのぼる。1 段は 4 周ほどで、落とした拍の数え
//! （`SpotTally`）で同じ拍のミスが重ならなければ次の段へ、重なればもう一度、もう一度が続けば 1 段下げる
//! （`next_step`）。目標の段を越えたらクリア。速度が変わるのは段の変わり目と手で変えたときだけで、
//! 変わった周は、つなぐときも助走（新しい速度の 1 小節と 4 拍のカウント）から入る。

use std::collections::BTreeMap;

use dtx::{DrumLane, DrumNote, TempoMap};

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
    /// 区間の中の拍（メトロノーム）。曲の時刻と、小節の頭か
    pub beats_ms: Vec<(f64, bool)>,
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
        let mut beats_ms = Vec::new();
        for m in section.first..=section.last {
            let from = tempo.measure_start_beat(m as usize);
            let to = tempo.measure_start_beat(m as usize + 1);
            let mut beat = from;
            while beat < to - 1e-6 {
                beats_ms.push((tempo.beat_to_ms(beat), beat == from));
                beat += 1.0;
            }
        }
        Self {
            section,
            start_ms: tempo.beat_to_ms(a),
            end_ms: tempo.beat_to_ms(b),
            run_up_ms: tempo.beat_to_ms(run_up),
            count_in_ms: std::array::from_fn(|i| tempo.beat_to_ms(count_from + i as f64)),
            tail_ms: tempo.beat_to_ms(b + RESTART_TAIL_BEATS),
            beats_ms,
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
    /// カウントのクリックの実時間（助走から入る周だけ）
    pub clicks: Vec<f64>,
    /// 区間の中の拍の実時間と、小節の頭か（メトロノーム）
    pub beats: Vec<(f64, bool)>,
}

impl Lap {
    /// 助走（カウント）から入る周か
    pub fn has_run_up(&self) -> bool {
        !self.clicks.is_empty()
    }

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
    /// 並べた周の速度（取り下げたときに前の周の速度に戻すため）
    speeds: Vec<f64>,
}

impl Looper {
    pub fn new(times: SectionTimes, style: LoopStyle) -> Self {
        Self {
            times,
            style,
            next_index: 0,
            cursor_ms: 0.0,
            speeds: Vec::new(),
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

    /// 次の周を速度 `speed` で並べる。前の周と速度が違うか `run_up` なら、つなぐときも助走から入る
    pub fn next_lap(&mut self, speed: f64, run_up: bool) -> Lap {
        let speed = speed.max(0.01);
        let t = &self.times;
        let run_up_ms = (t.start_ms - t.run_up_ms) / speed;
        let index = self.next_index;
        let changed = self
            .speeds
            .last()
            .is_some_and(|&prev| (prev - speed).abs() > 1e-9);
        let with_run_up = index == 0 || self.style == LoopStyle::Restart || changed || run_up;
        let start_ms = match (index, with_run_up) {
            (0, _) => 0.0,
            (_, false) => self.cursor_ms,
            (_, true) => self.cursor_ms + run_up_ms,
        };
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
            beats: Vec::new(),
        };
        lap.end_ms = lap.real_ms(t.end_ms);
        lap.audio.at_ms = lap.real_ms(from_ms);
        if with_run_up {
            lap.clicks = t.count_in_ms.iter().map(|&c| lap.real_ms(c)).collect();
        }
        lap.beats = t
            .beats_ms
            .iter()
            .map(|&(ms, bar)| (lap.real_ms(ms), bar))
            .collect();
        self.cursor_ms = lap.real_ms(to_ms);
        self.next_index += 1;
        self.speeds.push(speed);
        lap
    }

    /// 次に並べる周から区間を変える（苦手の所だけに絞る、元の区間に戻る）。変えた次の周は助走から入れる。
    /// 区間の音は元の区間の音の中に収まる範囲にする（呼ぶ側が決める）
    pub fn set_times(&mut self, times: SectionTimes) {
        self.times = times;
    }

    /// 次の周の音を実時間 `audio_ms` より前には始めない。つなぐときも効く（段の変わり目で止めたあと）
    pub fn resume_at(&mut self, audio_ms: f64) {
        if self.next_index > 0 {
            self.cursor_ms = self.cursor_ms.max(audio_ms);
        }
    }

    /// 最後に並べた周 `lap` を取り下げる（まだ始まっていない周を、決め直した速度で並べ直すため）。
    /// 最後に並べた周でなければ何もせず false
    pub fn unlay(&mut self, lap: &Lap) -> bool {
        if lap.index + 1 != self.next_index {
            return false;
        }
        self.next_index = lap.index;
        self.cursor_ms = lap.audio.at_ms;
        self.speeds.pop();
        true
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

/// 段の越え方（同じ拍を何周落としたら「もう一度」か）。落としたのは POOR と MISS。
/// 1 周のミスの数では合否を決めない（2026-11-22 ユーザー「ミス数じゃなくて、内容を見るべき」）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassRule {
    /// ゆるい: 段の中で同じ拍を 3 周
    Easy,
    /// ふつう: 2 周
    #[default]
    Normal,
    /// きびしい: 1 周でも
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

    /// 同じ拍を何周落としたら「もう一度」か
    pub fn weak_times(self) -> u32 {
        match self {
            Self::Easy => 3,
            Self::Normal => 2,
            Self::Strict => 1,
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
    /// 始めの速度から 10% ずつの段をのぼる。目標の段を越えたらクリア
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

/// 1 段の周の数（既定）。4 周叩いて、同じ拍のミスが重ならなければ次の段へ（2026-11-22 ユーザー「開始してクリアする
/// までのフローがよくわからん」。段をのぼってクリアする形に組み直した）
pub const STEP_LAPS: u32 = 4;
/// 選べる 1 段の周の数
pub const STEP_LAPS_CHOICES: [u32; 4] = [3, 4, 6, 8];
/// 段の速度の幅（%）。5% では助けにならない（2026-11-22 ユーザーの指定）
pub const STEP_SPEED: u32 = 10;
/// 同じ段で「もう一度」がこれだけ続いたら、1 段下げる
pub const DOWN_AFTER: u32 = 2;

/// 段の挑戦 1 回（その段の周をまとめたもの）
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt {
    /// 0 から数える
    pub index: usize,
    /// 速度（%）
    pub speed: u32,
    /// 最後まで叩いた周の数
    pub laps: u32,
    /// ノーミス（POOR と MISS が 0）の周の数
    pub clean_laps: u32,
    /// 判定した音符と、落とした音符（POOR と MISS）
    pub notes: u32,
    pub missed: u32,
    /// 拍ごとの落とした周の数
    pub spots: SpotTally,
    /// 同じ拍を何周落としたら「もう一度」か
    pub weak_times: u32,
    /// 段を越えたか。途中でやめた挑戦は None
    pub passed: Option<bool>,
}

impl Attempt {
    /// ミスが重なった拍（落とした周の多い順）
    pub fn weak(&self) -> Vec<(BeatSlot, &Spot)> {
        self.spots.weak(self.weak_times)
    }

    /// うっかり（重ならなかったミス）の拍の数
    pub fn slips(&self) -> usize {
        self.spots
            .spots()
            .filter(|(_, s)| s.laps < self.weak_times)
            .count()
    }

    /// 落とした音符の割合（0〜1）
    pub fn miss_rate(&self) -> f64 {
        if self.notes == 0 {
            0.0
        } else {
            f64::from(self.missed) / f64::from(self.notes)
        }
    }

    /// 等速以上で段を越えた（記録のクリア）
    pub fn cleared(&self) -> bool {
        self.passed == Some(true) && self.speed >= 100
    }
}

/// 段の挑戦のあと、次にすること
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// 段を越えたので、この速度（%）の段へ
    Up(u32),
    /// 同じ段をもう一度
    Again,
    /// もう一度が続いたので、この速度（%）の段へ下げる
    Down(u32),
    /// 目標の段を越えた（固定の速度なら、その段を越えた）
    Clear,
}

/// 速度 `speed` の段の挑戦のあと、次にすることを決める。`again_in_row` はこの挑戦を入れて、この段で続けて
/// 「もう一度」になった回数（越えたら 0）。下げるのは段階アップのときだけで、下限より下には下げない
pub fn next_step(plan: SpeedPlan, speed: u32, passed: bool, again_in_row: u32) -> Next {
    match plan {
        SpeedPlan::Fixed(_) if passed => Next::Clear,
        SpeedPlan::Fixed(_) => Next::Again,
        SpeedPlan::Ladder { target, .. } if passed => {
            if speed >= target {
                Next::Clear
            } else {
                Next::Up((speed + STEP_SPEED).min(target))
            }
        }
        SpeedPlan::Ladder { .. } => {
            if again_in_row >= DOWN_AFTER && speed >= SPEED_MIN + STEP_SPEED {
                Next::Down(speed - STEP_SPEED)
            } else {
                Next::Again
            }
        }
    }
}

impl SpeedPlan {
    /// 越えたらクリアになる段の速度（段階アップは目標、固定はその速度）
    pub fn goal(self) -> u32 {
        match self {
            Self::Fixed(s) => s,
            Self::Ladder { target, .. } => target,
        }
    }

    /// 段（下から）。始めの速度から 10% ずつ、目標で止める。固定はその速度の 1 段
    pub fn steps(self) -> Vec<u32> {
        let (start, target) = match self {
            Self::Fixed(s) => (s, s),
            Self::Ladder { start, target } => (start, target),
        };
        let mut out = vec![start];
        let mut s = start;
        while s < target {
            s = (s + STEP_SPEED).min(target);
            out.push(s);
        }
        out
    }

    /// 段に、下げて叩いた速度 `lowest`（%）までの段を下に足したもの
    pub fn stairs(self, lowest: u32) -> Vec<u32> {
        let mut out = self.steps();
        let mut s = out[0];
        while s >= lowest + STEP_SPEED && s >= SPEED_MIN + STEP_SPEED {
            s -= STEP_SPEED;
            out.insert(0, s);
        }
        out
    }
}

/// 区間の中の拍。小節と、小節の中の拍（0 から）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BeatSlot {
    pub measure: u32,
    pub beat: u32,
}

/// 区間の小節ごとの拍。音符がどの拍にあるかを決める
#[derive(Debug, Clone, PartialEq)]
pub struct BeatGrid {
    pub section: Section,
    /// 区間の小節の頭の拍（曲頭から）と、区間の終わり（次の小節の頭）
    starts: Vec<f64>,
}

impl BeatGrid {
    pub fn new(tempo: &TempoMap, section: Section) -> Self {
        let starts = (section.first..=section.last + 1)
            .map(|m| tempo.measure_start_beat(m as usize))
            .collect();
        Self { section, starts }
    }

    /// 小節 `measure` の拍の数（半端な拍も 1 拍に数える）。区間の外なら 0
    pub fn beats_in(&self, measure: u32) -> u32 {
        if !self.section.contains(measure) {
            return 0;
        }
        let i = (measure - self.section.first) as usize;
        let len = self.starts[i + 1] - self.starts[i];
        ((len - 1e-6).ceil().max(1.0)) as u32
    }

    /// 音符のある拍。区間の外（区切るときの着地など）は None
    pub fn slot(&self, note: &DrumNote) -> Option<BeatSlot> {
        let measure = note.at.measure;
        if !self.section.contains(measure) {
            return None;
        }
        let start = self.starts[(measure - self.section.first) as usize];
        let beat = ((note.beat - start + 1e-6).floor().max(0.0)) as u32;
        Some(BeatSlot {
            measure,
            beat: beat.min(self.beats_in(measure) - 1),
        })
    }
}

/// 落とした拍 1 つの数え
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spot {
    /// 落とした周の数（1 周に何個落としても 1 と数える）
    pub laps: u32,
    /// 落としたレーン（並べた順）
    pub lanes: Vec<DrumLane>,
}

/// 何周かぶんの、拍ごとに落とした周の数。1 回きりのミスはうっかり、同じ拍を何周も落としたら苦手とみなす
/// （2026-11-22 ユーザー「ミス数じゃなくて、内容を見るべき」）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpotTally {
    spots: BTreeMap<BeatSlot, Spot>,
    /// 数えた周の数
    pub laps: u32,
}

impl SpotTally {
    /// 1 周ぶんの落とした音符（拍とレーン）を足す
    pub fn add_lap(&mut self, misses: &[(BeatSlot, DrumLane)]) {
        self.laps += 1;
        let mut seen: Vec<BeatSlot> = Vec::new();
        for &(slot, lane) in misses {
            let spot = self.spots.entry(slot).or_default();
            if !seen.contains(&slot) {
                seen.push(slot);
                spot.laps += 1;
            }
            if !spot.lanes.contains(&lane) {
                spot.lanes.push(lane);
            }
        }
    }

    /// 拍 `slot` を落とした周の数
    pub fn count(&self, slot: BeatSlot) -> u32 {
        self.spots.get(&slot).map_or(0, |s| s.laps)
    }

    /// 落とした拍（拍の順）
    pub fn spots(&self) -> impl Iterator<Item = (BeatSlot, &Spot)> {
        self.spots.iter().map(|(&k, v)| (k, v))
    }

    /// 苦手の拍（`times` 周以上落とした拍）。落とした周の多い順、同じなら前から
    pub fn weak(&self, times: u32) -> Vec<(BeatSlot, &Spot)> {
        let mut v: Vec<_> = self.spots().filter(|(_, s)| s.laps >= times).collect();
        v.sort_by(|a, b| b.1.laps.cmp(&a.1.laps).then(a.0.cmp(&b.0)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dtx::NotePos;

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
        let a = l.next_lap(1.0, false);
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
        let b = l.next_lap(1.0, false);
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
        assert!(close(l.next_lap(1.0, false).start_ms, 8000.0));
    }

    #[test]
    fn slower_laps_stretch_the_timeline() {
        let mut l = Looper::new(
            SectionTimes::new(&tempo(), Section::new(2, 3)),
            LoopStyle::Connect,
        );
        // 50% なら助走も区間も 2 倍の長さ
        assert!(close(l.first_audio_ms(0.5), -4000.0));
        let a = l.next_lap(0.5, false);
        assert!(close(a.audio.at_ms, -4000.0));
        assert!(close(a.end_ms, 8000.0));
        assert!(close(a.real_ms(5000.0), 2000.0));
        // 次の周は等速。速度が変わるので、つなぐときも助走（2000ms）とカウントから入る:
        // 助走は 8000 から、T_1 = 10000、区間の 1000ms 目は 11000
        let b = l.next_lap(1.0, false);
        assert!(close(b.audio.at_ms, 8000.0) && close(b.audio.from_ms, 2000.0));
        assert!(close(b.start_ms, 10_000.0));
        assert!(close(b.real_ms(5000.0), 11_000.0));
        assert_eq!(b.clicks, vec![8000.0, 8500.0, 9000.0, 9500.0]);
        // 同じ速度の次の周は切れ目なく
        let c = l.next_lap(1.0, false);
        assert!(close(c.start_ms, 14_000.0) && c.clicks.is_empty());
        // 頼めば同じ速度でも助走から
        let d = l.next_lap(1.0, true);
        assert!(close(d.start_ms, 20_000.0) && d.has_run_up());
    }

    #[test]
    fn beats_follow_the_measures() {
        // 小節 1 から 2 拍（小節長 0.5 は次の小節にも続く）。小節 3 から 4 拍に戻す
        let tempo = TempoMap::build(&dtx::parse(
            "#BPM 120\n#00102: 0.5\n#00302: 1\n#00411: 01\n",
        ));
        let t = SectionTimes::new(&tempo, Section::new(1, 3));
        assert_eq!(
            t.beats_ms,
            vec![
                (2000.0, true),
                (2500.0, false),
                (3000.0, true),
                (3500.0, false),
                (4000.0, true),
                (4500.0, false),
                (5000.0, false),
                (5500.0, false)
            ]
        );
        let mut l = Looper::new(t, LoopStyle::Connect);
        let lap = l.next_lap(0.5, false);
        // 50% なら拍の間も 2 倍
        assert_eq!(lap.beats[1], (1000.0, false));
    }

    #[test]
    fn an_unstarted_lap_can_be_taken_back() {
        let mut l = Looper::new(
            SectionTimes::new(&tempo(), Section::new(2, 3)),
            LoopStyle::Connect,
        );
        l.next_lap(1.0, false);
        let b = l.next_lap(1.0, false);
        // 最後の周でなければ取り下げない
        let a_again = Lap {
            index: 0,
            ..b.clone()
        };
        assert!(!l.unlay(&a_again));
        assert!(l.unlay(&b));
        assert!(close(l.next_audio_ms(1.0), 4000.0));
        // 並べ直すと、速度が変わったので助走から
        let b2 = l.next_lap(0.8, false);
        assert_eq!(b2.index, 1);
        assert!(close(b2.audio.at_ms, 4000.0) && b2.has_run_up());
        assert!(close(b2.start_ms, 4000.0 + 2000.0 / 0.8));
    }

    #[test]
    fn restarted_laps_include_landing_tail_and_run_up() {
        let mut l = Looper::new(
            SectionTimes::new(&tempo(), Section::new(2, 3)),
            LoopStyle::Restart,
        );
        let a = l.next_lap(1.0, false);
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
        let b = l.next_lap(1.0, false);
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
        let lap = connect.next_lap(1.0, false);
        let got: Vec<(usize, bool)> = lap
            .notes(&notes)
            .iter()
            .map(|n| (n.note, n.judged))
            .collect();
        // 助走の音符（2000、3500）は見せるだけ。8000 は次の周の頭なので入らない
        assert_eq!(got, vec![(1, false), (2, false), (3, true), (4, true)]);
        let lap = connect.next_lap(1.0, false);
        let got: Vec<(usize, bool)> = lap
            .notes(&notes)
            .iter()
            .map(|n| (n.note, n.judged))
            .collect();
        assert_eq!(got, vec![(3, true), (4, true)]);

        let mut restart = Looper::new(times, LoopStyle::Restart);
        let lap = restart.next_lap(1.0, false);
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
        let a = connect.next_lap(1.0, false);
        let b = connect.next_lap(1.0, false);
        // 助走の空打ちはどの周にも入れない
        assert!(!a.owns_stray(-1000.0, 117.0));
        assert!(a.owns_stray(-100.0, 117.0));
        assert!(a.owns_stray(3800.0, 117.0));
        // 次の周の頭の少し前は次の周
        assert!(!a.owns_stray(3900.0, 117.0));
        assert!(b.owns_stray(3900.0, 117.0));

        let mut restart = Looper::new(times, LoopStyle::Restart);
        let a = restart.next_lap(1.0, false);
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
        assert!(close(restart.next_lap(1.0, false).audio.at_ms, -2000.0));
        // 音の終わり（5000）より後まで待つ
        restart.delay_to(6000.0);
        assert!(close(restart.next_audio_ms(1.0), 6000.0));
        let b = restart.next_lap(1.0, false);
        assert!(close(b.audio.at_ms, 6000.0) && close(b.start_ms, 8000.0));
        // 早い時刻を渡しても前へは詰めない（2 周目の音の終わりは 8000 + 5000）
        restart.delay_to(0.0);
        assert!(close(restart.next_audio_ms(1.0), 13_000.0));
        // つなぐときは切れ目を作らない
        let mut connect = Looper::new(times, LoopStyle::Connect);
        connect.next_lap(1.0, false);
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

    fn drum(lane: DrumLane, measure: u32, num: u32, den: u32, beat: f64) -> DrumNote {
        DrumNote {
            ms: 0.0,
            lane,
            wav: 1,
            at: NotePos::new(measure, num, den),
            beat,
        }
    }

    #[test]
    fn notes_fall_into_beats_of_their_measure() {
        // 小節 3 だけ 3/4 拍子（DTX の小節の長さは次に変えるまで続くので、小節 4 で戻す）
        let tempo = TempoMap::build(&dtx::parse(
            "#BPM 120\n#00302: 0.75\n#00402: 1\n#00711: 01\n",
        ));
        let g = BeatGrid::new(&tempo, Section::new(2, 4));
        assert_eq!(
            (g.beats_in(2), g.beats_in(3), g.beats_in(4), g.beats_in(5)),
            (4, 3, 4, 0)
        );
        // 小節 2 は 8 拍目から、小節 3 は 12 拍目から（3 拍）、小節 4 は 15 拍目から
        let slot = |m, b| {
            Some(BeatSlot {
                measure: m,
                beat: b,
            })
        };
        assert_eq!(g.slot(&drum(DrumLane::BD, 2, 0, 1, 8.0)), slot(2, 0));
        assert_eq!(g.slot(&drum(DrumLane::SD, 2, 7, 16, 9.75)), slot(2, 1));
        assert_eq!(g.slot(&drum(DrumLane::SD, 3, 2, 3, 14.0)), slot(3, 2));
        assert_eq!(g.slot(&drum(DrumLane::HH, 4, 15, 16, 18.75)), slot(4, 3));
        // 区切るときの着地（次の小節の頭）は区間の外
        assert_eq!(g.slot(&drum(DrumLane::CY, 5, 0, 1, 19.0)), None);
    }

    #[test]
    fn a_beat_missed_again_and_again_is_weak_but_a_single_slip_is_not() {
        let a = BeatSlot {
            measure: 61,
            beat: 3,
        };
        let b = BeatSlot {
            measure: 62,
            beat: 0,
        };
        let c = BeatSlot {
            measure: 63,
            beat: 2,
        };
        let mut t = SpotTally::default();
        // 1 周に同じ拍を 2 個落としても 1 周と数える
        t.add_lap(&[(b, DrumLane::LT), (b, DrumLane::BD)]);
        t.add_lap(&[]);
        t.add_lap(&[(c, DrumLane::SD)]);
        t.add_lap(&[(b, DrumLane::LT), (a, DrumLane::HT)]);
        t.add_lap(&[(b, DrumLane::BD)]);
        assert_eq!(t.laps, 5);
        assert_eq!((t.count(a), t.count(b), t.count(c)), (1, 3, 1));
        let weak = t.weak(PassRule::Normal.weak_times());
        assert_eq!(weak.len(), 1);
        assert_eq!(weak[0].0, b);
        assert_eq!(weak[0].1.lanes, vec![DrumLane::LT, DrumLane::BD]);
        // ゆるいは 3 周、きびしいは 1 回でも
        assert_eq!(t.weak(PassRule::Easy.weak_times()).len(), 1);
        let strict: Vec<_> = t
            .weak(PassRule::Strict.weak_times())
            .iter()
            .map(|w| w.0)
            .collect();
        assert_eq!(strict, vec![b, a, c]);
    }

    #[test]
    fn a_narrower_section_and_a_break_start_with_a_run_up() {
        let tempo = tempo();
        let mut l = Looper::new(
            SectionTimes::new(&tempo, Section::new(1, 4)),
            LoopStyle::Connect,
        );
        let first = l.next_lap(1.0, false);
        assert!(close(first.end_ms, 8000.0));
        // 区間を小節 2〜3 に絞り、止めて 12 秒から再開
        l.set_times(SectionTimes::new(&tempo, Section::new(2, 3)));
        l.resume_at(12000.0);
        let next = l.next_lap(1.0, true);
        assert!(next.has_run_up());
        assert!(close(next.audio.at_ms, 12000.0));
        // 助走は小節 1 の頭（曲の 2000ms）から、区間の頭は小節 2（曲の 4000ms）
        assert!(close(next.audio.from_ms, 2000.0));
        assert!(close(next.start_ms, 14000.0));
        assert!(close(next.end_ms, 18000.0));
    }

    fn at(measure: u32, beat: u32) -> BeatSlot {
        BeatSlot { measure, beat }
    }

    /// 4 周の挑戦。`misses` は周ごとの落とした拍
    fn attempt(speed: u32, misses: &[&[BeatSlot]]) -> Attempt {
        let mut spots = SpotTally::default();
        let mut missed = 0;
        for i in 0..4 {
            let m: Vec<(BeatSlot, DrumLane)> = misses
                .get(i)
                .map(|m| m.iter().map(|&b| (b, DrumLane::SD)).collect())
                .unwrap_or_default();
            missed += m.len() as u32;
            spots.add_lap(&m);
        }
        let mut a = Attempt {
            index: 0,
            speed,
            laps: 4,
            clean_laps: 4 - misses.iter().filter(|m| !m.is_empty()).count() as u32,
            notes: 4 * 40,
            missed,
            spots,
            weak_times: PassRule::Normal.weak_times(),
            passed: None,
        };
        a.passed = Some(a.weak().is_empty());
        a
    }

    const LADDER: SpeedPlan = SpeedPlan::Ladder {
        start: 70,
        target: 100,
    };

    #[test]
    fn single_slips_do_not_hold_back_the_step() {
        let a = attempt(80, &[&[at(61, 1)], &[], &[at(63, 3)]]);
        assert!(a.weak().is_empty());
        assert_eq!(a.slips(), 2);
        assert_eq!(a.passed, Some(true));
        assert_eq!(next_step(LADDER, 80, true, 0), Next::Up(90));
        // 目標の段を越えたらクリア
        assert_eq!(next_step(LADDER, 100, true, 0), Next::Clear);
        assert!(attempt(100, &[&[at(61, 1)]]).cleared());
        assert!(!attempt(90, &[]).cleared());
    }

    #[test]
    fn a_beat_missed_twice_means_again_and_twice_again_steps_down() {
        let a = attempt(80, &[&[at(62, 0)], &[], &[at(62, 0)]]);
        assert_eq!(a.weak().len(), 1);
        assert_eq!(a.passed, Some(false));
        assert_eq!(next_step(LADDER, 80, false, 1), Next::Again);
        assert_eq!(next_step(LADDER, 80, false, 2), Next::Down(70));
        // 始めの速度より下にも下げる（段を下に足す）
        assert_eq!(next_step(LADDER, 70, false, 2), Next::Down(60));
        // 下限より下には下げない
        assert_eq!(next_step(LADDER, SPEED_MIN, false, 5), Next::Again);
    }

    #[test]
    fn a_fixed_speed_is_one_step() {
        let plan = SpeedPlan::Fixed(80);
        assert_eq!(plan.steps(), vec![80]);
        assert_eq!(plan.goal(), 80);
        assert_eq!(next_step(plan, 80, true, 0), Next::Clear);
        // 固定は下げない
        assert_eq!(next_step(plan, 80, false, 4), Next::Again);
    }

    #[test]
    fn ladder_steps_by_ten() {
        assert_eq!(LADDER.steps(), vec![70, 80, 90, 100]);
        let l = SpeedPlan::Ladder {
            start: 75,
            target: 100,
        };
        assert_eq!(l.steps(), vec![75, 85, 95, 100]);
        // 下げた段は下に足す
        assert_eq!(LADDER.stairs(70), vec![70, 80, 90, 100]);
        assert_eq!(LADDER.stairs(60), vec![60, 70, 80, 90, 100]);
        assert_eq!(l.stairs(55), vec![55, 65, 75, 85, 95, 100]);
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
