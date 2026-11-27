//! 選曲の左上（ジャケットの左）に出す、選んでいる譜面のスコアのランキング（見本の案 A、2026-11-27）。
//! いつも上位 5 件を出し、行をクリックするか、V で板に入って ↑↓ と Enter で選ぶと、リプレイの残っている
//! 記録を再生する。パッドでは操作しない（ドラムで遊ぶ人はここを使わない想定、ユーザーの決め）。

use std::path::{Path, PathBuf};

use chrono::TimeZone;
use egui::{Color32, Pos2};
use i18n::t;
use records::RankEntry;

use crate::gfx::{Canvas, Gfx};
use crate::replay_view::grouped;
use crate::result_view::Rank;
use crate::settings_ui as ui;
use crate::sfx::Se;

/// 板の場所と大きさ（ジャケットの左の空いている所）
const X: f32 = 8.0;
const Y: f32 = 40.0;
const W: f32 = 306.0;
/// 記録が無いときの板の高さ（行があれば行の数に合わせて伸ばし、5 行で 308）
const EMPTY_H: f32 = 112.0;
/// 行の始まりと高さ、一度に見せる行の数
const ROW_Y: f32 = Y + 40.0;
const ROW_H: f32 = 47.0;
const ROWS: usize = 5;

const ORANGE: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
const GOLD: Color32 = Color32::from_rgb(255, 210, 58);
const PLAYABLE: Color32 = Color32::from_rgb(125, 240, 181);
const NO_REPLAY: Color32 = Color32::from_rgb(90, 86, 110);

/// ランキングの操作（キーボード）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankInput {
    Up,
    Down,
    Decide,
    Cancel,
}

/// 選曲の左上のランキング
#[derive(Default)]
pub struct Ranking {
    /// 読んだ譜面（`loaded` のときだけ意味がある。None は譜面を選んでいない）
    chart: Option<PathBuf>,
    /// 今の譜面のランキングを読んである（曲を動かしているあいだと、遊んだあとは読み直す）
    loaded: bool,
    /// 難易度の名前（MASTER など）
    level: String,
    entries: Vec<RankEntry>,
    /// 今の譜面と違う中身の譜面で遊んだ記録（譜面を書き換えた）
    changed: Vec<bool>,
    /// キーボードで板に入っている（↑↓ と Enter がランキングに効く）
    pub focus: bool,
    cursor: usize,
    /// 一番上に見せている行
    top: usize,
    /// マウスが乗っている行
    hover: Option<usize>,
}

impl Ranking {
    /// 譜面 `chart` のランキングを読んであるか
    pub fn is_for(&self, chart: Option<&Path>) -> bool {
        self.loaded && self.chart.as_deref() == chart
    }

    /// 曲を動かしている（止まるまで読まない。古い譜面の行は出さない）
    pub fn moving(&mut self) {
        if self.loaded {
            self.loaded = false;
            self.entries.clear();
            self.changed.clear();
            self.focus = false;
            self.hover = None;
        }
    }

    /// 遊んだあとなど、記録が変わったかもしれない（選んでいる行はそのまま、次に読み直す）
    pub fn invalidate(&mut self) {
        self.loaded = false;
    }

    /// 譜面 `chart` のランキングにする。同じ譜面を読み直すときは選んでいる行を残す。
    /// `chart_hash` は今の譜面の中身（読めなければ None）
    pub fn load(
        &mut self,
        chart: Option<PathBuf>,
        level: String,
        entries: Vec<RankEntry>,
        chart_hash: Option<u64>,
    ) {
        if self.chart != chart {
            self.cursor = 0;
            self.top = 0;
            self.focus = false;
        }
        self.changed = entries
            .iter()
            .map(|e| matches!((e.entry.chart_hash, chart_hash), (Some(a), Some(b)) if a != b))
            .collect();
        self.entries = entries;
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        self.top = self.top.min(self.entries.len().saturating_sub(ROWS));
        self.chart = chart;
        self.level = level;
        self.loaded = true;
        self.hover = None;
    }

    /// V：板に入る（行が無ければ入らない）。入っていれば出る
    pub fn toggle_focus(&mut self) -> Option<Se> {
        if self.focus {
            self.focus = false;
            return Some(Se::Cancel);
        }
        if !self.loaded || self.entries.is_empty() {
            return None;
        }
        self.focus = true;
        self.cursor = self.cursor.min(self.entries.len() - 1);
        self.follow();
        Some(Se::Decide)
    }

    /// 板に入っているときのキー。返すのは鳴らす音と、再生する記録（セッションの番号）
    pub fn input(&mut self, input: RankInput) -> (Option<Se>, Option<i64>) {
        let n = self.entries.len();
        match input {
            RankInput::Up | RankInput::Down if n > 0 => {
                self.cursor = if input == RankInput::Up {
                    (self.cursor + n - 1) % n
                } else {
                    (self.cursor + 1) % n
                };
                self.follow();
                (Some(Se::Move), None)
            }
            RankInput::Up | RankInput::Down => (None, None),
            RankInput::Decide => self.play(self.cursor),
            RankInput::Cancel => {
                self.focus = false;
                (Some(Se::Cancel), None)
            }
        }
    }

    /// 行 `i` の記録を再生する。リプレイの無い記録（引き継いだもの）は再生できない
    fn play(&self, i: usize) -> (Option<Se>, Option<i64>) {
        match self.entries.get(i) {
            Some(e) if e.replay => (Some(Se::Decide), Some(e.entry.session_id)),
            Some(_) => (Some(Se::Cancel), None),
            None => (None, None),
        }
    }

    /// 選んでいる行が見えるように送る
    fn follow(&mut self) {
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + ROWS {
            self.top = self.cursor + 1 - ROWS;
        }
    }

    /// 板の高さ。行は 5 行まで並べ、下に案内の 1 行
    fn height(&self) -> f32 {
        match self.entries.len().min(ROWS) {
            0 => EMPTY_H,
            n => ROW_Y - Y + n as f32 * ROW_H + 26.0,
        }
    }

    /// 点 `p`（NX の座標）が板の上か
    pub fn contains(&self, p: Pos2) -> bool {
        self.loaded
            && self.chart.is_some()
            && (X..X + W).contains(&p.x)
            && (Y..Y + self.height()).contains(&p.y)
    }

    /// 点 `p` の行
    fn row_at(&self, p: Pos2) -> Option<usize> {
        if !self.contains(p) || !(X + 8.0..X + W - 8.0).contains(&p.x) || p.y < ROW_Y {
            return None;
        }
        let row = ((p.y - ROW_Y) / ROW_H) as usize;
        let inside = p.y - ROW_Y - row as f32 * ROW_H < ROW_H - 6.0;
        let i = self.top + row;
        (row < ROWS && inside && i < self.entries.len()).then_some(i)
    }

    /// マウス：乗っている行を光らせ、押した行を再生する。返すのは鳴らす音と再生する記録
    pub fn mouse(
        &mut self,
        hover: Option<Pos2>,
        clicked: Option<Pos2>,
    ) -> (Option<Se>, Option<i64>) {
        self.hover = hover.and_then(|p| self.row_at(p));
        let Some(i) = clicked.and_then(|p| self.row_at(p)) else {
            return (None, None);
        };
        self.cursor = i;
        self.play(i)
    }

    /// ホイールで `lines` 行送る（上へ回すと正で、上の順位へ）
    pub fn scroll(&mut self, lines: i32) {
        let last = self.entries.len().saturating_sub(ROWS);
        self.top = self.top.saturating_add_signed(-lines as isize).min(last);
        if self.focus {
            self.cursor = self.cursor.clamp(self.top, self.top + ROWS - 1);
        }
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        if !self.loaded || self.chart.is_none() {
            return;
        }
        let h = self.height();
        let line = if self.focus {
            Color32::from_rgb(120, 220, 90)
        } else {
            Color32::from_rgb(80, 78, 100)
        };
        ui::round_fill(
            c,
            X,
            Y,
            W,
            h,
            8.0,
            Color32::from_rgba_unmultiplied(8, 8, 14, 215),
        );
        ui::round_frame(c, X, Y, W, h, 8.0, 2.0, line);
        ui::text(
            c,
            g,
            t("Ranking", "ランキング"),
            X + 14.0,
            Y + 9.0,
            &ui::bold(16.0, ui::TEXT),
            160.0,
        );
        ui::text_right(
            c,
            g,
            &self.level,
            X + W - 14.0,
            Y + 12.0,
            &ui::bold(13.0, ui::DIM),
            120.0,
        );
        if self.entries.is_empty() {
            ui::paragraph(
                c,
                g,
                t(
                    "No scores yet. Play this chart and your best runs show up here.",
                    "まだ記録がありません。この譜面を遊ぶと、ここにスコアの高い順に並びます。",
                ),
                X + 16.0,
                ROW_Y - 2.0,
                W - 32.0,
                22.0,
                Y + h - 8.0,
                &ui::plain(14.0, ui::DIM),
            );
            return;
        }
        for (row, i) in (self.top..self.entries.len()).take(ROWS).enumerate() {
            self.draw_row(c, g, i, ROW_Y + row as f32 * ROW_H);
        }
        let n = self.entries.len();
        if self.top > 0 {
            ui::triangle(c, X + W / 2.0, ROW_Y - 5.0, 8.0, -2, ui::MUTED);
        }
        if self.top + ROWS < n {
            ui::triangle(
                c,
                X + W / 2.0,
                ROW_Y + ROWS as f32 * ROW_H - 2.0,
                8.0,
                2,
                ui::MUTED,
            );
        }
        let help = if self.focus {
            t(
                "↑↓ Select · Enter Play · Esc Back",
                "↑↓ 選ぶ・Enter 再生・Esc 戻る",
            )
        } else {
            t(
                "Click or press V · ▶ = replay",
                "クリックか V で選ぶ・▶ はリプレイあり",
            )
        };
        ui::text(
            c,
            g,
            help,
            X + 14.0,
            Y + h - 22.0,
            &ui::plain(12.0, ui::MUTED),
            W - 28.0,
        );
    }

    fn draw_row(&self, c: &Canvas, g: &mut Gfx, i: usize, y: f32) {
        let e = &self.entries[i];
        let (x0, w, h) = (X + 8.0, W - 16.0, ROW_H - 6.0);
        if self.focus && i == self.cursor {
            ui::round_fill(c, x0, y, w, h, 6.0, Color32::from_rgb(34, 30, 48));
            ui::round_frame(c, x0, y, w, h, 6.0, 2.0, ORANGE);
        } else if self.hover == Some(i) {
            ui::round_fill(c, x0, y, w, h, 6.0, Color32::from_rgb(36, 33, 52));
        } else {
            ui::round_fill(c, x0, y, w, h, 6.0, Color32::from_rgb(18, 17, 28));
        }
        let mid = y + h / 2.0;
        let place = i + 1;
        ui::text_center(
            c,
            g,
            &place.to_string(),
            X + 28.0,
            mid - 13.0,
            &ui::bold(20.0, if place == 1 { GOLD } else { ui::TEXT }),
            28.0,
        );
        let rank = Rank::of(&e.entry.tally);
        let rank_color = if matches!(rank, Rank::SS | Rank::S) {
            ORANGE
        } else {
            ui::TEXT
        };
        ui::text_center(
            c,
            g,
            &format!("{rank:?}"),
            X + 58.0,
            mid - 15.0,
            &ui::bold(22.0, rank_color),
            32.0,
        );
        if let Some(s) = &e.entry.score {
            ui::text(
                c,
                g,
                &grouped(s.score),
                X + 80.0,
                mid - 19.0,
                &ui::bold(16.0, ui::TEXT),
                100.0,
            );
            ui::text(
                c,
                g,
                &format!("{:.2}%  {}", s.rate, day_label(e.entry.started_at)),
                X + 80.0,
                mid + 1.0,
                &ui::plain(12.0, ui::DIM),
                130.0,
            );
        }
        if let Some((tag, color)) = self.tag(i) {
            let style = ui::bold(11.0, Color32::from_rgb(24, 18, 30));
            let tw = ui::text_width(g, tag, &style) + 12.0;
            ui::round_fill(c, X + 182.0, mid - 18.0, tw, 17.0, 8.5, color);
            ui::text_center(c, g, tag, X + 182.0 + tw / 2.0, mid - 17.0, &style, tw);
        }
        let play = if e.replay { PLAYABLE } else { NO_REPLAY };
        ui::triangle(c, X + W - 30.0, mid, 13.0, 1, play);
    }

    /// 行 `i` の札（引き継いだ記録、譜面を書き換える前、途中でやめた）
    fn tag(&self, i: usize) -> Option<(&'static str, Color32)> {
        let e = &self.entries[i];
        if !e.replay {
            Some((t("Imported", "引き継ぎ"), Color32::from_rgb(170, 170, 190)))
        } else if self.changed[i] {
            Some((t("Edited", "譜面変更"), Color32::from_rgb(255, 154, 31)))
        } else if !e.entry.completed {
            Some((
                t("Unfinished", "途中まで"),
                Color32::from_rgb(150, 143, 172),
            ))
        } else {
            None
        }
    }
}

/// 遊んだ日（月と日）
fn day_label(started_at: i64) -> String {
    match chrono::Local.timestamp_opt(started_at, 0).single() {
        Some(at) => at.format(t("%b %-d", "%-m/%-d")).to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;
    use engine::judge::Tally;
    use records::{PlayEntry, PlayScore};

    fn entry(id: i64, replay: bool, hash: Option<u64>) -> RankEntry {
        RankEntry {
            entry: PlayEntry {
                session_id: id,
                started_at: 0,
                chart_hash: hash,
                tally: Tally::default(),
                completed: true,
                score: Some(PlayScore {
                    score: 100,
                    rate: 50.0,
                    progress: String::new(),
                    cleared: true,
                }),
            },
            replay,
        }
    }

    fn loaded(entries: Vec<RankEntry>) -> Ranking {
        let mut r = Ranking::default();
        r.load(Some("a.dtx".into()), "MASTER".into(), entries, Some(1));
        r
    }

    #[test]
    fn only_plays_with_a_replay_can_be_played() {
        let mut r = loaded(vec![entry(7, true, Some(1)), entry(3, false, None)]);
        assert_eq!(r.toggle_focus(), Some(Se::Decide));
        assert_eq!(r.input(RankInput::Decide), (Some(Se::Decide), Some(7)));
        r.input(RankInput::Down);
        // 引き継いだ記録はスコアだけ
        assert_eq!(r.input(RankInput::Decide), (Some(Se::Cancel), None));
        assert_eq!(r.tag(1).map(|t| t.0), Some(t("Imported", "引き継ぎ")));
        // 上は一番下へ回る
        r.input(RankInput::Down);
        assert_eq!(r.cursor, 0);
        r.input(RankInput::Up);
        assert_eq!(r.cursor, 1);
        assert_eq!(r.input(RankInput::Cancel), (Some(Se::Cancel), None));
        assert!(!r.focus);
    }

    #[test]
    fn the_board_follows_the_cursor_and_marks_edited_charts() {
        let mut r = loaded(
            (0..8)
                .map(|i| entry(i, true, Some(if i == 2 { 9 } else { 1 })))
                .collect(),
        );
        assert!(r.changed[2]);
        assert_eq!(r.tag(2).map(|t| t.0), Some(t("Edited", "譜面変更")));
        r.toggle_focus();
        for _ in 0..6 {
            r.input(RankInput::Down);
        }
        assert_eq!((r.cursor, r.top), (6, 2));
        // 同じ譜面を読み直しても選んでいる行はそのまま（リプレイから戻ったとき）
        r.invalidate();
        assert!(!r.is_for(Some(Path::new("a.dtx"))));
        r.load(
            Some("a.dtx".into()),
            "MASTER".into(),
            (0..8).map(|i| entry(i, true, Some(1))).collect(),
            Some(1),
        );
        assert_eq!((r.cursor, r.top, r.focus), (6, 2, true));
        // ほかの譜面にすると一番上から
        r.load(
            Some("b.dtx".into()),
            "BASIC".into(),
            vec![entry(1, true, None)],
            None,
        );
        assert_eq!((r.cursor, r.top, r.focus), (0, 0, false));
    }

    #[test]
    fn clicking_a_row_plays_it_and_the_wheel_scrolls() {
        let mut r = loaded((0..8).map(|i| entry(i, true, Some(1))).collect());
        // 2 行目の真ん中
        let second = pos2(X + 100.0, ROW_Y + ROW_H + 20.0);
        assert_eq!(
            r.mouse(Some(second), Some(second)),
            (Some(Se::Decide), Some(1))
        );
        assert_eq!(r.hover, Some(1));
        // 行と行のすきまや板の外は何もしない
        let gap = pos2(X + 100.0, ROW_Y + ROW_H - 2.0);
        assert_eq!(r.mouse(None, Some(gap)), (None, None));
        assert_eq!(r.mouse(None, Some(pos2(600.0, 200.0))), (None, None));
        assert_eq!(r.hover, None);
        // 下へ回すと送る（一番下で止まる）
        r.scroll(-5);
        assert_eq!(r.top, 3);
        assert_eq!(r.mouse(None, Some(second)).1, Some(4));
        r.scroll(1);
        assert_eq!(r.top, 2);
        r.scroll(9);
        assert_eq!(r.top, 0);
    }

    #[test]
    fn nothing_to_choose_without_records() {
        let mut r = loaded(Vec::new());
        assert_eq!(r.toggle_focus(), None);
        assert!(!r.focus);
        // 曲を動かしているあいだは板を出さない
        let mut r = loaded(vec![entry(1, true, None)]);
        r.toggle_focus();
        r.moving();
        assert!(!r.focus);
        assert!(!r.contains(pos2(100.0, 100.0)));
    }
}
