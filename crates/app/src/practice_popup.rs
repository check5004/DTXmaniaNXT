//! 練習の決めごとのポップアップ（選曲画面の上に出す）。
//!
//! 上流の選曲画面のポップアップメニュー（`CActSelectPopupMenu`：ソートやクイック設定）と同じ
//! 背景（`ScreenSelect sort menu background.png`）、カーソル（`ScreenConfig menu cursor.png`）、文字（DFP フォント）で、
//! 区間、つなぎ方、合格の基準、速度、音程を選ぶ。小節の帯と苦手なフレーズは、難易度パネルの場所に重ねて出す。
//!
//! 操作は上流のポップアップと同じ。↑↓（HT / LT）で選び、Enter（CY / RD）で決め、Esc（LC）で閉じる。
//! 数の項目は Enter で値を変える状態になり、↑↓ で変えて Enter で戻る。

use egui::{Color32, Pos2, Stroke, StrokeKind};
use engine::practice::{LoopStyle, PassRule, Section, SpeedPlan};

use crate::gfx::{Canvas, DFP, Gfx, src};
use crate::practice::{PracticeOptions, PracticeSetup};
use crate::sfx::Se;
use crate::stretch::Pitch;
use crate::text::Style;
use crate::ui::weakness_color;

/// ポップアップの左上と 1 行の高さ。上流のソートのポップアップは (460, 150) だが、
/// そこだと小節の帯に重なるので、曲のリストの上に出す
const X: f32 = 790.0;
const Y: f32 = 150.0;
const ROW_H: f32 = 32.0;
const W: f32 = 360.0;
const H: f32 = 450.0;

/// 難易度パネル（`5_status panel.png`）の場所と大きさ。小節の帯をここに重ねる
const PANEL: (f32, f32, f32, f32) = (130.0, 350.0, 561.0, 342.0);
/// 小節の帯の左と幅、上
const BAND_X: f32 = PANEL.0 + 16.0;
const BAND_W: f32 = PANEL.2 - 32.0;
const BAND_Y: f32 = PANEL.1 + 44.0;
/// 帯の 1 段の高さと、段の送り（下に小節の番号）
const CELL_H: f32 = 24.0;
const ROW_PITCH: f32 = 40.0;
/// 1 段に並べる小節の数の目安
const PER_ROW: usize = 64;

/// 選んだ区間の色
const SELECTION_COLOR: Color32 = Color32::from_rgb(0x40, 0xC8, 0xFF);
/// 練習でクリアした区間の印の色
const CLEARED_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);

/// ポップアップの行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Section,
    Length,
    Weak,
    Loop,
    Pass,
    Speed,
    SpeedValue,
    Target,
    Pitch,
    Start,
    Back,
}

const ROWS: [Row; 11] = [
    Row::Section,
    Row::Length,
    Row::Weak,
    Row::Loop,
    Row::Pass,
    Row::Speed,
    Row::SpeedValue,
    Row::Target,
    Row::Pitch,
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
            Row::Speed => s.toggle_ladder(),
            Row::Pitch => {
                let all = Pitch::ALL;
                let i = all.iter().position(|&p| p == s.pitch).unwrap_or(0);
                s.pitch = all[(i + 1) % all.len()];
            }
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
        self.hover = m.hover.and_then(|p| measure_at(n, p));
        if let Some(p) = m.pressed {
            if let Some(i) = row_at(p) {
                self.editing = false;
                self.cursor = i;
                return self.input(PopupInput::Decide);
            }
            self.drag = measure_at(n, p);
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
                    format!("{}-{}", x.first, x.last)
                }
            }),
            Row::Length => s.section.map_or("-".to_owned(), |x| {
                let m = x.measures();
                format!("{m} bar{}", if m == 1 { "" } else { "s" })
            }),
            Row::Weak => match (self.weak_index(), s.weak.phrases.len()) {
                (_, 0) => "-".to_owned(),
                (Some(i), n) => format!("{}/{n}", i + 1),
                (None, n) => format!("-/{n}"),
            },
            Row::Loop => match s.style {
                LoopStyle::Connect => "Connect",
                LoopStyle::Restart => "Restart",
            }
            .to_owned(),
            Row::Pass => match s.rule {
                PassRule::Easy => "Easy",
                PassRule::Normal => "Normal",
                PassRule::Strict => "Strict",
            }
            .to_owned(),
            Row::Speed => match s.speed {
                SpeedPlan::Ladder { .. } => "Ladder",
                SpeedPlan::Fixed(_) => "Fixed",
            }
            .to_owned(),
            Row::SpeedValue => format!("{}%", s.speed.first_speed()),
            Row::Target => match s.speed {
                SpeedPlan::Ladder { target, .. } => format!("{target}%"),
                SpeedPlan::Fixed(_) => "-".to_owned(),
            },
            Row::Pitch => match s.pitch {
                Pitch::Keep => "Keep",
                Pitch::Tape => "Tape",
            }
            .to_owned(),
            Row::Start | Row::Back => String::new(),
        }
    }

    fn name(&self, row: Row) -> &'static str {
        match row {
            Row::Section => "Section",
            Row::Length => "Length",
            Row::Weak => "Weak Phrase",
            Row::Loop => "Loop",
            Row::Pass => "Pass",
            Row::Speed => "Speed",
            Row::SpeedValue => match self.setup.speed {
                SpeedPlan::Ladder { .. } => "Start Speed",
                SpeedPlan::Fixed(_) => "Play Speed",
            },
            Row::Target => "Target",
            Row::Pitch => "Pitch",
            Row::Start => "START",
            Row::Back => "<< BACK",
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
                    "Connect: 区間の終わりから切れ目なく頭へ戻ります（ビート向き）。"
                }
                LoopStyle::Restart => {
                    "Restart: 次の小節の頭の 1 打まで叩き、助走からやり直します（フィル向き）。"
                }
            }
            .to_owned(),
            Row::Pass => match s.rule {
                PassRule::Easy => "Easy: MISS と POOR が 0 なら合格。",
                PassRule::Normal => "Normal: MISS と POOR が 0、PERFECT が 8 割以上で合格。",
                PassRule::Strict => {
                    "Strict: さらに PERFECT 9 割以上、空打ち 0、ばらつき 15ms 以下で合格。"
                }
            }
            .to_owned(),
            Row::Speed => match s.speed {
                SpeedPlan::Ladder { .. } => format!(
                    "Ladder: 合格が {} 周続くたびに 5% 上げ、目標の速度で続けて合格したらクリア。",
                    engine::practice::PASSES_TO_UP
                ),
                SpeedPlan::Fixed(_) => "Fixed: 同じ速度のまま回ります。".to_owned(),
            },
            Row::SpeedValue => "Enter のあと ↑↓ で 5% ずつ変えます（, と . でも）。".to_owned(),
            Row::Target => "段階アップの目標の速度。Enter のあと ↑↓ で選びます。".to_owned(),
            Row::Pitch => match s.pitch {
                Pitch::Keep => "Keep: 音程を保ったまま遅くします。",
                Pitch::Tape => "Tape: 音程ごと遅くします（テープと同じ）。",
            }
            .to_owned(),
            Row::Start => match s.section {
                Some(sec) if s.notes_in(sec) == 0 => "この区間には音符がありません。".to_owned(),
                _ => "この決めごとで練習を始めます。".to_owned(),
            },
            Row::Back => "選曲に戻ります。".to_owned(),
        }
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        self.draw_measures(c, g);
        // 背景
        match g
            .skin
            .tex_optional("Graphics/ScreenSelect sort menu background.png")
        {
            Some(t) => c.draw(&t, X, Y),
            None => c.fill(X, Y, W, H, Color32::from_rgb(30, 110, 170)),
        }
        // 題（上流は (+96, +4)）
        g.dfp(c, X + 96.0, Y + 4.0, "PRACTICE", false);
        // カーソル（上流：左の端 16、中を 16 ずつ 19 個、右の端 16）
        match g.skin.tex_optional("Graphics/ScreenConfig menu cursor.png") {
            Some(t) => {
                let y = Y + 6.0 + ROW_H * (self.cursor as f32 + 1.0);
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
                Y + 6.0 + ROW_H * (self.cursor as f32 + 1.0),
                336.0,
                32.0,
                Color32::from_rgba_unmultiplied(230, 220, 90, 120),
            ),
        }
        // 行（上流：名前は (+18, +40 + 32i)、値は (+200, …)。選んでいる行は強調の文字）
        for (i, &row) in ROWS.iter().enumerate() {
            let y = Y + 40.0 + ROW_H * i as f32;
            let selected = i == self.cursor;
            g.dfp(c, X + 18.0, y, self.name(row), selected && !self.editing);
            let v = self.value(row);
            if !v.is_empty() {
                g.dfp(c, X + 200.0, y, &v, selected);
            }
        }
        // 選んでいる行の説明
        let style = Style::edged(14.0, Color32::WHITE, Color32::BLACK, 1.5);
        let mut y = Y + 40.0 + ROW_H * ROWS.len() as f32 + 6.0;
        for line in g.texts.wrap(&self.help(self.row()), style.px, W - 36.0) {
            if y + 16.0 > Y + H - 4.0 {
                break;
            }
            if let Some(t) = g.texts.get(&line, &style) {
                t.draw(c, X + 18.0, y, 1.0, 1.0);
            }
            y += 17.0;
        }
    }

    /// 難易度パネルの場所に、小節の帯と苦手なフレーズを出す
    fn draw_measures(&self, c: &Canvas, g: &mut Gfx) {
        let s = &self.setup;
        let (px, py, pw, ph) = PANEL;
        // 下の難易度パネルの文字が透けないように、ほぼ不透明に
        c.fill(
            px,
            py,
            pw,
            ph,
            Color32::from_rgba_unmultiplied(10, 10, 18, 248),
        );
        c.frame(px, py, pw, ph, Color32::from_gray(150));
        g.dfp(c, px + 14.0, py + 8.0, "MEASURES", false);
        // 練習する曲（結果の画面から開いたときは、選曲のカーソルの曲と違うことがある）
        let song = if s.artist.is_empty() {
            s.title.clone()
        } else {
            format!("{} / {}", s.title, s.artist)
        };
        let left = px + 14.0 + DFP.width("MEASURES") + 16.0;
        let song_style = Style::edged(16.0, Color32::WHITE, Color32::BLACK, 1.0);
        if let Some(t) = g.texts.get(&song, &song_style) {
            let max_w = px + pw - 14.0 - left;
            t.draw_fit(c, (px + pw - 14.0 - t.w).max(left), py + 14.0, max_w);
        }
        let n = s.measure_count();
        let cleared: Vec<Section> = s.cleared_sections().iter().map(|c| c.0).collect();
        let bottom = draw_band(
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
            t.draw(c, BAND_X, y, 1.0, 1.0);
        }
        y += 28.0;
        // 苦手なフレーズ（今の区間は黄色）
        let small = Style::edged(14.0, Color32::from_gray(190), Color32::BLACK, 1.0);
        if let Some(t) = g
            .texts
            .get(&format!("苦手なフレーズ（{}）", weak_source(s)), &small)
        {
            t.draw(c, BAND_X, y, 1.0, 1.0);
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
                t.draw(c, BAND_X, y, 1.0, 1.0);
            }
            y += 21.0;
        }
    }
}

/// 帯に描くもの
pub struct BandData<'a> {
    /// 小節ごとの苦手度（記録が無ければ None）
    pub weakness: &'a [Option<f64>],
    /// 白い枠で囲む区間（苦手なフレーズ）
    pub phrases: Vec<Section>,
    /// 青く塗る区間
    pub selected: Option<Section>,
    /// 下に緑の印を付ける区間（練習でクリアした）
    pub cleared: Vec<Section>,
}

/// 帯の段の数と、1 段の小節の数、1 小節の幅
fn band_layout(n: usize) -> (usize, usize, f32) {
    let rows = n.div_ceil(PER_ROW).clamp(1, 4);
    let per_row = n.div_ceil(rows).max(1);
    (rows, per_row, BAND_W / per_row as f32)
}

/// 小節 `m` の左上
fn cell_pos(n: usize, m: usize) -> (f32, f32) {
    let (_, per_row, cw) = band_layout(n);
    let (r, col) = (m / per_row, m % per_row);
    (BAND_X + cw * col as f32, BAND_Y + ROW_PITCH * r as f32)
}

/// 帯の上の点にある小節
fn measure_at(n: usize, p: Pos2) -> Option<u32> {
    if n == 0 {
        return None;
    }
    let (rows, per_row, cw) = band_layout(n);
    let (dx, dy) = (p.x - BAND_X, p.y - BAND_Y);
    if !(0.0..BAND_W).contains(&dx) || dy < 0.0 {
        return None;
    }
    let r = (dy / ROW_PITCH) as usize;
    if r >= rows || dy - r as f32 * ROW_PITCH > CELL_H {
        return None;
    }
    let m = r * per_row + (dx / cw) as usize;
    (m < n).then_some(m as u32)
}

/// ポップアップの上の点にある行
fn row_at(p: Pos2) -> Option<usize> {
    if p.x < X + 12.0 || p.x > X + W - 12.0 {
        return None;
    }
    let i = ((p.y - (Y + 38.0)) / ROW_H).floor();
    (i >= 0.0 && (i as usize) < ROWS.len()).then_some(i as usize)
}

/// 小節の帯。苦手なほど赤く、記録の無い小節は暗い灰色。長い曲は 4 段まで折り返す。下の端の y を返す
pub fn draw_band(c: &Canvas, g: &mut Gfx, d: &BandData<'_>) -> f32 {
    let n = d.weakness.len();
    if n == 0 {
        return BAND_Y;
    }
    let (rows, per_row, cw) = band_layout(n);
    let gap = if cw >= 4.0 { 1.0 } else { 0.0 };
    for (m, v) in d.weakness.iter().enumerate() {
        let (x, y) = cell_pos(n, m);
        c.fill(x, y, cw - gap, CELL_H, weakness_color(*v));
    }
    let stroke = |x: f32, y: f32, w: f32, h: f32, color: Color32, width: f32| {
        c.painter().rect_stroke(
            c.rect(x, y, w, h),
            0.0,
            Stroke::new(width * c.scale(), color),
            StrokeKind::Outside,
        );
    };
    // 区間を段ごとに分けて (x, y, 幅) にする
    let spans = |s: Section| -> Vec<(f32, f32, f32)> {
        let (first, last) = (s.first as usize, (s.last as usize).min(n - 1));
        let mut out = Vec::new();
        let mut m = first;
        while m <= last {
            let row_end = (m / per_row + 1) * per_row - 1;
            let end = last.min(row_end);
            let (x, y) = cell_pos(n, m);
            out.push((x, y, cw * (end - m + 1) as f32 - gap));
            m = end + 1;
        }
        out
    };
    for &p in &d.phrases {
        for (x, y, w) in spans(p) {
            stroke(x, y, w, CELL_H, Color32::WHITE, 2.0);
        }
    }
    for &s in &d.cleared {
        for (x, y, w) in spans(s) {
            c.fill(x, y + CELL_H + 2.0, w, 3.0, CLEARED_COLOR);
        }
    }
    if let Some(s) = d.selected {
        for (x, y, w) in spans(s) {
            c.fill(
                x,
                y - 3.0,
                w,
                CELL_H + 6.0,
                SELECTION_COLOR.gamma_multiply(0.3),
            );
            stroke(x, y - 3.0, w, CELL_H + 6.0, SELECTION_COLOR, 2.5);
        }
    }
    // 小節の番号（DTX の小節番号）。重ならない間隔で
    let step = [1, 2, 4, 8, 16, 32, 64]
        .into_iter()
        .find(|&s| cw * s as f32 >= 32.0)
        .unwrap_or(128);
    let style = Style::plain(11.0, Color32::from_gray(170));
    for m in (0..n).step_by(step) {
        let (x, y) = cell_pos(n, m);
        if let Some(t) = g.texts.get(&m.to_string(), &style) {
            t.draw(c, x, y + CELL_H + 6.0, 1.0, 1.0);
        }
    }
    BAND_Y + ROW_PITCH * (rows - 1) as f32 + CELL_H + 20.0
}

fn section_text(s: Section) -> String {
    if s.first == s.last {
        format!("小節 {}", s.first)
    } else {
        format!("小節 {}〜{}", s.first, s.last)
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
    fn band_wraps_long_songs() {
        assert_eq!(band_layout(40), (1, 40, BAND_W / 40.0));
        assert_eq!(band_layout(150).0, 3);
        assert_eq!(band_layout(1000).0, 4);
        // 2 段目の頭の小節
        let (_, per_row, _) = band_layout(150);
        let (x, y) = cell_pos(150, per_row);
        assert_eq!(
            measure_at(150, Pos2::new(x + 1.0, y + 1.0)),
            Some(per_row as u32)
        );
        assert_eq!(measure_at(150, Pos2::new(BAND_X - 1.0, BAND_Y + 1.0)), None);
    }

    #[test]
    fn dragging_on_the_band_selects_a_range() {
        let mut p = popup(16);
        let at = |m: usize| {
            let (x, y) = cell_pos(16, m);
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
