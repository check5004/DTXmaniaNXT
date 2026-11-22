//! 練習の決めごとのポップアップ（選曲画面の上に出す）。
//!
//! 上流の選曲画面のポップアップメニュー（`CActSelectPopupMenu`：ソートやクイック設定）と同じ
//! 背景（`ScreenSelect sort menu background.png`）とカーソル（`ScreenConfig menu cursor.png`）で、
//! 区間、つなぎ方、苦手の見方、1 セットの周、速度、音程、メトロノームを選ぶ。文字は日本語なので、DFP フォントの代わりに
//! 同じ大きさの白い太字（選んでいる行は橙）で出す。小節の帯と苦手なフレーズは、難易度パネルの場所に重ねて出す。
//! NX v2.0 の選曲画面は後ろの絵が細かく、半透明の板の文字が読みにくいので、開いているあいだは後ろを暗くする。
//! v2 の背景の絵は下の端に案内の吹き出しがあるので、選んでいる行の説明はその下に板を敷いて出す。
//!
//! 操作は上流のポップアップと同じ。↑↓（HT / LT）で選び、Enter（CY / RD）で決め、Esc（LC）で閉じる。
//! 数の項目は Enter で値を変える状態になり、↑↓ で変えて Enter で戻る。

use egui::{Color32, Pos2};
use engine::practice::{LoopStyle, PassRule, SET_LAPS_CHOICES, SET_SPEED_STEP, Section, SpeedPlan};

use crate::band::{Band, BandData};
use crate::gfx::{Canvas, Gfx, src};
use crate::practice::{
    PracticeOptions, PracticeSetup, rule_detail, rule_label, section_label, style_label,
};
use crate::sfx::Se;
use crate::stretch::Pitch;
use crate::text::Style;

/// ポップアップの左上と 1 行の高さ。上流のソートのポップアップは (460, 150) だが、
/// そこだと小節の帯に重なるので、曲のリストの上に出す
const X: f32 = 790.0;
const Y: f32 = 150.0;
const ROW_H: f32 = 27.0;
const W: f32 = 360.0;
const H: f32 = 450.0;
/// 開いているあいだ、後ろの選曲画面を暗くする濃さ
const DIM: u8 = 150;
/// 説明の行の高さと、出す行の数の上限
const HELP_LINE_H: f32 = 17.0;
const HELP_LINES: usize = 4;

/// 難易度パネル（`5_status panel.png`）の場所と大きさ。小節の帯をここに重ねる
const PANEL: (f32, f32, f32, f32) = (130.0, 350.0, 561.0, 342.0);
/// 小節の帯（難易度パネルの中）
const BAND: Band = Band {
    x: PANEL.0 + 16.0,
    y: PANEL.1 + 44.0,
    w: PANEL.2 - 32.0,
    cell_h: 24.0,
    row_pitch: 40.0,
    max_rows: 4,
    striped: false,
};

/// ポップアップの行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Section,
    Length,
    Weak,
    Loop,
    Pass,
    SetLaps,
    Speed,
    SpeedValue,
    Target,
    Pitch,
    Metronome,
    Start,
    Back,
}

const ROWS: [Row; 13] = [
    Row::Section,
    Row::Length,
    Row::Weak,
    Row::Loop,
    Row::Pass,
    Row::SetLaps,
    Row::Speed,
    Row::SpeedValue,
    Row::Target,
    Row::Pitch,
    Row::Metronome,
    Row::Start,
    Row::Back,
];

impl Row {
    /// Enter で値を変える状態になる行
    fn editable(self) -> bool {
        matches!(
            self,
            Row::Section | Row::Length | Row::SpeedValue | Row::Target
        )
    }
}

/// 操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupInput {
    Up,
    Down,
    /// 区間を 1 小節前へ（どの行でも）
    Left,
    /// 区間を 1 小節後へ（どの行でも）
    Right,
    Decide,
    Cancel,
}

/// ポップアップを閉じるとき
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
pub struct PopupMouse {
    /// 押した場所
    pub pressed: Option<Pos2>,
    /// 押したまま
    pub down: bool,
    /// 指している場所
    pub hover: Option<Pos2>,
}

pub struct PracticePopup {
    pub setup: PracticeSetup,
    cursor: usize,
    editing: bool,
    /// 帯でドラッグを始めた小節
    drag: Option<u32>,
    /// 帯で指している小節
    hover: Option<u32>,
}

impl PracticePopup {
    pub fn new(setup: PracticeSetup) -> Self {
        Self {
            setup,
            cursor: 0,
            editing: false,
            drag: None,
            hover: None,
        }
    }

    fn row(&self) -> Row {
        ROWS[self.cursor]
    }

    /// 今の区間が何番目の苦手フレーズか
    fn weak_index(&self) -> Option<usize> {
        let s = self.setup.section?;
        self.setup
            .weak
            .phrases
            .iter()
            .position(|p| p.first == s.first && p.last == s.last)
    }

    pub fn input(&mut self, input: PopupInput) -> Response {
        let row = self.row();
        let weak = self.weak_index();
        let s = &mut self.setup;
        match input {
            PopupInput::Left | PopupInput::Right => {
                s.shift(if input == PopupInput::Left { -1 } else { 1 });
                return Response::se(Se::Move);
            }
            PopupInput::Up | PopupInput::Down if self.editing => {
                // 上流と同じく ↑ で値を増やす
                let d = if input == PopupInput::Up { 1 } else { -1 };
                match row {
                    Row::Section => s.shift(d),
                    Row::Length => s.resize(d),
                    Row::SpeedValue => s.nudge_speed(d),
                    Row::Target => {
                        if let SpeedPlan::Ladder { target, .. } = s.speed {
                            let t = SpeedPlan::TARGETS;
                            let i = t.iter().position(|&x| x == target).unwrap_or(0) as i64;
                            let next = (i + d).clamp(0, t.len() as i64 - 1) as usize;
                            s.set_target(t[next]);
                        }
                    }
                    _ => {}
                }
                return Response::se(Se::Move);
            }
            PopupInput::Up | PopupInput::Down => {
                let n = ROWS.len();
                self.cursor = if input == PopupInput::Up {
                    (self.cursor + n - 1) % n
                } else {
                    (self.cursor + 1) % n
                };
                return Response::se(Se::Move);
            }
            PopupInput::Cancel => {
                if self.editing {
                    self.editing = false;
                    return Response::se(Se::Cancel);
                }
                return Response {
                    se: Some(Se::Cancel),
                    done: Some(Done::Close),
                };
            }
            PopupInput::Decide => {}
        }
        if self.editing {
            self.editing = false;
            return Response::se(Se::Decide);
        }
        if row.editable() {
            // 段階アップでないときは目標が無い
            if row == Row::Target && !matches!(self.setup.speed, SpeedPlan::Ladder { .. }) {
                return Response::se(Se::Cancel);
            }
            self.editing = true;
            return Response::se(Se::Decide);
        }
        let s = &mut self.setup;
        match row {
            Row::Weak => {
                // 苦手なフレーズを順に選ぶ
                let phrases = &s.weak.phrases;
                if phrases.is_empty() {
                    return Response::se(Se::Cancel);
                }
                let next = weak.map_or(0, |i| (i + 1) % phrases.len());
                let p = &phrases[next];
                s.section = Some(Section::new(p.first, p.last));
            }
            Row::Loop => {
                s.style = match s.style {
                    LoopStyle::Connect => LoopStyle::Restart,
                    LoopStyle::Restart => LoopStyle::Connect,
                };
            }
            Row::Pass => {
                let all = PassRule::ALL;
                let i = all.iter().position(|&r| r == s.rule).unwrap_or(0);
                s.rule = all[(i + 1) % all.len()];
            }
            Row::SetLaps => {
                let all = SET_LAPS_CHOICES;
                let i = all.iter().position(|&n| n == s.set_laps).unwrap_or(0);
                s.set_laps = all[(i + 1) % all.len()];
            }
            Row::Speed => s.toggle_ladder(),
            Row::Pitch => {
                let all = Pitch::ALL;
                let i = all.iter().position(|&p| p == s.pitch).unwrap_or(0);
                s.pitch = all[(i + 1) % all.len()];
            }
            Row::Metronome => s.metronome = !s.metronome,
            Row::Start => {
                return match s.options() {
                    Some(o) => Response {
                        se: Some(Se::Decide),
                        done: Some(Done::Start(o)),
                    },
                    None => Response::se(Se::Cancel),
                };
            }
            Row::Back => {
                return Response {
                    se: Some(Se::Cancel),
                    done: Some(Done::Close),
                };
            }
            Row::Section | Row::Length | Row::SpeedValue | Row::Target => {}
        }
        Response::se(Se::Change)
    }

    /// マウス：行を押すとその行を決め、帯を押すかドラッグすると区間を選ぶ
    pub fn mouse(&mut self, m: PopupMouse) -> Response {
        let n = self.setup.measure_count();
        self.hover = m.hover.and_then(|p| BAND.measure_at(n, p));
        if let Some(p) = m.pressed {
            if let Some(i) = row_at(p) {
                self.editing = false;
                self.cursor = i;
                return self.input(PopupInput::Decide);
            }
            self.drag = BAND.measure_at(n, p);
        }
        if let Some(from) = self.drag {
            if let Some(to) = self.hover {
                self.setup.section = Some(Section::new(from.min(to), from.max(to)));
            }
            if !m.down {
                self.drag = None;
                return Response::se(Se::Change);
            }
        }
        Response::default()
    }

    /// 行の値
    fn value(&self, row: Row) -> String {
        let s = &self.setup;
        match row {
            Row::Section => s.section.map_or("-".to_owned(), |x| {
                if x.first == x.last {
                    x.first.to_string()
                } else {
                    format!("{}〜{}", x.first, x.last)
                }
            }),
            Row::Length => s
                .section
                .map_or("-".to_owned(), |x| format!("{} 小節", x.measures())),
            Row::Weak => match (self.weak_index(), s.weak.phrases.len()) {
                (_, 0) => "-".to_owned(),
                (Some(i), n) => format!("{}/{n}", i + 1),
                (None, n) => format!("-/{n}"),
            },
            Row::Loop => style_label(s.style).to_owned(),
            Row::Pass => rule_label(s.rule).to_owned(),
            Row::SetLaps => format!("{} 周", s.set_laps),
            Row::Speed => match s.speed {
                SpeedPlan::Ladder { .. } => "段階アップ",
                SpeedPlan::Fixed(_) => "固定",
            }
            .to_owned(),
            Row::SpeedValue => format!("{}%", s.speed.first_speed()),
            Row::Target => match s.speed {
                SpeedPlan::Ladder { target, .. } => format!("{target}%"),
                SpeedPlan::Fixed(_) => "-".to_owned(),
            },
            Row::Pitch => match s.pitch {
                Pitch::Keep => "保つ",
                Pitch::Tape => "一緒に下げる",
            }
            .to_owned(),
            Row::Metronome => if s.metronome { "オン" } else { "オフ" }.to_owned(),
            Row::Start | Row::Back => String::new(),
        }
    }

    fn name(&self, row: Row) -> &'static str {
        match row {
            Row::Section => "区間",
            Row::Length => "長さ",
            Row::Weak => "苦手なフレーズ",
            Row::Loop => "つなぎ方",
            Row::Pass => "苦手の見方",
            Row::SetLaps => "1 セットの周",
            Row::Speed => "速度の決め方",
            Row::SpeedValue => match self.setup.speed {
                SpeedPlan::Ladder { .. } => "始めの速度",
                SpeedPlan::Fixed(_) => "速度",
            },
            Row::Target => "目標の速度",
            Row::Pitch => "遅くしたときの音程",
            Row::Metronome => "メトロノーム",
            Row::Start => "練習を始める",
            Row::Back => "戻る",
        }
    }

    /// 行の説明（ポップアップの下に出す）
    fn help(&self, row: Row) -> String {
        let s = &self.setup;
        match row {
            Row::Section => {
                "練習する区間。Enter のあと ↑↓ で 1 小節ずつ動かします（←→ でも）。帯を押すかドラッグしても選べます。"
                    .to_owned()
            }
            Row::Length => "区間の長さ（小節）。Enter のあと ↑↓ で変えます。".to_owned(),
            Row::Weak => {
                if s.weak.phrases.is_empty() {
                    "苦手なフレーズはまだありません（遊んだ記録から出します）。".to_owned()
                } else {
                    "Enter で苦手なフレーズを、苦手な順に選びます。".to_owned()
                }
            }
            Row::Loop => match s.style {
                LoopStyle::Connect => {
                    "切れ目なく: 区間の終わりから切れ目なく頭へ戻ります（ビート向き）。"
                }
                LoopStyle::Restart => {
                    "毎回区切る: 次の小節の頭の 1 打まで叩き、助走からやり直します（フィル向き）。"
                }
            }
            .to_owned(),
            Row::Pass => format!(
                "{}: 1 セットで{}とみなし、その所だけの練習をすすめます。1 回きりのミスはうっかりとして数えません。",
                rule_label(s.rule),
                rule_detail(s.rule)
            ),
            Row::SetLaps => format!(
                "{} 周ごとに止まって、落とした所と次の一手（苦手の所だけ、速度を上げる・下げる）を出します。Enter で選びます。",
                s.set_laps
            ),
            Row::Speed => match s.speed {
                SpeedPlan::Ladder { .. } => format!(
                    "段階アップ: 苦手の無いセットのあとは {step}% 上げるのを、たくさん落としたら {step}% 下げるのをすすめます。目標の速度で苦手の無いセットになったらクリア。",
                    step = SET_SPEED_STEP,
                ),
                SpeedPlan::Fixed(_) => {
                    "固定: 速度を変えるのはすすめません。セットの合間に自分で上げ下げできます（練習中も , と . で 5% ずつ）。"
                        .to_owned()
                }
            },
            Row::SpeedValue => "Enter のあと ↑↓ で 5% ずつ変えます（, と . でも）。".to_owned(),
            Row::Target => "段階アップの目標の速度。Enter のあと ↑↓ で選びます。".to_owned(),
            Row::Pitch => match s.pitch {
                Pitch::Keep => "保つ: 音程を保ったまま遅くします。",
                Pitch::Tape => "一緒に下げる: 音程ごと遅くします（テープと同じ）。",
            }
            .to_owned(),
            Row::Metronome => {
                "オンなら区間のあいだ拍ごとに小さく鳴らします（小節の頭は強く）。セットの頭と速度が変わる周の前の 4 拍のカウントは、オフでも鳴ります。"
                    .to_owned()
            }
            Row::Start => match s.section {
                Some(sec) if s.notes_in(sec) == 0 => "この区間には音符がありません。".to_owned(),
                _ => "この決めごとで練習を始めます。".to_owned(),
            },
            Row::Back => "選曲に戻ります。".to_owned(),
        }
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        c.cover(Color32::from_black_alpha(DIM));
        self.draw_measures(c, g);
        // 背景（絵の下の端を覚えておき、説明はその下に出す）。v2 の絵は半透明で後ろの曲の帯が透けるので、
        // 行のある所の下にもう 1 枚暗い板を敷く
        c.fill(
            X,
            Y,
            W,
            40.0 + ROW_H * ROWS.len() as f32 + 6.0,
            Color32::from_black_alpha(DIM),
        );
        let bottom = match g
            .skin
            .tex_optional("Graphics/ScreenSelect sort menu background.png")
        {
            Some(t) => {
                c.draw(&t, X, Y);
                Y + t.h
            }
            None => {
                c.fill(X, Y, W, H, Color32::from_rgb(30, 110, 170));
                Y + H
            }
        };
        // 題（上流は (+96, +4)）
        label(c, g, X + 96.0, Y + 4.0, "区間練習", false, W - 110.0);
        // カーソル（上流：左の端 16、中を 16 ずつ 19 個、右の端 16）
        match g.skin.tex_optional("Graphics/ScreenConfig menu cursor.png") {
            Some(t) => {
                let y = Y + 9.0 + ROW_H * (self.cursor as f32 + 1.0);
                let h = t.h.min(32.0);
                let mut x = X + 12.0;
                c.part(&t, x, y, src(0.0, 0.0, 16.0, h));
                x += 16.0;
                for _ in 0..19 {
                    c.part(&t, x, y, src(8.0, 0.0, 16.0, h));
                    x += 16.0;
                }
                c.part(&t, x, y, src(16.0, 0.0, 16.0, h));
            }
            None => c.fill(
                X + 12.0,
                Y + 9.0 + ROW_H * (self.cursor as f32 + 1.0),
                336.0,
                ROW_H,
                Color32::from_rgba_unmultiplied(230, 220, 90, 120),
            ),
        }
        // 行（上流：名前は (+18, +40 + 32i)、値は (+200, …)。選んでいる行は強調の文字）
        for (i, &row) in ROWS.iter().enumerate() {
            let y = Y + 40.0 + ROW_H * i as f32;
            let selected = i == self.cursor;
            label(
                c,
                g,
                X + 18.0,
                y,
                self.name(row),
                selected && !self.editing,
                176.0,
            );
            let v = self.value(row);
            if !v.is_empty() {
                label(c, g, X + 200.0, y, &v, selected, W - 214.0);
            }
        }
        // 選んでいる行の説明（背景の絵の下に、結果の画面の板と同じ暗い板を敷いて）
        let style = Style::edged(14.0, Color32::WHITE, Color32::BLACK, 1.5);
        let lines: Vec<String> = g
            .texts
            .wrap(&self.help(self.row()), style.px, W - 24.0)
            .into_iter()
            .take(HELP_LINES)
            .collect();
        if !lines.is_empty() {
            let top = bottom + 4.0;
            let h = 10.0 + HELP_LINE_H * lines.len() as f32;
            c.fill(X, top, W, h, Color32::from_rgba_unmultiplied(8, 8, 16, 225));
            c.frame(X, top, W, h, Color32::from_gray(150));
            for (i, line) in lines.iter().enumerate() {
                if let Some(t) = g.texts.get(line, &style) {
                    t.draw(c, X + 12.0, top + 5.0 + HELP_LINE_H * i as f32, 1.0, 1.0);
                }
            }
        }
    }

    /// 難易度パネルの場所に、小節の帯と苦手なフレーズを出す
    fn draw_measures(&self, c: &Canvas, g: &mut Gfx) {
        let s = &self.setup;
        let (px, py, pw, ph) = PANEL;
        // 下の難易度パネルの文字が透けないように、不透明に
        c.fill(px, py, pw, ph, Color32::from_rgb(10, 10, 18));
        c.frame(px, py, pw, ph, Color32::from_gray(150));
        let heading_w = label(c, g, px + 14.0, py + 8.0, "小節", false, 200.0);
        // 練習する曲（結果の画面から開いたときは、選曲のカーソルの曲と違うことがある）
        let song = if s.artist.is_empty() {
            s.title.clone()
        } else {
            format!("{} / {}", s.title, s.artist)
        };
        let left = px + 14.0 + heading_w + 16.0;
        let song_style = Style::edged(16.0, Color32::WHITE, Color32::BLACK, 1.0);
        if let Some(t) = g.texts.get(&song, &song_style) {
            let max_w = px + pw - 14.0 - left;
            t.draw_fit(c, (px + pw - 14.0 - t.w).max(left), py + 14.0, max_w);
        }
        let n = s.measure_count();
        let cleared: Vec<Section> = s.cleared_sections().iter().map(|c| c.0).collect();
        let bottom = BAND.draw(
            c,
            g,
            &BandData {
                weakness: &s.weak.weakness,
                phrases: s
                    .weak
                    .phrases
                    .iter()
                    .map(|p| Section::new(p.first, p.last))
                    .collect(),
                selected: s.section,
                cleared,
            },
        );
        // 区間の中身、または指している小節
        let mut y = bottom + 8.0;
        let info = match (self.hover, s.section) {
            (Some(m), _) => match s.weak.weakness.get(m as usize).copied().flatten() {
                Some(v) => format!("小節 {m}：苦手度 {:.0}%", v * 100.0),
                None => format!("小節 {m}：記録なし"),
            },
            (None, Some(sec)) => format!(
                "区間 {}（{} 小節、音符 {}）",
                section_text(sec),
                sec.measures(),
                s.notes_in(sec)
            ),
            (None, None) if n == 0 => "小節がありません".to_owned(),
            (None, None) => "区間を選んでください".to_owned(),
        };
        let big = Style::edged(18.0, Color32::WHITE, Color32::BLACK, 1.5);
        if let Some(t) = g.texts.get(&info, &big) {
            t.draw(c, BAND.x, y, 1.0, 1.0);
        }
        y += 28.0;
        // 苦手なフレーズ（今の区間は黄色）
        let small = Style::edged(14.0, Color32::from_gray(190), Color32::BLACK, 1.0);
        if let Some(t) = g
            .texts
            .get(&format!("苦手なフレーズ（{}）", weak_source(s)), &small)
        {
            t.draw(c, BAND.x, y, 1.0, 1.0);
        }
        y += 20.0;
        let current = self.weak_index();
        for (i, p) in s.weak.phrases.iter().enumerate() {
            if y + 20.0 > py + ph - 6.0 {
                break;
            }
            let color = if current == Some(i) {
                Color32::from_rgb(255, 220, 0)
            } else {
                Color32::from_gray(225)
            };
            let line = format!(
                "{}. {}　苦手度 {:.0}%（音符 {}）",
                i + 1,
                section_text(Section::new(p.first, p.last)),
                p.weakness * 100.0,
                p.notes
            );
            if let Some(t) = g
                .texts
                .get(&line, &Style::edged(16.0, color, Color32::BLACK, 1.0))
            {
                t.draw(c, BAND.x, y, 1.0, 1.0);
            }
            y += 21.0;
        }
    }
}

/// ポップアップの上の点にある行
fn row_at(p: Pos2) -> Option<usize> {
    if p.x < X + 12.0 || p.x > X + W - 12.0 {
        return None;
    }
    let i = ((p.y - (Y + 38.0)) / ROW_H).floor();
    (i >= 0.0 && (i as usize) < ROWS.len()).then_some(i as usize)
}

fn section_text(s: Section) -> String {
    section_label(s)
}

/// ポップアップの行の文字。上流の DFP フォントと同じ高さの白い太字（強調は橙）。幅が `max_w` を超えるときは
/// 横に縮める。描いた幅を返す
fn label(c: &Canvas, g: &mut Gfx, x: f32, y: f32, text: &str, emphasized: bool, max_w: f32) -> f32 {
    let color = if emphasized {
        Color32::from_rgb(255, 170, 0)
    } else {
        Color32::WHITE
    };
    let style = Style {
        bold: true,
        ..Style::edged(18.0, color, Color32::BLACK, 2.0)
    };
    match g.texts.get(text, &style) {
        Some(t) => {
            t.draw_fit(c, x, y + 3.0, max_w);
            t.w.min(max_w)
        }
        None => 0.0,
    }
}

/// 苦手度をどの記録から出したか
fn weak_source(s: &PracticeSetup) -> String {
    match (s.weak.plays, s.weak.practice_laps) {
        (0, 0) => "まだ記録がありません".to_owned(),
        (p, 0) => format!("直近のプレイ {p} 回から"),
        (0, l) => format!("直近の練習 {l} 周から"),
        (p, l) => format!("直近のプレイ {p} 回と練習 {l} 周から"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::WeakMap;
    use engine::analysis::WeakPhrase;

    fn popup(measures: usize) -> PracticePopup {
        PracticePopup::new(PracticeSetup {
            path: "a.dtx".into(),
            title: String::new(),
            artist: String::new(),
            notes_per_measure: vec![4; measures],
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
                ],
                plays: 1,
                practice_laps: 0,
            },
            clears: Vec::new(),
            section: Some(Section::new(0, 3)),
            style: LoopStyle::Connect,
            rule: PassRule::Normal,
            speed: SpeedPlan::default(),
            pitch: Pitch::Keep,
            metronome: true,
            set_laps: engine::practice::SET_LAPS,
        })
    }

    #[test]
    fn editing_changes_values_with_up_and_down() {
        let mut p = popup(16);
        // Section を値を変える状態にして ↑ で 1 小節後へ
        p.input(PopupInput::Decide);
        p.input(PopupInput::Up);
        assert_eq!(p.setup.section, Some(Section::new(1, 4)));
        // Enter で戻って Length へ
        p.input(PopupInput::Decide);
        p.input(PopupInput::Down);
        assert_eq!(p.row(), Row::Length);
        p.input(PopupInput::Decide);
        p.input(PopupInput::Down);
        assert_eq!(p.setup.section, Some(Section::new(1, 3)));
        // Esc は値の変更をやめるだけ
        assert_eq!(p.input(PopupInput::Cancel).done, None);
        assert_eq!(p.input(PopupInput::Cancel).done, Some(Done::Close));
    }

    #[test]
    fn weak_phrases_are_picked_in_order() {
        let mut p = popup(16);
        p.cursor = ROWS.iter().position(|&r| r == Row::Weak).unwrap();
        p.input(PopupInput::Decide);
        assert_eq!(p.setup.section, Some(Section::new(4, 5)));
        assert_eq!(p.value(Row::Weak), "1/2");
        p.input(PopupInput::Decide);
        assert_eq!(p.setup.section, Some(Section::new(9, 9)));
        p.input(PopupInput::Decide);
        assert_eq!(p.setup.section, Some(Section::new(4, 5)));
    }

    #[test]
    fn start_needs_notes() {
        let mut p = popup(8);
        p.cursor = ROWS.iter().position(|&r| r == Row::Start).unwrap();
        assert!(matches!(
            p.input(PopupInput::Decide).done,
            Some(Done::Start(o)) if o.section == Section::new(0, 3)
        ));
        p.setup.notes_per_measure = vec![0; 8];
        assert_eq!(p.input(PopupInput::Decide).done, None);
    }

    #[test]
    fn target_only_with_the_ladder() {
        let mut p = popup(8);
        p.cursor = ROWS.iter().position(|&r| r == Row::Target).unwrap();
        p.setup.speed = SpeedPlan::Fixed(80);
        p.input(PopupInput::Decide);
        assert!(!p.editing);
        p.setup.speed = SpeedPlan::Ladder {
            start: 70,
            target: 100,
        };
        p.input(PopupInput::Decide);
        p.input(PopupInput::Up);
        assert!(matches!(
            p.setup.speed,
            SpeedPlan::Ladder { target: 105, .. }
        ));
        // 一番下より下げない
        p.input(PopupInput::Down);
        p.input(PopupInput::Down);
        assert!(matches!(
            p.setup.speed,
            SpeedPlan::Ladder { target: 100, .. }
        ));
    }

    #[test]
    fn dragging_on_the_band_selects_a_range() {
        let mut p = popup(16);
        let at = |m: usize| {
            let (x, y) = BAND.cell_pos(16, m);
            Pos2::new(x + 2.0, y + 2.0)
        };
        p.mouse(PopupMouse {
            pressed: Some(at(6)),
            down: true,
            hover: Some(at(6)),
        });
        p.mouse(PopupMouse {
            pressed: None,
            down: true,
            hover: Some(at(3)),
        });
        assert_eq!(p.setup.section, Some(Section::new(3, 6)));
        p.mouse(PopupMouse {
            pressed: None,
            down: false,
            hover: Some(at(3)),
        });
        assert!(p.drag.is_none());
    }
}
