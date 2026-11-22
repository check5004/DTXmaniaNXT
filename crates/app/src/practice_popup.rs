//! 練習の決めごとのポップアップ（選曲画面の上に出す）。
//!
//! 上流の選曲画面のポップアップメニュー（`CActSelectPopupMenu`：ソートやクイック設定）と同じ
//! 背景（`ScreenSelect sort menu background.png`）とカーソル（`ScreenConfig menu cursor.png`）で、
//! 区間、つなぎ方、速度（段）、音程、メトロノームを選ぶ（段の越え方と 1 段の周は設定の画面で）。行の下に、
//! のぼる段（「70 → 80 → 90 → 100%」）を出す。文字は日本語なので、DFP フォントの代わりに
//! 同じ大きさの白い太字（選んでいる行は橙）で出す。小節の帯と苦手なフレーズは、難易度パネルの場所に重ねて出す。
//! NX v2.0 の選曲画面は後ろの絵が細かく、半透明の板の文字が読みにくいので、開いているあいだは後ろを暗くする。
//! v2 の背景の絵は下の端に案内の吹き出しがあるので、選んでいる行の説明はその下に板を敷いて出す。
//!
//! 操作は上流のポップアップと同じ。↑↓（HT / LT）で選び、Enter（CY / RD）で決め、Esc（LC）で閉じる。
//! 数の項目は Enter で値を変える状態になり、↑↓ で変えて Enter で戻る。

use egui::{Color32, Pos2};
use engine::practice::{LoopStyle, STEP_SPEED, Section, SpeedPlan};
use i18n::{count, t, tf};

use crate::band::{Band, BandData};
use crate::gfx::{Canvas, Gfx, src};
use crate::practice::{PracticeOptions, PracticeSetup, pass_line, section_label, style_label};
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
    Speed,
    SpeedValue,
    Target,
    Pitch,
    Metronome,
    Start,
    Back,
}

const ROWS: [Row; 11] = [
    Row::Section,
    Row::Length,
    Row::Weak,
    Row::Loop,
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
                    tf!("{}–{}", "{}〜{}", x.first, x.last)
                }
            }),
            Row::Length => s.section.map_or("-".to_owned(), |x| measures(x.measures())),
            Row::Weak => match (self.weak_index(), s.weak.phrases.len()) {
                (_, 0) => "-".to_owned(),
                (Some(i), n) => format!("{}/{n}", i + 1),
                (None, n) => format!("-/{n}"),
            },
            Row::Loop => style_label(s.style).to_owned(),
            Row::Speed => match s.speed {
                SpeedPlan::Ladder { .. } => t("Step Up", "段階アップ"),
                SpeedPlan::Fixed(_) => t("Fixed", "固定"),
            }
            .to_owned(),
            Row::SpeedValue => format!("{}%", s.speed.first_speed()),
            Row::Target => match s.speed {
                SpeedPlan::Ladder { target, .. } => format!("{target}%"),
                SpeedPlan::Fixed(_) => "-".to_owned(),
            },
            Row::Pitch => match s.pitch {
                Pitch::Keep => t("Keep", "保つ"),
                Pitch::Tape => t("Lower", "一緒に下げる"),
            }
            .to_owned(),
            Row::Metronome => if s.metronome {
                t("On", "オン")
            } else {
                t("Off", "オフ")
            }
            .to_owned(),
            Row::Start | Row::Back => String::new(),
        }
    }

    fn name(&self, row: Row) -> &'static str {
        match row {
            Row::Section => t("Section", "区間"),
            Row::Length => t("Length", "長さ"),
            Row::Weak => t("Weak Phrases", "苦手なフレーズ"),
            Row::Loop => t("Looping", "つなぎ方"),
            Row::Speed => t("Speed Mode", "速度の決め方"),
            Row::SpeedValue => match self.setup.speed {
                SpeedPlan::Ladder { .. } => t("Start Speed", "始めの速度"),
                SpeedPlan::Fixed(_) => t("Speed", "速度"),
            },
            Row::Target => t("Target Speed", "目標の速度"),
            Row::Pitch => t("Pitch When Slowed", "遅くしたときの音程"),
            Row::Metronome => t("Metronome", "メトロノーム"),
            Row::Start => t("Start Practice", "練習を始める"),
            Row::Back => t("Back", "戻る"),
        }
    }

    /// 行の説明（ポップアップの下に出す）
    fn help(&self, row: Row) -> String {
        let s = &self.setup;
        match row {
            Row::Section => {
                t(
                    "The section to practice. Press Enter, then ↑↓ (or ←→) to move it one measure at a time. You can also click or drag on the measure strip.",
                    "練習する区間。Enter のあと ↑↓ で 1 小節ずつ動かします（←→ でも）。帯を押すかドラッグしても選べます。",
                )
                .to_owned()
            }
            Row::Length => t(
                "Length of the section in measures. Press Enter, then ↑↓ to change it.",
                "区間の長さ（小節）。Enter のあと ↑↓ で変えます。",
            )
            .to_owned(),
            Row::Weak => {
                if s.weak.phrases.is_empty() {
                    t(
                        "No weak phrases yet. They are found from your play records.",
                        "苦手なフレーズはまだありません（遊んだ記録から出します）。",
                    )
                    .to_owned()
                } else {
                    t(
                        "Press Enter to step through your weak phrases, weakest first.",
                        "Enter で苦手なフレーズを、苦手な順に選びます。",
                    )
                    .to_owned()
                }
            }
            Row::Loop => match s.style {
                LoopStyle::Connect => t(
                    "Seamless: goes straight from the end of the section back to its start. Good for grooves.",
                    "切れ目なく: 区間の終わりから切れ目なく頭へ戻ります（ビート向き）。",
                ),
                LoopStyle::Restart => t(
                    "Restart Each Loop: you play through the first hit of the next measure, then it restarts with a lead-in. Good for fills.",
                    "毎回区切る: 次の小節の頭の 1 打まで叩き、助走からやり直します（フィル向き）。",
                ),
            }
            .to_owned(),
            Row::Speed => match s.speed {
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
            Row::SpeedValue => t(
                "Speed of the first step. Press Enter, then ↑↓ to change it in 5% steps.",
                "はじめの段の速度。Enter のあと ↑↓ で 5% ずつ変えます。",
            )
            .to_owned(),
            Row::Target => t(
                "Speed of the top step. Pass it to clear. Press Enter, then ↑↓ to choose.",
                "いちばん上の段の速度。ここを越えたらクリア。Enter のあと ↑↓ で選びます。",
            )
            .to_owned(),
            Row::Pitch => match s.pitch {
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
            Row::Metronome => {
                t(
                    "When on, a soft click plays on every beat of the section (louder on the first beat of each measure). The 4-beat count-in before each step plays even when off.",
                    "オンなら区間のあいだ拍ごとに小さく鳴らします（小節の頭は強く）。段の頭の 4 拍のカウントは、オフでも鳴ります。",
                )
                .to_owned()
            }
            Row::Start => match s.section {
                Some(sec) if s.notes_in(sec) == 0 => {
                    t("This section has no notes.", "この区間には音符がありません。").to_owned()
                }
                _ => t(
                    "Start practicing with these settings.",
                    "この決めごとで練習を始めます。",
                )
                .to_owned(),
            },
            Row::Back => t("Return to song select.", "選曲に戻ります。").to_owned(),
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
        label(
            c,
            g,
            X + 96.0,
            Y + 4.0,
            t("Section Practice", "区間練習"),
            false,
            W - 110.0,
        );
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
        // のぼる段と、1 段の決まり（行の下）
        let y = Y + 46.0 + ROW_H * ROWS.len() as f32;
        let steps = self.setup.speed.steps();
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
        let accent = Style::edged(
            15.0,
            Color32::from_rgb(0xFF, 0xB8, 0x40),
            Color32::BLACK,
            1.5,
        );
        if let Some(t) = g.texts.get(&head, &accent) {
            t.draw_fit(c, X + 18.0, y, W - 36.0);
        }
        let grey = Style::edged(13.0, Color32::from_gray(200), Color32::BLACK, 1.0);
        let rule = tf!(
            "{} loops per step. Pass the {}% step to clear.",
            "1 段 {} 周。{}% の段を越えたらクリア",
            self.setup.step_laps,
            self.setup.speed.goal()
        );
        if let Some(t) = g.texts.get(&rule, &grey) {
            t.draw_fit(c, X + 18.0, y + 22.0, W - 36.0);
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
        let heading_w = label(
            c,
            g,
            px + 14.0,
            py + 8.0,
            t("Measures", "小節"),
            false,
            200.0,
        );
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
                Some(v) => tf!(
                    "Measure {m}: weakness {:.0}%",
                    "小節 {m}：苦手度 {:.0}%",
                    v * 100.0
                ),
                None => tf!("Measure {m}: no records", "小節 {m}：記録なし"),
            },
            (None, Some(sec)) => {
                let m = measures(sec.measures());
                let n = s.notes_in(sec);
                let notes = count(n, "note", "notes", "");
                tf!(
                    "{} ({m}, {notes})",
                    "区間 {}（{m}、音符 {n}）",
                    section_text(sec)
                )
            }
            (None, None) if n == 0 => t("No measures", "小節がありません").to_owned(),
            (None, None) => t("Choose a section", "区間を選んでください").to_owned(),
        };
        let big = Style::edged(18.0, Color32::WHITE, Color32::BLACK, 1.5);
        if let Some(t) = g.texts.get(&info, &big) {
            t.draw(c, BAND.x, y, 1.0, 1.0);
        }
        y += 28.0;
        // 苦手なフレーズ（今の区間は黄色）
        let small = Style::edged(14.0, Color32::from_gray(190), Color32::BLACK, 1.0);
        if let Some(t) = g.texts.get(
            &tf!("Weak phrases ({})", "苦手なフレーズ（{}）", weak_source(s)),
            &small,
        ) {
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
            let (n, notes) = (p.notes, count(p.notes, "note", "notes", ""));
            let line = tf!(
                "{}. {}   weakness {:.0}% ({notes})",
                "{}. {}　苦手度 {:.0}%（音符 {n}）",
                i + 1,
                section_text(Section::new(p.first, p.last)),
                p.weakness * 100.0
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
        (0, 0) => t("no records yet", "まだ記録がありません").to_owned(),
        (p, 0) => {
            let p = count(p, "play", "plays", "回");
            tf!("from your last {p}", "直近のプレイ {p}から")
        }
        (0, l) => {
            let l = count(l, "practice loop", "practice loops", "周");
            tf!("from your last {l}", "直近の練習 {l}から")
        }
        (p, l) => {
            let p = count(p, "play", "plays", "回");
            let l = count(l, "practice loop", "practice loops", "周");
            tf!(
                "from your last {p} and {l}",
                "直近のプレイ {p}と練習 {l}から"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::WeakMap;
    use engine::analysis::WeakPhrase;
    use engine::practice::PassRule;

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
            step_laps: engine::practice::STEP_LAPS,
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
