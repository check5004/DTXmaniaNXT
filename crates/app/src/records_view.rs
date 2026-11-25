//! 上達の記録の画面（見本の案 A「1 画面にまとめる」、2026-11-25）。
//!
//! - 上: 練習した時間（今日・今週・続けた日・これまでの数字と、日ごとの積み上げ棒。曲・区間練習・ほかを色で分ける）
//! - 左下: 曲ごとの達成率の一覧（最近遊んだ順。ベストと、初回からベストまでの伸び）
//! - 右下: 選んでいる曲の達成率の折れ線と、ずれのばらつき。区間練習でクリアした所に印を立てる
//!
//! 比べる相手は前の自分だけ。伸びはベストで比べ（調子の悪い日で減らない）、「続けて」は 2 日以上のときだけ出す。
//! ドラムでは HT・LT で曲を選び、SD・FT で期間を変え、CY でその曲を選曲で選び、LC で戻る。

use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, Shape, Stroke, StrokeKind};
use i18n::{t, tf};
use records::{ChartProgress, PlayTime};

use crate::gfx::{Canvas, Gfx};
use crate::progress::{self, SongTrend, Spent, TimeLog, duration_text};
use crate::settings_ui::{self as ui, Hint, Pad};
use crate::songs::SongList;
use crate::text::Style;

/// 上の板（練習した時間）、左下の板（曲の一覧）、右下の板（選んでいる曲）
const TOP: Rect = (40.0, 56.0, 1200.0, 236.0);
const LIST: Rect = (40.0, 304.0, 470.0, 380.0);
const CHART: Rect = (524.0, 304.0, 716.0, 380.0);
/// 棒グラフの場所
const BARS: Rect = (448.0, 104.0, 772.0, 150.0);
/// 一覧の行の上の端、高さ、間、見える行の数
const ROW_Y: f32 = 344.0;
const ROW_H: f32 = 60.0;
const ROW_GAP: f32 = 6.0;
const ROWS: usize = 5;
/// 折れ線に出すプレイの数（多いときは新しいほうから）
const MAX_POINTS: usize = 30;

type Rect = (f32, f32, f32, f32);

const PANEL_FILL: Color32 = Color32::from_rgb(10, 10, 18);
const ORANGE: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
const PLAY: Color32 = Color32::from_rgb(88, 200, 120);
const PRACTICE: Color32 = Color32::from_rgb(255, 154, 31);
const OTHER: Color32 = Color32::from_rgb(90, 170, 255);
const LINE: Color32 = Color32::from_rgb(120, 200, 255);
const SPREAD: Color32 = Color32::from_rgb(180, 150, 255);
const GRID: Color32 = Color32::from_rgb(52, 50, 64);

/// 練習した時間の棒グラフの期間
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// 直近 4 週間（日ごと）
    Weeks4,
    /// 直近 12 週間（日ごと）
    Weeks12,
    /// すべて（週ごと）
    All,
}

impl Period {
    const ALL: [Period; 3] = [Period::Weeks4, Period::Weeks12, Period::All];

    fn label(self) -> &'static str {
        match self {
            Period::Weeks4 => t("Last 4 Weeks", "直近 4 週間"),
            Period::Weeks12 => t("Last 12 Weeks", "直近 12 週間"),
            Period::All => t("All Time (by Week)", "すべて（週ごと）"),
        }
    }
}

/// 画面への操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordsInput {
    Up,
    Down,
    Left,
    Right,
    Decide,
    Cancel,
}

/// 操作の結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordsAction {
    None,
    /// 曲を選び直した
    Moved,
    /// 期間を変えた
    Changed,
    /// 選曲でこの譜面を選ぶ
    Pick(PathBuf),
    /// 戻る
    Back,
}

/// どこから開いたか（戻る先）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordsFrom {
    Title,
    Select,
}

pub struct RecordsView {
    pub from: RecordsFrom,
    log: TimeLog,
    today: NaiveDate,
    songs: Vec<SongTrend>,
    cursor: usize,
    /// 一覧の一番上に見えている行
    top: usize,
    period: Period,
    /// 記録を読めなかった（記録のファイルを開けない）
    unavailable: bool,
}

impl RecordsView {
    /// 記録から作る。`data` が None なら記録が読めなかった。`focus` の譜面があれば、その行を選んでおく
    pub fn new(
        from: RecordsFrom,
        data: Option<(Vec<PlayTime>, Vec<ChartProgress>)>,
        list: &SongList,
        focus: Option<&Path>,
    ) -> Self {
        let unavailable = data.is_none();
        let (times, charts) = data.unwrap_or_default();
        let songs: Vec<SongTrend> = charts
            .into_iter()
            .map(|record| {
                let found = list.find_chart(Path::new(&record.path));
                let label = found.map_or_else(String::new, |(level, c)| {
                    if c.label.is_empty() {
                        library::sort::DIFFICULTY_NAMES[level].to_owned()
                    } else {
                        c.label.clone()
                    }
                });
                SongTrend {
                    level: found.and_then(|(_, c)| c.head.level),
                    label,
                    record,
                }
            })
            .collect();
        let cursor = focus
            .and_then(|f| songs.iter().position(|s| Path::new(&s.record.path) == f))
            .unwrap_or(0);
        let mut view = Self {
            from,
            log: TimeLog::new(&times, progress::local_day),
            today: progress::today(),
            songs,
            cursor,
            top: 0,
            period: Period::Weeks4,
            unavailable,
        };
        view.scroll_to_cursor();
        view
    }

    pub fn input(&mut self, input: RecordsInput) -> RecordsAction {
        match input {
            RecordsInput::Up | RecordsInput::Down => {
                let n = self.songs.len();
                if n == 0 {
                    return RecordsAction::None;
                }
                let next = if input == RecordsInput::Up {
                    self.cursor.checked_sub(1)
                } else {
                    (self.cursor + 1 < n).then_some(self.cursor + 1)
                };
                match next {
                    Some(i) => {
                        self.cursor = i;
                        self.scroll_to_cursor();
                        RecordsAction::Moved
                    }
                    None => RecordsAction::None,
                }
            }
            RecordsInput::Left | RecordsInput::Right => {
                let i = Period::ALL
                    .iter()
                    .position(|&p| p == self.period)
                    .unwrap_or(0);
                let next = if input == RecordsInput::Left {
                    i.checked_sub(1)
                } else {
                    (i + 1 < Period::ALL.len()).then_some(i + 1)
                };
                match next {
                    Some(i) => {
                        self.period = Period::ALL[i];
                        RecordsAction::Changed
                    }
                    None => RecordsAction::None,
                }
            }
            RecordsInput::Decide => self
                .songs
                .get(self.cursor)
                .map_or(RecordsAction::None, |s| {
                    RecordsAction::Pick(PathBuf::from(&s.record.path))
                }),
            RecordsInput::Cancel => RecordsAction::Back,
        }
    }

    fn scroll_to_cursor(&mut self) {
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + ROWS {
            self.top = self.cursor + 1 - ROWS;
        }
    }

    /// 画面の下の操作の案内
    pub fn hints(&self) -> Vec<Hint<'static>> {
        let mut v: Vec<Hint<'static>> = vec![
            (&[Pad::HT], t("Up", "上")),
            (&[Pad::LT], t("Down", "下")),
            (&[Pad::SD, Pad::FT], t("Period (←→)", "期間（←→）")),
        ];
        if !self.songs.is_empty() {
            v.push((
                &[Pad::CY],
                t("Pick This Song (Enter)", "この曲を選ぶ（Enter）"),
            ));
        }
        v.push((&[Pad::LC], t("Back (Esc)", "戻る（Esc）")));
        v
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        match g.skin.tex("Graphics/4_background.png") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(20, 18, 30)),
        }
        c.cover(Color32::from_black_alpha(150));
        g.heading(c, 40.0, 14.0, t("Records", "上達の記録"));
        ui::text_right(
            c,
            g,
            self.period.label(),
            1240.0,
            20.0,
            &Style::edged(16.0, ui::DIM, Color32::BLACK, 1.0),
            400.0,
        );
        self.draw_time(c, g);
        self.draw_list(c, g);
        self.draw_song(c, g);
        ui::legend(c, g, &self.hints());
    }

    /// 上の板: 練習した時間
    fn draw_time(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = TOP;
        panel(c, TOP);
        let head = ui::bold(18.0, Color32::WHITE);
        let tw = ui::text(
            c,
            g,
            t("Practice Time", "練習した時間"),
            x + 18.0,
            y + 9.0,
            &head,
            300.0,
        );
        let total = self.log.total();
        let mut keys = vec![
            (PLAY, t("Songs", "曲")),
            (PRACTICE, t("Section Practice", "区間練習")),
        ];
        if total.other > 0 {
            keys.push((OTHER, t("Other", "そのほか")));
        }
        color_keys(c, g, x + 18.0 + tw + 24.0, y + 13.0, &keys);
        let today = self.log.day(self.today);
        let (week, week_days) = self.log.this_week(self.today);
        let streak = self.log.streak(self.today);
        let (third_label, third_value, third_color) = if streak >= 2 {
            (t("Streak", "続けて"), days_text(streak), ui::STRONG)
        } else {
            (
                t("Days Played", "遊んだ日"),
                days_text(self.log.played_days()),
                ui::TEXT,
            )
        };
        let tiles = [
            (
                t("Today", "今日"),
                String::new(),
                duration_text(today.total()),
                ui::TEXT,
            ),
            (
                t("This Week", "今週"),
                if week_days > 0 {
                    days_text(week_days)
                } else {
                    String::new()
                },
                duration_text(week.total()),
                ui::TEXT,
            ),
            (third_label, String::new(), third_value, third_color),
            (
                t("All Time", "これまで"),
                String::new(),
                duration_text(total.total()),
                ui::TEXT,
            ),
        ];
        let (tx, ty, tile_w, tile_h, gap) = (x + 18.0, y + 40.0, 165.0, 86.0, 10.0);
        for (i, (label, sub, value, color)) in tiles.iter().enumerate() {
            let bx = tx + (tile_w + gap) * (i % 2) as f32;
            let by = ty + (tile_h + gap) * (i / 2) as f32;
            tile(c, g, (bx, by, tile_w, tile_h), label, sub, value, *color);
        }
        if self.unavailable {
            let s = Style::edged(14.0, ui::MUTED, Color32::BLACK, 1.0);
            ui::text(
                c,
                g,
                t(
                    "Records can't be opened, so nothing is shown.",
                    "記録のファイルを開けないので、出せるものがありません。",
                ),
                BARS.0,
                BARS.1 + BARS.3 / 2.0 - 10.0,
                &s,
                BARS.2,
            );
            return;
        }
        let _ = (w, h);
        self.draw_bars(c, g);
    }

    /// 棒グラフ（日ごとか週ごと）
    fn draw_bars(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = BARS;
        let weekly = self.period == Period::All;
        let bars: Vec<(NaiveDate, Spent)> = match self.period {
            Period::Weeks4 => self.log.last_days(self.today, 28),
            Period::Weeks12 => self.log.last_days(self.today, 84),
            Period::All => {
                let mut w = self.log.weeks(self.today);
                // 長すぎると棒が細くなるので 3 年まで
                let cut = w.len().saturating_sub(156);
                w.drain(..cut);
                w
            }
        };
        let max_min = bars.iter().map(|(_, s)| s.total()).max().unwrap_or(0) as f32 / 60_000.0;
        let step = grid_step(max_min);
        let top = ((max_min / step).floor() + 1.0).max(2.0) * step;
        let label = Style::plain(11.0, ui::MUTED);
        let mut m = step;
        while m < top {
            let yy = y + h - h * m / top;
            c.fill(x, yy, w, 1.0, GRID);
            ui::text_right(c, g, &minutes_text(m), x - 6.0, yy - 7.0, &label, 60.0);
            m += step;
        }
        if bars.iter().all(|(_, s)| s.total() == 0) {
            let s = Style::edged(14.0, ui::MUTED, Color32::BLACK, 1.0);
            ui::text_center(
                c,
                g,
                t(
                    "Play songs or practice sections and your time shows up here.",
                    "曲や区間練習を遊ぶと、ここに時間が出ます。",
                ),
                x + w / 2.0,
                y + h / 2.0 - 10.0,
                &s,
                w,
            );
        }
        let n = bars.len().max(1);
        let bw = w / n as f32;
        let current = if weekly {
            progress::week_start(self.today)
        } else {
            self.today
        };
        // 日付の札は重ならないように間を空ける
        let label_every = ((44.0 / bw).ceil() as usize).max(1);
        let date_style = Style::plain(12.0, ui::MUTED);
        for (i, (day, s)) in bars.iter().enumerate() {
            let bx = x + i as f32 * bw + bw * 0.16;
            let bwid = (bw * 0.68).max(1.0);
            let base = y + h;
            let mut yy = base;
            for (ms, color) in [(s.play, PLAY), (s.practice, PRACTICE), (s.other, OTHER)] {
                let bh = h * (ms as f32 / 60_000.0) / top;
                if bh > 0.0 {
                    c.fill(bx, yy - bh, bwid, bh, color);
                    yy -= bh;
                }
            }
            if s.total() == 0 {
                c.fill(bx, base - 2.0, bwid, 2.0, Color32::from_rgb(60, 58, 72));
            }
            if *day == current {
                let r = c.rect(bx - 2.0, yy - 2.0, bwid + 4.0, base - yy + 3.0);
                c.painter().rect_stroke(
                    r,
                    0.0,
                    Stroke::new(2.0 * c.scale(), Color32::WHITE),
                    StrokeKind::Outside,
                );
                let st = Style {
                    bold: true,
                    ..Style::edged(12.0, ui::TEXT, Color32::BLACK, 1.0)
                };
                let name = if weekly {
                    t("This Week", "今週")
                } else {
                    t("Today", "今日")
                };
                ui::text_center(c, g, name, bx + bwid / 2.0, yy - 20.0, &st, 80.0);
            }
            // 日ごとは月曜に、週ごとは間を空けて日付
            let mark = if weekly {
                (bars.len() - 1 - i).is_multiple_of(label_every)
            } else {
                day.weekday().num_days_from_monday() == 0
                    && (i / 7) % label_every.div_ceil(7).max(1) == 0
            };
            if mark {
                ui::text(
                    c,
                    g,
                    &format!("{}/{}", day.month(), day.day()),
                    x + i as f32 * bw,
                    base + 5.0,
                    &date_style,
                    60.0,
                );
                if !weekly {
                    c.fill(x + i as f32 * bw, y, 1.0, h, Color32::from_rgb(45, 44, 56));
                }
            }
        }
        c.fill(x, y + h, w, 1.0, Color32::from_rgb(110, 108, 124));
    }

    /// 左下の板: 曲の一覧
    fn draw_list(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, _) = LIST;
        panel(c, LIST);
        let head = ui::bold(18.0, Color32::WHITE);
        ui::text(
            c,
            g,
            t("Accuracy by Song", "曲ごとの達成率"),
            x + 18.0,
            y + 9.0,
            &head,
            260.0,
        );
        let small = Style::plain(12.0, ui::MUTED);
        ui::text_right(
            c,
            g,
            t("Recently played first", "最近遊んだ順"),
            x + w - 18.0,
            y + 15.0,
            &small,
            160.0,
        );
        if self.songs.is_empty() {
            let s = Style::edged(14.0, ui::MUTED, Color32::BLACK, 1.0);
            ui::paragraph(
                c,
                g,
                t(
                    "No songs yet. Songs you play show up here.",
                    "まだ曲がありません。遊んだ曲がここに並びます。",
                ),
                x + 20.0,
                ROW_Y + 8.0,
                w - 40.0,
                20.0,
                ROW_Y + 80.0,
                &s,
            );
            return;
        }
        for (row, i) in (self.top..self.songs.len()).take(ROWS).enumerate() {
            let s = &self.songs[i];
            let on = i == self.cursor;
            let ry = ROW_Y + row as f32 * (ROW_H + ROW_GAP);
            let card = (x + 12.0, ry, w - 24.0, ROW_H);
            ui::round_fill(
                c,
                card.0,
                card.1,
                card.2,
                card.3,
                8.0,
                Color32::from_white_alpha(if on { 26 } else { 10 }),
            );
            if on {
                ui::round_frame(c, card.0, card.1, card.2, card.3, 8.0, 3.0, ORANGE);
            } else {
                ui::round_frame(
                    c,
                    card.0,
                    card.1,
                    card.2,
                    card.3,
                    8.0,
                    1.0,
                    Color32::from_white_alpha(40),
                );
            }
            let title = Style {
                bold: true,
                ..Style::edged(
                    16.0,
                    if on {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(225)
                    },
                    Color32::BLACK,
                    1.0,
                )
            };
            ui::text(
                c,
                g,
                &s.record.title,
                card.0 + 14.0,
                ry + 8.0,
                &title,
                card.2 - 230.0,
            );
            let plays = s.record.plays.len();
            let mut sub = level_text(s);
            if plays > 0 {
                if !sub.is_empty() {
                    sub.push('・');
                }
                sub += &i18n::count(plays, "play", "plays", "回");
            }
            ui::text(c, g, &sub, card.0 + 14.0, ry + 33.0, &small, card.2 - 230.0);
            let rates: Vec<f64> = s
                .record
                .plays
                .iter()
                .rev()
                .take(MAX_POINTS)
                .rev()
                .map(|p| p.rate)
                .collect();
            if rates.len() >= 2 {
                sparkline(
                    c,
                    (card.0 + card.2 - 196.0, ry + 14.0, 80.0, ROW_H - 28.0),
                    &rates,
                );
            }
            let right = card.0 + card.2 - 14.0;
            match s.best() {
                Some(best) => {
                    let st = Style {
                        bold: true,
                        ..Style::edged(
                            18.0,
                            if on {
                                ui::YELLOW
                            } else {
                                Color32::from_gray(230)
                            },
                            Color32::BLACK,
                            1.0,
                        )
                    };
                    ui::text_right(c, g, &format!("{best:.1}%"), right, ry + 6.0, &st, 100.0);
                    let (text, color) = match s.gain() {
                        Some(d) if d > 0.0 => (format!("+{d:.1}"), ui::STRONG),
                        // ベストは初回より下がらないので、ここは初回のまま
                        Some(_) => ("±0".to_owned(), ui::MUTED),
                        // 前の DTXMania から取り込んだ記録だけのときは、1 回目と言わない
                        None if s.record.plays.iter().all(|p| p.imported) => {
                            (t("Imported", "取り込んだ記録").to_owned(), ui::MUTED)
                        }
                        None => (t("1st play", "1 回目").to_owned(), ui::MUTED),
                    };
                    let st = Style {
                        bold: true,
                        ..Style::edged(13.0, color, Color32::BLACK, 1.0)
                    };
                    ui::text_right(c, g, &text, right, ry + 33.0, &st, 100.0);
                }
                None => {
                    ui::text_right(
                        c,
                        g,
                        t("Practice only", "練習だけ"),
                        right,
                        ry + 22.0,
                        &small,
                        120.0,
                    );
                }
            }
        }
        // 上と下にまだ行があれば三角で知らせる
        if self.top > 0 {
            ui::triangle(c, x + w / 2.0, ROW_Y - 8.0, 10.0, -2, ui::MUTED);
        }
        if self.top + ROWS < self.songs.len() {
            ui::triangle(c, x + w / 2.0, LIST.1 + LIST.3 - 8.0, 10.0, 2, ui::MUTED);
        }
    }

    /// 右下の板: 選んでいる曲の達成率とばらつき
    fn draw_song(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = CHART;
        panel(c, CHART);
        let Some(s) = self.songs.get(self.cursor) else {
            return;
        };
        let title = Style {
            bold: true,
            ..Style::edged(20.0, Color32::WHITE, Color32::BLACK, 1.5)
        };
        let tw = ui::text(c, g, &s.record.title, x + 18.0, y + 8.0, &title, 360.0);
        let level = level_text(s);
        ui::text(
            c,
            g,
            &level,
            x + 18.0 + tw + 12.0,
            y + 14.0,
            &Style::plain(13.0, ui::MUTED),
            140.0,
        );
        let dim = Style::edged(15.0, ui::DIM, Color32::BLACK, 1.0);
        let summary = match (s.first(), s.best(), s.record.plays.len()) {
            (Some(f), Some(b), n) if n >= 2 => tf!(
                "First {f:.1}% → Best {b:.1}%",
                "初回 {f:.1}% → ベスト {b:.1}%"
            ),
            (_, Some(b), _) => tf!("Best {b:.1}%", "ベスト {b:.1}%"),
            _ => String::new(),
        };
        ui::text_right(c, g, &summary, x + w - 18.0, y + 10.0, &dim, 280.0);
        // 譜面が読めず時間を見積もれなかったときは出さない
        if s.record.played_ms > 0 {
            let time = tf!(
                "Time on this chart: {}",
                "この譜面で遊んだ時間 {}",
                duration_text(s.record.played_ms)
            );
            ui::text_right(
                c,
                g,
                &time,
                x + w - 18.0,
                y + 32.0,
                &Style::plain(12.0, ui::MUTED),
                280.0,
            );
        }
        if s.record.plays.is_empty() {
            let st = Style::edged(14.0, ui::MUTED, Color32::BLACK, 1.0);
            ui::paragraph(
                c,
                g,
                t(
                    "Play this song to the end and your accuracy shows up here.",
                    "この曲を最後まで遊ぶと、ここに達成率が出ます。",
                ),
                x + 24.0,
                y + h / 2.0 - 20.0,
                w - 48.0,
                20.0,
                y + h,
                &st,
            );
            return;
        }
        rate_chart(c, g, s, (x + 18.0, y + 72.0, w - 36.0, 168.0));
    }
}

/// 板（練習メニューと同じ、濃い地に灰色の枠）
fn panel(c: &Canvas, (x, y, w, h): Rect) {
    c.fill(x, y, w, h, PANEL_FILL);
    c.frame(x, y, w, h, Color32::from_gray(150));
}

/// 数字の札（名前、名前の横の小さな添え字、大きな数字）
#[allow(clippy::too_many_arguments)]
fn tile(
    c: &Canvas,
    g: &mut Gfx,
    (x, y, w, h): Rect,
    label: &str,
    sub: &str,
    value: &str,
    color: Color32,
) {
    ui::round_fill(c, x, y, w, h, 8.0, Color32::from_white_alpha(14));
    ui::round_frame(c, x, y, w, h, 8.0, 1.0, Color32::from_white_alpha(40));
    let lw = ui::text(
        c,
        g,
        label,
        x + 14.0,
        y + 10.0,
        &Style::plain(14.0, ui::DIM),
        w - 28.0,
    );
    if !sub.is_empty() {
        ui::text(
            c,
            g,
            sub,
            x + 14.0 + lw + 8.0,
            y + 11.0,
            &Style::plain(13.0, ui::MUTED),
            w - lw - 36.0,
        );
    }
    let st = Style {
        bold: true,
        ..Style::edged(26.0, color, Color32::BLACK, 2.0)
    };
    ui::text(c, g, value, x + 14.0, y + 38.0, &st, w - 24.0);
}

/// 色の見方（四角と名前を横に並べる）
fn color_keys(c: &Canvas, g: &mut Gfx, x: f32, y: f32, keys: &[(Color32, &str)]) {
    let st = Style::plain(13.0, ui::DIM);
    let mut x = x;
    for (color, name) in keys {
        c.fill(x, y + 3.0, 12.0, 12.0, *color);
        x += 18.0 + ui::text(c, g, name, x + 18.0, y, &st, 200.0) + 16.0;
    }
}

/// 難易度とレベル（「MASTER 7.45」）
fn level_text(s: &SongTrend) -> String {
    match s.level {
        Some(l) if s.label.is_empty() => format!("{l:.2}"),
        Some(l) => format!("{} {l:.2}", s.label),
        None => s.label.clone(),
    }
}

fn days_text(n: usize) -> String {
    i18n::count(n, "day", "days", "日")
}

/// 棒グラフの目盛りの間（分）
fn grid_step(max_min: f32) -> f32 {
    match max_min {
        m if m <= 60.0 => 15.0,
        m if m <= 150.0 => 30.0,
        m if m <= 360.0 => 60.0,
        m if m <= 900.0 => 180.0,
        _ => 600.0,
    }
}

/// 目盛りの札（「30 分」「2 時間」）
fn minutes_text(m: f32) -> String {
    let m = m.round() as i64;
    if m < 60 {
        tf!("{m} min", "{m} 分")
    } else {
        let h = m / 60;
        tf!("{h} h", "{h} 時間")
    }
}

/// 小さな折れ線（一覧の行）
fn sparkline(c: &Canvas, (x, y, w, h): Rect, vals: &[f64]) {
    let lo = vals.iter().copied().fold(f64::INFINITY, f64::min) - 2.0;
    let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max) + 2.0;
    let n = vals.len();
    let pts: Vec<_> = vals
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            c.pos(
                x + w * i as f32 / (n - 1) as f32,
                y + h - h * ((v - lo) / (hi - lo)) as f32,
            )
        })
        .collect();
    c.painter().add(PathShape::line(
        pts.clone(),
        PathStroke::new(2.0 * c.scale(), LINE),
    ));
    if let Some(&last) = pts.last() {
        c.painter().circle_filled(last, 3.0 * c.scale(), LINE);
    }
}

/// 達成率の目盛り（下・上・刻み）。開きが大きいときは刻みを広げて、目盛りを 8 本ほどまでにする
fn rate_axis(min: f64, max: f64) -> (f64, f64, f64) {
    let step = match max - min {
        d if d <= 30.0 => 5.0,
        d if d <= 60.0 => 10.0,
        _ => 20.0,
    };
    let hi = (((max + step * 0.6) / step).ceil() * step).min(100.0);
    let lo = (((min - step * 0.8) / step).floor() * step)
        .min(hi - step * 2.0)
        .max(0.0);
    (lo, hi, step)
}

/// 達成率の折れ線（プレイの順）と、区間練習でクリアした所の印、下にずれのばらつき
fn rate_chart(c: &Canvas, g: &mut Gfx, s: &SongTrend, (x, y, w, h): Rect) {
    let all = &s.record.plays;
    let skip = all.len().saturating_sub(MAX_POINTS);
    let plays = &all[skip..];
    let n = plays.len();
    let rates: Vec<f64> = plays.iter().map(|p| p.rate).collect();
    let min = rates.iter().copied().fold(f64::INFINITY, f64::min);
    let max = rates.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (lo, hi, step) = rate_axis(min, max);
    let left = x + 44.0;
    let cw = w - 56.0;
    let px = |i: usize| left + cw * (i as f32 + 0.5) / n as f32;
    let py = |v: f64| y + h - h * ((v - lo) / (hi - lo).max(1.0)) as f32;
    let axis = Style::plain(12.0, ui::MUTED);
    let mut v = lo;
    while v <= hi + 1e-9 {
        let yy = py(v);
        c.fill(left, yy, cw, 1.0, GRID);
        ui::text_right(c, g, &format!("{v:.0}%"), left - 8.0, yy - 8.0, &axis, 44.0);
        v += step;
    }
    if skip > 0 {
        ui::text(
            c,
            g,
            &tf!("Last {MAX_POINTS} plays", "直近 {MAX_POINTS} 回"),
            left,
            y - 22.0,
            &axis,
            160.0,
        );
    }
    // 区間練習でクリアした所（そのあとのプレイの前に立てる）
    let first_at = plays.first().map_or(i64::MIN, |p| p.started_at);
    let mut slots: Vec<(usize, Vec<String>)> = Vec::new();
    for k in s.record.clears.iter().filter(|k| k.cleared_at >= first_at) {
        let after = plays
            .iter()
            .filter(|p| p.started_at <= k.cleared_at)
            .count();
        let label = if k.first == k.last {
            tf!("✓ M{}", "✓ 小節 {}", k.first)
        } else {
            tf!("✓ M{}–{}", "✓ 小節 {}〜{}", k.first, k.last)
        };
        match slots.iter_mut().find(|(a, _)| *a == after) {
            Some((_, v)) => v.push(label),
            None => slots.push((after, vec![label])),
        }
    }
    let tag = ui::bold(12.0, Color32::from_rgb(40, 20, 0));
    for (after, labels) in &slots {
        let cx = if *after == 0 {
            left + 4.0
        } else if *after >= n {
            (px(n - 1) + left + cw) / 2.0
        } else {
            (px(after - 1) + px(*after)) / 2.0
        };
        let dash = Shape::dashed_line(
            &[c.pos(cx, y), c.pos(cx, y + h)],
            Stroke::new(2.0 * c.scale(), PRACTICE),
            4.0 * c.scale(),
            4.0 * c.scale(),
        );
        c.painter().extend(dash);
        let text = if labels.len() > 2 {
            format!("{} +{}", labels[0], labels.len() - 1)
        } else {
            labels.join(" ")
        };
        let lw = ui::text_width(g, &text, &tag).min(220.0) + 14.0;
        let lx = (cx - lw / 2.0).clamp(x, x + w - lw);
        ui::round_fill(c, lx, y - 10.0, lw, 20.0, 9.0, PRACTICE);
        ui::text_center(c, g, &text, lx + lw / 2.0, y - 8.0, &tag, lw - 10.0);
    }
    // ベストの線
    let best = max;
    let bi = rates.iter().position(|&r| r == best).unwrap_or(0);
    let yb = py(best);
    c.painter().extend(Shape::dashed_line(
        &[c.pos(left, yb), c.pos(left + cw, yb)],
        Stroke::new(1.0 * c.scale(), Color32::from_rgb(150, 135, 60)),
        5.0 * c.scale(),
        5.0 * c.scale(),
    ));
    let pts: Vec<_> = rates
        .iter()
        .enumerate()
        .map(|(i, &r)| (px(i), py(r)))
        .collect();
    if n >= 2 {
        c.painter().add(PathShape::line(
            pts.iter().map(|&(a, b)| c.pos(a, b)).collect(),
            PathStroke::new(3.0 * c.scale(), LINE),
        ));
    }
    for (i, &(cx, cy)) in pts.iter().enumerate() {
        if i == bi {
            ui::dot(c, cx, cy, 8.0, ui::YELLOW);
        } else {
            ui::dot(c, cx, cy, 5.5, Color32::from_rgb(10, 8, 18));
            ui::dot(c, cx, cy, 4.0, LINE);
        }
    }
    let (bx, by) = pts[bi];
    let st = Style {
        bold: true,
        ..Style::edged(13.0, ui::YELLOW, Color32::BLACK, 1.0)
    };
    let label = tf!("Best {best:.1}%", "ベスト {best:.1}%");
    let lw = ui::text_width(g, &label, &st);
    ui::text(
        c,
        g,
        &label,
        (bx - lw / 2.0).clamp(left, left + cw - lw),
        by - 28.0,
        &st,
        200.0,
    );
    // 日付（変わった所だけ。前の札と重なるなら出さない）
    let mut last_day = None;
    let mut last_x = f32::NEG_INFINITY;
    for (i, p) in plays.iter().enumerate() {
        let d = progress::local_day(p.started_at);
        if Some(d) != last_day && px(i) - last_x >= 40.0 {
            ui::text_center(
                c,
                g,
                &format!("{}/{}", d.month(), d.day()),
                px(i),
                y + h + 6.0,
                &axis,
                60.0,
            );
            last_x = px(i);
        }
        last_day = Some(d);
    }
    // ずれのばらつき（小さいほど上）
    let spreads: Vec<(usize, f64)> = plays
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.offset_sd_ms.map(|v| (i, v)))
        .collect();
    if spreads.len() < 2 {
        return;
    }
    // 見出しは 1 行にまとめ、左の値の札とぶつからないようにする
    let head = y + h + 30.0;
    let hw = ui::text(
        c,
        g,
        t("Timing Spread", "ばらつき"),
        x,
        head,
        &ui::bold(13.0, ui::DIM),
        120.0,
    );
    ui::text(
        c,
        g,
        t("smaller is steadier", "小さいほど安定"),
        x + hw + 10.0,
        head + 2.0,
        &Style::plain(11.0, ui::MUTED),
        160.0,
    );
    let sy = head + 24.0;
    let sh = 46.0;
    let smin = spreads.iter().map(|s| s.1).fold(f64::INFINITY, f64::min);
    let smax = spreads
        .iter()
        .map(|s| s.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = (smax - smin).max(4.0);
    let mid = (smax + smin) / 2.0;
    let (slo, shi) = (mid - span / 2.0 - 1.0, mid + span / 2.0 + 1.0);
    let sp = |v: f64| sy + sh * ((v - slo) / (shi - slo)) as f32;
    c.fill(left, sy + sh, cw, 1.0, GRID);
    let spts: Vec<_> = spreads.iter().map(|&(i, v)| (px(i), sp(v))).collect();
    c.painter().add(PathShape::line(
        spts.iter().map(|&(a, b)| c.pos(a, b)).collect(),
        PathStroke::new(2.0 * c.scale(), SPREAD),
    ));
    for &(a, b) in &spts {
        ui::dot(c, a, b, 3.0, SPREAD);
    }
    let vs = Style::edged(12.0, Color32::from_rgb(200, 180, 255), Color32::BLACK, 1.0);
    let (fx, fy) = spts[0];
    let first = spreads[0].1;
    ui::text_right(
        c,
        g,
        &format!("±{first:.0}ms"),
        fx - 8.0,
        fy - 8.0,
        &vs,
        70.0,
    );
    let (lx, ly) = spts[spts.len() - 1];
    let last = spreads[spreads.len() - 1].1;
    let st = Style { bold: true, ..vs };
    // 右の端からはみ出すときは点の上に寄せる
    let label = format!("±{last:.0}ms");
    let lw = ui::text_width(g, &label, &st);
    let right = x + w + 12.0;
    if lx + 8.0 + lw <= right {
        ui::text(c, g, &label, lx + 8.0, ly - 8.0, &st, 70.0);
    } else {
        ui::text_right(c, g, &label, right, ly - 24.0, &st, 70.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use records::ProgressPlay;

    fn chart(path: &str, rates: &[f64]) -> ChartProgress {
        ChartProgress {
            path: path.into(),
            title: path.into(),
            artist: String::new(),
            plays: rates
                .iter()
                .map(|&rate| ProgressPlay {
                    session_id: 0,
                    started_at: 0,
                    rate,
                    offset_sd_ms: None,
                    imported: false,
                })
                .collect(),
            clears: Vec::new(),
            last_played: 0,
            played_ms: 0,
        }
    }

    fn view(n: usize, focus: Option<&str>) -> RecordsView {
        let charts = (0..n)
            .map(|i| chart(&format!("{i}.dtx"), &[80.0]))
            .collect();
        RecordsView::new(
            RecordsFrom::Title,
            Some((Vec::new(), charts)),
            &SongList::default(),
            focus.map(Path::new),
        )
    }

    #[test]
    fn moves_through_songs_and_scrolls() {
        let mut v = view(7, None);
        assert_eq!(v.input(RecordsInput::Up), RecordsAction::None);
        for _ in 0..6 {
            assert_eq!(v.input(RecordsInput::Down), RecordsAction::Moved);
        }
        assert_eq!(v.input(RecordsInput::Down), RecordsAction::None);
        assert_eq!((v.cursor, v.top), (6, 2));
        assert_eq!(
            v.input(RecordsInput::Decide),
            RecordsAction::Pick(PathBuf::from("6.dtx"))
        );
        assert_eq!(v.input(RecordsInput::Cancel), RecordsAction::Back);
    }

    #[test]
    fn opens_on_the_focused_chart() {
        let v = view(9, Some("7.dtx"));
        assert_eq!((v.cursor, v.top), (7, 3));
        // 一覧に無ければ一番上
        assert_eq!(view(3, Some("x.dtx")).cursor, 0);
    }

    #[test]
    fn period_stops_at_both_ends() {
        let mut v = view(0, None);
        assert_eq!(v.input(RecordsInput::Left), RecordsAction::None);
        assert_eq!(v.input(RecordsInput::Right), RecordsAction::Changed);
        assert_eq!(v.input(RecordsInput::Right), RecordsAction::Changed);
        assert_eq!(v.period, Period::All);
        assert_eq!(v.input(RecordsInput::Right), RecordsAction::None);
        // 曲が無ければ決めても何もしない
        assert_eq!(v.input(RecordsInput::Decide), RecordsAction::None);
    }

    #[test]
    fn rate_axis_keeps_a_few_lines() {
        assert_eq!(rate_axis(73.6, 97.6), (65.0, 100.0, 5.0));
        assert_eq!(rate_axis(91.6, 91.6), (85.0, 95.0, 5.0));
        assert_eq!(rate_axis(23.9, 91.6), (0.0, 100.0, 20.0));
        assert_eq!(rate_axis(40.0, 85.0), (30.0, 100.0, 10.0));
        assert_eq!(rate_axis(99.0, 100.0), (90.0, 100.0, 5.0));
    }

    #[test]
    fn grid_steps_grow_with_the_longest_bar() {
        assert_eq!(grid_step(40.0), 15.0);
        assert_eq!(grid_step(100.0), 30.0);
        assert_eq!(grid_step(300.0), 60.0);
        let _ja = i18n::scoped(i18n::Lang::Ja);
        assert_eq!(minutes_text(30.0), "30 分");
        assert_eq!(minutes_text(120.0), "2 時間");
    }
}
