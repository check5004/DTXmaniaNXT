//! 基礎練習の画面（タイトルの「Drills」から開く。2026-11-25 見本の案 A、2026-11-27 に型が増えて 3 列に）。
//!
//! 左に分け方、真ん中にその分け方のドリル、右に選んだドリルの手順の絵とテンポと段、これまでの最高。
//! 左の列では HT/LT で分け方、CY/RD か FT で真ん中の列へ。真ん中の列では HT/LT でドリル、SD/FT でテンポ（5 ずつ）、
//! CY/RD で始め、LC で左の列へ戻る（左の列の LC はタイトルへ）。テンポはドリルごとに覚える。
//! 1 回は決まった長さ（カウント 1 小節 + 4 段 × 4 小節）で必ず終わり、終えたらこの画面の同じドリルに戻る。
//! 次に何をするかは自分で選ぶ（勝手に次のドリルやテンポへ進めない）。
//! 足のパッドはメニューに使わない（誤って踏むため）。

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use egui::{Color32, Stroke};
use engine::drill::{
    BPM_STEP, COUNT_IN_MEASURES, Category, DEFAULT_RISE, DRILLS, Drill, Hand, RISES, Run,
    STEP_MEASURES, STEPS, nudge_bpm,
};
use i18n::{count, t, tf};

use crate::drill::{
    self, DrillRecord, LEFT_COLOR, RIGHT_COLOR, category_name, hand_letter, pattern_line,
};
use crate::gfx::{Canvas, Gfx};
use crate::settings_ui::{self as ui, Pad};
use crate::sfx::Se;
use crate::text::Style;

/// 左の列（分け方）、真ん中の列（ドリル）、右の板
const CATS: (f32, f32, f32, f32) = (40.0, 62.0, 226.0, 618.0);
const LIST: (f32, f32, f32, f32) = (278.0, 62.0, 330.0, 618.0);
const DETAIL: (f32, f32, f32, f32) = (620.0, 62.0, 620.0, 618.0);
/// 分け方の行と、ドリルの行の高さ
const CAT_H: f32 = 48.0;
const ROW_H: f32 = 42.0;
/// 真ん中の列の、ドリルの行が始まる所（板の上から）
const LIST_TOP: f32 = 44.0;
/// 左の列の下の、上げ幅とタイトルへの行の場所（板の下の端から）
const FOOT_H: f32 = 92.0;
const FOOT_ROW_H: f32 = 36.0;

/// 選べる行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
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
}

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
}

/// 操作の結果
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Response {
    pub se: Option<Se>,
    pub done: Option<Done>,
    /// 覚えておく値（テンポ、上げ幅、選んだドリル）が変わった
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Done {
    Start(Run),
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
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            last: String::new(),
            rise: DEFAULT_RISE,
            bpm: BTreeMap::new(),
        }
    }
}

pub struct DrillMenu {
    focus: Focus,
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
    records: HashMap<&'static str, DrillRecord>,
    entered: Instant,
}

/// 左の列の行（分け方、上げ幅、タイトルへ）
fn left_items() -> Vec<Item> {
    let mut v: Vec<Item> = Category::ALL.into_iter().map(Item::Category).collect();
    v.extend([Item::Rise, Item::Back]);
    v
}

/// 分け方のドリル（`DRILLS` の添字、並べる順）
fn drills_of(c: Category) -> Vec<usize> {
    (0..DRILLS.len())
        .filter(|&i| DRILLS[i].category == c)
        .collect()
}

/// 1 回の長さ（秒。カウントの小節も入れて）
pub fn run_seconds(run: &Run) -> f64 {
    let quarters = run.drill.bar_len() * 4.0;
    let bar = |bpm: u32| quarters * 60.0 / f64::from(bpm.max(1));
    f64::from(COUNT_IN_MEASURES) * bar(run.bpms[0])
        + run
            .bpms
            .iter()
            .map(|&b| f64::from(STEP_MEASURES) * bar(b))
            .sum::<f64>()
}

impl DrillMenu {
    pub fn new(prefs: &Prefs, records: HashMap<&'static str, DrillRecord>) -> Self {
        let bpm = DRILLS
            .iter()
            .filter_map(|d| prefs.bpm.get(d.id).map(|&b| (d.id, nudge_bpm(b, 0))))
            .collect();
        let rise = if RISES.contains(&prefs.rise) {
            prefs.rise
        } else {
            DEFAULT_RISE
        };
        let mut m = Self {
            focus: Focus::Left,
            left: 0,
            category: 0,
            picked: [0; Category::ALL.len()],
            top: 0,
            rise,
            bpm,
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
            m.left = c;
            m.picked[c] = drills_of(DRILLS[i].category)
                .iter()
                .position(|&x| x == i)
                .unwrap_or(0);
            m.focus = Focus::Drills;
        }
        m.follow();
        m
    }

    /// 選んでいる行
    pub fn selected(&self) -> Item {
        match self.focus {
            Focus::Left => left_items()[self.left],
            Focus::Drills => Item::Drill(self.drill_index()),
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
        Run::new(d, self.bpm_of(d), self.rise)
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
        }
    }

    pub fn input(&mut self, input: Input) -> Response {
        match self.focus {
            Focus::Left => self.input_left(input),
            Focus::Drills => self.input_drills(input),
        }
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
                    if let Item::Category(_) = items[to] {
                        // 分け方を動かすと真ん中の列もその分け方に
                        self.category = to;
                        self.follow();
                        r.changed = true;
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
                Item::Category(_) if input == Input::Right => return self.enter(),
                _ => {}
            },
            Input::Decide => match items[self.left] {
                Item::Category(_) | Item::Drill(_) => return self.enter(),
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
        }
        r
    }

    /// 左の列の分け方から真ん中の列へ
    fn enter(&mut self) -> Response {
        self.focus = Focus::Drills;
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
                let dir = if input == Input::Left { -1 } else { 1 };
                let d = self.drill();
                let now = self.bpm_of(d);
                let to = nudge_bpm(now, dir);
                if to != now {
                    self.bpm.insert(d.id, to);
                    r.se = Some(Se::Change);
                    r.changed = true;
                }
            }
            Input::Decide => {
                r.se = Some(Se::Decide);
                r.done = Some(Done::Start(self.run()));
            }
            Input::Cancel => {
                // 左の列へ戻る（分け方の行に）
                self.focus = Focus::Left;
                self.left = self.category;
                r.se = Some(Se::Cancel);
            }
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
            Item::Drill(_) => {
                v.push((&[Pad::SD, Pad::FT], t("Tempo", "テンポ")));
                v.push((&[Pad::CY, Pad::RD], t("Start (Enter)", "始める（Enter）")));
                v.push((&[Pad::LC], t("Categories (Esc)", "分け方へ（Esc）")));
                return v;
            }
            Item::Category(_) => {
                v.push((&[Pad::CY, Pad::RD], t("Open (Enter)", "開く（Enter）")));
            }
            Item::Rise => {
                v.push((&[Pad::SD, Pad::FT], t("Change", "変える")));
            }
            Item::Back => {
                v.push((
                    &[Pad::CY, Pad::RD],
                    t("Back to Title (Enter)", "タイトルへ（Enter）"),
                ));
            }
        }
        v.push((&[Pad::LC], t("Back to Title (Esc)", "タイトルへ（Esc）")));
        v
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
        for (k, cat) in Category::ALL.into_iter().enumerate() {
            let ry = y + 12.0 + k as f32 * CAT_H;
            let open = k == self.category;
            let on = left && self.left == k;
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
            match self.records.get(d.id).and_then(|r| r.best) {
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
        draw_pattern(c, g, d, (x + 20.0, y + 58.0, w - 40.0, 156.0));
        ui::text(
            c,
            g,
            t("R = right hand   L = left hand", "R = 右手　L = 左手"),
            x + 20.0,
            y + 222.0,
            &plain(13.0, ui::MUTED),
            200.0,
        );
        ui::text_right(
            c,
            g,
            &pattern_line(d),
            x + w - 20.0,
            y + 222.0,
            &plain(13.0, ui::MUTED),
            w - 260.0,
        );
        c.fill(
            x + 20.0,
            y + 250.0,
            w - 40.0,
            1.0,
            Color32::from_white_alpha(40),
        );
        // テンポ
        let tempo_on = matches!(self.selected(), Item::Drill(_));
        ui::text(
            c,
            g,
            t("Tempo", "テンポ"),
            x + 20.0,
            y + 264.0,
            &plain(15.0, ui::DIM),
            200.0,
        );
        let arrow = if tempo_on { ACCENT } else { ui::MUTED };
        ui::triangle(c, x + 30.0, y + 334.0, 10.0, -1, arrow);
        ui::triangle(c, x + 222.0, y + 334.0, 10.0, 1, arrow);
        ui::text_center(
            c,
            g,
            &run.target().to_string(),
            x + 126.0,
            y + 300.0,
            &bold(58.0, ui::TEXT),
            170.0,
        );
        ui::text_center(
            c,
            g,
            "BPM",
            x + 126.0,
            y + 368.0,
            &plain(13.0, ui::MUTED),
            100.0,
        );
        ui::text(
            c,
            g,
            &tf!("SD ◀ · FT ▶ by {BPM_STEP}", "SD ◀・FT ▶ で {BPM_STEP} ずつ"),
            x + 20.0,
            y + 398.0,
            &plain(13.0, ui::MUTED),
            230.0,
        );
        // 段（テンポを上げないドリルは同じテンポのまま）
        let none = [None; STEPS];
        let (sx, sw) = (x + 270.0, w - 290.0);
        crate::drill_hud::stairs(
            c,
            g,
            &run.bpms,
            None,
            &none,
            false,
            (sx, y + 270.0, sw, 136.0),
        );
        let secs = run_seconds(&run).round() as u32;
        let line = if d.fixed_tempo() || self.rise == 0 {
            tf!(
                "{STEPS} steps × {STEP_MEASURES} bars at one tempo · about {secs} s",
                "{STEPS} 段 × {STEP_MEASURES} 小節を同じテンポで・約 {secs} 秒"
            )
        } else {
            tf!(
                "{STEP_MEASURES} bars per step · {STEPS} steps · about {secs} s",
                "1 段 {STEP_MEASURES} 小節・{STEPS} 段で約 {secs} 秒"
            )
        };
        ui::text_center(
            c,
            g,
            &line,
            sx + sw / 2.0,
            y + 416.0,
            &plain(14.0, ui::DIM),
            sw,
        );
        c.fill(
            x + 20.0,
            y + 450.0,
            w - 40.0,
            1.0,
            Color32::from_white_alpha(40),
        );
        // これまで
        ui::text(
            c,
            g,
            t("Best", "これまで"),
            x + 20.0,
            y + 468.0,
            &plain(15.0, ui::DIM),
            120.0,
        );
        let rec = self.records.get(d.id);
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
                y + 480.0,
                16.0,
            ),
            None => {
                ui::text(
                    c,
                    g,
                    t("No clean step yet", "まだミスなしの段はありません"),
                    x + 120.0,
                    y + 468.0,
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
                y + 468.0,
                &plain(14.0, ui::MUTED),
                150.0,
            );
            draw_trail(c, &r.trail, (x + w - 176.0, y + 462.0, 150.0, 34.0));
        }
        // 始める
        let (bx, by, bw, bh) = (x + w - 250.0, y + DETAIL.3 - 72.0, 230.0, 52.0);
        let on = tempo_on;
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
            t("Start", "始める"),
            bx + 70.0,
            by + 12.0,
            &bold(22.0, ui::TEXT),
            bw - 80.0,
        );
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
fn draw_pattern(c: &Canvas, g: &mut Gfx, d: &Drill, (x, y, w, h): (f32, f32, f32, f32)) {
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
    let notes = d.notes();
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

    fn menu() -> DrillMenu {
        DrillMenu::new(&Prefs::default(), HashMap::new())
    }

    #[test]
    fn every_drill_is_in_one_category_and_the_left_column_ends_with_the_foot_rows() {
        let all: Vec<usize> = Category::ALL.into_iter().flat_map(drills_of).collect();
        assert_eq!(all.len(), DRILLS.len());
        for i in 0..DRILLS.len() {
            assert_eq!(all.iter().filter(|x| **x == i).count(), 1);
        }
        let left = left_items();
        assert_eq!(left.len(), Category::ALL.len() + 2);
        assert_eq!(left[left.len() - 2..], [Item::Rise, Item::Back]);
        // 分け方が全部、左の列に収まる
        let foot = CATS.1 + CATS.3 - FOOT_H;
        assert!(CATS.1 + 12.0 + Category::ALL.len() as f32 * CAT_H <= foot);
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
        let again = DrillMenu::new(&p, HashMap::new());
        assert_eq!(again.drill().id, second.id);
        assert!(matches!(again.selected(), Item::Drill(_)));
        assert_eq!(again.bpm_of(first), start + BPM_STEP);
        // 別の分け方のドリルでも
        let p = Prefs {
            last: "gallop".to_owned(),
            ..Prefs::default()
        };
        let m = DrillMenu::new(&p, HashMap::new());
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
        let r = m.input(Input::Decide);
        let Some(Done::Start(run)) = r.done else {
            panic!("{r:?}");
        };
        assert_eq!(run.drill.id, m.drill().id);
        assert_eq!(run.target(), m.bpm_of(m.drill()));
        // 一番下の「タイトルに戻る」で決定しても戻る
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
}
