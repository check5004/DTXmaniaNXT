//! 演奏画面（ドラム）。上流 DTXManiaNX の `CStagePerfDrumsScreen` とその部品（`CActPerfDrumsLaneFlushD`、
//! `CActPerfDrumsPad`、`CActPerfDrumsChipFireD`、`CActPerfDrumsJudgementString`、`CActPerfCommonCombo`、
//! `CActPerfDrumsComboDGB`、`CActPerfDrumsGauge`、`CActPerfDrumsScore`、`CActPerfDrumsStatusPanel`、
//! `CActPerfPanelString`、`CActPerfProgressBar`）と同じ画像を同じ座標と順で描く。
//! 座標と動きの速さは上流のコードから写している。設定は上流の既定（レーンの並び A、RD は右端、
//! 判定の文字はコマ送り、XG のスコアとゲージ）に合わせる。
//!
//! 判定は `Play` が MIDI の時刻と音の再生位置で行う。この画面は `Play` から出来事（叩いた・判定した）を
//! 受け取り、光やコンボなどの動きを実時間で進めて描くだけ。

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use dtx::DrumLane;
use egui::Color32;
use engine::judge::{Judgement, Tally};

use crate::game::{Fx, GaugeRule, Play};
use crate::gfx::{Canvas, Gfx, Glyphs, Opt, Src, Tex, fit_scale, src};
use crate::text::Style;

/// 判定の線の y（上流 `nJudgeLinePosY.Drums` の既定）
pub const JUDGE_LINE_Y: f32 = 561.0;
/// 判定の線より上のどこまでチップを描くか（上流は判定の線から 600 ドットを超えたら打ち切る）
const DRAW_AHEAD_PX: f32 = 600.0;
/// 画面に入ってから暗いところから明るくなるまで
const FADE_IN: Duration = Duration::from_millis(500);

/// 判定の線より上に見えている時間（ミリ秒）。`px_per_ms` は落ちる速さ
pub fn view_ahead_ms(px_per_ms: f32) -> f64 {
    f64::from(DRAW_AHEAD_PX / px_per_ms.max(0.01))
}

/// 上流の `ELane`（ドラムの 10 レーン）。表の添字に使う
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    LC,
    HH,
    SD,
    BD,
    HT,
    LT,
    FT,
    CY,
    LP,
    RD,
}

impl Lane {
    /// 叩いた・判定したレーンを上流のレーンに（上流 `nパッド0Atoレーン07`、`nチャンネル0Atoレーン07`）
    fn of(l: DrumLane) -> Self {
        match l {
            DrumLane::HH | DrumLane::HHO => Self::HH,
            DrumLane::SD => Self::SD,
            DrumLane::BD => Self::BD,
            DrumLane::HT => Self::HT,
            DrumLane::LT => Self::LT,
            DrumLane::FT => Self::FT,
            DrumLane::CY => Self::CY,
            DrumLane::RD => Self::RD,
            DrumLane::LC => Self::LC,
            DrumLane::LP | DrumLane::LBD => Self::LP,
        }
    }

    fn i(self) -> usize {
        self as usize
    }
}

/// レーンの枠（7_Paret.png）を描く順と、置く x・切り出す x・幅（上流 `CActPerfDrumsLaneFlushD` の「動くレーン」）
const PANEL: [(f32, f32, f32); 10] = [
    (416.0, 121.0, 51.0), // LP
    (467.0, 172.0, 57.0), // SD
    (524.0, 229.0, 49.0), // HT
    (691.0, 396.0, 54.0), // FT
    (642.0, 347.0, 49.0), // LT
    (573.0, 278.0, 69.0), // BD
    (367.0, 72.0, 49.0),  // HH
    (815.0, 520.0, 38.0), // RD
    (745.0, 450.0, 70.0), // CY
    (295.0, 0.0, 72.0),   // LC
];

/// レーンの光の x と幅と画像（上流 `stレーンサイズ` と `strファイル名`、`ELane` の順）
const FLUSH: [(f32, f32, &str); 10] = [
    (298.0, 64.0, "leftcymbal"),
    (370.0, 46.0, "hihat"),
    (470.0, 54.0, "snare"),
    (582.0, 60.0, "bass"),
    (528.0, 46.0, "hitom"),
    (645.0, 46.0, "lowtom"),
    (694.0, 46.0, "floortom"),
    (748.0, 64.0, "cymbal"),
    (419.0, 48.0, "leftpedal"),
    (815.0, 38.0, "ridecymbal"),
];

/// パッドの x と 7_pads.png の切り出しの左上（上流 `CActPerfDrumsPad.st基本位置`、`ELane` の順に並べ替えた）
const PADS: [(f32, f32, f32); 10] = [
    (263.0, 0.0, 0.0),    // LC
    (336.0, 96.0, 0.0),   // HH
    (446.0, 0.0, 96.0),   // SD
    (565.0, 0.0, 192.0),  // BD
    (510.0, 96.0, 96.0),  // HT
    (622.0, 192.0, 96.0), // LT
    (672.0, 288.0, 96.0), // FT
    (735.0, 192.0, 0.0),  // CY
    (396.0, 96.0, 192.0), // LP
    (791.0, 288.0, 0.0),  // RD
];
/// パッドを描く順（上流 `CActPerfDrumsPad.n描画順`）
const PAD_ORDER: [Lane; 10] = [
    Lane::LP,
    Lane::BD,
    Lane::SD,
    Lane::FT,
    Lane::LT,
    Lane::HT,
    Lane::RD,
    Lane::CY,
    Lane::HH,
    Lane::LC,
];

/// 火花の中心（+320 する前。上流 `nレーンの中央X座標`）と画像の名前
const FIRE: [(f32, &str); 10] = [
    (7.0, "LC"),
    (71.0, "HH"),
    (176.0, "SD"),
    (293.0, "BD"),
    (230.0, "HT"),
    (349.0, "LT"),
    (398.0, "FT"),
    (464.0, "CY"),
    (124.0, "LP"),
    (514.0, "RD"),
];
/// 飛び散るチップの切り出しの左と幅（上流 `nノーツの左上X座標`、`nノーツの幅`）
const PIECES: [(f32, f32); 10] = [
    (538.0, 64.0),
    (70.0, 46.0),
    (126.0, 54.0),
    (0.0, 60.0),
    (190.0, 46.0),
    (246.0, 46.0),
    (302.0, 46.0),
    (358.0, 60.0),
    (660.0, 48.0),
    (432.0, 48.0),
];

/// 判定の文字を出すレーンの x と幅（上流 `sizeXW`）と、縦の段（上流 `n文字の縦表示位置`）
const JUDGE_POS: [(i32, i32, i32); 10] = [
    (290, 80, -1),
    (367, 46, 1),
    (470, 54, 1),
    (582, 60, 2),
    (528, 46, 0),
    (645, 46, 0),
    (694, 46, 1),
    (748, 64, -1),
    (419, 46, 2),
    (815, 80, 1),
];

/// チップの x と、7_chips_drums.png の切り出しの左と幅（上流 `nチャンネルtoX座標` と `tUpdateAndDraw_Chip_Drums`）
fn chip(l: DrumLane) -> (f32, f32, f32) {
    match l {
        DrumLane::HH => (370.0, 70.0, 56.0),
        DrumLane::SD => (470.0, 126.0, 64.0),
        DrumLane::BD => (582.0, 0.0, 70.0),
        DrumLane::HT => (527.0, 190.0, 56.0),
        DrumLane::LT => (645.0, 246.0, 56.0),
        DrumLane::CY => (748.0, 358.0, 74.0),
        DrumLane::FT => (694.0, 302.0, 56.0),
        // HH オープンは 1 ドット右（上流 eHHOGraphics A の x + 14 - 13）
        DrumLane::HHO => (374.0, 612.0, 48.0),
        DrumLane::RD => (815.0, 432.0, 48.0),
        DrumLane::LC => (298.0, 538.0, 74.0),
        DrumLane::LP | DrumLane::LBD => (419.0, 660.0, 58.0),
    }
}

/// 判定の画像の列（上流の 7_judge strings.png の並び）
fn judge_column(j: Judgement) -> f32 {
    match j {
        Judgement::Perfect => 0.0,
        Judgement::Great => 1.0,
        Judgement::Good => 2.0,
        Judgement::Poor => 3.0,
        Judgement::Miss => 4.0,
    }
}

/// 経過した時間を、`step` ごとに 1 進むカウンタの値に（上流 `CCounter`）
fn count(since: Instant, now: Instant, step_ms: f64) -> f64 {
    (now.saturating_duration_since(since).as_secs_f64() * 1000.0 / step_ms).floor()
}

/// 動きのばらつきに使う乱数（xorshift）
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
            | 1;
        Self(seed)
    }

    /// 0 以上 `n` 未満
    fn below(&mut self, n: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % u64::from(n.max(1))) as u32
    }
}

/// パッドの動き（上流 `STパッド状態`）
#[derive(Debug, Clone, Copy)]
struct PadState {
    /// 明るさ（叩くと 6、18 ミリ秒ごとに 1 下がる）
    bright: i32,
    /// 下への沈み（0〜11）と、その速さ
    dip: i32,
    dip_speed: i32,
    bright_at: Instant,
    dip_at: Instant,
}

impl PadState {
    fn new(now: Instant) -> Self {
        Self {
            bright: 0,
            dip: 0,
            dip_speed: 0,
            bright_at: now,
            dip_at: now,
        }
    }

    fn hit(&mut self, now: Instant) {
        if self.bright == 0 {
            self.bright_at = now;
        }
        if self.dip_speed == 0 && self.dip == 0 {
            self.dip_at = now;
        }
        self.bright = 6;
        self.dip_speed = 2;
    }

    fn update(&mut self, now: Instant) {
        const BRIGHT_STEP: Duration = Duration::from_millis(18);
        const DIP_STEP: Duration = Duration::from_millis(8);
        while self.bright > 0 && now.saturating_duration_since(self.bright_at) >= BRIGHT_STEP {
            self.bright -= 1;
            self.bright_at += BRIGHT_STEP;
        }
        while (self.dip_speed != 0 || self.dip != 0)
            && now.saturating_duration_since(self.dip_at) >= DIP_STEP
        {
            self.dip += self.dip_speed;
            if self.dip > 11 {
                self.dip = 11;
                self.dip_speed = -1;
            } else if self.dip < 0 {
                self.dip = 0;
                self.dip_speed = 0;
            }
            self.dip_at += DIP_STEP;
        }
    }
}

/// 火花のまわりに散る星 1 つ（上流 `ST青い星`）
#[derive(Debug, Clone, Copy)]
struct Star {
    lane: Lane,
    start: Instant,
    steps: u32,
    x: f32,
    y: f32,
    ax: f32,
    ay: f32,
    radius: f32,
}

/// 飛び散るチップのかけら（上流 `ST飛び散るチップ`）
#[derive(Debug, Clone, Copy)]
struct Piece {
    lane: Lane,
    start: Instant,
    steps: u32,
    xl: f32,
    xr: f32,
    y: f32,
    ax: f32,
    ay: f32,
}

/// 判定の文字（上流 `CActPerfCommonJudgementString.st状態`）
#[derive(Debug, Clone, Copy)]
struct JudgeShow {
    judgement: Judgement,
    /// ずれ（ミリ秒、マイナスは早い）。見逃しは None
    lag: Option<i64>,
    start: Instant,
}

/// スコア（上流 `CActPerfCommonScore`：本当の点数へ 10 ミリ秒ごとに近づける）
#[derive(Debug, Clone, Copy)]
struct Score {
    truth: f64,
    shown: i64,
    step: i64,
    at: Instant,
}

/// 進み具合のバーの 1 区間（上流 `CProgressSection`）
#[derive(Debug, Clone, Copy, Default)]
struct Section {
    notes: u32,
    hits: u32,
}

/// 進み具合のバーの区間の数（上流 `nSectionIntervalCount`）
const SECTIONS: usize = 64;

pub struct PlayView {
    entered: Instant,
    /// 難易度の名前（7_Difficulty.png のどこを切り出すかを Script/difficult.dtxs で引く）
    diff_panel: (f32, f32),
    /// 叩いた時刻と強さ（レーンの光）
    flush: [Option<(Instant, u8)>; 10],
    pads: [PadState; 10],
    fires: [Option<Instant>; 10],
    stars: Vec<Star>,
    pieces: Vec<Piece>,
    judges: [Option<JudgeShow>; 10],
    tally: Tally,
    /// コンボが増えた時刻（数字が跳ねる）
    combo_jump: Option<Instant>,
    /// 100 コンボごとの爆発（何百コンボ目か、始めた時刻）
    bomb: Option<(u32, Instant)>,
    bombed: Vec<u32>,
    gauge: Gauge,
    score: Score,
    sections: [Section; SECTIONS],
    /// 音符の数（スコアと達成率の分母）
    notes: usize,
    rng: Rng,
    /// STAGE FAILED を始めた時刻と、そのときの曲の時刻（譜面はそこで止める）
    failed: Option<(Instant, f64)>,
    /// 早く叩いた数と遅く叩いた数（上流 `nTimingHitCount`。見逃しは数えない）
    early: u32,
    late: u32,
    lag_digits: LagDigits,
}

/// 判定のずれの数字の出し方（NX の `ShowLagTime` と `ShowLagTimeColor`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagDigits {
    pub show: LagShow,
    /// 早いを赤、遅いを青にする（NX の既定は逆）
    pub swapped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LagShow {
    Off,
    On,
    /// PERFECT 以外だけ
    NotPerfect,
}

impl LagDigits {
    /// 設定の `show_lag_time` と `show_lag_time_color` から
    pub fn from_config(c: &crate::config::Config) -> Self {
        let show = match c.show_lag_time {
            0 => LagShow::Off,
            2 => LagShow::NotPerfect,
            _ => LagShow::On,
        };
        Self {
            show,
            swapped: c.show_lag_time_color == 1,
        }
    }

    fn shows(self, j: Judgement) -> bool {
        match self.show {
            LagShow::Off => false,
            LagShow::On => true,
            LagShow::NotPerfect => j != Judgement::Perfect,
        }
    }
}

/// 演奏を終えたときの数字（結果の画面と記録に渡す）
#[derive(Debug, Clone, PartialEq)]
pub struct Finish {
    pub score: i64,
    /// 達成率（練習は 0）
    pub rate: f64,
    /// 進み具合のバー（上流の score.ini と同じ 64 文字）
    pub progress: String,
    pub early: u32,
    pub late: u32,
    /// 7_Difficulty.png の切り出しの左上
    pub diff_panel: (f32, f32),
}

impl PlayView {
    /// `label` は選んだ難易度の名前（set.def の `#L1LABEL` など。無ければ空）
    pub fn new(play: &Play, label: &str, lag_digits: LagDigits) -> Self {
        let now = Instant::now();
        let mut sections = [Section::default(); SECTIONS];
        if play.practice.is_none() {
            for n in play.judge.notes() {
                if let Some(s) = sections.get_mut(section_of(n.ms, play.last_chip_ms)) {
                    s.notes += 1;
                }
            }
        }
        Self {
            entered: now,
            diff_panel: difficulty_cell("7", label).unwrap_or((0.0, 0.0)),
            flush: [None; 10],
            pads: [PadState::new(now); 10],
            fires: [None; 10],
            stars: Vec::new(),
            pieces: Vec::new(),
            judges: [None; 10],
            tally: Tally::default(),
            combo_jump: None,
            bomb: None,
            bombed: Vec::new(),
            gauge: Gauge::new(play.gauge),
            score: Score {
                truth: 0.0,
                shown: 0,
                step: 0,
                at: now,
            },
            sections,
            notes: play.judge.notes().len(),
            rng: Rng::new(),
            failed: None,
            early: 0,
            late: 0,
            lag_digits,
        }
    }

    /// 演奏を終えたときの数字。`now_ms` は曲の時刻（ここまでの区間をたどり着いたことにする）
    pub fn finish(&self, play: &Play, now_ms: f64) -> Finish {
        let practice = play.practice.is_some();
        Finish {
            score: self.score.truth as i64,
            rate: self.rate(practice),
            progress: if practice {
                String::new()
            } else {
                progress_text(&self.sections, play.last_chip_ms, now_ms)
            },
            early: self.early,
            late: self.late,
            diff_panel: self.diff_panel,
        }
    }

    /// ゲージが尽きた（上流 `IsFailed`。Risky なら残りの回数が尽きた）
    pub fn gauge_empty(&self) -> bool {
        self.gauge.empty()
    }

    /// STAGE FAILED を始める。`now_ms` は今の曲の時刻（譜面をそこで止める）
    pub fn start_failure(&mut self, now_ms: f64) {
        self.failed.get_or_insert((Instant::now(), now_ms));
    }

    /// STAGE FAILED の途中なら、止めた曲の時刻
    pub fn frozen_ms(&self) -> Option<f64> {
        self.failed.map(|(_, ms)| ms)
    }

    /// STAGE FAILED を出し終えた
    pub fn failure_done(&self) -> bool {
        self.failed.is_some_and(|(at, _)| at.elapsed() >= FAIL_FOR)
    }

    /// `Play` から出来事を受け取る。毎フレーム描く前に呼ぶ
    pub fn feed(&mut self, play: &mut Play) {
        let now = Instant::now();
        let practice = play.practice.is_some();
        for fx in play.take_fx() {
            match fx {
                Fx::Hit { lane, velocity } => {
                    let l = Lane::of(lane);
                    self.flush[l.i()] = Some((now, velocity));
                    self.pads[l.i()].hit(now);
                }
                Fx::Judged {
                    lane,
                    judgement,
                    offset_ms,
                    note_ms,
                } => {
                    let l = Lane::of(lane);
                    match offset_ms {
                        Some(o) if o > 0.0 => self.late += 1,
                        Some(_) => self.early += 1,
                        None => {}
                    }
                    self.judges[l.i()] = Some(JudgeShow {
                        judgement,
                        lag: offset_ms.map(|o| o as i64),
                        start: now,
                    });
                    if !matches!(judgement, Judgement::Poor | Judgement::Miss) {
                        self.start_fire(l, now);
                    }
                    let before = self.tally.combo;
                    self.tally.add(judgement);
                    if self.tally.combo > before {
                        self.combo_jump = Some(now);
                    }
                    self.gauge.apply(judgement);
                    if !practice {
                        self.add_score(judgement, now);
                        if matches!(
                            judgement,
                            Judgement::Perfect | Judgement::Great | Judgement::Good
                        ) && let Some(s) = self
                            .sections
                            .get_mut(section_of(note_ms, play.last_chip_ms))
                        {
                            s.hits += 1;
                        }
                    }
                }
                Fx::StrayMiss { lane } => {
                    // 上流の Tight: MISS を出し、コンボを切り、MISS と同じだけゲージを減らす（MISS の数には入れない）
                    self.judges[Lane::of(lane).i()] = Some(JudgeShow {
                        judgement: Judgement::Miss,
                        lag: None,
                        start: now,
                    });
                    self.tally.combo = 0;
                    self.gauge.apply(Judgement::Miss);
                }
            }
        }
        // 練習は周を足すたびに音符が増える
        self.notes = play.judge.notes().len();
    }

    /// 火花と星と飛び散るチップを出す（上流 `CActPerfDrumsChipFireD.Start`）
    fn start_fire(&mut self, l: Lane, now: Instant) {
        self.fires[l.i()] = Some(now);
        let cx = FIRE[l.i()].0 + 320.0;
        if self.stars.len() < 240 {
            for _ in 0..16 {
                let deg = self.rng.below(360) as f32;
                let (sin, cos) = deg.to_radians().sin_cos();
                self.stars.push(Star {
                    lane: l,
                    start: now,
                    steps: 0,
                    x: cx,
                    y: 555.0,
                    ax: 0.9 * cos,
                    ay: 0.9 * sin - 0.1,
                    radius: 0.3 + self.rng.below(30) as f32 / 100.0,
                });
            }
        }
        if self.pieces.len() < 8 {
            let w = PIECES[l.i()].1;
            let x = FIRE[l.i()].0 + w + 312.0;
            let shift = match l {
                Lane::LC | Lane::BD | Lane::CY => 0.0,
                Lane::HH => 20.0,
                _ => 10.0,
            };
            let deg = 1.0_f32.to_radians();
            self.pieces.push(Piece {
                lane: l,
                start: now,
                steps: 0,
                xl: x + shift,
                xr: x,
                y: 564.0,
                ax: 0.91 * deg.cos() + 0.3,
                ay: 0.91 * (deg.sin() - 0.8),
            });
        }
    }

    /// XG のスコア（上流 `tProcessChipHit` の「100万/{1275+50×(総ノーツ数-50)}」）
    fn add_score(&mut self, j: Judgement, now: Instant) {
        let s = &mut self.score;
        let delta = score_delta(j, &self.tally, self.notes, s.truth);
        if delta <= 0.0 {
            return;
        }
        s.truth = (s.truth + delta).min(MAX_SCORE);
        s.step = (((s.truth - s.shown as f64) / 20.0) as i64).max(1);
        s.at = now;
    }

    /// 演奏の達成率（上流 `tCalculatePlayingSkill`）。練習では出さない（上流のトレーニングと同じ）
    fn rate(&self, practice: bool) -> f64 {
        if practice || self.notes == 0 {
            return 0.0;
        }
        let n = self.notes as f64;
        let t = &self.tally;
        100.0 * f64::from(t.perfect) / n * 0.85
            + 100.0 * f64::from(t.great) / n * 0.35
            + 100.0 * f64::from(t.max_combo) / n * 0.15
    }

    /// 1 フレーム描く。`now_ms` は曲の時刻（表示の補正を足したもの）
    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, play: &Play, now_ms: f64, px_per_ms: f32) {
        let now = Instant::now();
        let skin = &g.skin;
        // 背景
        match skin.tex("Graphics/7_background.jpg") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(8, 8, 14)),
        }
        self.draw_jacket(c, g, play);
        self.draw_lanes(c, g, now);
        self.draw_lines(c, g, play, now_ms, px_per_ms);
        self.draw_chips(c, g, play, now_ms, px_per_ms, now);
        self.draw_progress(c, g, play, now_ms);
        // 判定の線（8×8 の画像を横に並べる。上流は D3D の繰り返しで 559 ドットに広げている）
        if let Some(t) = g.skin.tex("Graphics/ScreenPlayDrums hit-bar.png") {
            c.draw_opt(
                &t,
                295.0,
                JUDGE_LINE_Y,
                Opt {
                    src: Some(src(0.0, 0.0, t.w, 6.0)),
                    sx: 559.0 / t.w,
                    ..Default::default()
                },
            );
        }
        self.draw_pads(c, g, now);
        let practice = play.practice.is_some();
        self.draw_score(c, g, now, practice);
        self.draw_status(c, g, play, practice);
        self.draw_gauge(c, g, px_per_ms);
        self.draw_combo(c, g, play, now);
        self.draw_judges(c, g, now);
        self.draw_fire(c, g, now);
        if let Some(p) = &play.practice {
            crate::practice_hud::draw(c, g, p, now_ms, play.last_chip_ms);
        }
        self.draw_failed(c, g, now);
        // 入ってきたときのフェード
        let t = now.saturating_duration_since(self.entered).as_secs_f32() / FADE_IN.as_secs_f32();
        if t < 1.0 {
            c.cover(Color32::from_black_alpha(((1.0 - t) * 255.0) as u8));
        }
    }

    /// ジャケットと曲名（上流 `CActPerfPanelString`）
    fn draw_jacket(&self, c: &Canvas, g: &mut Gfx, play: &Play) {
        if let Some(t) = g.skin.tex("Graphics/7_JacketPanel.png") {
            c.draw(&t, 915.0, 287.0);
        }
        let jacket = match &play.preimage {
            Some(p) => g.images.get(p).or_else(|| {
                g.images
                    .failed(p)
                    .then(|| g.skin.tex("Graphics/5_preimage default.png"))
                    .flatten()
            }),
            None => g.skin.tex("Graphics/5_preimage default.png"),
        };
        if let Some(t) = jacket {
            // 上流は 245 に縮めて (400, -227) へ動かし、画面の中心のまわりに 0.3 ラジアン回す
            let f = fit_scale(&t, 245.0);
            let (cx, cy) = rotate_about_center(400.0, -227.0, 0.3);
            c.draw_rot(
                &t,
                cx,
                cy,
                0.3,
                Opt {
                    sx: f,
                    sy: f,
                    ..Default::default()
                },
            );
        }
        let edge = |px: f32| Style {
            px,
            color: Color32::WHITE,
            bold: false,
            shadow: None,
            edge: Some((Color32::BLACK, px / 12.0)),
            gradient: None,
        };
        if let Some(t) = g.texts.get(&play.title, &edge(20.0)) {
            t.draw_fit(c, 950.0, 630.0, 320.0);
        }
        if let Some(t) = g.texts.get(&play.artist, &edge(15.0)) {
            t.draw_fit(c, 950.0, 665.0, 320.0);
        }
    }

    /// レーンの枠と光（上流 `CActPerfDrumsLaneFlushD`）
    fn draw_lanes(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        if let Some(t) = g.skin.tex("Graphics/7_Paret.png") {
            for (x, sx, w) in PANEL {
                c.part(&t, x, 0.0, src(sx, 0.0, w, 720.0));
            }
        }
        for (i, slot) in self.flush.iter_mut().enumerate() {
            let Some((at, velocity)) = *slot else {
                continue;
            };
            // 強く叩くほど下から始まる（上流：(1 - 強さ) × 55 から 90 まで 3 ミリ秒ごと）
            let from = ((1.0 - f64::from(velocity) / 127.0) * 55.0).floor();
            let v = from + count(at, now, 3.0);
            if v >= 90.0 {
                *slot = None;
                continue;
            }
            let (x, w, name) = FLUSH[i];
            let Some(t) = g
                .skin
                .tex(&format!("Graphics/ScreenPlayDrums lane flush {name}.png"))
            else {
                continue;
            };
            let y = 700.0 - (v as i32 * 740 / 100) as f32;
            let alpha = (y / 255.0).min(1.0);
            for k in 0..3 {
                let mut n = 0.0;
                while n < w {
                    c.draw_opt(
                        &t,
                        x + n,
                        y,
                        Opt {
                            src: Some(src(k as f32 * 42.0, 0.0, (w - n).min(42.0), 128.0)),
                            alpha,
                            ..Default::default()
                        },
                    );
                    n += 42.0;
                }
            }
        }
    }

    /// 小節線と拍線（上流 `tUpdateAndDraw_Chip_BarLine` と拍線）。練習では周ごとに並べる
    fn draw_lines(&self, c: &Canvas, g: &mut Gfx, play: &Play, now_ms: f64, px_per_ms: f32) {
        let Some(t) = g.skin.tex("Graphics/7_chips_drums.png") else {
            return;
        };
        let y_of = |ms: f64| JUDGE_LINE_Y - ((ms - now_ms) as f32) * px_per_ms;
        let draw = |ms: f64, bar: bool| {
            let y = y_of(ms);
            if (-4.0..724.0).contains(&y) {
                let sy = if bar { 769.0 } else { 772.0 };
                c.part(&t, 295.0, y - 1.0, src(0.0, sy, 559.0, 2.0));
            }
        };
        match &play.practice {
            None => {
                for l in &play.lines {
                    draw(l.ms, l.bar);
                }
            }
            Some(p) => {
                for lap in &p.laps {
                    let span = lap.plan.audio;
                    for l in play
                        .lines
                        .iter()
                        .filter(|l| l.ms >= span.from_ms && l.ms < span.to_ms)
                    {
                        draw(lap.plan.real_ms(l.ms), l.bar);
                    }
                    // 周の頭に何周目かを出す
                    let y = y_of(lap.plan.start_ms);
                    if (0.0..JUDGE_LINE_Y).contains(&y)
                        && let Some(text) = g.texts.get(
                            &format!("{} 周目", lap.plan.index + 1),
                            &Style {
                                px: 14.0,
                                color: Color32::WHITE,
                                bold: true,
                                shadow: None,
                                edge: Some((Color32::BLACK, 1.5)),
                                gradient: None,
                            },
                        )
                    {
                        text.draw(c, 300.0, y - text.h - 2.0, 1.0, 1.0);
                    }
                }
            }
        }
    }

    /// チップ（上流 `tUpdateAndDraw_Chip_PatternOnly_Drums` の模様を描いてから `tUpdateAndDraw_Chip_Drums`）
    fn draw_chips(
        &self,
        c: &Canvas,
        g: &Gfx,
        play: &Play,
        now_ms: f64,
        px_per_ms: f32,
        now: Instant,
    ) {
        let Some(t) = g.skin.tex("Graphics/7_chips_drums.png") else {
            return;
        };
        let y_of = |ms: f64| JUDGE_LINE_Y - ((ms - now_ms) as f32) * px_per_ms;
        let visible =
            |y: f32| y > -64.0 && y < 720.0 + 64.0 && y > JUDGE_LINE_Y - DRAW_AHEAD_PX - 64.0;
        // 模様は 70 ミリ秒ごとに 8 コマ
        let frame = count(self.entered, now, 70.0) as u32 % 8;
        let notes = play.judge.notes();
        let from = notes.partition_point(|n| n.ms < now_ms - 1000.0);
        let mut shown: Vec<(DrumLane, f32, f32)> = Vec::new();
        for (i, n) in notes.iter().enumerate().skip(from) {
            let y = y_of(n.ms);
            if y < -64.0 {
                break;
            }
            if play.judge.is_done(i) || !visible(y) {
                continue;
            }
            shown.push((n.lane, y, 1.0));
        }
        // 練習の助走など、判定しない音符は薄く
        if let Some(p) = &play.practice {
            for &(ms, lane) in &p.faint {
                let y = y_of(ms);
                if visible(y) {
                    shown.push((lane, y, 0.35));
                }
            }
        }
        for &(lane, y, alpha) in &shown {
            let (x, sx, w) = chip(lane);
            c.draw_opt(
                &t,
                x - 5.0,
                y - 32.0,
                Opt {
                    src: Some(src(sx, 128.0 + frame as f32 * 64.0, w, 64.0)),
                    alpha,
                    ..Default::default()
                },
            );
        }
        for &(lane, y, alpha) in &shown {
            let (x, sx, w) = chip(lane);
            c.draw_opt(
                &t,
                x - 5.0,
                y - 32.0,
                Opt {
                    src: Some(src(sx, 0.0, w, 64.0)),
                    alpha,
                    ..Default::default()
                },
            );
        }
    }

    /// 進み具合のバー（上流 `CActPerfProgressBar`）
    fn draw_progress(&self, c: &Canvas, g: &Gfx, play: &Play, now_ms: f64) {
        const X: f32 = 855.0;
        const Y: f32 = 15.0;
        const W: f32 = 20.0;
        const H: f32 = 540.0;
        if let Some(t) = g.skin.tex("Graphics/7_Drum_Progress_bg.png") {
            c.draw(&t, X - 2.0, Y - 15.0);
        }
        // 5 ドットごとの縞
        c.fill(X, Y, W + 2.0, H, Color32::from_rgb(10, 10, 10));
        let mut y = 5.0;
        while y < H {
            c.fill(X, Y + y, W + 2.0, 5.0, Color32::from_rgb(14, 14, 14));
            y += 10.0;
        }
        c.fill(X + W, Y, 1.0, H, Color32::from_white_alpha(64));
        c.fill(X + W + 1.0, Y, 1.0, H, Color32::from_white_alpha(32));
        if play.practice.is_some() || play.last_chip_ms <= 0.0 {
            return;
        }
        let done = ((now_ms / play.last_chip_ms) as f32 * H).clamp(0.0, H);
        if done > 0.0 {
            c.fill(X, Y + H - done, W, done, Color32::from_white_alpha(48));
            c.fill(
                X,
                Y + H - done,
                W,
                done.min(8.0),
                Color32::from_white_alpha(128),
            );
        }
        // 1 区間の幅は音符の数で 1〜10 ブロック（上流の基準は 1600 音符）
        let per_block = 1600.0 / 10.0 / SECTIONS as f64;
        let mut below = H;
        for (i, s) in self.sections.iter().enumerate() {
            let top = (H - (i as f32 + 1.0) * H / SECTIONS as f32).round();
            let h = below - top;
            below = top;
            let blocks = ((f64::from(s.notes) / per_block) as u32 + 1).min(10);
            let w = blocks as f32 * 2.0;
            let color = if section_end_ms(i, play.last_chip_ms) > now_ms {
                Color32::from_white_alpha(64)
            } else if s.notes == 0 {
                continue;
            } else if s.hits >= s.notes {
                Color32::from_rgba_unmultiplied(255, 255, 0, 192)
            } else {
                Color32::from_rgb(0, 191, 255)
            };
            c.fill(X, Y + top, w, h, color);
        }
    }

    /// パッド（上流 `CActPerfDrumsPad`）
    fn draw_pads(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        for p in &mut self.pads {
            p.update(now);
        }
        let pads = g.skin.tex("Graphics/7_pads.png");
        let flush = g.skin.tex("Graphics/ScreenPlayDrums pads flush.png");
        for l in PAD_ORDER {
            let (x, sx, sy) = PADS[l.i()];
            let st = self.pads[l.i()];
            let y = 570.0 + st.dip as f32;
            let s = src(sx, sy, 96.0, 96.0);
            if let Some(t) = &pads {
                c.part(t, x, y, s);
            }
            if let Some(t) = &flush
                && st.bright > 0
            {
                c.draw_opt(
                    t,
                    x,
                    y,
                    Opt {
                        src: Some(s),
                        alpha: (st.bright * 50) as f32 / 255.0,
                        ..Default::default()
                    },
                );
            }
        }
    }

    /// スコア（上流 `CActPerfDrumsScore`）
    fn draw_score(&mut self, c: &Canvas, g: &Gfx, now: Instant, practice: bool) {
        // 練習ではスコアを出さない（その場所に練習の欄を出す）
        if practice {
            return;
        }
        let s = &mut self.score;
        const STEP: Duration = Duration::from_millis(10);
        while now.saturating_duration_since(s.at) >= STEP && (s.shown as f64) < s.truth {
            s.shown = (s.shown + s.step).min(s.truth as i64);
            s.at += STEP;
        }
        if let Some(t) = g.skin.tex("Graphics/7_score numbersGD.png") {
            draw_score_digits(c, &t, 40.0, 41.0, 13.0, s.shown);
        }
    }

    /// STAGE FAILED（上流 `CActPerfStageFailure`：7_stage_failed.jpg の左右の半分が真ん中へ閉じる）
    fn draw_failed(&self, c: &Canvas, g: &Gfx, now: Instant) {
        let Some((at, _)) = self.failed else {
            return;
        };
        let Some(t) = g.skin.tex("Graphics/7_stage_failed.jpg") else {
            c.cover(Color32::from_black_alpha(200));
            return;
        };
        let v = now.saturating_duration_since(at).as_secs_f64() * 1000.0 / FAIL_CLOSE;
        if v < 1.0 {
            let x = (640.0 * (std::f64::consts::FRAC_PI_2 * v).cos()) as f32;
            c.part(&t, 0.0, 0.0, src(x, 0.0, 640.0 - x, 720.0));
            c.part(&t, 640.0 + x, 0.0, src(640.0, 0.0, 640.0 - x, 720.0));
        } else {
            c.draw(&t, 0.0, 0.0);
        }
    }

    /// 左のパネル（上流 `CActPerfDrumsStatusPanel`）
    fn draw_status(&self, c: &Canvas, g: &mut Gfx, play: &Play, practice: bool) {
        draw_skill_panel(
            c,
            g,
            (22.0, 250.0),
            &PanelNumbers {
                tally: &self.tally,
                rate: self.rate(practice),
                level: play.level.unwrap_or(0.0),
                diff_panel: self.diff_panel,
            },
            &LARGE,
        );
    }

    /// ゲージ（上流 `CActPerfDrumsGauge`）
    fn draw_gauge(&self, c: &Canvas, g: &Gfx, px_per_ms: f32) {
        const X: f32 = 294.0;
        let Some(frame) = g.skin.tex("Graphics/7_Gauge.png") else {
            return;
        };
        c.part(&frame, X, 626.0, src(0.0, 0.0, frame.w, 47.0));
        // 落ちる速さのアイコン（上流の速さの段 n は 0.089375 × (n + 1) ドット/ミリ秒）
        if let Some(t) = g.skin.tex("Graphics/7_panel_icons.jpg") {
            // 段は 21 まで。NX v2.0 の絵は 16 段しか無いので、絵にある一番下の段で止める
            let rows = (t.h / 48.0).floor().max(1.0);
            let n = (px_per_ms / 0.089375 - 1.0)
                .round()
                .clamp(0.0, (rows - 1.0).min(20.0));
            c.draw_opt(
                &t,
                -37.0 + X + frame.w,
                634.0,
                Opt {
                    src: Some(src(0.0, n * 48.0, 42.0, 48.0)),
                    sx: 0.761_904_8,
                    sy: 2.0 / 3.0,
                    ..Default::default()
                },
            );
        }
        let bar = src(0.0, 0.0, frame.w - 63.0, 31.0);
        if self.gauge.value >= 1.0
            && let Some(t) = g.skin.tex("Graphics/7_gauge_bar.jpg")
        {
            c.part(&t, X + 20.0, 635.0, bar);
        } else if let Some(t) = g.skin.tex("Graphics/7_gauge_bar.png") {
            c.draw_opt(
                &t,
                X + 20.0,
                635.0,
                Opt {
                    src: Some(bar),
                    sx: self.gauge.value.max(0.0) as f32,
                    ..Default::default()
                },
            );
        }
        c.part(&frame, X, 626.0, src(0.0, 47.0, frame.w, 47.0));
    }

    /// コンボ（上流 `CActPerfCommonCombo.tDrawCombo_Drums` を右上 (1245, 60) に。10 コンボから出す）
    fn draw_combo(&mut self, c: &Canvas, g: &Gfx, play: &Play, now: Instant) {
        let combo = self.tally.combo;
        // 100 コンボごとの爆発（上流 `CActPerfDrumsComboDGB`）
        let hundred = combo / 100;
        if combo >= 100 && combo > hundred + 100 && !self.bombed.contains(&hundred) {
            self.bombed.push(hundred);
            self.bomb = Some((hundred, now));
        }
        if let Some((_, at)) = self.bomb {
            let f = count(at, now, 20.0);
            if f >= 14.0 || combo < 100 {
                self.bomb = None;
            } else if let Some(t) = g.skin.tex("Graphics/7_combobomb.png") {
                c.draw_opt(
                    &t,
                    845.0,
                    -130.0,
                    Opt {
                        src: Some(src(0.0, f as f32 * 340.0, 360.0, 340.0)),
                        sx: 1.5,
                        sy: 1.5,
                        add: true,
                        ..Default::default()
                    },
                );
            }
        }
        if combo < 10 {
            return;
        }
        let Some(t) = g.skin.tex("Graphics/ScreenPlayDrums combo.png") else {
            return;
        };
        // 拍に合わせて少し沈む（上流 `ctComboTimer`：1 拍を 16 に分けた最初の 3 つは 8、ほかは 2）
        let sixteenth = 60_000.0 / play.bpm.max(1.0) / 16.0;
        let tick = count(self.entered, now, sixteenth) as u64 % 16 + 1;
        let pulse = if tick < 4 { 8.0 } else { 2.0 };
        // 跳ねる（2 ミリ秒ごとに 3 進み、180 までの sin）
        let jump_index = self
            .combo_jump
            .map_or(i64::MAX, |at| count(at, now, 2.0) as i64 * 3);
        let jump = |i: i64| {
            if (0..180).contains(&i) {
                (-15.0 * (std::f64::consts::PI * i as f64 / 180.0).sin()) as i32 as f32
            } else {
                0.0
            }
        };
        c.part(&t, 985.0, 222.0 + pulse, src(0.0, 320.0, 250.0, 60.0));
        let digits: Vec<u32> = combo
            .to_string()
            .chars()
            .rev()
            .filter_map(|ch| ch.to_digit(10))
            .collect();
        let n = digits.len() as i64;
        let big = g.skin.tex("Graphics/ScreenPlayDrums combo_2.png");
        let mut x = 1245.0;
        for (i, &d) in digits.iter().enumerate() {
            let j = jump(jump_index - (n - i as i64 - 1));
            if n < 4 {
                x -= 120.0 - 6.0;
                c.part(
                    &t,
                    x,
                    60.0 + j + pulse,
                    src((d % 5) as f32 * 120.0, (d / 5) as f32 * 160.0, 120.0, 160.0),
                );
            } else if let Some(b) = &big {
                x -= 96.0 - 6.0;
                c.part(
                    b,
                    x,
                    80.0 + j + pulse,
                    src((d % 5) as f32 * 96.0, (d / 5) as f32 * 128.0, 96.0, 128.0),
                );
            }
        }
    }

    /// 判定の文字とずれ（上流 `CActPerfDrumsJudgementString` のコマ送り：24 コマを 14 ミリ秒ごと）
    fn draw_judges(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        let strings = g.skin.tex("Graphics/7_judge strings.png");
        let lag = g.skin.tex("Graphics/7_lag numbers.png");
        let digits = self.lag_digits;
        for (i, slot) in self.judges.iter_mut().enumerate() {
            let Some(j) = *slot else { continue };
            let frame = count(j.start, now, 14.0);
            if frame >= 23.0 {
                *slot = None;
                continue;
            }
            let (lx, lw, row) = JUDGE_POS[i];
            let xc = lx + lw / 2;
            let x = (xc - 122) as f32;
            let y = (348 + row * 32 - 87) as f32;
            if let Some(t) = &strings {
                c.part(
                    t,
                    x,
                    y,
                    src(
                        judge_column(j.judgement) * 250.0,
                        frame as f32 * 170.0,
                        250.0,
                        170.0,
                    ),
                );
            }
            if let (Some(t), Some(ms)) = (&lag, j.lag)
                && digits.shows(j.judgement)
            {
                let text = ms.to_string();
                let mut x = (xc - text.len() as i32 * 15 / 2) as f32;
                for ch in text.chars() {
                    let p = if ch == '-' {
                        11
                    } else {
                        ch.to_digit(10).unwrap_or(0) as i32
                    };
                    // 早い（マイナス）は左上の色、遅い（プラス）は右下の色の数字（入れ替えの設定なら逆）
                    let (ox, oy) = if (ms < 0) != digits.swapped {
                        (0.0, 0.0)
                    } else {
                        (64.0, 64.0)
                    };
                    let s = src(
                        ox + (p % 4) as f32 * 15.0,
                        oy + (p / 4) as f32 * 19.0,
                        15.0,
                        19.0,
                    );
                    c.part(t, x, y + 34.0, s);
                    x += 15.0;
                }
            }
        }
    }

    /// 火花・星・飛び散るチップ（上流 `CActPerfDrumsChipFireD`）
    fn draw_fire(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        // 星：7 ミリ秒ごとに 40 コマ
        self.stars.retain_mut(|s| {
            let v = count(s.start, now, 7.0).min(40.0) as u32;
            while s.steps < v {
                s.x += s.ax;
                s.y -= s.ay;
                s.ay *= 1.01;
                s.ay -= 0.0204;
                s.steps += 1;
            }
            if v >= 40 {
                return false;
            }
            if let Some(t) = g.skin.tex(&format!(
                "Graphics/ScreenPlayDrums chip star_{}.png",
                FIRE[s.lane.i()].1
            )) {
                let f = s.radius * (std::f32::consts::FRAC_PI_2 * v as f32 / 100.0).cos();
                c.draw_rot(
                    &t,
                    s.x,
                    s.y,
                    0.0,
                    Opt {
                        sx: f,
                        sy: f,
                        add: true,
                        ..Default::default()
                    },
                );
            }
            true
        });
        // 飛び散るチップ：10 ミリ秒ごとに 44 コマ、半透明（120/255）
        let chips = g.skin.tex("Graphics/7_chips_drums.png");
        self.pieces.retain_mut(|p| {
            let v = count(p.start, now, 10.0).min(44.0) as u32;
            let gravity = (0.031_f32 * 2.0).exp() / 2.0;
            while p.steps < v {
                p.xl += p.ax * (120.0_f32).to_radians().cos() * 5.0;
                p.xr += p.ax * (60.0_f32).to_radians().cos() * 5.0;
                p.y += p.ay * (60.0_f32).to_radians().sin() * 10.0 - gravity;
                p.ax *= 0.995;
                p.ay += 0.031;
                p.steps += 1;
            }
            if v >= 44 {
                return false;
            }
            if let Some(t) = &chips {
                let (left, w) = PIECES[p.lane.i()];
                let o = Opt {
                    src: Some(src(left, 640.0, ((w + 10.0) / 2.0).floor(), 64.0)),
                    alpha: 120.0 / 255.0,
                    ..Default::default()
                };
                let turn = 0.09 * v as f32;
                c.draw_rot(t, p.xl - 50.0, p.y, turn, o);
                c.draw_rot(t, p.xr - 50.0, p.y, -turn, o);
            }
            true
        });
        // 火花：3 ミリ秒ごとに 70 コマ、加算で 2 枚重ね
        for (i, slot) in self.fires.iter_mut().enumerate() {
            let Some(at) = *slot else { continue };
            let v = count(at, now, 3.0);
            if v > 70.0 {
                *slot = None;
                continue;
            }
            let Some(t) = g.skin.tex(&format!(
                "Graphics/ScreenPlayDrums chip fire_{}.png",
                FIRE[i].1
            )) else {
                continue;
            };
            let f = 0.4 + 0.8 * (v / 50.0 * std::f64::consts::FRAC_PI_2).cos() as f32;
            let o = Opt {
                sx: f,
                sy: f,
                add: true,
                ..Default::default()
            };
            let x = FIRE[i].0 + 320.0;
            c.draw_rot(&t, x, 565.0, 0.0, o);
            c.draw_rot(&t, x, 565.0, 0.0, o);
        }
    }
}

/// ゲージがここまで減ったら STAGE FAILED（上流 `GAUGE_MIN`。0 を少し下回るまでは耐える）
const GAUGE_MIN: f64 = -0.1;
/// STAGE FAILED の画像が閉じきるまで（上流は 2 ミリ秒 × 100）
const FAIL_CLOSE: f64 = 200.0;
/// STAGE FAILED を出しておく時間（上流は 2 ミリ秒 × 1000）
const FAIL_FOR: Duration = Duration::from_millis(2000);

/// 7_SkillPanel.png に出す数字
pub struct PanelNumbers<'a> {
    pub tally: &'a Tally,
    /// 達成率（0〜100）
    pub rate: f64,
    pub level: f64,
    /// 7_Difficulty.png の切り出しの左上
    pub diff_panel: (f32, f32),
}

/// 7_SkillPanel.png と、判定の数と割合、達成率、スキル、難易度とレベル。
/// 演奏の画面（上流 `CActPerfDrumsStatusPanel`）と結果の画面（`CActResultParameterPanel`）は
/// 置く場所と大きい数字の送りだけが違う
pub fn draw_skill_panel(
    c: &Canvas,
    g: &mut Gfx,
    (x, y): (f32, f32),
    n: &PanelNumbers<'_>,
    large: &Glyphs<'_>,
) {
    if let Some(t) = g.skin.tex("Graphics/7_SkillPanel.png") {
        c.draw(&t, x, y);
    }
    // 名前（上流は Config.ini の名前、無ければ GUEST）
    if let Some(t) = g.texts.get("GUEST", &Style::plain(20.0, Color32::WHITE)) {
        t.draw(c, x - 2.0, y + 26.0, 1.0, 1.0);
    }
    let small = g.skin.tex("Graphics/7_Ratenumber_s.png");
    let large_tex = g.skin.tex("Graphics/7_Ratenumber_l.png");
    let t = n.tally;
    let judged = t.perfect + t.great + t.good + t.poor + t.miss;
    // 割合は判定した数に対して（練習でも、ここまでの周の割合になる）
    let percent = |k: u32| {
        if judged == 0 {
            0.0
        } else {
            (100.0 * f64::from(k) / f64::from(judged)).round()
        }
    };
    let rows = [t.perfect, t.great, t.good, t.poor, t.miss, t.max_combo];
    for (k, count) in rows.into_iter().enumerate() {
        let ry = y + 72.0 + k as f32 * 30.0;
        SMALL.draw(c, small.as_ref(), x + 80.0, ry, &format!("{count:>4}"));
        SMALL.draw(
            c,
            small.as_ref(),
            x + 167.0,
            ry,
            &format!("{:>3}%", percent(count)),
        );
    }
    if n.rate >= 100.0 {
        if let Some(t) = g.skin.tex("Graphics/7_skill max.png") {
            c.draw(&t, x + 127.0, y + 277.0);
        }
    } else {
        large.draw(
            c,
            large_tex.as_ref(),
            x + 58.0,
            y + 277.0,
            &format!("{:>6.2}", n.rate),
        );
        if let Some(t) = g.skin.tex("Graphics/7_RatePercent_l.png") {
            c.draw(&t, x + 217.0, y + 287.0);
        }
    }
    let skill = n.rate * n.level * 0.2;
    large.draw(
        c,
        large_tex.as_ref(),
        x + 88.0,
        y + 363.0,
        &format!("{skill:>6.2}"),
    );
    if let Some(t) = g.skin.tex("Graphics/7_Difficulty.png") {
        let (sx, sy) = n.diff_panel;
        c.part(&t, x + 14.0, y + 266.0, src(sx, sy, 60.0, 60.0));
    }
    let digits = g.skin.tex("Graphics/7_LevelNumber.png");
    LEVEL.draw(
        c,
        digits.as_ref(),
        x + 18.0,
        y + 290.0,
        &format!("{:.2}", n.level),
    );
}

/// 7_score numbersGD.png のスコア（7 桁の右寄せ）と「SCORE」の文字
pub fn draw_score_digits(c: &Canvas, t: &Tex, x: f32, y: f32, label_y: f32, score: i64) {
    for (i, ch) in format!("{score:>7}").chars().enumerate() {
        if let Some(d) = ch.to_digit(10) {
            c.part(
                t,
                x + i as f32 * 34.0,
                y,
                src(d as f32 * 36.0, 0.0, 36.0, 50.0),
            );
        }
    }
    c.part(t, x, label_y, src(0.0, 50.0, 86.0, 28.0));
}

/// XG のスコアの満点
const MAX_SCORE: f64 = 1_000_000.0;

/// XG のスコアの増え方（上流 `tProcessChipHit` の「100万/{1275+50×(総ノーツ数-50)}」）。
/// `tally` は今の判定を数えたあと、`truth` は今までの本当の点数。
/// 上流は音符が 50 より少ない曲で 100 万を超えるので、100 万で止める
fn score_delta(j: Judgement, tally: &Tally, notes: usize, truth: f64) -> f64 {
    let n = notes as f64;
    let base = MAX_SCORE / (1275.0 + 50.0 * (n - 50.0));
    let combo = f64::from(tally.combo);
    let all_perfect = f64::from(tally.perfect) >= n;
    let delta = match j {
        Judgement::Perfect if combo < n => base,
        // 最後まで全部 PERFECT なら 100 万に合わせる
        Judgement::Perfect if all_perfect => return (MAX_SCORE - truth).max(0.0),
        Judgement::Great => base * 0.5,
        Judgement::Good => base * 0.2,
        _ => 0.0,
    };
    let delta = if combo < 50.0 {
        delta * combo
    } else if combo != n && !all_perfect {
        delta * 50.0
    } else {
        delta
    };
    delta.trunc().min(MAX_SCORE - truth).max(0.0)
}

/// ゲージ（上流 `CActPerfCommonGauge`）
#[derive(Debug, Clone, Copy, PartialEq)]
struct Gauge {
    /// 0〜1（尽きるのは `GAUGE_MIN`）
    value: f64,
    rule: GaugeRule,
    /// Risky の残りの回数
    risky_left: u32,
}

impl Gauge {
    /// 上流 `Init`。ふつうは `GAUGE_INITIAL`（2/3）、Risky は満タン（1 回なら空）から
    fn new(rule: GaugeRule) -> Self {
        Self {
            value: match rule.risky {
                0 => 2.0 / 3.0,
                1 => 0.0,
                _ => 1.0,
            },
            rule,
            risky_left: rule.risky,
        }
    }

    /// 上流 `IsFailed`
    fn empty(&self) -> bool {
        if self.rule.risky > 0 {
            self.risky_left == 0
        } else {
            self.value <= GAUGE_MIN
        }
    }

    /// 判定でゲージを動かす（上流 `Damage`）
    fn apply(&mut self, j: Judgement) {
        let rule = self.rule;
        let hazard = rule.hazard && matches!(j, Judgement::Great | Judgement::Good);
        let bad = hazard || matches!(j, Judgement::Poor | Judgement::Miss);
        let delta = if rule.risky > 0 {
            // Risky はゲージが増えず、ミスのたびに同じだけ減る。最後の 1 回は減らさずに尽きる
            if bad {
                let d = if self.risky_left <= 1 {
                    0.0
                } else {
                    -1.0 / f64::from(rule.risky - 1)
                };
                self.risky_left = self.risky_left.saturating_sub(1);
                d
            } else {
                0.0
            }
        } else if hazard {
            // HAZARD の GREAT・GOOD は MISS の減り（ゲージの減り方はかけない）
            gauge_delta(Judgement::Miss)
        } else if j == Judgement::Miss {
            gauge_delta(j) * rule.miss_factor
        } else {
            gauge_delta(j)
        };
        self.value = (self.value + delta).clamp(GAUGE_MIN, 1.0);
    }
}

/// XG のゲージの増減（上流 `CActPerfCommonGauge.Damage`。MISS にはゲージの減り方の数をかけて使う）
fn gauge_delta(j: Judgement) -> f64 {
    match j {
        Judgement::Perfect => 0.005,
        Judgement::Great => 0.001,
        Judgement::Good => 0.0,
        Judgement::Poor => -0.017,
        Judgement::Miss => -0.041,
    }
}

/// 時刻が進み具合のバーのどの区間か
fn section_of(ms: f64, last_ms: f64) -> usize {
    if last_ms <= 0.0 {
        return 0;
    }
    ((ms * SECTIONS as f64 / last_ms).max(0.0) as usize).min(SECTIONS - 1)
}

/// 区間 `i` の終わりの時刻（上流は「(i + 1) × 最後のチップの時刻 / 64 − 1」を過ぎたらたどり着いたとする）
fn section_end_ms(i: usize, last_ms: f64) -> f64 {
    (i as f64 + 1.0) * last_ms / SECTIONS as f64 - 1.0
}

/// 進み具合のバーの文字列（上流 `GetScoreIniString`）。
/// '0' はたどり着いていない、'1' は取りこぼしあり、'2' は全部叩けた、'3' は音符なし
fn progress_text(sections: &[Section], last_ms: f64, now_ms: f64) -> String {
    sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if section_end_ms(i, last_ms) > now_ms {
                '0'
            } else if s.notes == 0 {
                '3'
            } else if s.hits >= s.notes {
                '2'
            } else {
                '1'
            }
        })
        .collect()
}

/// 画面の中心を原点に y を上にした座標 (x, y) を、原点のまわりに `angle` 回して、画面の座標にする（上流の `tDraw3D`）
pub fn rotate_about_center(x: f32, y: f32, angle: f32) -> (f32, f32) {
    let (sin, cos) = angle.sin_cos();
    let rx = x * cos - y * sin;
    let ry = x * sin + y * cos;
    (640.0 + rx, 360.0 - ry)
}

/// スキンの Script/difficult.dtxs から、場面 `scene` の難易度の名前 `label` の画像の切り出しの左上を引く
/// （上流 `tSetDifficultyLabelFromScript`。行は「場面,名前,x,y[,大文字小文字を区別するなら 1]」）
pub fn difficulty_cell(scene: &str, label: &str) -> Option<(f32, f32)> {
    let bytes = crate::skin::read("Script/difficult.dtxs")?;
    let (text, _) = dtx::decode(&bytes);
    parse_difficulty(&text, scene, label)
}

fn parse_difficulty(text: &str, scene: &str, label: &str) -> Option<(f32, f32)> {
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with("//") {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if !(4..=5).contains(&f.len()) || f[0] != scene {
            continue;
        }
        let same = if f.len() == 5 && f[4] == "1" {
            f[1] == label
        } else {
            f[1].eq_ignore_ascii_case(label)
        };
        if !same {
            continue;
        }
        return Some((f[2].trim().parse().ok()?, f[3].trim().parse().ok()?));
    }
    None
}

/// 7_Ratenumber_s.png の文字（上流 `st小文字位置`：20×26、送りは 20）
static SMALL: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(
        glyph_table(20.0, 26.0, 20.0, &[('%', 200.0), ('.', 210.0)], 20.0, 20.0).into_boxed_slice(),
    ),
    missing: 20.0,
});
/// 7_Ratenumber_l.png の文字（上流 `st大文字位置`：28×42、送りは 29、点は幅 10 で送り 12）
static LARGE: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(glyph_table(28.0, 42.0, 29.0, &[('.', 280.0)], 10.0, 12.0).into_boxed_slice()),
    missing: 29.0,
});
/// 結果の画面の 7_Ratenumber_l.png の文字（上流 `CActResultParameterPanel.tDrawStringLarge`：送りは 28）
pub static LARGE_RESULT: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(glyph_table(28.0, 42.0, 28.0, &[('.', 280.0)], 10.0, 12.0).into_boxed_slice()),
    missing: 28.0,
});
/// 7_LevelNumber.png の文字（上流 `st難易度数字位置`：16×32、点は幅 5 で送り 5）
static LEVEL: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(glyph_table(16.0, 32.0, 16.0, &[('.', 160.0)], 5.0, 5.0).into_boxed_slice()),
    missing: 16.0,
});

/// 0〜9 が `w` ずつ並んだ数字の表を作る。`extra` はほかの文字とその x（幅 `extra_w`、送り `extra_adv`）
fn glyph_table(
    w: f32,
    h: f32,
    adv: f32,
    extra: &[(char, f32)],
    extra_w: f32,
    extra_adv: f32,
) -> Vec<(char, Src, f32)> {
    let mut out: Vec<(char, Src, f32)> = (0..10u8)
        .map(|d| (char::from(b'0' + d), src(f32::from(d) * w, 0.0, w, h), adv))
        .collect();
    for &(ch, x) in extra {
        // 上流の小さい数字は % と点も 20 の幅で切り出す
        let ew = if ch == '.' { extra_w } else { w };
        out.push((ch, src(x, 0.0, ew, h), extra_adv));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jacket_center_matches_upstream_transform() {
        // 上流：Translation(400, -227) のあと RotationZ(0.3)
        let (x, y) = rotate_about_center(400.0, -227.0, 0.3);
        assert!((x - 1089.2).abs() < 0.5, "{x}");
        assert!((y - 458.7).abs() < 0.5, "{y}");
    }

    #[test]
    fn difficulty_script_lookup() {
        let s = "// comment\n7,BASIC,0,360\n7,Master,0,300,1\n6,BASIC,0,50\n";
        assert_eq!(parse_difficulty(s, "7", "basic"), Some((0.0, 360.0)));
        assert_eq!(parse_difficulty(s, "6", "BASIC"), Some((0.0, 50.0)));
        // 5 つ目が 1 なら大文字小文字を区別する
        assert_eq!(parse_difficulty(s, "7", "MASTER"), None);
        assert_eq!(parse_difficulty(s, "7", "Master"), Some((0.0, 300.0)));
        assert_eq!(parse_difficulty(s, "7", "EXTREME"), None);
    }

    #[test]
    fn pad_dips_and_comes_back() {
        let t0 = Instant::now();
        let mut p = PadState::new(t0);
        p.hit(t0);
        p.update(t0 + Duration::from_millis(48));
        // 8 ミリ秒ごとに 2 ずつ沈み、11 で戻り始める
        assert_eq!(p.dip, 11);
        assert_eq!(p.dip_speed, -1);
        p.update(t0 + Duration::from_millis(48 + 8 * 12));
        assert_eq!((p.dip, p.dip_speed), (0, 0));
        // 明るさは 18 ミリ秒ごとに 1 下がる
        assert_eq!(p.bright, 0);
    }

    #[test]
    fn sections_split_the_song_into_64() {
        assert_eq!(section_of(0.0, 64_000.0), 0);
        assert_eq!(section_of(1_000.0, 64_000.0), 1);
        assert_eq!(section_of(64_000.0, 64_000.0), 63);
        assert_eq!(section_of(-5.0, 64_000.0), 0);
    }

    #[test]
    fn lag_digits_follow_the_config() {
        let mut c = crate::config::Config::default();
        let on = LagDigits::from_config(&c);
        assert_eq!(on.show, LagShow::On);
        assert!(!on.swapped);
        c.show_lag_time = 0;
        assert_eq!(LagDigits::from_config(&c).show, LagShow::Off);
        c.show_lag_time = 2;
        c.show_lag_time_color = 1;
        let d = LagDigits::from_config(&c);
        assert_eq!(d.show, LagShow::NotPerfect);
        assert!(d.swapped);
        assert!(!d.shows(Judgement::Perfect));
        assert!(d.shows(Judgement::Great));
    }

    /// 全部 PERFECT で叩いたときのスコア
    fn all_perfect_score(notes: usize) -> f64 {
        let mut t = Tally::default();
        let mut truth = 0.0;
        for _ in 0..notes {
            t.add(Judgement::Perfect);
            truth = (truth + score_delta(Judgement::Perfect, &t, notes, truth)).min(MAX_SCORE);
        }
        truth
    }

    #[test]
    fn gauge_follows_the_damage_level() {
        let mut g = Gauge::new(GaugeRule::default());
        g.apply(Judgement::Miss);
        assert!((g.value - (2.0 / 3.0 - 0.0205)).abs() < 1e-9);
        let mut small = Gauge::new(GaugeRule {
            miss_factor: 0.25,
            ..GaugeRule::default()
        });
        small.apply(Judgement::Miss);
        assert!((small.value - (2.0 / 3.0 - 0.01025)).abs() < 1e-9);
        // POOR には減り方をかけない
        small.apply(Judgement::Poor);
        assert!((small.value - (2.0 / 3.0 - 0.01025 - 0.017)).abs() < 1e-9);
    }

    #[test]
    fn hazard_drains_on_great_and_good() {
        let mut g = Gauge::new(GaugeRule {
            hazard: true,
            ..GaugeRule::default()
        });
        g.apply(Judgement::Perfect);
        g.apply(Judgement::Great);
        g.apply(Judgement::Good);
        assert!((g.value - (2.0 / 3.0 + 0.005 - 0.082)).abs() < 1e-9);
    }

    #[test]
    fn risky_fails_after_n_misses() {
        let mut g = Gauge::new(GaugeRule {
            risky: 3,
            ..GaugeRule::default()
        });
        assert_eq!(g.value, 1.0);
        g.apply(Judgement::Perfect);
        assert_eq!(g.value, 1.0);
        g.apply(Judgement::Miss);
        assert!((g.value - 0.5).abs() < 1e-9 && !g.empty());
        g.apply(Judgement::Poor);
        assert!(g.value.abs() < 1e-9 && !g.empty());
        g.apply(Judgement::Miss);
        assert!(g.empty());
        // 1 回なら空から始まり、1 回で尽きる
        let mut one = Gauge::new(GaugeRule {
            risky: 1,
            ..GaugeRule::default()
        });
        assert_eq!(one.value, 0.0);
        one.apply(Judgement::Great);
        assert!(!one.empty());
        one.apply(Judgement::Miss);
        assert!(one.empty());
        one.apply(Judgement::Miss);
        assert_eq!(one.value, 0.0);
    }

    #[test]
    fn all_perfect_is_exactly_a_million() {
        assert_eq!(all_perfect_score(37), MAX_SCORE);
        assert_eq!(all_perfect_score(50), MAX_SCORE);
        assert_eq!(all_perfect_score(400), MAX_SCORE);
        assert_eq!(all_perfect_score(1234), MAX_SCORE);
    }

    #[test]
    fn score_grows_with_the_combo_up_to_50() {
        let mut t = Tally::default();
        t.add(Judgement::Perfect);
        t.add(Judgement::Perfect);
        // 400 音符：100 万 / (1275 + 50 × 350) = 53.28…、コンボ 2 なら 2 倍
        assert_eq!(score_delta(Judgement::Perfect, &t, 400, 0.0), 106.0);
        assert_eq!(score_delta(Judgement::Great, &t, 400, 0.0), 53.0);
        assert_eq!(score_delta(Judgement::Miss, &t, 400, 0.0), 0.0);
    }

    #[test]
    fn progress_text_marks_each_section() {
        let mut sections = [Section::default(); SECTIONS];
        sections[0] = Section { notes: 4, hits: 4 };
        sections[1] = Section { notes: 4, hits: 3 };
        sections[2] = Section { notes: 4, hits: 0 };
        // 区間 3 は音符なし
        sections[4] = Section { notes: 1, hits: 1 };
        // 64 秒の曲の 4.5 秒まで：区間 0〜3 にたどり着いた
        let t = progress_text(&sections, 64_000.0, 4_500.0);
        assert_eq!(t.len(), SECTIONS);
        assert_eq!(&t[..6], "211300");
        assert!(t[4..].chars().all(|c| c == '0'));
        // 最後まで
        let t = progress_text(&sections, 64_000.0, 66_000.0);
        assert_eq!(&t[..5], "21132");
        assert!(t[5..].chars().all(|c| c == '3'));
    }

    #[test]
    fn small_digits_skip_spaces_but_advance() {
        let g = &*SMALL;
        assert!(g.table.iter().any(|(c, _, _)| *c == '%'));
        assert_eq!(g.missing, 20.0);
    }
}
