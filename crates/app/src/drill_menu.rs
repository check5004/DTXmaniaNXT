//! 基礎練習の画面（タイトルの「Drills」から開く。2026-11-25 見本の案 A、2026-11-27 に型が増えて 3 列に）。
//!
//! 左に分け方、真ん中にその分け方のドリル、右に選んだドリルの手順の絵、やり方と手順、テンポと段、これまでの最高。
//! 左の列の一番上は「今日のメニュー」（`today`）。真ん中の列にその日の並び、右に全体の帯と選んだものの中身を出し、
//! SD/FT で長さ（5・10・15 分、始めたら変えない）、CY で選んだものから始める。
//! 左の列では HT/LT で分け方、CY か FT で真ん中の列へ。真ん中の列では HT/LT でドリル、SD/FT でテンポ（5 ずつ）、
//! HH でやり方、RD で手順（右から・左から）、CY で始め、LC で左の列へ戻る（左の列の LC はタイトルへ）。
//! テンポはドリルごとに、やり方と手順は全体で覚える。
//! 1 回はやり方ごとに決まった長さで必ず終わり（限界チャレンジは 3 つ落とすか上の端まで）、
//! 終えたらこの画面の同じドリルに戻る。
//! 次に何をするかは自分で選ぶ（勝手に次のドリルやテンポへ進めない）。
//! 足のパッドはメニューに使わない（誤って踏むため）。

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use chrono::{Datelike, NaiveDate};
use egui::{Color32, Stroke};
use engine::drill::{
    BLIND_ALPHA, BPM_STEP, COUNT_IN_MEASURES, Category, DEFAULT_RISE, DRILLS, Drill, Hand,
    LIMIT_RISE, LIMIT_STEP_MEASURES, MANY_MISSES, RISES, Run, Way, nudge_bpm,
};
use i18n::{count, t, tf};

use crate::band::{Band, BandData};
use crate::drill::{
    self, ACCENT_COLOR, DrillRecords, LEFT_COLOR, RIGHT_COLOR, category_name, hand_letter,
    lead_name, pattern_line, way_name,
};
use crate::drill_hud::{DEMO_COLOR, Stairs, clock};
use crate::gfx::{Canvas, Gfx};
use crate::settings_ui::{self as ui, Pad};
use crate::sfx::Se;
use crate::text::Style;
use crate::today::{self, LENGTHS, Plan, Progress, SongPick, Task};

/// 左の列（分け方）、真ん中の列（ドリル）、右の板
const CATS: (f32, f32, f32, f32) = (40.0, 62.0, 226.0, 618.0);
const LIST: (f32, f32, f32, f32) = (278.0, 62.0, 330.0, 618.0);
const DETAIL: (f32, f32, f32, f32) = (620.0, 62.0, 620.0, 618.0);
/// 分け方の行と、ドリルの行の高さ
const CAT_H: f32 = 45.0;
const ROW_H: f32 = 42.0;
/// 真ん中の列の、ドリルの行が始まる所（板の上から）
const LIST_TOP: f32 = 44.0;
/// 左の列の、今日のメニューの行と分け方の間
const TODAY_GAP: f32 = 12.0;
/// 今日のメニューの役目の札の幅
const ROLE_W: f32 = 74.0;
/// 左の列の下の、上げ幅とタイトルへの行の場所（板の下の端から）
const FOOT_H: f32 = 92.0;
const FOOT_ROW_H: f32 = 36.0;

/// 選べる行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// 左の列の一番上の、今日のメニュー（真ん中の列にいるときも）
    Today,
    /// 左の列の分け方
    Category(Category),
    /// 真ん中の列のドリル（`DRILLS` の添字）
    Drill(usize),
    /// 1 段ごとのテンポの上げ幅
    Rise,
    /// タイトルに戻る
    Back,
}

/// 操作している列
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// 左の列（分け方・上げ幅・タイトルへ）
    Left,
    /// 真ん中の列（ドリル）
    Drills,
    /// 真ん中の列（今日のメニュー）
    Today,
    /// 右の欄（選んだドリルのやり方・手順・テンポと、始める）
    Detail,
}

/// 右の欄の行（上から）。ドリルを CY で選ぶと「始める」に来る
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailRow {
    Mode,
    Lead,
    Tempo,
    Start,
}

const DETAIL_ROWS: [DetailRow; 4] = [
    DetailRow::Mode,
    DetailRow::Lead,
    DetailRow::Tempo,
    DetailRow::Start,
];

/// 操作（パッドとキーボードから）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    /// テンポを下げる（上げ幅の行なら上げ幅を下げる）
    Left,
    Right,
    Decide,
    Cancel,
    /// やり方を次へ（キーボードの Tab）
    Way,
    /// 手順の右から・左からを入れ替える（キーボードの H）
    Lead,
}

/// 操作の結果
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Response {
    pub se: Option<Se>,
    pub done: Option<Done>,
    /// 覚えておく値（テンポ、上げ幅、選んだドリル）が変わった
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    Start(Run),
    /// 今日のメニューを、この進み具合で、この番目から始める
    Today(Box<Progress>, usize),
    Back,
}

/// 覚えておく値（設定のファイルへ）
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// 最後に選んだドリル
    pub last: String,
    pub rise: u32,
    /// ドリルごとの目標のテンポ（既定から変えたもの）
    pub bpm: BTreeMap<String, u32>,
    /// やり方（[`Way::key`]）
    pub way: String,
    /// 手順を左からにする
    pub lefty: bool,
    /// 今日のメニューの長さ（分）
    pub today_minutes: u32,
    /// 最後に今日のメニューを開いていた（次も今日のメニューの行から）
    pub on_today: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            last: String::new(),
            rise: DEFAULT_RISE,
            bpm: BTreeMap::new(),
            way: Way::Climb.key(),
            lefty: false,
            today_minutes: today::DEFAULT_LENGTH,
            on_today: false,
        }
    }
}

/// 今日のメニューのために、開くときに渡すもの
#[derive(Debug, Clone)]
pub struct TodaySetup {
    pub day: NaiveDate,
    /// 曲の苦手な所（記録から。無ければ曲の代わりに型を足す）
    pub song: Option<SongPick>,
    /// 始めたメニュー（今日のものだけ）
    pub progress: Option<Progress>,
    /// 今日のメニューを最後までやった
    pub done_today: bool,
    /// 今日のメニューの列を開いておく（今日のメニューの結果から戻ったとき）
    pub open: bool,
}

/// 今日のメニューの列の中身
struct TodayState {
    minutes: u32,
    day: NaiveDate,
    song: Option<SongPick>,
    progress: Option<Progress>,
    done_today: bool,
    /// 真ん中の列で選んでいるもの
    cursor: usize,
    /// 出している並び（始めていれば、始めたもの）
    plan: Plan,
}

pub struct DrillMenu {
    focus: Focus,
    /// 真ん中の列に今日のメニューを出している（左の列で今日のメニューの行にいるか、その列にいる）
    showing_today: bool,
    today: TodayState,
    /// 左の列の行（分け方、上げ幅、タイトルへ）
    left: usize,
    /// 真ん中の列に出している分け方
    category: usize,
    /// 分け方ごとに選んでいるドリル（分け方の中の何番目か）
    picked: [usize; Category::ALL.len()],
    /// 真ん中の列の一番上に見えている行
    top: usize,
    rise: u32,
    bpm: HashMap<&'static str, u32>,
    /// やり方（ドリルで使えなければ、そのドリルではテンポを上げる）
    way: Way,
    lefty: bool,
    /// 右の欄で選んでいる行
    detail: DetailRow,
    records: DrillRecords,
    entered: Instant,
}

/// 左の列の行（今日のメニュー、分け方、上げ幅、タイトルへ）
fn left_items() -> Vec<Item> {
    let mut v = vec![Item::Today];
    v.extend(Category::ALL.into_iter().map(Item::Category));
    v.extend([Item::Rise, Item::Back]);
    v
}

/// 分け方のドリル（`DRILLS` の添字、並べる順）
fn drills_of(c: Category) -> Vec<usize> {
    (0..DRILLS.len())
        .filter(|&i| DRILLS[i].category == c)
        .collect()
}

/// 1 回の長さ（秒。カウントの小節も入れて。限界チャレンジは上の端まで叩いたとき）
pub fn run_seconds(run: &Run) -> f64 {
    let quarters = run.drill.bar_len() * 4.0;
    let bar = |bpm: u32| quarters * 60.0 / f64::from(bpm.max(1));
    f64::from(COUNT_IN_MEASURES) * bar(run.bpms[0])
        + run
            .bpms
            .iter()
            .map(|&b| f64::from(run.step_measures) * bar(b))
            .sum::<f64>()
}

/// 左の列の、分け方 `c`（`Category::ALL` の添字）の行
fn left_of(c: usize) -> usize {
    c + 1
}

impl DrillMenu {
    pub fn new(prefs: &Prefs, records: DrillRecords, setup: TodaySetup) -> Self {
        let bpm = DRILLS
            .iter()
            .filter_map(|d| prefs.bpm.get(d.id).map(|&b| (d.id, nudge_bpm(b, 0))))
            .collect();
        let rise = if RISES.contains(&prefs.rise) {
            prefs.rise
        } else {
            DEFAULT_RISE
        };
        let minutes = if LENGTHS.contains(&prefs.today_minutes) {
            prefs.today_minutes
        } else {
            today::DEFAULT_LENGTH
        };
        let progress = setup.progress.filter(|p| p.plan.day == setup.day);
        let mut m = Self {
            focus: Focus::Left,
            showing_today: true,
            today: TodayState {
                minutes: progress.as_ref().map_or(minutes, |p| p.plan.minutes),
                day: setup.day,
                song: setup.song,
                done_today: setup.done_today || progress.as_ref().is_some_and(Progress::complete),
                cursor: progress.as_ref().map_or(0, Progress::first_open),
                progress,
                plan: Plan {
                    day: setup.day,
                    minutes,
                    steps: Vec::new(),
                },
            },
            left: 0,
            category: 0,
            picked: [0; Category::ALL.len()],
            top: 0,
            rise,
            bpm,
            way: Way::from_key(&prefs.way).unwrap_or(Way::Climb),
            lefty: prefs.lefty,
            detail: DetailRow::Start,
            records,
            entered: Instant::now(),
        };
        // 最後に選んだドリルがあれば、その分け方のそのドリルから（終えて戻ってきたときもここ）
        if let Some(i) = DRILLS.iter().position(|d| d.id == prefs.last) {
            let c = Category::ALL
                .iter()
                .position(|&c| c == DRILLS[i].category)
                .unwrap_or(0);
            m.category = c;
            m.picked[c] = drills_of(DRILLS[i].category)
                .iter()
                .position(|&x| x == i)
                .unwrap_or(0);
            if !setup.open && !prefs.on_today {
                m.left = left_of(c);
                m.focus = Focus::Drills;
                m.showing_today = false;
            }
        }
        // 今日のメニューの結果から戻ったら、今日のメニューの次のものに
        if setup.open {
            m.focus = Focus::Today;
            m.left = 0;
            m.showing_today = true;
        }
        m.follow();
        m.refresh_today();
        m
    }

    /// 今日のメニューの並びを作り直す（始めていれば始めたもの）。テンポや手順を変えたときも
    fn refresh_today(&mut self) {
        let plan = match &self.today.progress {
            Some(p) => p.plan.clone(),
            None => {
                let tempo = |d: &Drill| self.bpm_of(d);
                today::plan(
                    self.today.minutes,
                    &today::Inputs {
                        day: self.today.day,
                        records: &self.records,
                        tempo: &tempo,
                        rise: self.rise,
                        lefty: self.lefty,
                        song: self.today.song.as_ref(),
                    },
                )
            }
        };
        self.today.cursor = self.today.cursor.min(plan.steps.len().saturating_sub(1));
        self.today.plan = plan;
    }

    /// 今日のメニューを途中まで進めている（長さは変えない）
    fn today_locked(&self) -> bool {
        self.today
            .progress
            .as_ref()
            .is_some_and(|p| p.started() && !p.complete())
    }

    /// 今日のメニューの `k` 番目を終えた
    fn today_done(&self, k: usize) -> bool {
        self.today
            .progress
            .as_ref()
            .is_some_and(|p| p.done.get(k) == Some(&true))
    }

    /// 選んでいる行
    pub fn selected(&self) -> Item {
        match self.focus {
            Focus::Left => left_items()[self.left],
            Focus::Drills | Focus::Detail => Item::Drill(self.drill_index()),
            Focus::Today => Item::Today,
        }
    }

    /// 真ん中の列の分け方
    fn category(&self) -> Category {
        Category::ALL[self.category]
    }

    /// 真ん中の列で選んでいるドリルの `DRILLS` の添字
    fn drill_index(&self) -> usize {
        let list = drills_of(self.category());
        list[self.picked[self.category].min(list.len() - 1)]
    }

    /// 右の板に出すドリル（左の列にいるときは、真ん中の列で選んでいるもの）
    fn drill(&self) -> &'static Drill {
        &DRILLS[self.drill_index()]
    }

    /// ドリルの目標のテンポ
    pub fn bpm_of(&self, d: &Drill) -> u32 {
        self.bpm.get(d.id).copied().unwrap_or(d.default_bpm)
    }

    /// 選んでいるドリルで始めるときの 1 回
    pub fn run(&self) -> Run {
        let d = self.drill();
        Run::with(d, self.way, self.bpm_of(d), self.rise, self.lefty)
    }

    /// 選んでいるドリルで使うやり方
    fn way(&self) -> Way {
        self.way.for_drill(self.drill())
    }

    /// 覚えておく値
    pub fn prefs(&self) -> Prefs {
        Prefs {
            last: self.drill().id.to_owned(),
            rise: self.rise,
            bpm: DRILLS
                .iter()
                .filter_map(|d| {
                    let b = self.bpm.get(d.id)?;
                    (*b != d.default_bpm).then(|| (d.id.to_owned(), *b))
                })
                .collect(),
            way: self.way.key(),
            lefty: self.lefty,
            today_minutes: self.today.minutes,
            on_today: self.showing_today,
        }
    }

    pub fn input(&mut self, input: Input) -> Response {
        let r = match self.focus {
            Focus::Left => self.input_left(input),
            Focus::Drills => self.input_drills(input),
            Focus::Today => self.input_today(input),
            Focus::Detail => self.input_detail(input),
        };
        // テンポ・上げ幅・手順が変われば、今日のメニューのテンポも変わる
        if r.changed {
            self.refresh_today();
        }
        r
    }

    /// 左の列（分け方・上げ幅・タイトルへ）
    fn input_left(&mut self, input: Input) -> Response {
        let items = left_items();
        let mut r = Response::default();
        match input {
            Input::Up | Input::Down => {
                let to = if input == Input::Up {
                    self.left.checked_sub(1)
                } else {
                    (self.left + 1 < items.len()).then_some(self.left + 1)
                };
                if let Some(to) = to {
                    self.left = to;
                    match items[to] {
                        Item::Category(c) => {
                            // 分け方を動かすと真ん中の列もその分け方に
                            self.category = Category::ALL.iter().position(|&x| x == c).unwrap_or(0);
                            self.showing_today = false;
                            self.follow();
                            r.changed = true;
                        }
                        Item::Today => {
                            self.showing_today = true;
                            r.changed = true;
                        }
                        _ => {}
                    }
                    r.se = Some(Se::Move);
                }
            }
            Input::Left | Input::Right => match items[self.left] {
                Item::Rise => {
                    let dir = if input == Input::Left { -1 } else { 1 };
                    let i = RISES.iter().position(|&x| x == self.rise).unwrap_or(0);
                    let j = (i as i32 + dir).clamp(0, RISES.len() as i32 - 1) as usize;
                    if j != i {
                        self.rise = RISES[j];
                        r.se = Some(Se::Change);
                        r.changed = true;
                    }
                }
                Item::Category(_) | Item::Today if input == Input::Right => return self.enter(),
                _ => {}
            },
            Input::Decide => match items[self.left] {
                Item::Category(_) | Item::Drill(_) | Item::Today => return self.enter(),
                Item::Rise => {
                    // 決定で上げ幅を順に回す
                    let i = RISES.iter().position(|&x| x == self.rise).unwrap_or(0);
                    self.rise = RISES[(i + 1) % RISES.len()];
                    r.se = Some(Se::Change);
                    r.changed = true;
                }
                Item::Back => {
                    r.se = Some(Se::Cancel);
                    r.done = Some(Done::Back);
                }
            },
            Input::Cancel => {
                r.se = Some(Se::Cancel);
                r.done = Some(Done::Back);
            }
            // やり方と手順は真ん中の列で選ぶ
            Input::Way | Input::Lead => {}
        }
        r
    }

    /// 左の列の分け方（か今日のメニュー）から真ん中の列へ
    fn enter(&mut self) -> Response {
        if left_items()[self.left] == Item::Today {
            self.focus = Focus::Today;
            self.today.cursor = self.today.progress.as_ref().map_or(0, Progress::first_open);
        } else {
            self.focus = Focus::Drills;
        }
        self.follow();
        Response {
            se: Some(Se::Decide),
            done: None,
            changed: true,
        }
    }

    /// 真ん中の列（ドリル）
    fn input_drills(&mut self, input: Input) -> Response {
        let n = drills_of(self.category()).len();
        let mut r = Response::default();
        let at = self.picked[self.category];
        match input {
            Input::Up | Input::Down => {
                let to = if input == Input::Up {
                    at.checked_sub(1)
                } else {
                    (at + 1 < n).then_some(at + 1)
                };
                if let Some(to) = to {
                    self.picked[self.category] = to;
                    self.follow();
                    r.se = Some(Se::Move);
                    r.changed = true;
                }
            }
            Input::Left | Input::Right => {
                return self.nudge_tempo(if input == Input::Left { -1 } else { 1 });
            }
            // 1 打では始めない。右の欄の「始める」へ移り、もう一度 CY で始める
            // （やり方と手順も右の欄で変える。何気なく叩いても、すぐには始まらないように）
            Input::Decide => {
                self.focus = Focus::Detail;
                self.detail = DetailRow::Start;
                r.se = Some(Se::Decide);
            }
            Input::Cancel => {
                // 左の列へ戻る（分け方の行に）
                self.focus = Focus::Left;
                self.left = left_of(self.category);
                r.se = Some(Se::Cancel);
            }
            Input::Way => return self.step_way(1),
            Input::Lead => return self.toggle_lead(),
        }
        r
    }

    /// 右の欄（やり方・手順・テンポ・始める）
    fn input_detail(&mut self, input: Input) -> Response {
        let mut r = Response::default();
        let at = DETAIL_ROWS
            .iter()
            .position(|&x| x == self.detail)
            .unwrap_or(DETAIL_ROWS.len() - 1);
        match input {
            Input::Up | Input::Down => {
                let to = if input == Input::Up {
                    at.checked_sub(1)
                } else {
                    (at + 1 < DETAIL_ROWS.len()).then_some(at + 1)
                };
                if let Some(to) = to {
                    self.detail = DETAIL_ROWS[to];
                    r.se = Some(Se::Move);
                }
            }
            Input::Left | Input::Right => {
                let dir = if input == Input::Left { -1 } else { 1 };
                return match self.detail {
                    DetailRow::Mode => self.step_way(dir),
                    DetailRow::Lead => self.toggle_lead(),
                    DetailRow::Tempo => self.nudge_tempo(dir),
                    DetailRow::Start => r,
                };
            }
            Input::Decide => {
                if self.detail == DetailRow::Start {
                    r.se = Some(Se::Decide);
                    r.done = Some(Done::Start(self.run()));
                } else {
                    // 値は SD・FT で変える。決定は「始める」へ
                    self.detail = DetailRow::Start;
                    r.se = Some(Se::Move);
                }
            }
            Input::Cancel => {
                self.focus = Focus::Drills;
                r.se = Some(Se::Cancel);
            }
            Input::Way => return self.step_way(1),
            Input::Lead => return self.toggle_lead(),
        }
        r
    }

    /// 選んでいるドリルのテンポを `dir`（-1 か 1）だけ動かす
    fn nudge_tempo(&mut self, dir: i32) -> Response {
        let mut r = Response::default();
        let d = self.drill();
        let now = self.bpm_of(d);
        let to = nudge_bpm(now, dir);
        if to != now {
            self.bpm.insert(d.id, to);
            r.se = Some(Se::Change);
            r.changed = true;
        }
        r
    }

    /// やり方を次（`dir` が 1）か前（-1）へ。選んでいるドリルで使えないものは飛ばす
    fn step_way(&mut self, dir: i32) -> Response {
        let d = self.drill();
        let way = self.way();
        self.way = if dir < 0 {
            way.prev_for(d)
        } else {
            way.next_for(d)
        };
        Response {
            se: Some(Se::Change),
            done: None,
            changed: true,
        }
    }

    /// 手順の右から・左からを入れ替える
    fn toggle_lead(&mut self) -> Response {
        self.lefty = !self.lefty;
        Response {
            se: Some(Se::Change),
            done: None,
            changed: true,
        }
    }

    /// 真ん中の列（今日のメニュー）
    fn input_today(&mut self, input: Input) -> Response {
        let n = self.today.plan.steps.len();
        let mut r = Response::default();
        match input {
            Input::Up | Input::Down => {
                let at = self.today.cursor;
                let to = if input == Input::Up {
                    at.checked_sub(1)
                } else {
                    (at + 1 < n).then_some(at + 1)
                };
                if let Some(to) = to {
                    self.today.cursor = to;
                    r.se = Some(Se::Move);
                }
            }
            Input::Left | Input::Right => {
                // 途中まで進めたメニューの長さは変えない（終えた分が消えるので）
                if self.today_locked() {
                    return r;
                }
                let dir = if input == Input::Left { -1 } else { 1 };
                let i = LENGTHS
                    .iter()
                    .position(|&m| m == self.today.minutes)
                    .unwrap_or(1);
                let j = (i as i32 + dir).clamp(0, LENGTHS.len() as i32 - 1) as usize;
                if j != i {
                    self.today.minutes = LENGTHS[j];
                    // 終えたメニューの長さを変えたら、新しいメニューにする
                    self.today.progress = None;
                    self.today.cursor = 0;
                    r.se = Some(Se::Change);
                    r.changed = true;
                }
            }
            Input::Decide => {
                if n == 0 {
                    return r;
                }
                // 終えたメニューは、もう一度はじめから数える
                let progress = match &self.today.progress {
                    Some(p) if !p.complete() => p.clone(),
                    _ => Progress::new(self.today.plan.clone()),
                };
                r.se = Some(Se::Decide);
                r.done = Some(Done::Today(Box::new(progress), self.today.cursor));
            }
            Input::Cancel => {
                self.focus = Focus::Left;
                self.left = 0;
                r.se = Some(Se::Cancel);
            }
            Input::Way | Input::Lead => {}
        }
        r
    }

    /// 真ん中の列で選んでいる行が見えるように送る
    fn follow(&mut self) {
        let n = drills_of(self.category()).len();
        let at = self.picked[self.category].min(n.saturating_sub(1));
        let rows = visible_rows();
        if at < self.top + 1 {
            self.top = at.saturating_sub(1);
        } else if at + 2 > self.top + rows {
            self.top = (at + 2).saturating_sub(rows);
        }
        self.top = self.top.min(n.saturating_sub(rows));
    }
}

/// 真ん中の列に見える行の数
fn visible_rows() -> usize {
    ((LIST.3 - LIST_TOP - 12.0) / ROW_H) as usize
}

// ---- 描く ----

fn plain(px: f32, color: Color32) -> Style {
    ui::plain(px, color)
}

fn bold(px: f32, color: Color32) -> Style {
    ui::bold(px, color)
}

const ACCENT: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
/// 音符の四角の中の、叩く手の文字（どのレーンの色の上でも読める暗い色）
const NOTE_LETTER: Color32 = Color32::from_rgb(16, 16, 24);
const BEST: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);
const CATEGORY: Color32 = Color32::from_rgb(0xB4, 0x8C, 0xFF);

impl DrillMenu {
    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        match g.skin.tex("Graphics/4_background.png") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(20, 24, 40)),
        }
        c.cover(Color32::from_rgba_premultiplied(5, 3, 12, 170));
        ui::text(
            c,
            g,
            t("Drills", "基礎練習"),
            44.0,
            16.0,
            &bold(26.0, ui::TEXT),
            400.0,
        );
        self.draw_categories(c, g);
        self.draw_list(c, g);
        self.draw_detail(c, g);
        ui::legend(c, g, &self.hints());
        // 入ってきたときのフェード
        let a = 1.0 - (self.entered.elapsed().as_secs_f32() / 0.25).min(1.0);
        if a > 0.0 {
            c.cover(Color32::from_black_alpha((a * 255.0) as u8));
        }
    }

    fn hints(&self) -> Vec<ui::Hint<'static>> {
        let mut v: Vec<ui::Hint<'static>> =
            vec![(&[Pad::HT], t("Up", "上")), (&[Pad::LT], t("Down", "下"))];
        match self.selected() {
            Item::Today if self.focus == Focus::Today => {
                if !self.today_locked() {
                    v.push((&[Pad::SD, Pad::FT], t("Length", "長さ")));
                }
                v.push((&[Pad::CY], t("Start (Enter)", "始める（Enter）")));
                v.push((&[Pad::LC], t("Back (Esc)", "戻る（Esc）")));
                return v;
            }
            Item::Today => {
                v.push((&[Pad::CY], t("Open (Enter)", "開く（Enter）")));
            }
            Item::Drill(_) if self.focus == Focus::Detail => {
                let change = match self.detail {
                    DetailRow::Mode => Some(t("Mode", "やり方")),
                    DetailRow::Lead => Some(t("Lead Hand", "手順")),
                    DetailRow::Tempo => Some(t("Tempo", "テンポ")),
                    DetailRow::Start => None,
                };
                if let Some(change) = change {
                    v.push((&[Pad::SD, Pad::FT], change));
                    v.push((&[Pad::CY], t("OK (Enter)", "決める（Enter）")));
                } else {
                    v.push((&[Pad::CY], t("Start (Enter)", "始める（Enter）")));
                }
                v.push((&[Pad::LC], t("Back (Esc)", "戻る（Esc）")));
                return v;
            }
            Item::Drill(_) => {
                v.push((&[Pad::SD, Pad::FT], t("Tempo", "テンポ")));
                v.push((&[Pad::CY], t("Select (Enter)", "選ぶ（Enter）")));
                v.push((&[Pad::LC], t("Categories (Esc)", "カテゴリへ（Esc）")));
                return v;
            }
            Item::Category(_) => {
                v.push((&[Pad::CY], t("Open (Enter)", "開く（Enter）")));
            }
            Item::Rise => {
                v.push((&[Pad::SD, Pad::FT], t("Change", "変更")));
            }
            Item::Back => {
                v.push((
                    &[Pad::CY],
                    t("Back to Title (Enter)", "タイトルへ（Enter）"),
                ));
            }
        }
        v.push((&[Pad::LC], t("Back to Title (Esc)", "タイトルへ（Esc）")));
        v
    }

    /// 札の行（やり方・手順）。札は（名前、選んでいるか、使えるか）。入りきらなければ字を小さく。
    /// `active` は右の欄でこの行を選んでいるとき（SD・FT で変わる）
    fn chip_row(
        c: &Canvas,
        g: &mut Gfx,
        label: &str,
        chips: &[(String, bool, bool)],
        y: f32,
        active: bool,
    ) {
        let (x, _, w, _) = DETAIL;
        ui::text(c, g, label, x + 20.0, y + 4.0, &plain(15.0, ui::DIM), 90.0);
        let (from, room) = (x + 104.0, w - 124.0);
        let gap = 6.0;
        let mut px = 14.0;
        let widths = |g: &mut Gfx, px: f32| -> Vec<f32> {
            chips
                .iter()
                .map(|(t, ..)| ui::text_width(g, t, &bold(px, ui::TEXT)) + px * 1.4)
                .collect()
        };
        let mut ws = widths(g, px);
        while px > 11.0 && ws.iter().sum::<f32>() + gap * (chips.len() as f32 - 1.0) > room {
            px -= 1.0;
            ws = widths(g, px);
        }
        let mut cx = from;
        for ((text, on, fits), cw) in chips.iter().zip(ws) {
            let h = 26.0;
            if *on {
                let fill = if active {
                    ACCENT
                } else {
                    ACCENT.gamma_multiply(0.6)
                };
                ui::round_fill(c, cx, y, cw, h, h / 2.0, fill);
                ui::text_center(
                    c,
                    g,
                    text,
                    cx + cw / 2.0,
                    y + (h - px) / 2.0 - 2.0,
                    &bold(px, Color32::from_rgb(40, 24, 0)),
                    cw,
                );
            } else {
                let ink = if *fits {
                    ui::DIM
                } else {
                    ui::MUTED.gamma_multiply(0.5)
                };
                ui::round_frame(c, cx, y, cw, h, h / 2.0, 1.0, ink.gamma_multiply(0.7));
                ui::text_center(
                    c,
                    g,
                    text,
                    cx + cw / 2.0,
                    y + (h - px) / 2.0 - 2.0,
                    &plain(px, ink),
                    cw,
                );
            }
            cx += cw + gap;
        }
    }

    /// やり方の絵の下のひとこと（段の数・長さ・終わり方）
    fn way_line(&self, run: &Run) -> String {
        let secs = run_seconds(run).round() as u32;
        let steps = run.steps();
        let bars = run.step_measures;
        match run.way {
            Way::Climb if run.drill.fixed_tempo() || self.rise == 0 => tf!(
                "{steps} steps × {bars} bars at one tempo · about {secs} s",
                "{steps} 段 × {bars} 小節を同じテンポで・約 {secs} 秒"
            ),
            Way::Climb => tf!(
                "{bars} bars per step · {steps} steps · about {secs} s",
                "1 段 {bars} 小節・{steps} 段で約 {secs} 秒"
            ),
            Way::Limit => tf!(
                "+{LIMIT_RISE} every {LIMIT_STEP_MEASURES} bars · ends at {MANY_MISSES} misses in a step",
                "{LIMIT_STEP_MEASURES} 小節ごとに +{LIMIT_RISE}・同じ段で {MANY_MISSES} つ落としたら終わり"
            ),
            Way::Endurance(_) => tf!(
                "One tempo for {} · checked every 30 s",
                "テンポを変えずに {}・30 秒ごとに見る",
                clock(f64::from(secs) * 1000.0)
            ),
            Way::Blind => tf!(
                "Notes fade each step, then vanish · about {secs} s",
                "段ごとに薄くなり、最後は見えない・約 {secs} 秒"
            ),
            Way::Echo => tf!(
                "Hear a bar (no notes), then play it · about {secs} s",
                "お手本 1 小節（音だけ）→ 自分 1 小節・約 {secs} 秒"
            ),
        }
    }

    /// 選んでいる行の帯。`strong` は操作している列（枠を太く明るく）
    fn row_mark(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32), strong: bool) {
        ui::round_fill(c, x, y, w, h, 6.0, ACCENT.gamma_multiply(0.18));
        if strong {
            ui::round_frame(c, x, y, w, h, 6.0, 2.0, ACCENT);
        } else {
            ui::round_frame(c, x, y, w, h, 6.0, 1.0, ACCENT.gamma_multiply(0.5));
            c.fill(x, y + 6.0, 4.0, h - 12.0, ACCENT);
        }
    }

    /// 左の列（分け方と、上げ幅・タイトルへ）
    fn draw_categories(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = CATS;
        ui::panel(c, x, y, w, h);
        let left = self.focus == Focus::Left;
        let items = left_items();
        self.draw_today_row(c, g, (x, y + 12.0, w));
        c.fill(
            x + 16.0,
            y + 12.0 + CAT_H + 5.0,
            w - 32.0,
            1.0,
            Color32::from_white_alpha(50),
        );
        for (k, cat) in Category::ALL.into_iter().enumerate() {
            let ry = y + 12.0 + CAT_H + TODAY_GAP + k as f32 * CAT_H;
            let open = !self.showing_today && k == self.category;
            let on = left && self.left == left_of(k);
            if open {
                Self::row_mark(c, (x + 8.0, ry, w - 16.0, CAT_H - 6.0), on);
            }
            let style = if open {
                bold(17.0, ui::TEXT)
            } else {
                plain(17.0, ui::DIM)
            };
            ui::text(
                c,
                g,
                category_name(cat),
                x + 22.0,
                ry + 9.0,
                &style,
                w - 70.0,
            );
            ui::text_right(
                c,
                g,
                &drills_of(cat).len().to_string(),
                x + w - 20.0,
                ry + 12.0,
                &plain(14.0, if open { ACCENT } else { ui::MUTED }),
                40.0,
            );
        }
        // 上げ幅とタイトルへ
        let fy = y + h - FOOT_H;
        c.fill(x + 16.0, fy, w - 32.0, 1.0, Color32::from_white_alpha(50));
        for (k, item) in [Item::Rise, Item::Back].into_iter().enumerate() {
            let ry = fy + 10.0 + k as f32 * FOOT_ROW_H;
            let on = left && items[self.left] == item;
            if on {
                Self::row_mark(c, (x + 8.0, ry, w - 16.0, FOOT_ROW_H - 4.0), true);
            }
            let style = if on {
                bold(15.0, ui::TEXT)
            } else {
                plain(15.0, ui::DIM)
            };
            let label = match item {
                Item::Rise => t("Rise per Step", "上げ幅（1 段ごと）"),
                _ => t("Back to Title", "タイトルに戻る"),
            };
            ui::text(c, g, label, x + 22.0, ry + 6.0, &style, w - 90.0);
            if item == Item::Rise {
                let v = if self.rise == 0 {
                    t("Steady", "上げない").to_owned()
                } else {
                    format!("+{}", self.rise)
                };
                if on {
                    ui::triangle(
                        c,
                        x + w - 82.0,
                        ry + FOOT_ROW_H / 2.0 - 2.0,
                        6.0,
                        -1,
                        ACCENT,
                    );
                    ui::triangle(c, x + w - 18.0, ry + FOOT_ROW_H / 2.0 - 2.0, 6.0, 1, ACCENT);
                }
                ui::text_center(
                    c,
                    g,
                    &v,
                    x + w - 50.0,
                    ry + 6.0,
                    &bold(15.0, ui::TEXT),
                    56.0,
                );
            }
        }
    }

    /// 真ん中の列（分け方のドリル）
    fn draw_list(&self, c: &Canvas, g: &mut Gfx) {
        if self.showing_today {
            self.draw_today_list(c, g);
            return;
        }
        let (x, y, w, h) = LIST;
        ui::panel(c, x, y, w, h);
        let cat = self.category();
        ui::text(
            c,
            g,
            category_name(cat),
            x + 18.0,
            y + 12.0,
            &bold(15.0, CATEGORY),
            w - 36.0,
        );
        let list = drills_of(cat);
        let rows = visible_rows();
        let at = self.picked[self.category].min(list.len() - 1);
        let mut ry = y + LIST_TOP;
        for (k, &i) in list.iter().enumerate().skip(self.top).take(rows) {
            let d = &DRILLS[i];
            let on = k == at;
            if on {
                Self::row_mark(
                    c,
                    (x + 8.0, ry, w - 16.0, ROW_H - 6.0),
                    self.focus == Focus::Drills,
                );
            }
            let style = if on {
                bold(16.0, ui::TEXT)
            } else {
                plain(16.0, ui::DIM)
            };
            ui::text(c, g, drill::name(d), x + 20.0, ry + 8.0, &style, w - 112.0);
            match self.records.get(&(d.id, self.lefty)).and_then(|r| r.best) {
                Some(b) => best_pill(
                    c,
                    g,
                    &format!("✓ {b}"),
                    x + w - 18.0,
                    ry + (ROW_H - 6.0) / 2.0,
                    13.0,
                ),
                None => {
                    ui::text_right(
                        c,
                        g,
                        "—",
                        x + w - 24.0,
                        ry + 9.0,
                        &plain(15.0, ui::MUTED),
                        40.0,
                    );
                }
            }
            ry += ROW_H;
        }
        if self.top + rows < list.len() {
            ui::text_center(
                c,
                g,
                t("▼ More", "▼ まだあります"),
                x + w / 2.0,
                y + h - 26.0,
                &plain(13.0, ui::MUTED),
                w,
            );
        }
        if self.top > 0 {
            ui::text_center(
                c,
                g,
                "▲",
                x + w - 30.0,
                y + 12.0,
                &plain(13.0, ui::MUTED),
                30.0,
            );
        }
    }

    fn draw_detail(&self, c: &Canvas, g: &mut Gfx) {
        if self.showing_today {
            self.draw_today_detail(c, g);
            return;
        }
        let (x, y, w, _) = DETAIL;
        ui::panel(c, x, y, w, DETAIL.3);
        let d = self.drill();
        let run = self.run();
        // 分け方の札と名前
        let cat = category_name(d.category);
        let cw = ui::text_width(g, cat, &bold(13.0, Color32::BLACK)) + 22.0;
        ui::round_fill(c, x + 20.0, y + 20.0, cw, 26.0, 13.0, CATEGORY);
        ui::text(
            c,
            g,
            cat,
            x + 31.0,
            y + 24.0,
            &bold(13.0, Color32::BLACK),
            cw,
        );
        ui::text(
            c,
            g,
            drill::name(d),
            x + 32.0 + cw,
            y + 15.0,
            &bold(24.0, ui::TEXT),
            w - cw - 52.0,
        );
        // 手順の絵と、手の見方・型の説明
        draw_pattern(c, g, d, self.lefty, (x + 20.0, y + 56.0, w - 40.0, 136.0));
        let key = if d.has_accents() {
            t(
                "R = right hand   L = left hand   > = accent",
                "R = 右手　L = 左手　> = アクセント",
            )
        } else {
            t("R = right hand   L = left hand", "R = 右手　L = 左手")
        };
        let key_w = ui::text(
            c,
            g,
            key,
            x + 20.0,
            y + 198.0,
            &plain(13.0, ui::MUTED),
            300.0,
        );
        ui::text_right(
            c,
            g,
            &pattern_line(d),
            x + w - 20.0,
            y + 198.0,
            &plain(13.0, ui::MUTED),
            w - 60.0 - key_w,
        );
        let rule = |c: &Canvas, ry: f32| {
            c.fill(x + 20.0, ry, w - 40.0, 1.0, Color32::from_white_alpha(40));
        };
        rule(c, y + 222.0);
        // やり方と手順の行
        let way = self.way();
        let ways: Vec<(String, bool, bool)> = WAY_CHIPS
            .into_iter()
            .map(|k| {
                let on = same_chip(k, way);
                let label = match k {
                    _ if on => way_name(way),
                    Way::Endurance(_) => t("Endurance", "持久").to_owned(),
                    _ => way_name(k),
                };
                (label, on, k.fits(d))
            })
            .collect();
        // 右の欄を操作しているときは、選んでいる行に帯を出す
        let row = (self.focus == Focus::Detail).then_some(self.detail);
        match row {
            Some(DetailRow::Mode) => Self::row_mark(c, (x + 10.0, y + 228.0, w - 20.0, 34.0), true),
            Some(DetailRow::Lead) => Self::row_mark(c, (x + 10.0, y + 262.0, w - 20.0, 34.0), true),
            Some(DetailRow::Tempo) => Self::row_mark(c, (x + 10.0, y + 308.0, 250.0, 148.0), true),
            _ => {}
        }
        Self::chip_row(
            c,
            g,
            t("Mode", "やり方"),
            &ways,
            y + 232.0,
            row == Some(DetailRow::Mode),
        );
        let leads: Vec<(String, bool, bool)> = [false, true]
            .into_iter()
            .map(|l| (lead_name(l).to_owned(), l == self.lefty, true))
            .collect();
        Self::chip_row(
            c,
            g,
            t("Lead Hand", "手順"),
            &leads,
            y + 266.0,
            row == Some(DetailRow::Lead),
        );
        rule(c, y + 304.0);
        // テンポ（SD・FT で変わるのは、ドリルの列か、右の欄のテンポの行）
        let tempo_on = self.focus == Focus::Drills || row == Some(DetailRow::Tempo);
        let label = if way == Way::Limit {
            t("Start Tempo", "始めのテンポ")
        } else {
            t("Tempo", "テンポ")
        };
        ui::text(
            c,
            g,
            label,
            x + 20.0,
            y + 316.0,
            &plain(15.0, ui::DIM),
            200.0,
        );
        let arrow = if tempo_on { ACCENT } else { ui::MUTED };
        ui::triangle(c, x + 30.0, y + 378.0, 10.0, -1, arrow);
        ui::triangle(c, x + 222.0, y + 378.0, 10.0, 1, arrow);
        ui::text_center(
            c,
            g,
            &run.target().to_string(),
            x + 126.0,
            y + 342.0,
            &bold(58.0, ui::TEXT),
            170.0,
        );
        ui::text_center(
            c,
            g,
            "BPM",
            x + 126.0,
            y + 410.0,
            &plain(13.0, ui::MUTED),
            100.0,
        );
        ui::text(
            c,
            g,
            &tf!("SD ◀ · FT ▶ by {BPM_STEP}", "SD ◀・FT ▶ で {BPM_STEP} ずつ"),
            x + 20.0,
            y + 432.0,
            &plain(13.0, ui::MUTED),
            230.0,
        );
        // やり方の絵とひとこと
        let (sx, sw) = (x + 270.0, w - 290.0);
        draw_way(c, g, &run, (sx, y + 314.0, sw, 116.0));
        ui::text_center(
            c,
            g,
            &self.way_line(&run),
            sx + sw / 2.0,
            y + 436.0,
            &plain(14.0, ui::DIM),
            sw + 20.0,
        );
        rule(c, y + 464.0);
        // これまで（手順ごと）
        ui::text(
            c,
            g,
            t("Best", "ベスト"),
            x + 20.0,
            y + 482.0,
            &plain(15.0, ui::DIM),
            120.0,
        );
        let rec = self.records.get(&(d.id, self.lefty));
        match rec.and_then(|r| r.best) {
            Some(b) => best_pill(
                c,
                g,
                &if d.fixed_tempo() {
                    tf!("✓ clean at {b}", "✓ {b} でミスなし")
                } else {
                    tf!("✓ up to {b}", "✓ {b} まで")
                },
                x + 250.0,
                y + 494.0,
                16.0,
            ),
            None => {
                ui::text(
                    c,
                    g,
                    t("No clean step yet", "まだミスなしの段はありません"),
                    x + 120.0,
                    y + 482.0,
                    &plain(15.0, ui::MUTED),
                    300.0,
                );
            }
        }
        if let Some(r) = rec.filter(|r| r.runs > 0) {
            let runs = count(r.runs as usize, "run", "runs", "回");
            let when = r.last_at.map(month_day).unwrap_or_default();
            ui::text(
                c,
                g,
                &format!("{when} · {runs}"),
                x + 266.0,
                y + 482.0,
                &plain(14.0, ui::MUTED),
                150.0,
            );
            draw_trail(c, &r.trail, (x + w - 176.0, y + 476.0, 150.0, 34.0));
        }
        // 始める（ドリルの列で CY を叩くとここへ来て、もう一度 CY で始める）
        start_button(c, g, t("Start", "始める"), row == Some(DetailRow::Start));
    }
}

/// 右のパネルの右下の、CY で始めるボタン。`on` は操作している列のとき
fn start_button(c: &Canvas, g: &mut Gfx, label: &str, on: bool) {
    let (x, y, w, _) = DETAIL;
    {
        let (bx, by, bw, bh) = (x + w - 250.0, y + DETAIL.3 - 72.0, 230.0, 52.0);
        ui::round_fill(
            c,
            bx,
            by,
            bw,
            bh,
            8.0,
            Color32::from_rgba_unmultiplied(40, 30, 10, 230),
        );
        ui::round_frame(
            c,
            bx,
            by,
            bw,
            bh,
            8.0,
            2.0,
            if on { ACCENT } else { ui::MUTED },
        );
        let chip = bold(14.0, Color32::BLACK);
        ui::round_fill(c, bx + 16.0, by + 15.0, 36.0, 22.0, 11.0, Pad::CY.color());
        ui::text_center(c, g, "CY", bx + 34.0, by + 17.0, &chip, 36.0);
        ui::text(
            c,
            g,
            label,
            bx + 70.0,
            by + 12.0,
            &bold(22.0, ui::TEXT),
            bw - 80.0,
        );
    }
}

// ---- 今日のメニュー ----

/// 曲や型の長さ（「1:05」）。約何分かは [`about_minutes`]
fn step_time(secs: f64) -> String {
    clock(secs * 1000.0)
}

/// 全体の長さ（「約 10 分」）
fn about_minutes(secs: f64) -> String {
    let m = (secs / 60.0).round().max(1.0) as u32;
    tf!("About {m} min", "約 {m} 分")
}

/// 日付（「11月27日」）
fn day_text(d: NaiveDate) -> String {
    tf!("{}/{}", "{}月{}日", d.month(), d.day())
}

/// 役目の札（色の地に暗い字）
fn role_pill(c: &Canvas, g: &mut Gfx, role: today::Role, (x, y): (f32, f32), dim: bool) {
    let color = if dim {
        role.color().gamma_multiply(0.4)
    } else {
        role.color()
    };
    ui::round_fill(c, x, y, ROLE_W, 22.0, 11.0, color);
    ui::text_center(
        c,
        g,
        role.label(),
        x + ROLE_W / 2.0,
        y + 3.0,
        &bold(12.0, NOTE_LETTER),
        ROLE_W - 8.0,
    );
}

impl DrillMenu {
    /// 左の列の一番上の行。右に長さか、進み具合か、終えた印
    fn draw_today_row(&self, c: &Canvas, g: &mut Gfx, (x, ry, w): (f32, f32, f32)) {
        let on = self.focus == Focus::Left && self.left == 0;
        if self.showing_today {
            Self::row_mark(c, (x + 8.0, ry, w - 16.0, CAT_H - 6.0), on);
        }
        ui::text(
            c,
            g,
            t("Today's Menu", "今日のメニュー"),
            x + 22.0,
            ry + 9.0,
            &bold(17.0, if self.showing_today { ACCENT } else { ui::TEXT }),
            w - 80.0,
        );
        let st = &self.today;
        let right = match &st.progress {
            _ if st.done_today => "✓".to_owned(),
            Some(p) if p.started() => format!("{}/{}", p.done_count(), p.done.len()),
            _ => tf!("{} min", "{} 分", st.minutes),
        };
        let color = if st.done_today { BEST } else { ui::MUTED };
        ui::text_right(
            c,
            g,
            &right,
            x + w - 20.0,
            ry + 12.0,
            &plain(14.0, color),
            60.0,
        );
    }

    /// 真ん中の列（今日のメニューの並び）
    fn draw_today_list(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = LIST;
        ui::panel(c, x, y, w, h);
        let st = &self.today;
        ui::text(
            c,
            g,
            t("Today's Menu", "今日のメニュー"),
            x + 18.0,
            y + 12.0,
            &bold(15.0, ACCENT),
            w - 150.0,
        );
        ui::text_right(
            c,
            g,
            &about_minutes(st.plan.seconds()),
            x + w - 18.0,
            y + 13.0,
            &plain(14.0, ui::DIM),
            130.0,
        );
        let active = self.focus == Focus::Today;
        let mut ry = y + LIST_TOP;
        for (k, step) in st.plan.steps.iter().enumerate().take(today::MAX_STEPS) {
            let done = self.today_done(k);
            let on = k == st.cursor;
            if on {
                Self::row_mark(c, (x + 8.0, ry, w - 16.0, ROW_H - 6.0), active);
            }
            role_pill(c, g, step.role, (x + 16.0, ry + 7.0), done);
            let style = match (on, done) {
                (true, _) => bold(16.0, ui::TEXT),
                (false, true) => plain(16.0, ui::MUTED),
                (false, false) => plain(16.0, ui::DIM),
            };
            ui::text(
                c,
                g,
                &step.name(),
                x + 26.0 + ROLE_W,
                ry + 8.0,
                &style,
                w - ROLE_W - 100.0,
            );
            if done {
                best_pill(c, g, "✓", x + w - 18.0, ry + (ROW_H - 6.0) / 2.0, 13.0);
            } else {
                ui::text_right(
                    c,
                    g,
                    &step_time(step.seconds()),
                    x + w - 20.0,
                    ry + 10.0,
                    &plain(14.0, ui::MUTED),
                    60.0,
                );
            }
            ry += ROW_H;
        }
        // 下に進み具合
        let foot = match &st.progress {
            _ if st.done_today => (t("Done for today ✓", "今日の分は完了 ✓").to_owned(), BEST),
            Some(p) if p.started() => (
                tf!(
                    "{} of {} done",
                    "{1} 個中 {0} 個 完了",
                    p.done_count(),
                    p.done.len()
                ),
                ui::DIM,
            ),
            _ => (
                t(
                    "A new mix every day, based on your records",
                    "記録をもとに、毎日中身が変わります",
                )
                .to_owned(),
                ui::MUTED,
            ),
        };
        ui::text_center(
            c,
            g,
            &foot.0,
            x + w / 2.0,
            y + h - 30.0,
            &plain(13.0, foot.1),
            w - 24.0,
        );
    }

    /// 右のパネル（今日のメニュー）。上に長さと全体の帯、下に選んだものの中身
    fn draw_today_detail(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = DETAIL;
        ui::panel(c, x, y, w, h);
        let st = &self.today;
        ui::text(
            c,
            g,
            t("Today's Menu", "今日のメニュー"),
            x + 20.0,
            y + 15.0,
            &bold(24.0, ui::TEXT),
            w - 200.0,
        );
        ui::text_right(
            c,
            g,
            &day_text(st.day),
            x + w - 20.0,
            y + 22.0,
            &plain(15.0, ui::DIM),
            160.0,
        );
        // 長さ（途中まで進めたら変えられない）
        let locked = self.today_locked();
        let chips: Vec<(String, bool, bool)> = LENGTHS
            .iter()
            .map(|&m| (tf!("{m} min", "{m} 分"), m == st.minutes, !locked))
            .collect();
        let active = self.focus == Focus::Today;
        Self::chip_row(c, g, t("Length", "長さ"), &chips, y + 60.0, active);
        // 全体の帯
        self.draw_strip(c, g, (x + 20.0, y + 104.0, w - 40.0, 40.0));
        let left_secs: f64 = st
            .plan
            .steps
            .iter()
            .enumerate()
            .filter(|(k, _)| !self.today_done(*k))
            .map(|(_, s)| s.seconds() + 10.0)
            .sum();
        let status = match &st.progress {
            Some(p) if p.started() && !p.complete() => tf!(
                "{} of {} done · {} left",
                "{1} 個中 {0} 個 完了・残り {2}",
                p.done_count(),
                p.done.len(),
                about_minutes(left_secs)
            ),
            _ => tf!(
                "{} steps · {}",
                "{} 個・{}",
                st.plan.steps.len(),
                about_minutes(st.plan.seconds())
            ),
        };
        ui::text(
            c,
            g,
            &status,
            x + 20.0,
            y + 152.0,
            &plain(14.0, ui::DIM),
            w - 40.0,
        );
        c.fill(
            x + 20.0,
            y + 182.0,
            w - 40.0,
            1.0,
            Color32::from_white_alpha(40),
        );
        // 選んでいるもの
        let Some(step) = st.plan.steps.get(st.cursor) else {
            return;
        };
        let done = self.today_done(st.cursor);
        role_pill(c, g, step.role, (x + 20.0, y + 198.0), false);
        ui::text(
            c,
            g,
            &step.name(),
            x + 30.0 + ROLE_W,
            y + 192.0,
            &bold(22.0, ui::TEXT),
            w - ROLE_W - 60.0,
        );
        ui::text(
            c,
            g,
            step.role.reason(),
            x + 20.0,
            y + 230.0,
            &plain(14.0, ui::DIM),
            w - 40.0,
        );
        match &step.task {
            Task::Drill(run) => {
                draw_pattern(
                    c,
                    g,
                    run.drill,
                    run.lefty,
                    (x + 20.0, y + 258.0, w - 40.0, 118.0),
                );
                let tempo = match run.way {
                    Way::Climb | Way::Limit if run.bpms.len() > 1 => tf!(
                        "{} → {} BPM",
                        "{} → {} BPM",
                        run.bpms[0],
                        run.bpms[run.bpms.len().min(4) - 1]
                    ),
                    _ => format!("{} BPM", run.bpms[0]),
                };
                let tempo = if run.way == Way::Limit {
                    tf!("From {} BPM", "{} BPM から", run.bpms[0])
                } else {
                    tempo
                };
                let rows = [
                    (t("Mode", "やり方"), way_name(run.way)),
                    (t("Lead Hand", "手順"), lead_name(run.lefty).to_owned()),
                    (t("Tempo", "テンポ"), tempo),
                ];
                for (i, (k, v)) in rows.iter().enumerate() {
                    let ry = y + 392.0 + i as f32 * 30.0;
                    ui::text(c, g, k, x + 20.0, ry, &plain(15.0, ui::DIM), 90.0);
                    ui::text(c, g, v, x + 110.0, ry - 1.0, &bold(16.0, ui::TEXT), 170.0);
                }
                let (sx, sw) = (x + 300.0, w - 320.0);
                draw_way(c, g, run, (sx, y + 388.0, sw, 86.0));
                ui::text_center(
                    c,
                    g,
                    &self.way_line(run),
                    sx + sw / 2.0,
                    y + 480.0,
                    &plain(13.0, ui::DIM),
                    sw + 20.0,
                );
            }
            Task::Song { pick, seconds } => {
                if !pick.artist.is_empty() {
                    ui::text(
                        c,
                        g,
                        &pick.artist,
                        x + 20.0,
                        y + 256.0,
                        &plain(15.0, ui::MUTED),
                        w - 40.0,
                    );
                }
                let band = Band {
                    x: x + 20.0,
                    y: y + 290.0,
                    w: w - 40.0,
                    cell_h: 30.0,
                    row_pitch: 50.0,
                    max_rows: 2,
                    striped: false,
                };
                let bottom = band.draw(
                    c,
                    g,
                    &BandData {
                        weakness: &pick.weakness,
                        phrases: vec![pick.section],
                        selected: Some(pick.section),
                        cleared: Vec::new(),
                    },
                );
                let rows = [
                    (
                        t("Section", "区間"),
                        crate::practice::section_label(pick.section),
                    ),
                    (
                        t("Time", "時間"),
                        tf!(
                            "Up to {} (ends after the step in play)",
                            "最大 {}（時間が来たら、その段の終わりで終了）",
                            clock(f64::from(*seconds) * 1000.0)
                        ),
                    ),
                ];
                for (i, (k, v)) in rows.iter().enumerate() {
                    let ry = bottom + 20.0 + i as f32 * 30.0;
                    ui::text(c, g, k, x + 20.0, ry, &plain(15.0, ui::DIM), 90.0);
                    ui::text(
                        c,
                        g,
                        v,
                        x + 110.0,
                        ry - 1.0,
                        &bold(16.0, ui::TEXT),
                        w - 130.0,
                    );
                }
            }
        }
        // 途中まで進めたメニューは、次のものなら「続ける」
        let next = st
            .progress
            .as_ref()
            .filter(|p| p.started() && !p.complete())
            .map(Progress::first_open);
        let label = if done {
            t("Play Again", "もう一度")
        } else if next == Some(st.cursor) {
            t("Continue", "続ける")
        } else if next.is_none() && st.cursor == 0 {
            t("Start", "始める")
        } else {
            t("Start Here", "ここから始める")
        };
        start_button(c, g, label, active);
    }

    /// 全体の帯（長さに比べた幅。終えたものは暗く ✓、選んでいるものに白い枠）
    fn draw_strip(&self, c: &Canvas, g: &mut Gfx, (x, y, w, h): (f32, f32, f32, f32)) {
        let steps = &self.today.plan.steps;
        if steps.is_empty() {
            return;
        }
        let gap = 4.0;
        let total: f64 = steps.iter().map(today::Step::seconds).sum();
        let room = w - gap * (steps.len() as f32 - 1.0);
        let mut cx = x;
        for (k, step) in steps.iter().enumerate() {
            let bw = (room * (step.seconds() / total.max(1.0)) as f32).max(8.0);
            let done = self.today_done(k);
            let color = step.role.color();
            let fill = if done {
                color.gamma_multiply(0.3)
            } else {
                color.gamma_multiply(0.85)
            };
            ui::round_fill(c, cx, y, bw, h, 4.0, fill);
            if k == self.today.cursor {
                ui::round_frame(
                    c,
                    cx - 2.0,
                    y - 2.0,
                    bw + 4.0,
                    h + 4.0,
                    5.0,
                    2.0,
                    Color32::WHITE,
                );
            }
            let label = if done { "✓" } else { step.role.label() };
            let style = bold(12.0, if done { BEST } else { NOTE_LETTER });
            if ui::text_width(g, label, &style) + 8.0 < bw {
                ui::text_center(c, g, label, cx + bw / 2.0, y + h / 2.0 - 8.0, &style, bw);
            }
            cx += bw + gap;
        }
    }
}

/// やり方の札（持久は長さをまとめて 1 つ。HH で長さも順に回る）
const WAY_CHIPS: [Way; 5] = [
    Way::Climb,
    Way::Limit,
    Way::Endurance(0),
    Way::Blind,
    Way::Echo,
];

/// 同じ札か（持久は長さを問わない）
fn same_chip(a: Way, b: Way) -> bool {
    matches!((a, b), (Way::Endurance(_), Way::Endurance(_))) || a == b
}

/// やり方の絵（テンポとテンポの段・消え方・交代・区切り）
fn draw_way(c: &Canvas, g: &mut Gfx, run: &Run, (x, y, w, h): (f32, f32, f32, f32)) {
    let none = vec![None; run.steps()];
    match run.way {
        Way::Climb | Way::Blind => {
            let fades: &[f32] = if run.way == Way::Blind {
                &BLIND_ALPHA
            } else {
                &[]
            };
            crate::drill_hud::stairs(
                c,
                g,
                &Stairs {
                    bpms: &run.bpms,
                    current: None,
                    grades: &none,
                    marks: false,
                    fades,
                    flag: run.way == Way::Climb,
                },
                (x, y + 4.0, w, h - 4.0),
            );
        }
        Way::Limit => {
            // 始めの 8 段と、その先も続くこと
            let n = run.steps().min(8);
            crate::drill_hud::stairs(
                c,
                g,
                &Stairs {
                    bpms: &run.bpms[..n],
                    current: None,
                    grades: &none[..n],
                    marks: false,
                    fades: &[],
                    flag: false,
                },
                (x, y + 4.0, w - 30.0, h - 4.0),
            );
            if run.steps() > n {
                ui::text_center(c, g, "…", x + w - 14.0, y + 8.0, &bold(18.0, ui::DIM), 30.0);
            }
        }
        Way::Echo => {
            // 1 段の小節の並び（お手本と自分の交代）
            let n = run.step_measures as usize;
            let gap = 8.0;
            let bw = (w - gap * (n as f32 - 1.0)) / n as f32;
            for i in 0..n {
                let bx = x + i as f32 * (bw + gap);
                let demo = i % 2 == 0;
                let (color, label) = if demo {
                    (DEMO_COLOR, t("Listen", "聞く"))
                } else {
                    (ACCENT, t("Play", "叩く"))
                };
                let (by, bh) = (y + 34.0, h - 40.0);
                if demo {
                    ui::round_frame(c, bx, by, bw, bh, 6.0, 2.0, color);
                    // 音の波
                    for k in 0..3 {
                        let r = 6.0 + k as f32 * 6.0;
                        c.painter().circle_stroke(
                            c.pos(bx + bw / 2.0 - 10.0, by + bh / 2.0),
                            r * c.scale(),
                            Stroke::new(
                                1.5 * c.scale(),
                                color.gamma_multiply(1.0 - k as f32 * 0.25),
                            ),
                        );
                    }
                } else {
                    ui::round_fill(c, bx, by, bw, bh, 6.0, color.gamma_multiply(0.85));
                }
                ui::text_center(
                    c,
                    g,
                    label,
                    bx + bw / 2.0,
                    y + 8.0,
                    &bold(14.0, color),
                    bw + gap,
                );
            }
        }
        Way::Endurance(_) => {
            // 30 秒ほどの区切りの並びと、区切りの終わりの時刻
            let n = run.steps();
            let gap = 6.0;
            let pw = (w - gap * (n as f32 - 1.0)) / n as f32;
            let bar_ms = run.drill.bar_len() * 4.0 * 60_000.0 / f64::from(run.target().max(1));
            for i in 0..n {
                let px = x + i as f32 * (pw + gap);
                ui::round_fill(
                    c,
                    px,
                    y + 40.0,
                    pw,
                    h - 64.0,
                    6.0,
                    ACCENT.gamma_multiply(0.25 + 0.6 * (i + 1) as f32 / n as f32),
                );
                let at = f64::from((i as u32 + 1) * run.step_measures) * bar_ms;
                ui::text_center(
                    c,
                    g,
                    &crate::drill_hud::up_to(at),
                    px + pw / 2.0,
                    y + h - 20.0,
                    &plain(13.0, ui::DIM),
                    pw + gap,
                );
            }
            ui::text_center(
                c,
                g,
                &tf!("{} BPM all the way", "ずっと {} で", run.target()),
                x + w / 2.0,
                y + 8.0,
                &bold(15.0, ui::TEXT),
                w,
            );
        }
    }
}

/// 最高の札（緑の丸い札。右の端 `right`、縦の真ん中 `cy`）
fn best_pill(c: &Canvas, g: &mut Gfx, text: &str, right: f32, cy: f32, px: f32) {
    let style = bold(px, Color32::from_rgb(10, 40, 20));
    let w = ui::text_width(g, text, &style) + 18.0;
    let h = px + 10.0;
    ui::round_fill(c, right - w, cy - h / 2.0, w, h, h / 2.0, BEST);
    ui::text(c, g, text, right - w + 9.0, cy - px * 0.62, &style, w);
}

/// 月/日（UNIX 秒から。ローカルの時刻）
fn month_day(at: i64) -> String {
    use chrono::{Datelike, TimeZone};
    match chrono::Local.timestamp_opt(at, 0).single() {
        Some(d) => format!("{}/{}", d.month(), d.day()),
        None => String::new(),
    }
}

/// 最近の回の最高のテンポの線
fn draw_trail(c: &Canvas, trail: &[u32], (x, y, w, h): (f32, f32, f32, f32)) {
    if trail.len() < 2 {
        return;
    }
    let lo = *trail.iter().min().unwrap_or(&0) as f32;
    let hi = *trail.iter().max().unwrap_or(&0) as f32;
    let span = (hi - lo).max(10.0);
    let step = w / (trail.len() - 1) as f32;
    let pts: Vec<egui::Pos2> = trail
        .iter()
        .enumerate()
        .map(|(i, &b)| c.pos(x + i as f32 * step, y + h - (b as f32 - lo) / span * h))
        .collect();
    let p = c.painter();
    p.add(egui::Shape::line(
        pts.clone(),
        Stroke::new(1.5 * c.scale(), BEST),
    ));
    for q in pts {
        p.circle_filled(q, 3.0 * c.scale(), BEST);
    }
}

/// 手順の絵。型の小節を横に並べ、レーンごとの行に音符の四角（演奏の画面と同じレーンの色）。
/// 叩く手を下（1 つのレーン）か四角の中に
/// 手順の絵。`lefty` なら R と L を入れ替えて描く
fn draw_pattern(
    c: &Canvas,
    g: &mut Gfx,
    d: &Drill,
    lefty: bool,
    (x, y, w, h): (f32, f32, f32, f32),
) {
    ui::round_fill(
        c,
        x,
        y,
        w,
        h,
        6.0,
        Color32::from_rgba_unmultiplied(6, 6, 12, 200),
    );
    ui::round_frame(c, x, y, w, h, 6.0, 1.0, Color32::from_white_alpha(50));
    let mut notes = d.notes();
    if lefty {
        for n in &mut notes {
            n.hand = n.hand.mirrored();
        }
    }
    let lanes = d.lanes();
    let measures = d.measures();
    let label_w = 44.0;
    let (gx, gw) = (x + label_w, w - label_w - 12.0);
    let one_lane = lanes.len() == 1;
    let top = y + 22.0;
    let rows_h = if one_lane { h - 56.0 } else { h - 32.0 };
    let row_h = rows_h / lanes.len() as f32;
    let mw = gw / measures as f32;
    // 小節の区切りと拍の番号（拍子どおり。7/8 などはクリックのまとまりごと）
    let beats = d.count_in_clicks();
    for m in 0..measures {
        let mx = gx + m as f32 * mw;
        for (b, &(pos, _)) in beats.iter().enumerate() {
            let bx = mx + pos as f32 * mw;
            ui::text(
                c,
                g,
                &(b + 1).to_string(),
                bx + 3.0,
                y + 4.0,
                &plain(11.0, ui::MUTED),
                20.0,
            );
            if b > 0 {
                c.fill(bx, top, 1.0, rows_h, Color32::from_white_alpha(25));
            }
        }
        if m > 0 {
            c.fill(
                mx,
                top - 4.0,
                2.0,
                rows_h + 8.0,
                Color32::from_white_alpha(90),
            );
        }
    }
    for (li, &lane) in lanes.iter().enumerate() {
        let ry = top + li as f32 * row_h;
        let color = ui::lane_color(lane);
        ui::text(
            c,
            g,
            lane.name(),
            x + 10.0,
            ry + row_h / 2.0 - 9.0,
            &bold(14.0, color),
            label_w - 12.0,
        );
        for n in notes.iter().filter(|n| n.lane == lane) {
            let cell_w = mw / n.cells as f32;
            let cx = gx + n.measure as f32 * mw + (n.cell as f32 + 0.5) * cell_w;
            let size = (cell_w * 0.72).min(row_h * 0.62).clamp(8.0, 26.0);
            let (nx, ny) = (cx - size / 2.0, ry + row_h / 2.0 - size / 2.0);
            // 開いたハイハットは、譜面の絵と同じく中を抜く（閉じたハイハットと同じ色なので形で分ける）
            let hollow = lane == dtx::DrumLane::HHO;
            if hollow {
                ui::round_frame(c, nx, ny, size, size, size * 0.22, 2.0, color);
            } else {
                ui::round_fill(c, nx, ny, size, size, size * 0.22, color);
            }
            // アクセントは、楽譜と同じく音符の上に >
            if n.accent {
                let px = (ny - ry + 5.0).clamp(10.0, 15.0);
                ui::text_center(
                    c,
                    g,
                    ">",
                    cx,
                    ny - px - 1.0,
                    &bold(px, ACCENT_COLOR),
                    cell_w.max(12.0),
                );
            }
            let (letter, hand_color) = match n.hand {
                Hand::Right => ("R", RIGHT_COLOR),
                Hand::Left => ("L", LEFT_COLOR),
                h @ (Hand::Foot | Hand::LeftFoot) => (hand_letter(h), ui::DIM),
            };
            if one_lane {
                ui::text_center(
                    c,
                    g,
                    letter,
                    cx,
                    y + h - 30.0,
                    &bold((cell_w * 1.25).clamp(9.0, 15.0), hand_color),
                    cell_w.max(12.0),
                );
            } else if size >= 14.0 && !n.hand.is_foot() {
                ui::text_center(
                    c,
                    g,
                    letter,
                    cx,
                    ry + row_h / 2.0 - size * 0.42,
                    &bold(size * 0.62, if hollow { color } else { NOTE_LETTER }),
                    size,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> TodaySetup {
        TodaySetup {
            day: NaiveDate::from_ymd_opt(2026, 11, 27).unwrap(),
            song: None,
            progress: None,
            done_today: false,
            open: false,
        }
    }

    /// はじめて開いた画面（今日のメニューの行にいる）から、分け方の最初（タイミング）へ動かしたもの
    fn menu() -> DrillMenu {
        let mut m = DrillMenu::new(&Prefs::default(), HashMap::new(), setup());
        assert_eq!(m.selected(), Item::Today);
        m.input(Input::Down);
        m
    }

    #[test]
    fn every_drill_is_in_one_category_and_the_left_column_ends_with_the_foot_rows() {
        let all: Vec<usize> = Category::ALL.into_iter().flat_map(drills_of).collect();
        assert_eq!(all.len(), DRILLS.len());
        for i in 0..DRILLS.len() {
            assert_eq!(all.iter().filter(|x| **x == i).count(), 1);
        }
        let left = left_items();
        assert_eq!(left.len(), Category::ALL.len() + 3);
        assert_eq!(left[0], Item::Today);
        assert_eq!(left[left.len() - 2..], [Item::Rise, Item::Back]);
        // 今日のメニューと分け方が全部、左の列に収まる
        let foot = CATS.1 + CATS.3 - FOOT_H;
        assert!(CATS.1 + 12.0 + CAT_H + TODAY_GAP + Category::ALL.len() as f32 * CAT_H <= foot);
    }

    #[test]
    fn categories_open_into_the_drill_column_and_back() {
        let mut m = menu();
        assert_eq!(m.selected(), Item::Category(Category::Timing));
        // 分け方を動かすと真ん中の列も変わる（まだ入らない）
        let r = m.input(Input::Down);
        assert_eq!(r.se, Some(Se::Move));
        assert_eq!(m.selected(), Item::Category(Category::Rudiments));
        assert_eq!(m.drill().category, Category::Rudiments);
        // CY で入ると、その分け方の最初のドリル
        let r = m.input(Input::Decide);
        assert_eq!(r.done, None);
        let Item::Drill(i) = m.selected() else {
            panic!("{:?}", m.selected())
        };
        assert_eq!(i, drills_of(Category::Rudiments)[0]);
        m.input(Input::Down);
        let second = m.drill().id;
        // LC で左の列（同じ分け方）に戻り、もう一度 LC でタイトルへ
        let r = m.input(Input::Cancel);
        assert_eq!(r.done, None);
        assert_eq!(m.selected(), Item::Category(Category::Rudiments));
        assert_eq!(m.drill().id, second);
        // FT でも入れて、選んでいたドリルのまま
        m.input(Input::Right);
        assert_eq!(m.drill().id, second);
        m.input(Input::Cancel);
        assert_eq!(m.input(Input::Cancel).done, Some(Done::Back));
    }

    #[test]
    fn tempo_moves_by_five_and_is_kept_per_drill() {
        let mut m = menu();
        m.input(Input::Decide);
        let first = m.drill();
        let start = m.bpm_of(first);
        let r = m.input(Input::Right);
        assert_eq!(r.se, Some(Se::Change));
        assert!(r.changed);
        assert_eq!(m.bpm_of(first), start + BPM_STEP);
        m.input(Input::Down);
        let second = m.drill();
        assert_ne!(first.id, second.id);
        assert_eq!(m.bpm_of(second), second.default_bpm);
        let p = m.prefs();
        assert_eq!(p.last, second.id);
        assert_eq!(p.bpm.get(first.id), Some(&(start + BPM_STEP)));
        // 覚えた値から開き直すと、同じドリルの所（真ん中の列）に戻る
        let again = DrillMenu::new(&p, HashMap::new(), setup());
        assert_eq!(again.drill().id, second.id);
        assert!(matches!(again.selected(), Item::Drill(_)));
        assert_eq!(again.bpm_of(first), start + BPM_STEP);
        // 別の分け方のドリルでも
        let p = Prefs {
            last: "gallop".to_owned(),
            ..Prefs::default()
        };
        let m = DrillMenu::new(&p, HashMap::new(), setup());
        assert_eq!(m.drill().id, "gallop");
        let Item::Drill(i) = m.selected() else {
            panic!()
        };
        assert_eq!(DRILLS[i].category, Category::Feet);
    }

    #[test]
    fn start_builds_a_run_and_back_leaves() {
        let mut m = menu();
        // 左の列で決定すると入るだけ
        assert_eq!(m.input(Input::Decide).done, None);
        // ドリルの列で 1 回叩いただけでは始めない（右の欄の「始める」へ）
        assert_eq!(m.input(Input::Decide).done, None);
        assert_eq!((m.focus, m.detail), (Focus::Detail, DetailRow::Start));
        let r = m.input(Input::Decide);
        let Some(Done::Start(run)) = r.done else {
            panic!("{r:?}");
        };
        assert_eq!(run.drill.id, m.drill().id);
        assert_eq!(run.target(), m.bpm_of(m.drill()));
        // 右の欄 → ドリルの列 → 左の列。一番下の「タイトルに戻る」で決定しても戻る
        m.input(Input::Cancel);
        assert_eq!(m.focus, Focus::Drills);
        m.input(Input::Cancel);
        for _ in 0..20 {
            m.input(Input::Down);
        }
        assert_eq!(m.selected(), Item::Back);
        assert_eq!(m.input(Input::Decide).done, Some(Done::Back));
    }

    #[test]
    fn the_rise_row_changes_the_steps() {
        let mut m = menu();
        while m.selected() != Item::Rise {
            m.input(Input::Down);
        }
        assert_eq!(m.rise, DEFAULT_RISE);
        m.input(Input::Left);
        m.input(Input::Left);
        assert_eq!(m.rise, 0);
        // 上げ幅の行にいても、始めるのは真ん中の列で選んでいるドリル
        let run = m.run();
        assert!(run.bpms.iter().all(|&b| b == run.target()));
        assert_eq!(m.prefs().rise, 0);
        // 上げ幅の行の FT は入らない
        m.input(Input::Right);
        assert_eq!(m.selected(), Item::Rise);
    }

    #[test]
    fn the_list_scrolls_to_keep_the_cursor_visible() {
        for cat in Category::ALL {
            let mut m = menu();
            while m.selected() != Item::Category(cat) {
                m.input(Input::Down);
            }
            m.input(Input::Decide);
            let n = drills_of(cat).len();
            for _ in 0..n + 2 {
                let at = m.picked[m.category];
                assert!(
                    at >= m.top && at < m.top + visible_rows(),
                    "{cat:?} {at} {}",
                    m.top
                );
                m.input(Input::Down);
            }
            assert_eq!(m.picked[m.category], n - 1);
            for _ in 0..n {
                m.input(Input::Up);
                let at = m.picked[m.category];
                assert!(at >= m.top && at < m.top + visible_rows());
            }
            assert_eq!(m.top, 0);
        }
    }

    #[test]
    fn a_run_takes_about_three_quarters_of_a_minute() {
        let run = Run::new(Drill::find("paradiddle").unwrap(), 100, 10);
        let s = run_seconds(&run);
        assert!((35.0..60.0).contains(&s), "{s}");
        // どのドリルも、はじめのテンポなら 1 分かからない
        for d in DRILLS {
            let s = run_seconds(&Run::new(d, d.default_bpm, DEFAULT_RISE));
            assert!((20.0..60.0).contains(&s), "{} {s}", d.id);
        }
    }

    #[test]
    fn every_mode_ends_in_a_known_time_for_every_drill() {
        for d in DRILLS {
            for way in Way::ALL {
                let run = Run::with(d, way, d.default_bpm, DEFAULT_RISE, false);
                let s = run_seconds(&run);
                let limit = match run.way {
                    // 上の端まで叩いても 2 分半かからない
                    Way::Limit => 150.0,
                    Way::Endurance(m) => f64::from(m) * 60.0 * 1.2 + 10.0,
                    _ => 75.0,
                };
                assert!(s < limit, "{} {way:?} {s}", d.id);
            }
        }
    }

    #[test]
    fn the_mode_and_lead_hand_are_changed_in_the_right_panel() {
        let mut m = menu();
        // 左の列では使わない（キーボードの Tab・H）
        assert_eq!(m.input(Input::Way), Response::default());
        assert_eq!(m.input(Input::Lead), Response::default());
        m.input(Input::Decide);
        assert_eq!(m.run().way, Way::Climb);
        // CY で右の欄へ。上へ行くとテンポ・手順・やり方の行
        m.input(Input::Decide);
        for _ in 0..5 {
            m.input(Input::Up);
        }
        assert_eq!(m.detail, DetailRow::Mode);
        let r = m.input(Input::Right);
        assert_eq!(r.se, Some(Se::Change));
        assert!(r.changed);
        assert_eq!(m.run().way, Way::Limit);
        m.input(Input::Right);
        assert_eq!(m.run().way, Way::Endurance(1));
        m.input(Input::Left);
        assert_eq!(m.run().way, Way::Limit);
        m.input(Input::Right);
        m.input(Input::Down);
        assert_eq!(m.detail, DetailRow::Lead);
        let r = m.input(Input::Right);
        assert!(r.changed);
        assert!(m.run().lefty);
        // テンポの行は SD・FT でテンポ
        m.input(Input::Down);
        let before = m.run().target();
        m.input(Input::Right);
        assert_eq!(m.run().target(), before + BPM_STEP);
        m.input(Input::Left);
        // 覚えて、開き直しても同じ
        let p = m.prefs();
        assert_eq!(p.way, "endurance1");
        assert!(p.lefty);
        let again = DrillMenu::new(&p, HashMap::new(), setup());
        assert_eq!(again.run().way, Way::Endurance(1));
        assert!(again.run().lefty);
        // 値の行で CY を叩くと「始める」へ移るだけ。始めるのは「始める」で CY
        assert_eq!(m.input(Input::Decide).done, None);
        assert_eq!(m.detail, DetailRow::Start);
        assert_eq!(m.input(Input::Lead).done, None);
        assert!(matches!(m.input(Input::Decide).done, Some(Done::Start(_))));
        // キーボードの Tab・H はドリルの列でも効く
        m.input(Input::Cancel);
        m.input(Input::Way);
        assert_eq!(m.run().way, Way::Endurance(2));
    }

    #[test]
    fn modes_a_drill_cannot_use_fall_back_to_speed_up() {
        let p = Prefs {
            last: "gap_2".to_owned(),
            way: Way::Echo.key(),
            ..Prefs::default()
        };
        let mut m = DrillMenu::new(&p, HashMap::new(), setup());
        assert_eq!(m.run().way, Way::Climb);
        // 次は使えるものだけ（聞いてまねるの次のテンポを上げるの、さらに次）
        m.input(Input::Way);
        assert_eq!(m.run().way, Way::Endurance(1));
        // ほかのドリルに移れば、選んだやり方のまま
        let p = Prefs {
            last: "paradiddle".to_owned(),
            way: Way::Echo.key(),
            ..Prefs::default()
        };
        assert_eq!(
            DrillMenu::new(&p, HashMap::new(), setup()).run().way,
            Way::Echo
        );
    }

    #[test]
    fn records_are_kept_apart_for_each_lead_hand() {
        let mut records = DrillRecords::new();
        records.insert(
            ("paradiddle", false),
            crate::drill::DrillRecord {
                best: Some(120),
                ..Default::default()
            },
        );
        let p = Prefs {
            last: "paradiddle".to_owned(),
            ..Prefs::default()
        };
        let mut m = DrillMenu::new(&p, records, setup());
        let best = |m: &DrillMenu| m.records.get(&(m.drill().id, m.lefty)).and_then(|r| r.best);
        assert_eq!(best(&m), Some(120));
        m.input(Input::Lead);
        assert_eq!(best(&m), None);
    }

    #[test]
    fn todays_menu_is_first_and_starts_from_the_picked_step() {
        let mut m = DrillMenu::new(&Prefs::default(), HashMap::new(), setup());
        assert!(m.showing_today);
        assert_eq!(m.today.minutes, today::DEFAULT_LENGTH);
        let ten = m.today.plan.seconds();
        // CY で列に入り、SD/FT で長さ
        m.input(Input::Decide);
        assert_eq!(m.focus, Focus::Today);
        let r = m.input(Input::Right);
        assert!(r.changed);
        assert_eq!(m.today.minutes, 15);
        assert_eq!(m.prefs().today_minutes, 15);
        assert!(m.today.plan.seconds() > ten);
        // 端では止まる
        assert_eq!(m.input(Input::Right).se, None);
        m.input(Input::Down);
        m.input(Input::Down);
        let r = m.input(Input::Decide);
        let Some(Done::Today(p, from)) = r.done else {
            panic!("{r:?}")
        };
        assert_eq!(from, 2);
        assert!(!p.started());
        assert_eq!(p.plan, m.today.plan);
        // 次に開いたときも今日のメニューの行から
        assert!(m.prefs().on_today);
        let again = DrillMenu::new(&m.prefs(), HashMap::new(), setup());
        assert!(again.showing_today);
        assert_eq!(again.selected(), Item::Today);
        // LC で左の列の今日のメニューの行へ
        m.input(Input::Cancel);
        assert_eq!(m.focus, Focus::Left);
        assert_eq!(m.selected(), Item::Today);
        // 分け方へ動くと、真ん中の列も分け方に
        m.input(Input::Down);
        assert!(!m.showing_today);
        assert_eq!(m.selected(), Item::Category(Category::Timing));
    }

    #[test]
    fn a_started_menu_comes_back_at_the_next_step_and_keeps_its_length() {
        let fresh = DrillMenu::new(&Prefs::default(), HashMap::new(), setup());
        let mut p = Progress::new(fresh.today.plan.clone());
        p.done[0] = true;
        p.done[1] = true;
        let mut m = DrillMenu::new(
            &Prefs {
                last: "paradiddle".into(),
                today_minutes: 5,
                ..Prefs::default()
            },
            HashMap::new(),
            TodaySetup {
                progress: Some(p.clone()),
                open: true,
                ..setup()
            },
        );
        // 今日のメニューの結果から戻ると、次のものを選んでいる（長さは始めたもの）
        assert_eq!(m.focus, Focus::Today);
        assert_eq!(m.today.cursor, 2);
        assert_eq!(m.today.minutes, today::DEFAULT_LENGTH);
        assert_eq!(m.today.plan, p.plan);
        assert!(m.today_locked());
        assert!(!m.input(Input::Right).changed);
        let Some(Done::Today(back, 2)) = m.input(Input::Decide).done else {
            panic!()
        };
        assert_eq!(back.done_count(), 2);
        // 前の日のメニューは使わない
        let old = DrillMenu::new(
            &Prefs::default(),
            HashMap::new(),
            TodaySetup {
                day: NaiveDate::from_ymd_opt(2026, 11, 28).unwrap(),
                progress: Some(p),
                ..setup()
            },
        );
        assert!(old.today.progress.is_none());
        assert_eq!(old.today.cursor, 0);
    }
}
