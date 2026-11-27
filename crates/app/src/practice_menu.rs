//! 練習メニュー（区間練習の入口で、練習の結果のあとに戻ってくる所）。選曲画面の上に、画面いっぱいに出す。
//!
//! 左に苦手な所（遊んだ記録から）を譜面の小さな絵つきで並べ、右に選んでいる所の大きな絵を出す。
//! よくミスする音には印を付け、選んでいる所の音（BGM とドラム）は App が流す（[`PracticeMenu::shown`]）。
//! 苦手な所の下に「やり方」（パート練習・お手本と交互など。パートや向きを選ぶやり方はその下に 1 行）、
//! 「自分で選ぶ」（区間と長さ）、「速度・つなぎ方」（開くと右に行が出る）、「選曲に戻る」。
//! いくつかの所をめぐるやり方（苦手めぐり・フィルだけ・つなぎ目）では、苦手な所の代わりにめぐる所を
//! めぐる順に並べ、「自分で選ぶ」の行は出さない。どの札から始めても、1 か所目からめぐる。
//!
//! 練習の結果からはここに戻る。苦手な所の並びは開いたときのまま変えず（練習で苦手度が下がっても消えない）、
//! クリアした所には印を付け、カーソルを次のまだクリアしていない所に移す。勝手には始めない。
//!
//! 操作は設定の画面と同じ（ドラムだけで操作できるように）。HT・LT（↑↓）で選び、SD・FT（←→）で区間や値を
//! 変え、CY・RD（Enter）で決め、LC（Esc）で戻る。足のパッドは使わない。

use std::time::{Duration, Instant};

use egui::{Color32, Pos2, Stroke, StrokeKind};
use engine::practice::{LoopStyle, STEP_SPEED, Section, SpeedPlan};
use engine::practice_way::{Build, Way};
use i18n::{count, t, tf};

use crate::band::{Band, BandData, CLEARED_COLOR};
use crate::chart_picture::{Look, WEAK_COLOR};
use crate::gfx::{Canvas, Gfx};
use crate::practice::{
    PracticeOptions, PracticeSetup, build_label, no_stops_line, part_label, pass_line,
    section_label, style_label, way_help, way_label,
};
use crate::settings_ui::{self as ui, Pad};
use crate::sfx::Se;
use crate::stretch::Pitch;
use crate::text::Style;
use crate::ui::weakness_color;

/// 後ろの選曲画面を暗くする濃さ
const DIM: u8 = 215;
/// 曲全体の小節の帯の板
const TOP: (f32, f32, f32, f32) = (40.0, 62.0, 1200.0, 62.0);
const BAND: Band = Band {
    x: TOP.0 + 16.0,
    y: TOP.1 + 12.0,
    w: TOP.2 - 32.0,
    cell_h: 22.0,
    row_pitch: 40.0,
    max_rows: 1,
    striped: false,
};
/// 左の一覧の板と、苦手な所の札の高さ・間
const LIST: (f32, f32, f32, f32) = (40.0, 136.0, 470.0, 546.0);
const CARD_Y: f32 = LIST.1 + 36.0;
const CARD_H: f32 = 64.0;
const CARD_GAP: f32 = 6.0;
const ROW_H: f32 = 30.0;
/// 右の板（選んでいる所の絵）
const DETAIL: (f32, f32, f32, f32) = (526.0, 136.0, 714.0, 546.0);
const PICTURE: (f32, f32, f32, f32) = (DETAIL.0 + 20.0, DETAIL.1 + 56.0, DETAIL.2 - 40.0, 250.0);
/// 練習を始めるボタン
const START: (f32, f32, f32, f32) = (DETAIL.0 + DETAIL.2 - 250.0, DETAIL.1 + 478.0, 230.0, 48.0);
/// クリアの知らせを出しておく時間
const BANNER_FOR: Duration = Duration::from_secs(5);

const ORANGE: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
const GREY: Color32 = Color32::from_gray(200);
const PANEL_FILL: Color32 = Color32::from_rgb(10, 10, 18);

/// 苦手な所
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub section: Section,
    /// 開いたときの苦手度（0〜1）
    pub weakness: f64,
}

/// 練習を終えて戻ってきたとき
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Finished {
    pub section: Section,
    /// 目標の段を越えた
    pub cleared: bool,
    /// 目標の速度（%）
    pub speed: u32,
}

/// 一覧の行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Spot(usize),
    /// やり方（SD・FT で切り替える）
    Way,
    /// パート練習で叩くパート、積み上げの向き（そのやり方のときだけ出る）
    WayOption,
    /// 自分で選ぶ区間（SD・FT で 1 小節ずつ動かす）
    Section,
    /// 自分で選ぶ区間の長さ
    Length,
    /// 速度・つなぎ方（開くと右に行が出る）
    Settings,
    Back,
}

impl Item {
    /// 決めると練習を始める行
    fn starts(self) -> bool {
        matches!(
            self,
            Item::Spot(_) | Item::Way | Item::WayOption | Item::Section | Item::Length
        )
    }
}

/// 「速度・つなぎ方」の行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetRow {
    Loop,
    Speed,
    SpeedValue,
    Target,
    Pitch,
    Metronome,
    Done,
}

const SET_ROWS: [SetRow; 7] = [
    SetRow::Loop,
    SetRow::Speed,
    SetRow::SpeedValue,
    SetRow::Target,
    SetRow::Pitch,
    SetRow::Metronome,
    SetRow::Done,
];

/// 操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuInput {
    Up,
    Down,
    Left,
    Right,
    Decide,
    Cancel,
}

/// メニューを閉じるとき
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Done {
    Start(PracticeOptions),
    Close,
}

/// 操作の結果
#[derive(Debug, Default, PartialEq)]
pub struct Response {
    pub se: Option<Se>,
    pub done: Option<Done>,
}

impl Response {
    fn se(se: Se) -> Self {
        Self {
            se: Some(se),
            done: None,
        }
    }
}

/// マウスの操作（NX の座標）
#[derive(Debug, Clone, Copy, Default)]
pub struct MenuMouse {
    /// 押した場所
    pub pressed: Option<Pos2>,
    /// 押したまま
    pub down: bool,
    /// 指している場所
    pub hover: Option<Pos2>,
}

/// 選んでいる所の音の具合（右の板に出す）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sound {
    /// 曲の音を読んでいる
    Loading,
    /// 流している。`Some` は今鳴っている所（区間の頭から 0〜1）
    Playing(Option<f32>),
    /// 音が出せない（出力が無い、読めなかった）
    Off,
}

pub struct PracticeMenu {
    pub setup: PracticeSetup,
    spots: Vec<Spot>,
    /// 自分で選ぶ区間（曲に小節が無ければ None）
    custom: Option<Section>,
    cursor: usize,
    /// 「速度・つなぎ方」を開いているなら、その中の行
    settings: Option<usize>,
    /// 絵に出している区間（最後に選んだ苦手な所か、自分で選ぶ区間）
    shown: Option<Section>,
    banner: Option<(String, Instant)>,
    /// 帯でドラッグを始めた小節と、指している小節
    drag: Option<u32>,
    hover: Option<u32>,
}

impl PracticeMenu {
    /// `spots` は前に開いたときの苦手な所（同じ曲の練習から戻ってきたとき。無ければ記録から出す）。
    /// `finished` は今終えた練習
    pub fn new(setup: PracticeSetup, spots: Option<Vec<Spot>>, finished: Option<Finished>) -> Self {
        let spots = spots.unwrap_or_else(|| {
            setup
                .weak
                .phrases
                .iter()
                .map(|p| Spot {
                    section: Section::new(p.first, p.last),
                    weakness: p.weakness,
                })
                .collect()
        });
        let mut setup = setup;
        let sections: Vec<Section> = spots.iter().map(|p| p.section).collect();
        setup.reshuffle_weak_tour(&sections);
        let target = finished.map(|f| f.section).or(setup.section);
        let target_spot = target.and_then(|s| spots.iter().position(|p| p.section == s));
        let mut custom = setup.default_custom();
        if let Some(s) = target.filter(|_| target_spot.is_none()) {
            custom = Some(s);
        }
        let mut m = Self {
            setup,
            spots,
            custom,
            cursor: 0,
            settings: None,
            shown: None,
            banner: None,
            drag: None,
            hover: None,
        };
        m.cursor = match (target, target_spot) {
            // めぐるやり方は「やり方」の行から（決めればもう一度めぐる）
            _ if m.setup.way.tours() => m.item_index(Item::Way),
            (Some(_), Some(i)) if finished.is_some_and(|f| f.cleared) => m.next_open_spot(i),
            (Some(_), Some(i)) => i,
            (Some(_), None) => m.item_index(Item::Section),
            (None, _) if m.spots.is_empty() => m.item_index(Item::Section),
            (None, _) => 0,
        };
        if let Some(f) = finished.filter(|f| f.cleared) {
            m.banner = Some((
                tf!(
                    "Cleared! {} ({}%)",
                    "クリア！　{}（{}%）",
                    section_label(f.section),
                    f.speed
                ),
                Instant::now() + BANNER_FOR,
            ));
        }
        m.sync();
        m
    }

    /// 苦手な所（次に開くときに同じ並びで出す）
    pub fn spots(&self) -> &[Spot] {
        &self.spots
    }

    /// 絵に出している区間（この区間の音を流す）
    pub fn shown(&self) -> Option<Section> {
        self.shown
    }

    /// 一覧の札：苦手な所。めぐるやり方なら、めぐる所（めぐる順）
    fn cards(&self) -> Vec<Spot> {
        if !self.setup.way.tours() {
            return self.spots.clone();
        }
        self.setup
            .stops()
            .into_iter()
            .map(|section| Spot {
                section,
                weakness: self.recorded_weakness(section).unwrap_or(0.0),
            })
            .collect()
    }

    fn items(&self) -> Vec<Item> {
        (0..self.cards().len())
            .map(Item::Spot)
            .chain(self.rows())
            .collect()
    }

    /// 苦手な所の下の行
    fn rows(&self) -> Vec<Item> {
        let mut v = vec![Item::Way];
        if matches!(self.setup.way, Way::Part | Way::Build) {
            v.push(Item::WayOption);
        }
        if !self.setup.way.tours() {
            v.extend([Item::Section, Item::Length]);
        }
        v.extend([Item::Settings, Item::Back]);
        v
    }

    fn item(&self) -> Item {
        self.items()[self.cursor.min(self.items().len() - 1)]
    }

    fn item_index(&self, item: Item) -> usize {
        self.items().iter().position(|&i| i == item).unwrap_or(0)
    }

    /// 区間がクリアした区間なら、一番速い速度（%）
    fn cleared_speed(&self, s: Section) -> Option<u32> {
        self.setup
            .cleared_sections()
            .into_iter()
            .find(|c| c.0 == s)
            .map(|c| c.1)
    }

    /// 苦手な所 `i` の次の、まだクリアしていない所（無ければ `i`）
    fn next_open_spot(&self, i: usize) -> usize {
        let n = self.spots.len();
        (1..n)
            .map(|d| (i + d) % n)
            .find(|&j| self.cleared_speed(self.spots[j].section).is_none())
            .unwrap_or(i)
    }

    /// 区間の記録の苦手度（小節の平均）。記録が無ければ None
    fn recorded_weakness(&self, s: Section) -> Option<f64> {
        let w = &self.setup.weak.weakness;
        let v: Vec<f64> = (s.first..=s.last)
            .filter_map(|m| w.get(m as usize).copied().flatten())
            .collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    }

    /// 今の苦手度（記録が変わったら変わる）。区間に記録が無ければ開いたときの値
    fn weakness_now(&self, spot: &Spot) -> f64 {
        self.recorded_weakness(spot.section)
            .unwrap_or(spot.weakness)
    }

    /// 札に出す苦手度。めぐる所（フィル・つなぎ目）は、記録が無ければ None
    fn card_weakness(&self, spot: &Spot) -> Option<f64> {
        if self.setup.way.tours() {
            self.recorded_weakness(spot.section)
        } else {
            Some(self.weakness_now(spot))
        }
    }

    /// カーソルの行に合わせて、練習する区間と絵に出す区間を決める。めぐるやり方では練習する区間は変えず、
    /// 「やり方」の行では 1 か所目を絵に出す
    fn sync(&mut self) {
        let tours = self.setup.way.tours();
        let s = match self.item() {
            Item::Spot(i) => self.cards().get(i).map(|c| c.section),
            Item::Section | Item::Length => self.custom,
            Item::Way if tours => {
                if let Some(c) = self.cards().first() {
                    self.shown = Some(c.section);
                }
                return;
            }
            Item::Way | Item::WayOption | Item::Settings | Item::Back => return,
        };
        if !tours {
            self.setup.section = s;
        }
        self.shown = s;
    }

    pub fn input(&mut self, input: MenuInput) -> Response {
        if let Some(row) = self.settings {
            return self.settings_input(row, input);
        }
        let item = self.item();
        match input {
            MenuInput::Up | MenuInput::Down => {
                let n = self.items().len();
                let next = if input == MenuInput::Up {
                    self.cursor.checked_sub(1)
                } else {
                    Some(self.cursor + 1).filter(|&i| i < n)
                };
                match next {
                    Some(i) => {
                        self.cursor = i;
                        self.sync();
                        Response::se(Se::Move)
                    }
                    None => Response::default(),
                }
            }
            MenuInput::Left | MenuInput::Right => {
                let d = if input == MenuInput::Left { -1 } else { 1 };
                match item {
                    Item::Way => {
                        let was = self.setup.way.tours();
                        self.setup.way = self.setup.way.step(d);
                        // 札の数が変わるので、カーソルは「やり方」の行に置き直す
                        self.cursor = self.item_index(Item::Way);
                        if was && !self.setup.way.tours() {
                            self.shown = self.setup.section;
                        }
                        self.sync();
                        return Response::se(Se::Move);
                    }
                    Item::WayOption => return self.change_option(d),
                    _ => {}
                }
                let before = self.setup.section;
                match item {
                    Item::Section => self.setup.shift(d),
                    Item::Length => self.setup.resize(d),
                    _ => return Response::default(),
                }
                self.custom = self.setup.section;
                self.shown = self.setup.section;
                if before == self.setup.section {
                    Response::se(Se::Cancel)
                } else {
                    Response::se(Se::Move)
                }
            }
            MenuInput::Cancel => Response {
                se: Some(Se::Cancel),
                done: Some(Done::Close),
            },
            MenuInput::Decide => match item {
                _ if item.starts() => self.start(),
                Item::Settings => {
                    self.settings = Some(0);
                    Response::se(Se::Decide)
                }
                _ => Response {
                    se: Some(Se::Cancel),
                    done: Some(Done::Close),
                },
            },
        }
    }

    /// パート練習のパート、積み上げの向きを `d`（-1 か 1）動かす。パートは曲に音符のあるものだけ
    fn change_option(&mut self, d: i64) -> Response {
        let s = &mut self.setup;
        match s.way {
            Way::Part => {
                let parts = s.parts();
                let i = parts.iter().position(|&p| p == s.part).unwrap_or(0) as i64;
                let next = parts[(i + d).rem_euclid(parts.len() as i64) as usize];
                if next == s.part {
                    return Response::se(Se::Cancel);
                }
                s.part = next;
            }
            Way::Build => {
                s.build = match s.build {
                    Build::Forward => Build::Backward,
                    Build::Backward => Build::Forward,
                };
            }
            _ => return Response::default(),
        }
        Response::se(Se::Move)
    }

    fn start(&self) -> Response {
        match self.setup.options() {
            Some(o) => Response {
                se: Some(Se::Decide),
                done: Some(Done::Start(o)),
            },
            None => Response::se(Se::Cancel),
        }
    }

    fn settings_input(&mut self, row: usize, input: MenuInput) -> Response {
        let r = SET_ROWS[row];
        match input {
            MenuInput::Up | MenuInput::Down => {
                let next = if input == MenuInput::Up {
                    row.checked_sub(1)
                } else {
                    Some(row + 1).filter(|&i| i < SET_ROWS.len())
                };
                match next {
                    Some(i) => {
                        self.settings = Some(i);
                        Response::se(Se::Move)
                    }
                    None => Response::default(),
                }
            }
            MenuInput::Left | MenuInput::Right => {
                if r == SetRow::Done {
                    return Response::default();
                }
                let d = if input == MenuInput::Left { -1 } else { 1 };
                if self.adjust(r, d) {
                    Response::se(Se::Move)
                } else {
                    Response::se(Se::Cancel)
                }
            }
            MenuInput::Cancel => {
                self.settings = None;
                Response::se(Se::Cancel)
            }
            MenuInput::Decide if r == SetRow::Done => {
                self.settings = None;
                Response::se(Se::Decide)
            }
            // 値の行で決めたら、値を 1 つ後へ（切り替えの行はそれで切り替わる）
            MenuInput::Decide if self.adjust(r, 1) => Response::se(Se::Change),
            MenuInput::Decide => Response::se(Se::Cancel),
        }
    }

    /// 行 `row` の値を `d`（-1 か 1）動かす。動かせなければ false
    fn adjust(&mut self, row: SetRow, d: i64) -> bool {
        let s = &mut self.setup;
        let before = (s.style, s.speed, s.pitch, s.metronome);
        match row {
            SetRow::Loop => {
                s.style = match s.style {
                    LoopStyle::Connect => LoopStyle::Restart,
                    LoopStyle::Restart => LoopStyle::Connect,
                };
            }
            SetRow::Speed => s.toggle_ladder(),
            SetRow::SpeedValue => s.nudge_speed(d),
            SetRow::Target => {
                // 段階アップでないときは目標が無い
                let SpeedPlan::Ladder { target, .. } = s.speed else {
                    return false;
                };
                let t = SpeedPlan::TARGETS;
                let i = t.iter().position(|&x| x == target).unwrap_or(0) as i64;
                let next = (i + d).clamp(0, t.len() as i64 - 1) as usize;
                s.set_target(t[next]);
            }
            SetRow::Pitch => {
                let all = Pitch::ALL;
                let i = all.iter().position(|&p| p == s.pitch).unwrap_or(0) as i64;
                s.pitch = all[(i + d).rem_euclid(all.len() as i64) as usize];
            }
            SetRow::Metronome => s.metronome = !s.metronome,
            SetRow::Done => return false,
        }
        before != (s.style, s.speed, s.pitch, s.metronome)
    }

    /// マウス：札や行を押すとそこを選び（選んでいる札をもう一度押すと始める）、帯を押すかドラッグすると
    /// 自分で選ぶ区間を決める
    pub fn mouse(&mut self, m: MenuMouse) -> Response {
        let n = self.setup.measure_count();
        self.hover = m.hover.and_then(|p| BAND.measure_at(n, p));
        if let Some(p) = m.pressed {
            if self.settings.is_some() {
                if let Some(i) = set_row_at(p) {
                    self.settings = Some(i);
                    return self.settings_input(i, MenuInput::Decide);
                }
                if !in_rect(p, DETAIL) {
                    self.settings = None;
                }
            }
            if in_rect(p, START) && self.settings.is_none() && self.shown.is_some() {
                return self.start();
            }
            if let Some(i) = self.item_at(p) {
                let again = i == self.cursor;
                self.cursor = i;
                self.sync();
                let item = self.item();
                return match item {
                    Item::Spot(_) if again => self.start(),
                    Item::Settings | Item::Back => self.input(MenuInput::Decide),
                    _ => Response::se(Se::Move),
                };
            }
            self.drag = BAND.measure_at(n, p);
            if self.drag.is_some() {
                self.cursor = self.item_index(Item::Section);
            }
        }
        if let Some(from) = self.drag {
            if let Some(to) = self.hover {
                let s = Section::new(from.min(to), from.max(to));
                self.custom = Some(s);
                self.setup.section = Some(s);
                self.shown = Some(s);
            }
            if !m.down {
                self.drag = None;
                return Response::se(Se::Change);
            }
        }
        Response::default()
    }

    /// 苦手な所の札の間（札の高さと間）。下の行が増えたら、板に収まるよう札を低くする
    fn card_pitch(&self) -> f32 {
        let n = self.cards().len().max(1) as f32;
        let room = LIST.1 + LIST.3 - 8.0 - CARD_Y - 10.0 - self.rows().len() as f32 * ROW_H;
        (CARD_H + CARD_GAP).min(room / n)
    }

    /// 一覧の行の場所（y と高さ）
    fn item_rect(&self, i: usize) -> (f32, f32) {
        let n = self.cards().len();
        if i < n {
            let pitch = self.card_pitch();
            (CARD_Y + i as f32 * pitch, pitch - CARD_GAP)
        } else {
            (self.rows_top() + (i - n) as f32 * ROW_H, ROW_H)
        }
    }

    /// 苦手な所の下の行の上の端
    fn rows_top(&self) -> f32 {
        let n = self.cards().len();
        if n == 0 {
            // 苦手な所が無いことを書く 2 行の下
            CARD_Y + 56.0
        } else {
            CARD_Y + n as f32 * self.card_pitch() + 10.0
        }
    }

    fn item_at(&self, p: Pos2) -> Option<usize> {
        if p.x < LIST.0 + 10.0 || p.x > LIST.0 + LIST.2 - 10.0 {
            return None;
        }
        (0..self.items().len()).find(|&i| {
            let (y, h) = self.item_rect(i);
            p.y >= y && p.y < y + h
        })
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx, sound: Sound) {
        c.cover(Color32::from_black_alpha(DIM));
        g.heading(c, 40.0, 16.0, t("Section Practice", "区間練習"));
        let s = &self.setup;
        let song = if s.artist.is_empty() {
            s.title.clone()
        } else {
            format!("{} / {}", s.title, s.artist)
        };
        let style = Style::edged(18.0, Color32::WHITE, Color32::BLACK, 1.5);
        if let Some(tx) = g.texts.get(&song, &style) {
            let max_w = 760.0;
            let w = tx.w.min(max_w);
            tx.draw_fit(c, 1240.0 - w, 24.0, max_w);
        }
        self.draw_band(c, g);
        self.draw_list(c, g);
        match self.settings {
            Some(row) => self.draw_settings(c, g, row),
            None => self.draw_detail(c, g, sound),
        }
        if let Some((text, until)) = &self.banner {
            let now = Instant::now();
            if now < *until {
                let left = until.duration_since(now).as_secs_f32();
                draw_banner(c, g, text, left.min(0.5) * 2.0);
            }
        }
        ui::legend(c, g, &self.hints());
    }

    fn draw_band(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = TOP;
        panel(c, (x, y, w, h));
        let cleared: Vec<Section> = self.setup.cleared_sections().iter().map(|c| c.0).collect();
        BAND.draw(
            c,
            g,
            &BandData {
                weakness: &self.setup.weak.weakness,
                phrases: self.cards().iter().map(|s| s.section).collect(),
                selected: self.shown,
                cleared,
            },
        );
        // 指している小節（マウス）
        if let Some(m) = self.hover {
            let tip = match self.setup.weak.weakness.get(m as usize).copied().flatten() {
                Some(v) => tf!(
                    "Measure {m}: weakness {:.0}%",
                    "小節 {m}：苦手度 {:.0}%",
                    v * 100.0
                ),
                None => tf!("Measure {m}: no records", "小節 {m}：記録なし"),
            };
            let style = Style::edged(13.0, Color32::WHITE, Color32::BLACK, 1.0);
            if let Some(tx) = g.texts.get(&tip, &style) {
                tx.draw(c, x + w - 16.0 - tx.w, y + h + 2.0, 1.0, 1.0);
            }
        }
    }

    fn draw_list(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, _) = LIST;
        panel(c, LIST);
        let head = Style {
            bold: true,
            ..Style::edged(18.0, Color32::WHITE, Color32::BLACK, 1.5)
        };
        let way = self.setup.way;
        let heading = if way.tours() {
            t("Tour Stops", "めぐる所")
        } else {
            t("Weak Spots", "苦手な所")
        };
        if let Some(tx) = g.texts.get(heading, &head) {
            tx.draw(c, x + 18.0, y + 8.0, 1.0, 1.0);
        }
        let small = Style::edged(12.0, Color32::from_gray(190), Color32::BLACK, 1.0);
        let source = match way {
            Way::Fills => t("found in the chart", "譜面から見つけたフィル").to_owned(),
            Way::Seams => t("found in the chart", "譜面から見つけた変わり目").to_owned(),
            _ => weak_source(&self.setup),
        };
        if let Some(tx) = g.texts.get(&source, &small) {
            tx.draw_fit(c, (x + w - 18.0 - tx.w).max(x + 120.0), y + 14.0, w - 140.0);
        }
        let current = self.item();
        let cards = self.cards();
        if cards.is_empty() {
            let style = Style::edged(14.0, GREY, Color32::BLACK, 1.0);
            let text = if way.tours() {
                no_stops_line(way)
            } else {
                t(
                    "No weak spots yet. Play the song through once and they show up here. For now, pick a section yourself.",
                    "苦手な所はまだありません。1 回通して遊ぶと、ここに出ます。今は区間を自分で選んでください。",
                )
            };
            let lines = g.texts.wrap(text, 14.0, w - 40.0);
            for (i, line) in lines.iter().take(3).enumerate() {
                if let Some(tx) = g.texts.get(line, &style) {
                    tx.draw(c, x + 20.0, CARD_Y + i as f32 * 18.0, 1.0, 1.0);
                }
            }
        }
        for (i, spot) in cards.iter().enumerate() {
            let (cy, ch) = self.item_rect(i);
            let on = current == Item::Spot(i);
            let card = (x + 12.0, cy, w - 24.0, ch);
            round_fill(
                c,
                card,
                8.0,
                Color32::from_white_alpha(if on { 26 } else { 10 }),
            );
            if on {
                round_frame(c, card, 8.0, ORANGE, 3.0);
            } else {
                round_frame(c, card, 8.0, Color32::from_white_alpha(40), 1.0);
            }
            let num = Style {
                bold: true,
                ..Style::edged(24.0, if on { ORANGE } else { GREY }, Color32::BLACK, 1.5)
            };
            if let Some(tx) = g.texts.get(&(i + 1).to_string(), &num) {
                tx.draw(c, x + 32.0 - tx.w / 2.0, cy + (ch - tx.h) / 2.0, 1.0, 1.0);
            }
            self.setup.picture.draw(
                c,
                g,
                (x + 50.0, cy + 8.0, 246.0, ch - 16.0),
                spot.section,
                &self.setup.marks,
                Look::thumbnail(),
            );
            let tx_x = x + 308.0;
            let label = Style::edged(14.0, Color32::from_gray(225), Color32::BLACK, 1.0);
            if let Some(tx) = g.texts.get(&section_label(spot.section), &label) {
                tx.draw_fit(c, tx_x, cy + 6.0, w - 24.0 - (tx_x - x) - 8.0);
            }
            let pct = Style::edged(12.0, GREY, Color32::BLACK, 1.0);
            match self.card_weakness(spot) {
                Some(wk) => {
                    let bar = (tx_x, cy + 28.0, 100.0, 8.0);
                    round_fill(c, bar, 4.0, Color32::from_white_alpha(30));
                    round_fill(
                        c,
                        (bar.0, bar.1, bar.2 * wk.clamp(0.0, 1.0) as f32, bar.3),
                        4.0,
                        weakness_color(Some(wk)),
                    );
                    if let Some(tx) = g.texts.get(&format!("{:.0}%", wk * 100.0), &pct) {
                        tx.draw(c, bar.0 + bar.2 + 6.0, cy + 24.0, 1.0, 1.0);
                    }
                }
                None => {
                    if let Some(tx) = g.texts.get(t("no records", "記録なし"), &pct) {
                        tx.draw(c, tx_x, cy + 24.0, 1.0, 1.0);
                    }
                }
            }
            if let Some(speed) = self.cleared_speed(spot.section).filter(|_| !way.tours()) {
                let badge = (tx_x, cy + (ch - 18.0).min(40.0), 84.0, 18.0);
                round_fill(c, badge, 10.0, CLEARED_COLOR);
                let st = Style {
                    bold: true,
                    ..Style::plain(13.0, Color32::from_rgb(0, 40, 10))
                };
                if let Some(tx) = g.texts.get(&format!("✓ {speed}%"), &st) {
                    tx.draw(
                        c,
                        badge.0 + (badge.2 - tx.w) / 2.0,
                        badge.1 + (badge.3 - tx.h) / 2.0,
                        1.0,
                        1.0,
                    );
                }
            }
        }
        // 苦手な所の下の行
        let top = self.rows_top();
        c.fill(
            x + 18.0,
            top - 6.0,
            w - 36.0,
            1.0,
            Color32::from_white_alpha(50),
        );
        let n = cards.len();
        for (k, item) in self.rows().into_iter().enumerate() {
            let (ry, rh) = self.item_rect(n + k);
            let on = current == item;
            if on {
                round_fill(
                    c,
                    (x + 12.0, ry, w - 24.0, rh - 2.0),
                    6.0,
                    ORANGE.gamma_multiply(0.9),
                );
            }
            let ink = if on {
                Color32::from_rgb(40, 20, 0)
            } else {
                Color32::WHITE
            };
            let name = Style {
                bold: true,
                ..if on {
                    Style::plain(16.0, ink)
                } else {
                    Style::edged(16.0, ink, Color32::BLACK, 1.5)
                }
            };
            let label = match item {
                Item::Way => t("Mode", "やり方"),
                Item::WayOption if self.setup.way == Way::Part => t("Your Part", "叩くパート"),
                Item::WayOption => t("Direction", "向き"),
                Item::Section => t("Pick Your Own", "自分で選ぶ"),
                Item::Length => t("Length", "長さ"),
                Item::Settings => t("Speed & Looping", "速度・くり返し方"),
                _ => t("Back to Song Select", "選曲に戻る"),
            };
            if let Some(tx) = g.texts.get(label, &name) {
                tx.draw(c, x + 30.0, ry + (rh - tx.h) / 2.0 - 1.0, 1.0, 1.0);
            }
            let value = match item {
                Item::Way => way_label(self.setup.way).to_owned(),
                Item::WayOption if self.setup.way == Way::Part => {
                    part_label(self.setup.part).to_owned()
                }
                Item::WayOption => build_label(self.setup.build).to_owned(),
                Item::Section => self.custom.map(section_label).unwrap_or_default(),
                Item::Length => self
                    .custom
                    .map(|s| measures(s.measures()))
                    .unwrap_or_default(),
                Item::Settings => {
                    let p = self.setup.speed;
                    match p {
                        SpeedPlan::Ladder { start, target } => format!("{start} → {target}%"),
                        SpeedPlan::Fixed(s) => format!("{s}%"),
                    }
                }
                _ => String::new(),
            };
            if !value.is_empty() {
                let vs = if on {
                    Style::plain(15.0, ink)
                } else {
                    Style::edged(15.0, Color32::from_gray(230), Color32::BLACK, 1.0)
                };
                if let Some(tx) = g.texts.get(&value, &vs) {
                    let right = x + w - 40.0;
                    tx.draw(c, right - tx.w, ry + (rh - tx.h) / 2.0 - 1.0, 1.0, 1.0);
                }
                if on
                    && matches!(
                        item,
                        Item::Way | Item::WayOption | Item::Section | Item::Length
                    )
                {
                    let cy = ry + rh / 2.0 - 1.0;
                    let vw = g.texts.get(&value, &vs).map_or(60.0, |tx| tx.w);
                    ui::triangle(c, x + w - 52.0 - vw, cy, 10.0, -1, ink);
                    ui::triangle(c, x + w - 28.0, cy, 10.0, 1, ink);
                }
            }
        }
    }

    /// 右の板：選んでいる所の大きな絵、のぼる段、練習を始めるボタン
    fn draw_detail(&self, c: &Canvas, g: &mut Gfx, sound: Sound) {
        let (x, y, w, _) = DETAIL;
        panel(c, DETAIL);
        let left = x + 20.0;
        let right = x + w - 20.0;
        let Some(section) = self.shown else {
            let style = Style::edged(16.0, GREY, Color32::BLACK, 1.0);
            if let Some(tx) = g.texts.get(
                t("This song has no measures.", "この曲には小節がありません。"),
                &style,
            ) {
                tx.draw(c, left, y + 20.0, 1.0, 1.0);
            }
            return;
        };
        let cards = self.cards();
        let tours = self.setup.way.tours();
        let spot = cards.iter().position(|p| p.section == section);
        // 題：札（苦手 1・1 か所目・自分で選ぶ）と区間
        let badge = match spot {
            Some(i) if tours => tf!("Spot {}", "{} か所目", i + 1),
            Some(i) => tf!("Weak {}", "苦手 {}", i + 1),
            None => t("Your Pick", "自分で選ぶ").to_owned(),
        };
        let bs = Style {
            bold: true,
            ..Style::plain(14.0, Color32::from_rgb(40, 20, 0))
        };
        let mut tx_x = left;
        if let Some(tx) = g.texts.get(&badge, &bs) {
            let bw = tx.w + 24.0;
            round_fill(c, (left, y + 16.0, bw, 26.0), 13.0, ORANGE);
            tx.draw(c, left + 12.0, y + 16.0 + (26.0 - tx.h) / 2.0, 1.0, 1.0);
            tx_x += bw + 12.0;
        }
        let title = Style {
            bold: true,
            ..Style::edged(22.0, Color32::WHITE, Color32::BLACK, 1.5)
        };
        if let Some(tx) = g.texts.get(&section_label(section), &title) {
            tx.draw(c, tx_x, y + 14.0, 1.0, 1.0);
        }
        let info = match spot.and_then(|i| self.card_weakness(&cards[i])) {
            Some(wk) => Some((
                tf!("weakness {:.0}%", "苦手度 {:.0}%", wk * 100.0),
                weakness_color(Some(wk)),
            )),
            None => {
                let n = self.setup.notes_in(section);
                Some((count(n, "note", "notes", "音符"), GREY))
            }
        };
        if let Some((s, color)) = info {
            let st = Style::edged(15.0, color, Color32::BLACK, 1.0);
            if let Some(tx) = g.texts.get(&s, &st) {
                tx.draw(c, right - tx.w, y + 20.0, 1.0, 1.0);
            }
        }
        // 絵
        let playhead = match sound {
            Sound::Playing(p) => p,
            _ => None,
        };
        self.setup.picture.draw(
            c,
            g,
            PICTURE,
            section,
            &self.setup.marks,
            Look {
                playhead,
                ..Look::default()
            },
        );
        // 絵の下：印の意味と、音の具合
        let ly = PICTURE.1 + PICTURE.3 + 12.0;
        let ring = (left + 2.0, ly + 3.0, 18.0, 12.0);
        round_fill(c, ring, 3.0, WEAK_COLOR.gamma_multiply(0.35));
        round_frame(c, ring, 3.0, WEAK_COLOR, 1.6);
        let small = Style::edged(13.0, Color32::from_gray(210), Color32::BLACK, 1.0);
        if let Some(tx) = g.texts.get(
            t(
                "Notes you often miss (×: times missed)",
                "よくミスする音（×はミスした回数）",
            ),
            &small,
        ) {
            tx.draw(c, left + 28.0, ly, 1.0, 1.0);
        }
        let audio = match sound {
            Sound::Loading => Some(t("♪ Loading the song…", "♪ 曲を読み込んでいます…")),
            Sound::Playing(_) => Some(t(
                "♪ Playing with the count-in, as in practice",
                "♪ 練習と同じく、カウントから流しています",
            )),
            Sound::Off => None,
        };
        if let Some(a) = audio
            && let Some(tx) = g.texts.get(a, &small)
        {
            tx.draw(c, right - tx.w, ly, 1.0, 1.0);
        }
        c.fill(
            left,
            ly + 30.0,
            right - left,
            1.0,
            Color32::from_white_alpha(50),
        );
        // のぼる段と決まり（ほかのやり方は、やり方の説明）
        let s = &self.setup;
        let steps = s.speed.steps();
        let path = steps
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(" → ");
        let head = if steps.len() > 1 {
            tf!("{path}% ({} steps)", "{path}%（{} 段）", steps.len())
        } else {
            tf!("{path}% (1 step)", "{path}%（1 段）")
        };
        let accent = Style::edged(18.0, ORANGE, Color32::BLACK, 1.5);
        let grey = Style::edged(14.0, GREY, Color32::BLACK, 1.0);
        let py = ly + 42.0;
        let mut below = py + 54.0;
        let head = if s.way.tours() {
            let secs = s.tour_seconds();
            let time = if secs < 60.0 {
                t("about 1 min", "1 分ほど").to_owned()
            } else {
                let m = (secs / 60.0).round() as u32;
                tf!("about {m} min", "約 {m} 分")
            };
            tf!(
                "{}% the whole way · {time}",
                "{}% のまま・{time}",
                s.speed.first_speed()
            )
        } else {
            head
        };
        if s.way == Way::Climb {
            if let Some(tx) = g.texts.get(&head, &accent) {
                tx.draw_fit(c, left, py, right - left);
            }
            let rule = tf!(
                "{} loops per step. Pass the {}% step to clear. {}",
                "1 段 {} 周。{}% の段を越えたらクリア。{}",
                s.step_laps,
                s.speed.goal(),
                style_label(s.style)
            );
            if let Some(tx) = g.texts.get(&rule, &grey) {
                tx.draw_fit(c, left, py + 28.0, right - left);
            }
        } else {
            let mut hx = left;
            if let Some(tx) = g.texts.get(way_label(s.way), &accent) {
                tx.draw(c, left, py, 1.0, 1.0);
                hx += tx.w + 16.0;
            }
            let speed = Style::edged(14.0, Color32::from_gray(225), Color32::BLACK, 1.0);
            if let Some(tx) = g.texts.get(&head, &speed) {
                tx.draw_fit(c, hx, py + 3.0, right - hx);
            }
            let help = way_help(s.way, s.part, s.build, section.measures(), cards.len());
            let lines = g.texts.wrap(&help, 14.0, right - left);
            for (i, line) in lines.iter().take(3).enumerate() {
                if let Some(tx) = g.texts.get(line, &grey) {
                    tx.draw(c, left, py + 28.0 + i as f32 * 19.0, 1.0, 1.0);
                }
            }
            below = py + 28.0 + lines.len().min(3) as f32 * 19.0 + 6.0;
        }
        if let Some(speed) = self.cleared_speed(section).filter(|_| !tours) {
            let st = Style::edged(14.0, CLEARED_COLOR, Color32::BLACK, 1.0);
            if let Some(tx) = g
                .texts
                .get(&tf!("Cleared at {speed}%", "クリア済み（{speed}%）"), &st)
            {
                tx.draw(c, left, below, 1.0, 1.0);
            }
        }
        // 練習を始めるボタン（音符が無ければ知らせる）
        if !s.playable(section) {
            let text = if tours {
                no_stops_line(s.way)
            } else if s.notes_in(section) == 0 {
                t(
                    "This section has no notes.",
                    "この区間には音符がありません。",
                )
            } else {
                t(
                    "This section has no notes for your part.",
                    "この区間には、選んだパートの音符がありません。",
                )
            };
            let st = Style::edged(15.0, WEAK_COLOR, Color32::BLACK, 1.0);
            if let Some(tx) = g.texts.get(text, &st) {
                tx.draw(c, right - tx.w, START.1 + 14.0, 1.0, 1.0);
            }
            return;
        }
        let on = self.item().starts();
        round_fill(
            c,
            START,
            10.0,
            Color32::from_white_alpha(if on { 28 } else { 12 }),
        );
        round_frame(
            c,
            START,
            10.0,
            if on {
                ORANGE
            } else {
                Color32::from_white_alpha(60)
            },
            2.0,
        );
        let chip = Style {
            bold: true,
            ..Style::plain(13.0, Color32::BLACK)
        };
        let mut bx = START.0 + 14.0;
        if let Some(tx) = g.texts.get(Pad::CY.name(), &chip) {
            let cw = tx.w + 14.0;
            round_fill(c, (bx, START.1 + 14.0, cw, 20.0), 10.0, Pad::CY.color());
            tx.draw(c, bx + 7.0, START.1 + 14.0 + (20.0 - tx.h) / 2.0, 1.0, 1.0);
            bx += cw + 10.0;
        }
        let label = Style {
            bold: true,
            ..Style::edged(20.0, Color32::WHITE, Color32::BLACK, 1.5)
        };
        if let Some(tx) = g.texts.get(t("Start Practice", "練習を始める"), &label) {
            tx.draw_fit(
                c,
                bx,
                START.1 + (START.3 - tx.h) / 2.0,
                START.0 + START.2 - 12.0 - bx,
            );
        }
    }

    /// 右の板：速度・つなぎ方の行
    fn draw_settings(&self, c: &Canvas, g: &mut Gfx, cursor: usize) {
        let (x, y, w, _) = DETAIL;
        panel(c, DETAIL);
        let left = x + 20.0;
        let right = x + w - 20.0;
        g.heading(c, left, y + 12.0, t("Speed & Looping", "速度・くり返し方"));
        for (i, &row) in SET_ROWS.iter().enumerate() {
            let (ry, rh) = (set_row_y(i), ROW_H);
            let on = i == cursor;
            if on {
                round_fill(
                    c,
                    (left - 8.0, ry, right - left + 16.0, rh - 2.0),
                    6.0,
                    ORANGE.gamma_multiply(0.9),
                );
            }
            let ink = if on {
                Color32::from_rgb(40, 20, 0)
            } else {
                Color32::WHITE
            };
            let st = Style {
                bold: true,
                ..if on {
                    Style::plain(16.0, ink)
                } else {
                    Style::edged(16.0, ink, Color32::BLACK, 1.5)
                }
            };
            if let Some(tx) = g.texts.get(self.set_name(row), &st) {
                tx.draw(c, left + 8.0, ry + (rh - tx.h) / 2.0 - 1.0, 1.0, 1.0);
            }
            let v = self.set_value(row);
            if !v.is_empty()
                && let Some(tx) = g.texts.get(&v, &st)
            {
                tx.draw(c, x + 400.0, ry + (rh - tx.h) / 2.0 - 1.0, 1.0, 1.0);
                if on {
                    let cy = ry + rh / 2.0 - 1.0;
                    ui::triangle(c, x + 386.0, cy, 10.0, -1, ink);
                    ui::triangle(c, right - 10.0, cy, 10.0, 1, ink);
                }
            }
        }
        // のぼる段
        let s = &self.setup;
        let steps = s.speed.steps();
        let path = steps
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(" → ");
        let head = if steps.len() > 1 {
            tf!("{path}% ({} steps)", "{path}%（{} 段）", steps.len())
        } else {
            tf!("{path}% (1 step)", "{path}%（1 段）")
        };
        let py = set_row_y(SET_ROWS.len()) + 12.0;
        let accent = Style::edged(18.0, ORANGE, Color32::BLACK, 1.5);
        if let Some(tx) = g.texts.get(&head, &accent) {
            tx.draw_fit(c, left, py, right - left);
        }
        // 選んでいる行の説明
        let style = Style::edged(14.0, Color32::WHITE, Color32::BLACK, 1.0);
        let lines = g
            .texts
            .wrap(&self.set_help(SET_ROWS[cursor]), 14.0, right - left);
        for (i, line) in lines.iter().take(4).enumerate() {
            if let Some(tx) = g.texts.get(line, &style) {
                tx.draw(c, left, py + 34.0 + i as f32 * 19.0, 1.0, 1.0);
            }
        }
    }

    fn set_name(&self, row: SetRow) -> &'static str {
        match row {
            SetRow::Loop => t("Looping", "くり返し方"),
            SetRow::Speed => t("Speed Mode", "速度の決め方"),
            SetRow::SpeedValue => match self.setup.speed {
                SpeedPlan::Ladder { .. } => t("Start Speed", "始めの速度"),
                SpeedPlan::Fixed(_) => t("Speed", "速度"),
            },
            SetRow::Target => t("Target Speed", "目標の速度"),
            SetRow::Pitch => t("Pitch When Slowed", "遅くしたときの音程"),
            SetRow::Metronome => t("Metronome", "メトロノーム"),
            SetRow::Done => t("Done", "閉じる"),
        }
    }

    fn set_value(&self, row: SetRow) -> String {
        let s = &self.setup;
        match row {
            SetRow::Loop => style_label(s.style).to_owned(),
            SetRow::Speed => match s.speed {
                SpeedPlan::Ladder { .. } => t("Step Up", "段階アップ"),
                SpeedPlan::Fixed(_) => t("Fixed", "固定"),
            }
            .to_owned(),
            SetRow::SpeedValue => format!("{}%", s.speed.first_speed()),
            SetRow::Target => match s.speed {
                SpeedPlan::Ladder { target, .. } => format!("{target}%"),
                SpeedPlan::Fixed(_) => "-".to_owned(),
            },
            SetRow::Pitch => match s.pitch {
                Pitch::Keep => t("Keep", "保つ"),
                Pitch::Tape => t("Lower", "一緒に下げる"),
            }
            .to_owned(),
            SetRow::Metronome => if s.metronome { "ON" } else { "OFF" }.to_owned(),
            SetRow::Done => String::new(),
        }
    }

    /// 行の説明
    fn set_help(&self, row: SetRow) -> String {
        let s = &self.setup;
        match row {
            SetRow::Loop => match s.style {
                LoopStyle::Connect => t(
                    "Seamless: goes straight from the end of the section back to its start. Good for grooves.",
                    "切れ目なく: 区間の終わりから切れ目なく頭へ戻ります（ビート向き）。",
                ),
                LoopStyle::Restart => t(
                    "Restart Each Loop: you play through the first hit of the next measure, then it restarts from one measure before the section. Good for fills.",
                    "毎回区切る: 次の小節の頭の 1 打まで叩き、区間の 1 小節前からやり直します（フィル向き）。",
                ),
            }
            .to_owned(),
            SetRow::Speed => match s.speed {
                SpeedPlan::Ladder { .. } => tf!(
                    "Step Up: climbs from the start speed to the target speed in {STEP_SPEED}% steps. {}",
                    "段階アップ: 始めの速度から目標の速度まで、{STEP_SPEED}% ずつの段をのぼります。{}",
                    step_rule(s)
                ),
                SpeedPlan::Fixed(_) => tf!(
                    "Fixed: a single step at this speed. {}",
                    "固定: この速度の 1 段だけです。{}",
                    step_rule(s)
                ),
            },
            SetRow::SpeedValue => match s.speed {
                SpeedPlan::Ladder { .. } => t(
                    "Speed of the first step. Changes in 5% steps.",
                    "はじめの段の速度。5% ずつ変わります。",
                ),
                SpeedPlan::Fixed(_) => t(
                    "Practice speed. Changes in 5% steps.",
                    "練習する速度。5% ずつ変わります。",
                ),
            }
            .to_owned(),
            SetRow::Target => match s.speed {
                SpeedPlan::Ladder { .. } => t(
                    "Speed of the top step. Pass it to clear.",
                    "いちばん上の段の速度。ここを越えたらクリアです。",
                ),
                SpeedPlan::Fixed(_) => t(
                    "No target while the speed is fixed.",
                    "速度を固定にしているので、目標はありません。",
                ),
            }
            .to_owned(),
            SetRow::Pitch => match s.pitch {
                Pitch::Keep => t(
                    "Keep: slows the music down without changing its pitch.",
                    "保つ: 音程を保ったまま遅くします。",
                ),
                Pitch::Tape => t(
                    "Lower: the pitch drops with the speed, like a slowed-down tape.",
                    "一緒に下げる: 音程ごと遅くします（テープと同じ）。",
                ),
            }
            .to_owned(),
            SetRow::Metronome => t(
                "When ON, a soft click plays on every beat of the section (louder on the first beat of each measure). The 4-beat count-in before each step plays even when OFF.",
                "ON なら、区間のあいだ拍ごとに小さくクリックを鳴らします（小節の頭は強く）。各段の始めの 4 拍のカウントは、OFF でも鳴ります。",
            )
            .to_owned(),
            SetRow::Done => t(
                "Back to the list. These settings are kept for next time.",
                "一覧に戻ります。この設定は次の練習でも使います。",
            )
            .to_owned(),
        }
    }

    /// 画面の下に出す、パッドの役目
    fn hints(&self) -> Vec<ui::Hint<'static>> {
        let mut v: Vec<ui::Hint<'static>> =
            vec![(&[Pad::HT], t("Up", "上")), (&[Pad::LT], t("Down", "下"))];
        if let Some(row) = self.settings {
            let r = SET_ROWS[row];
            if r != SetRow::Done {
                v.push((&[Pad::SD, Pad::FT], t("←→ Change", "←→ 値を変える")));
            }
            let decide = if r == SetRow::Done {
                t("Close (Enter)", "閉じる（Enter）")
            } else {
                t("Next Value (Enter)", "次の値（Enter）")
            };
            v.push((&[Pad::CY, Pad::RD], decide));
            v.push((&[Pad::LC], t("Close (Esc)", "閉じる（Esc）")));
            return v;
        }
        let item = self.item();
        match item {
            Item::Way => v.push((&[Pad::SD, Pad::FT], t("←→ Mode", "←→ やり方を変える"))),
            Item::WayOption => v.push((&[Pad::SD, Pad::FT], t("←→ Change", "←→ 変える"))),
            Item::Section => v.push((&[Pad::SD, Pad::FT], t("←→ Move", "←→ 区間を動かす"))),
            Item::Length => v.push((&[Pad::SD, Pad::FT], t("←→ Length", "←→ 長さを変える"))),
            _ => {}
        }
        let decide = match item {
            Item::Settings => t("Open (Enter)", "開く（Enter）"),
            Item::Back => t("Song Select (Enter)", "選曲へ（Enter）"),
            _ => t("Start Practice (Enter)", "練習を始める（Enter）"),
        };
        v.push((&[Pad::CY, Pad::RD], decide));
        v.push((&[Pad::LC], t("Song Select (Esc)", "選曲へ（Esc）")));
        v
    }
}

impl PracticeSetup {
    /// 自分で選ぶ区間のはじめ（音符のある最初の小節から 4 小節）
    fn default_custom(&self) -> Option<Section> {
        let n = self.measure_count();
        if n == 0 {
            return None;
        }
        let first = self
            .notes_per_measure
            .iter()
            .position(|&k| k > 0)
            .unwrap_or(0) as u32;
        let last = (first + 3).min(n as u32 - 1);
        Some(Section::new(first, last))
    }
}

/// 「速度・つなぎ方」の行の上の端
fn set_row_y(i: usize) -> f32 {
    DETAIL.1 + 52.0 + i as f32 * ROW_H
}

fn set_row_at(p: Pos2) -> Option<usize> {
    if p.x < DETAIL.0 + 10.0 || p.x > DETAIL.0 + DETAIL.2 - 10.0 {
        return None;
    }
    (0..SET_ROWS.len()).find(|&i| {
        let y = set_row_y(i);
        p.y >= y && p.y < y + ROW_H
    })
}

fn in_rect(p: Pos2, (x, y, w, h): (f32, f32, f32, f32)) -> bool {
    p.x >= x && p.x < x + w && p.y >= y && p.y < y + h
}

fn panel(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32)) {
    c.fill(x, y, w, h, PANEL_FILL);
    c.frame(x, y, w, h, Color32::from_gray(150));
}

fn round_fill(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32), r: f32, color: Color32) {
    c.painter()
        .rect_filled(c.rect(x, y, w, h), r * c.scale(), color);
}

fn round_frame(c: &Canvas, (x, y, w, h): (f32, f32, f32, f32), r: f32, color: Color32, width: f32) {
    c.painter().rect_stroke(
        c.rect(x, y, w, h),
        r * c.scale(),
        Stroke::new(width * c.scale(), color),
        StrokeKind::Inside,
    );
}

/// 上の真ん中に、緑の札でクリアを知らせる
fn draw_banner(c: &Canvas, g: &mut Gfx, text: &str, alpha: f32) {
    let style = Style {
        bold: true,
        ..Style::plain(22.0, Color32::from_rgb(0, 40, 10))
    };
    let Some(tx) = g.texts.get(text, &style) else {
        return;
    };
    let (w, h) = (tx.w + 48.0, 44.0);
    let (x, y) = (640.0 - w / 2.0, 8.0);
    round_fill(c, (x, y, w, h), 22.0, CLEARED_COLOR.gamma_multiply(alpha));
    tx.draw(c, x + 24.0, y + (h - tx.h) / 2.0, 1.0, alpha);
}

/// 1 段の決まり（「1 段 4 周。同じ拍で 2 回ミスしなければ、次の段へ。」）
fn step_rule(s: &PracticeSetup) -> String {
    tf!(
        "{} loops per step. {}",
        "1 段 {} 周。{}。",
        s.step_laps,
        pass_line(s.rule)
    )
}

/// 「4 小節」
fn measures(n: u32) -> String {
    count(n, "measure", "measures", "小節")
}

/// 苦手度をどの記録から出したか
fn weak_source(s: &PracticeSetup) -> String {
    match (s.weak.plays, s.weak.practice_laps) {
        (0, 0) => t("no records yet", "まだ記録がありません").to_owned(),
        (p, 0) => {
            let p = count(p, "play", "plays", "回");
            tf!("based on {p}", "直近のプレイ {p}から")
        }
        (0, l) => {
            let l = count(l, "practice loop", "practice loops", "周");
            tf!("based on {l}", "直近の練習 {l}から")
        }
        (p, l) => {
            let p = count(p, "play", "plays", "回");
            let l = count(l, "practice loop", "practice loops", "周");
            tf!("based on {p} and {l}", "直近のプレイ {p}と練習 {l}から")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart_picture::{ChartPicture, Marks};
    use crate::game::WeakMap;
    use engine::analysis::WeakPhrase;
    use engine::practice::PassRule;
    use records::ClearRecord;

    fn setup(measures: usize, previous: Option<Section>) -> PracticeSetup {
        PracticeSetup {
            path: "a.dtx".into(),
            title: String::new(),
            artist: String::new(),
            notes_per_measure: vec![4; measures],
            lanes_per_measure: Vec::new(),
            weak: WeakMap {
                weakness: vec![None; measures],
                phrases: vec![
                    WeakPhrase {
                        first: 4,
                        last: 5,
                        weakness: 0.8,
                        notes: 8,
                    },
                    WeakPhrase {
                        first: 9,
                        last: 9,
                        weakness: 0.5,
                        notes: 4,
                    },
                    WeakPhrase {
                        first: 12,
                        last: 13,
                        weakness: 0.3,
                        notes: 8,
                    },
                ],
                plays: 1,
                practice_laps: 0,
            },
            clears: Vec::new(),
            picture: ChartPicture::default(),
            marks: Marks::new(),
            section: previous.or(Some(Section::new(4, 5))),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            step_laps: engine::practice::STEP_LAPS,
            way: Default::default(),
            part: Default::default(),
            build: Default::default(),
            fills: Vec::new(),
            seams: Vec::new(),
            weak_tour: Vec::new(),
            measure_ms: Vec::new(),
        }
    }

    fn clear(first: u32, last: u32) -> ClearRecord {
        ClearRecord {
            first,
            last,
            speed: 1.0,
            cleared_at: 0,
            overcome: None,
        }
    }

    #[test]
    fn opens_on_the_weakest_spot_so_deciding_starts_right_away() {
        let mut m = PracticeMenu::new(setup(16, None), None, None);
        assert_eq!(m.item(), Item::Spot(0));
        assert_eq!(m.shown(), Some(Section::new(4, 5)));
        assert!(matches!(
            m.input(MenuInput::Decide).done,
            Some(Done::Start(o)) if o.section == Section::new(4, 5)
        ));
    }

    #[test]
    fn moving_through_the_spots_changes_the_section() {
        let mut m = PracticeMenu::new(setup(16, None), None, None);
        m.input(MenuInput::Down);
        assert_eq!(m.shown(), Some(Section::new(9, 9)));
        // 苦手な所では SD・FT は何もしない
        assert_eq!(m.input(MenuInput::Right), Response::default());
        assert_eq!(m.shown(), Some(Section::new(9, 9)));
    }

    #[test]
    fn pick_your_own_moves_and_resizes_with_left_and_right() {
        let mut m = PracticeMenu::new(setup(16, None), None, None);
        m.cursor = m.item_index(Item::Section);
        m.sync();
        // 音符のある最初の小節から 4 小節
        assert_eq!(m.shown(), Some(Section::new(0, 3)));
        m.input(MenuInput::Right);
        assert_eq!(m.shown(), Some(Section::new(1, 4)));
        m.input(MenuInput::Down);
        assert_eq!(m.item(), Item::Length);
        m.input(MenuInput::Left);
        assert_eq!(m.shown(), Some(Section::new(1, 3)));
        assert!(matches!(
            m.input(MenuInput::Decide).done,
            Some(Done::Start(o)) if o.section == Section::new(1, 3)
        ));
    }

    #[test]
    fn a_previous_section_that_is_not_a_spot_opens_on_pick_your_own() {
        let m = PracticeMenu::new(setup(16, Some(Section::new(7, 8))), None, None);
        assert_eq!(m.item(), Item::Section);
        assert_eq!(m.shown(), Some(Section::new(7, 8)));
    }

    #[test]
    fn after_a_clear_the_cursor_moves_to_the_next_open_spot_without_starting() {
        let mut s = setup(16, None);
        s.clears = vec![clear(4, 5), clear(9, 9)];
        let m = PracticeMenu::new(
            s,
            None,
            Some(Finished {
                section: Section::new(4, 5),
                cleared: true,
                speed: 100,
            }),
        );
        // 9 もクリア済みなので 12〜13 へ
        assert_eq!(m.item(), Item::Spot(2));
        assert!(m.banner.is_some());
        // 途中でやめたときは同じ所のまま
        let m = PracticeMenu::new(
            setup(16, None),
            None,
            Some(Finished {
                section: Section::new(9, 9),
                cleared: false,
                speed: 100,
            }),
        );
        assert_eq!(m.item(), Item::Spot(1));
        assert!(m.banner.is_none());
    }

    #[test]
    fn spots_keep_their_order_when_coming_back() {
        let first = PracticeMenu::new(setup(16, None), None, None);
        let kept = first.spots().to_vec();
        // 練習で苦手度が下がり、記録からは 4〜5 が消えた
        let mut s = setup(16, None);
        s.weak.phrases.remove(0);
        let m = PracticeMenu::new(s, Some(kept), None);
        assert_eq!(m.spots().len(), 3);
        assert_eq!(m.spots()[0].section, Section::new(4, 5));
    }

    #[test]
    fn speed_and_looping_open_on_the_right_and_close_with_cancel() {
        let mut m = PracticeMenu::new(setup(16, None), None, None);
        m.cursor = m.item_index(Item::Settings);
        m.input(MenuInput::Decide);
        assert_eq!(m.settings, Some(0));
        // つなぎ方は ◀▶ で切り替わる
        m.input(MenuInput::Right);
        assert_eq!(m.setup.style, LoopStyle::Restart);
        // 始めの速度は 5% ずつ
        m.input(MenuInput::Down);
        m.input(MenuInput::Down);
        let before = m.setup.speed.first_speed();
        m.input(MenuInput::Right);
        assert_eq!(m.setup.speed.first_speed(), before + 5);
        // 閉じても練習は始まらない
        assert_eq!(m.input(MenuInput::Cancel).done, None);
        assert_eq!(m.settings, None);
        assert_eq!(m.item(), Item::Settings);
    }

    #[test]
    fn cancel_and_back_close_the_menu() {
        let mut m = PracticeMenu::new(setup(16, None), None, None);
        assert_eq!(m.input(MenuInput::Cancel).done, Some(Done::Close));
        m.cursor = m.item_index(Item::Back);
        assert_eq!(m.input(MenuInput::Decide).done, Some(Done::Close));
    }

    #[test]
    fn the_legend_follows_the_row_and_never_uses_the_feet() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        let mut m = PracticeMenu::new(setup(16, None), None, None);
        let names = |m: &PracticeMenu| m.hints().iter().map(|h| h.1).collect::<Vec<_>>();
        assert!(names(&m).contains(&"練習を始める（Enter）"));
        assert!(!names(&m).contains(&"←→ 区間を動かす"));
        m.cursor = m.item_index(Item::Section);
        assert!(names(&m).contains(&"←→ 区間を動かす"));
        assert!(m.hints().iter().all(|h| !h.0.contains(&Pad::BD)));
    }

    #[test]
    fn a_song_without_records_opens_on_pick_your_own() {
        let mut s = setup(8, None);
        s.weak.phrases.clear();
        s.section = Some(Section::new(0, 3));
        let m = PracticeMenu::new(s, None, None);
        assert_eq!(m.item(), Item::Section);
        assert_eq!(m.shown(), Some(Section::new(0, 3)));
    }

    #[test]
    fn the_mode_row_switches_ways_and_shows_the_part_or_direction() {
        use dtx::DrumLane;
        use engine::practice_way::{Part, lane_mask};
        let mut s = setup(16, None);
        s.lanes_per_measure = vec![lane_mask([DrumLane::SD, DrumLane::BD]); 16];
        let mut m = PracticeMenu::new(s, None, None);
        // やり方の行は苦手な所のすぐ下
        assert_eq!(m.items()[3], Item::Way);
        m.cursor = m.item_index(Item::Way);
        assert!(!m.items().contains(&Item::WayOption));
        m.input(MenuInput::Right);
        assert_eq!(m.setup.way, Way::Part);
        // パート練習は、叩くパートの行が出る（曲に音符のあるパートだけ）
        assert!(m.items().contains(&Item::WayOption));
        m.input(MenuInput::Down);
        assert_eq!(m.item(), Item::WayOption);
        assert_eq!(m.setup.part, Part::Hands);
        m.input(MenuInput::Right);
        assert_eq!(m.setup.part, Part::Feet);
        m.input(MenuInput::Right);
        assert_eq!(m.setup.part, Part::Snare);
        m.input(MenuInput::Right);
        assert_eq!(m.setup.part, Part::Hands);
        // 決めると、絵に出していた区間をそのやり方で始める
        assert!(matches!(
            m.input(MenuInput::Decide).done,
            Some(Done::Start(o)) if o.way == Way::Part && o.section == Section::new(4, 5)
        ));
        // 積み上げは向きの行
        m.cursor = m.item_index(Item::Way);
        for _ in 0..3 {
            m.input(MenuInput::Right);
        }
        assert_eq!(m.setup.way, Way::Build);
        m.input(MenuInput::Down);
        m.input(MenuInput::Left);
        assert_eq!(m.setup.build, Build::Backward);
        // 左の端からは右の端へ回る
        m.cursor = m.item_index(Item::Way);
        for _ in 0..4 {
            m.input(MenuInput::Left);
        }
        assert_eq!(m.setup.way, Way::Climb);
        m.input(MenuInput::Left);
        assert_eq!(m.setup.way, Way::Fade);
        assert!(!m.items().contains(&Item::WayOption));
    }

    #[test]
    fn touring_ways_list_the_stops_and_start_from_the_first() {
        let mut s = setup(16, None);
        s.fills = vec![Section::new(3, 3), Section::new(7, 7)];
        let mut m = PracticeMenu::new(s, None, None);
        m.cursor = m.item_index(Item::Way);
        // 苦手めぐり：苦手な所 3 か所を、まぜた順にめぐる
        for _ in 0..3 {
            m.input(MenuInput::Right);
        }
        assert_eq!(m.setup.way, Way::Tour);
        assert_eq!(m.item(), Item::Way);
        let mut tour: Vec<Section> = m.cards().iter().map(|c| c.section).collect();
        tour.sort_by_key(|s| s.first);
        assert_eq!(
            tour,
            vec![Section::new(4, 5), Section::new(9, 9), Section::new(12, 13)]
        );
        // 自分で選ぶ行は出ない
        assert_eq!(m.items().len(), 3 + 3);
        assert!(!m.items().contains(&Item::Section));
        // フィルだけ：札は見つけたフィル。2 か所目の札で決めても、1 か所目からめぐる（選んでいた区間は変えない）
        m.input(MenuInput::Right);
        m.input(MenuInput::Right);
        assert_eq!(m.setup.way, Way::Fills);
        assert_eq!(m.item(), Item::Way);
        assert_eq!(m.shown(), Some(Section::new(3, 3)));
        m.cursor = 1;
        m.sync();
        assert_eq!(m.shown(), Some(Section::new(7, 7)));
        let Some(Done::Start(o)) = m.input(MenuInput::Decide).done else {
            panic!("did not start");
        };
        assert_eq!(o.way, Way::Fills);
        assert_eq!(
            o.stops.as_slice(),
            &[Section::new(3, 3), Section::new(7, 7)]
        );
        assert_eq!(o.section, Section::new(4, 5));
        // つなぎ目：見つからなければ始められない
        m.cursor = m.item_index(Item::Way);
        m.input(MenuInput::Right);
        assert_eq!(m.setup.way, Way::Seams);
        assert!(m.cards().is_empty());
        assert_eq!(m.input(MenuInput::Decide).done, None);
        // めぐるやり方をやめると、絵は選んでいた区間に戻る
        m.input(MenuInput::Right);
        assert_eq!(m.setup.way, Way::Break);
        assert_eq!(m.shown(), Some(Section::new(4, 5)));
        assert!(m.items().contains(&Item::Section));
    }

    #[test]
    fn five_spots_and_every_row_fit_in_the_list() {
        let mut s = setup(16, None);
        for k in 0..2 {
            s.weak.phrases.push(engine::analysis::WeakPhrase {
                first: k,
                last: k,
                weakness: 0.2,
                notes: 4,
            });
        }
        s.way = Way::Part;
        let m = PracticeMenu::new(s, None, None);
        assert_eq!(m.spots().len(), 5);
        let last = m.items().len() - 1;
        let (y, h) = m.item_rect(last);
        assert!(y + h <= LIST.1 + LIST.3, "{} > {}", y + h, LIST.1 + LIST.3);
        let (_, card) = m.item_rect(0);
        assert!(card >= 54.0, "{card}");
    }
}
