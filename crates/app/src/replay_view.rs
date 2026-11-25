//! リプレイの画面。記録したプレイを演奏の画面のまま再生し（叩いた所は音符と一緒に流れる白い線）、
//! CY で止めると、止めた所の 2 小節を横の譜面とずれのグラフで見せる（見本の案 C、2026-11-25）。
//! 入口は結果の画面（HH）と、選曲の簡易設定の「この曲のリプレイ」から開くプレイの一覧（[`ReplayList`]）。

use chrono::TimeZone;
use dtx::DrumLane;
use egui::Color32;
use engine::judge::Judgement;
use engine::practice::Section;
use i18n::{t, tf};
use records::PlayEntry;

use crate::game::{Outcome, Play};
use crate::gfx::{Canvas, Gfx};
use crate::lag_view::judgement_color;
use crate::play_view::PlayView;
use crate::practice::section_label;
use crate::result_view::{Rank, ResultView};
use crate::settings_ui::{self as ui, Pad};
use crate::sfx::Se;

/// 横の譜面の行の順（上から。楽譜と同じくシンバルが上、バスドラが下）。開いたハイハットは閉じたハイハットの行
const ROWS: [DrumLane; 11] = [
    DrumLane::LC,
    DrumLane::CY,
    DrumLane::RD,
    DrumLane::HH,
    DrumLane::HT,
    DrumLane::SD,
    DrumLane::LT,
    DrumLane::FT,
    DrumLane::BD,
    DrumLane::LBD,
    DrumLane::LP,
];

fn row_of(lane: DrumLane) -> DrumLane {
    if lane == DrumLane::HHO {
        DrumLane::HH
    } else {
        lane
    }
}

/// 再生中の HT・LT で飛ぶ小節の数と、止めて見る小節の数
const JUMP_MEASURES: i32 = 4;
const DETAIL_MEASURES: u32 = 2;

const PANEL_FILL: Color32 = Color32::from_rgb(10, 10, 18);
const PANEL_LINE: Color32 = Color32::from_rgb(70, 72, 88);
const ORANGE: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x40);
const MISS_RED: Color32 = Color32::from_rgb(240, 96, 96);
const REPLAY_RED: Color32 = Color32::from_rgb(220, 50, 70);

/// リプレイを閉じたら戻る先
pub enum Back {
    /// 来たときの結果の画面
    Result(Box<Outcome>, Box<ResultView>),
    /// 選曲の上のプレイの一覧（選んでいた行のまま）
    List(Box<ReplayList>),
}

/// 再生しているリプレイ
pub struct ReplayRun {
    pub play: Box<Play>,
    pub view: Box<PlayView>,
    pub back: Back,
    /// 板に出す、曲名と遊んだ日時
    label: String,
    /// 小節の頭の時刻（判定の時刻。最後は曲の終わりの小節線）
    bars: Vec<f64>,
    /// 小節ごとの MISS の数（板の赤い印）
    misses: Vec<u32>,
    /// 見られる最後の小節（途中でやめたプレイは、やめた所まで）
    last: u32,
    /// 最後に判定した音符の時刻（再生し終えたら、そこを横の譜面で見せる）
    last_note_ms: f64,
    /// 止めて見ている小節（最初の小節。再生中は None）
    detail: Option<u32>,
    /// 最後まで再生した（CY で最初から）
    ended: bool,
}

/// 遊んだ日時（板と一覧に出す）
pub fn when_label(started_at: i64) -> String {
    match chrono::Local.timestamp_opt(started_at, 0).single() {
        Some(at) => at.format(t("%b %-d %H:%M", "%-m月%-d日 %H:%M")).to_string(),
        None => String::new(),
    }
}

/// 3 桁ごとに区切った数
fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// 曲の時刻（ミリ秒）を「1:05」に
fn clock(ms: f64) -> String {
    let s = (ms.max(0.0) / 1000.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

impl ReplayRun {
    pub fn new(play: Box<Play>, mut view: Box<PlayView>, back: Back, when: &str) -> Self {
        view.show_all_chips();
        let mut bars: Vec<f64> = play.lines.iter().filter(|l| l.bar).map(|l| l.ms).collect();
        if bars.is_empty() {
            bars.push(0.0);
        }
        if bars.len() < 2 {
            bars.push(bars[0] + 2000.0);
        }
        let measures = bars.len() - 1;
        let mut misses = vec![0; measures];
        let mut last_note_ms: f64 = 0.0;
        let (end_ms, judged) = play
            .replay()
            .map_or((play.end_ms, &[][..]), |f| (f.end_ms, f.judged.as_slice()));
        for (n, j) in play.judge.notes().iter().zip(judged) {
            let Some((judgement, _)) = j else {
                continue;
            };
            last_note_ms = last_note_ms.max(n.ms);
            if *judgement == Judgement::Miss
                && let Some(m) = misses.get_mut(n.at.measure as usize)
            {
                *m += 1;
            }
        }
        let label = format!("{}　{when}", play.title);
        let mut run = Self {
            play,
            view,
            back,
            label,
            bars,
            misses,
            last: measures.saturating_sub(1) as u32,
            last_note_ms,
            detail: None,
            ended: false,
        };
        run.last = run.measure_at(end_ms);
        run
    }

    /// 閉じて、戻る先を返す（再生している曲は捨てる）
    pub fn into_back(self) -> Back {
        self.back
    }

    /// 判定の時刻 `ms` がある小節
    fn measure_at(&self, ms: f64) -> u32 {
        let m = self.bars.partition_point(|&b| b <= ms).saturating_sub(1);
        (m as u32).min(self.last)
    }

    /// 小節 `m` の頭の時刻
    fn bar_ms(&self, m: u32) -> f64 {
        self.bars[(m as usize).min(self.bars.len() - 1)]
    }

    /// 止めた所 `ms` で見せる 2 小節の最初の小節。止めたのは気になった音のあとなので、
    /// 小節の頭から 4 分の 3 までなら 1 つ前の小節から見せる
    fn window_at(&self, ms: f64) -> u32 {
        let m = self.measure_at(ms);
        let len = self.bar_ms(m + 1) - self.bar_ms(m);
        let first = self.measure_at(ms - 0.75 * len);
        first.min(self.last.saturating_sub(DETAIL_MEASURES - 1))
    }

    /// 止めているか（止めた所の横の譜面を出している）
    pub fn paused(&self) -> bool {
        self.detail.is_some()
    }

    /// 時間を進める。最後まで再生したら止めて、最後の 2 小節を見せる
    pub fn tick(&mut self, view_ahead_ms: f64) {
        self.play.view_ahead_ms = view_ahead_ms;
        self.play.update();
        self.view.feed(&mut self.play);
        if !self.ended && !self.play.paused() && self.play.finished() {
            self.play.toggle_pause();
            self.ended = true;
            self.detail = Some(self.window_at(self.last_note_ms));
        }
    }

    /// CY：止める・つづける（再生し終えていたら最初から）
    pub fn toggle(&mut self) {
        if self.ended {
            self.ended = false;
            self.detail = None;
            self.play.replay_jump(f64::MIN);
            self.view.rewind(&mut self.play);
            if self.play.paused() {
                self.play.toggle_pause();
            }
            return;
        }
        self.detail = if self.play.toggle_pause() {
            Some(self.window_at(self.play.replay_ms()))
        } else {
            None
        };
    }

    /// HT（-1）・LT（+1）：再生中は 4 小節、止めているときは見ている 2 小節ずつ前後へ
    pub fn step(&mut self, dir: i32) -> Option<Se> {
        let target = match self.detail {
            Some(first) => {
                let max = self.last.saturating_sub(DETAIL_MEASURES - 1) as i32;
                let f = (first as i32 + DETAIL_MEASURES as i32 * dir).clamp(0, max) as u32;
                if f == first {
                    return None;
                }
                self.detail = Some(f);
                self.ended = false;
                f
            }
            None => {
                let now = self.play.replay_ms();
                let m = self.measure_at(now) as i32;
                let target = (m + JUMP_MEASURES * dir).clamp(0, self.last as i32) as u32;
                // 最後の小節の近くで先へ送っても、前へは戻さない
                if dir > 0 && self.lead_in(target) <= now {
                    return None;
                }
                target
            }
        };
        self.jump_to_measure(target);
        Some(Se::Move)
    }

    /// 小節 `m` へ飛ぶときの行き先（少し前から聴けるように、1 拍前）
    fn lead_in(&self, m: u32) -> f64 {
        let start = self.bar_ms(m);
        start - (self.bar_ms(m + 1) - start) / 4.0
    }

    /// 小節 `m` の 1 拍前へ飛ぶ
    fn jump_to_measure(&mut self, m: u32) {
        let to = self.lead_in(m) + self.play.song_offset_ms;
        self.play.replay_jump(to);
        self.view.rewind(&mut self.play);
    }

    /// RD で区間練習にする小節（止めていれば見ている 2 小節、再生中は今の所の 2 小節）
    pub fn section(&self) -> Section {
        let first = self
            .detail
            .unwrap_or_else(|| self.window_at(self.play.replay_ms()));
        Section::new(first, (first + DETAIL_MEASURES - 1).min(self.last))
    }

    /// 画面の下に出すパッドの役目
    fn hints(&self) -> Vec<ui::Hint<'static>> {
        let play: ui::Hint<'static> = if self.ended {
            (
                &[Pad::CY],
                t("Play from Start (Space)", "最初から再生（Space）"),
            )
        } else if self.paused() {
            (&[Pad::CY], t("Play (Space)", "再生（Space）"))
        } else {
            (&[Pad::CY], t("Pause (Space)", "一時停止（Space）"))
        };
        let step: ui::Hint<'static> = if self.paused() {
            (
                &[Pad::HT, Pad::LT],
                t(
                    "Previous / Next 2 Measures (← →)",
                    "2 小節もどる・すすむ（← →）",
                ),
            )
        } else {
            (
                &[Pad::HT, Pad::LT],
                t(
                    "Back / Forward 4 Measures (← →)",
                    "4 小節もどる・すすむ（← →）",
                ),
            )
        };
        let back: ui::Hint<'static> = match self.back {
            Back::Result(..) => (&[Pad::LC], t("Back to Results (Esc)", "結果へ戻る（Esc）")),
            Back::List(_) => (&[Pad::LC], t("Back to List (Esc)", "一覧へ戻る（Esc）")),
        };
        vec![
            play,
            step,
            (&[Pad::RD], t("Practice Here (P)", "ここを区間練習（P）")),
            back,
        ]
    }

    /// 1 フレーム描く
    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, px_per_ms: f32, visual_offset_ms: f64) {
        let now = self.play.view_ms(visual_offset_ms);
        self.view.draw(c, g, &self.play, now, px_per_ms);
        self.view.draw_takes(c, &self.play, now, px_per_ms);
        self.draw_panel(c, g);
        if let Some(first) = self.detail {
            c.cover(Color32::from_black_alpha(150));
            self.draw_detail(c, g, first);
        }
        ui::legend(c, g, &self.hints());
    }

    /// 右下の板（REPLAY の札、曲名と日時、進み具合と MISS のあった小節の印）
    fn draw_panel(&self, c: &Canvas, g: &mut Gfx) {
        // 曲名とアーティスト（演奏の画面の右下）を隠す
        c.fill(932.0, 600.0, 348.0, 90.0, Color32::from_rgb(14, 12, 20));
        let (x0, y0, w, h) = (940.0, 606.0, 322.0, 80.0);
        ui::round_fill(c, x0, y0, w, h, 5.0, Color32::from_rgb(8, 8, 14));
        ui::round_frame(c, x0, y0, w, h, 5.0, 1.0, Color32::from_rgb(90, 90, 110));
        ui::round_fill(c, x0 + 10.0, y0 + 9.0, 72.0, 20.0, 4.0, REPLAY_RED);
        ui::text_center(
            c,
            g,
            "REPLAY",
            x0 + 46.0,
            y0 + 11.0,
            &ui::bold(12.0, Color32::WHITE),
            70.0,
        );
        ui::text(
            c,
            g,
            &self.label,
            x0 + 92.0,
            y0 + 10.0,
            &ui::plain(14.0, ui::DIM),
            w - 100.0,
        );
        // 進み具合（MISS のあった小節に赤い印）
        let (bx0, bx1, by) = (x0 + 12.0, x0 + w - 12.0, y0 + 46.0);
        let end = self
            .play
            .replay()
            .map_or(self.play.end_ms, |f| f.end_ms)
            .max(1.0);
        let x_of = |ms: f64| bx0 + (bx1 - bx0) * (ms / end).clamp(0.0, 1.0) as f32;
        let head = self.play.replay_ms();
        c.fill(bx0, by - 3.0, bx1 - bx0, 6.0, Color32::from_rgb(50, 50, 64));
        c.fill(
            bx0,
            by - 3.0,
            x_of(head) - bx0,
            6.0,
            Color32::from_rgb(200, 200, 215),
        );
        for (m, &n) in self.misses.iter().enumerate() {
            if n == 0 {
                continue;
            }
            let mid = (self.bar_ms(m as u32) + self.bar_ms(m as u32 + 1)) / 2.0;
            c.fill(x_of(mid) - 1.0, by - 10.0, 2.0, 5.0, MISS_RED);
        }
        ui::dot(c, x_of(head), by, 6.0, Color32::WHITE);
        let status = if self.ended {
            t("End", "終わり").to_owned()
        } else if self.paused() {
            tf!(
                "Paused {} / {}",
                "一時停止中 {} / {}",
                clock(head),
                clock(end)
            )
        } else {
            format!("▶ {} / {}", clock(head), clock(end))
        };
        let small = ui::plain(12.0, ui::MUTED);
        ui::text(c, g, &status, bx0, by + 9.0, &small, 150.0);
        // 白い線の見方は再生中だけ（止めると横の譜面に見方を出す）
        if self.paused() {
            return;
        }
        let how = if self.view.reverse() {
            t(
                "White line = your hit (above = early)",
                "白い線＝叩いた所（上なら早い）",
            )
        } else {
            t(
                "White line = your hit (below = early)",
                "白い線＝叩いた所（下なら早い）",
            )
        };
        ui::text_right(c, g, how, bx1, by + 9.0, &small, 180.0);
    }

    /// 止めた所の 2 小節（横の譜面とずれのグラフ）
    fn draw_detail(&self, c: &Canvas, g: &mut Gfx, first: u32) {
        let Some(f) = self.play.replay() else {
            return;
        };
        let last = (first + DETAIL_MEASURES - 1).min(self.last);
        ui::round_fill(c, 60.0, 70.0, 1160.0, 530.0, 8.0, PANEL_FILL);
        ui::round_frame(
            c,
            60.0,
            70.0,
            1160.0,
            530.0,
            8.0,
            2.0,
            Color32::from_rgb(120, 120, 140),
        );
        let section = section_label(Section::new(first, last));
        let title = if self.ended {
            tf!("End of Replay · {section}", "再生し終えました　{section}")
        } else {
            tf!("Paused · {section}", "一時停止　{section}")
        };
        ui::text(c, g, &title, 84.0, 82.0, &ui::bold(20.0, ui::TEXT), 600.0);

        let (t0, t1) = (self.bar_ms(first), self.bar_ms(last + 1));
        let notes = self.play.judge.notes();
        let shown: Vec<usize> = (0..notes.len())
            .filter(|&i| notes[i].ms >= t0 - 1.0 && notes[i].ms < t1 - 1.0)
            .collect();
        // まとめ（MISS と GREAT の数、ずれの平均）
        let (mut miss, mut great, mut sum, mut hits) = (0, 0, 0.0, 0);
        for &i in &shown {
            match f.judged.get(i).copied().flatten() {
                Some((Judgement::Miss, _)) => miss += 1,
                Some((j, o)) => {
                    if j == Judgement::Great {
                        great += 1;
                    }
                    if let Some(o) = o {
                        sum += o;
                        hits += 1;
                    }
                }
                None => {}
            }
        }
        let mut summary = format!("MISS {miss}　GREAT {great}");
        if hits > 0 {
            let mean = sum / f64::from(hits);
            let lean = if mean <= -3.0 {
                t(" (early)", "（早め）")
            } else if mean >= 3.0 {
                t(" (late)", "（遅め）")
            } else {
                ""
            };
            summary.push_str(&tf!(
                "　Avg offset {mean:+.0} ms{lean}",
                "　ずれの平均 {mean:+.0} ms{lean}"
            ));
        }
        ui::text_right(
            c,
            g,
            &summary,
            1196.0,
            88.0,
            &ui::plain(15.0, ui::DIM),
            560.0,
        );

        // 横の譜面
        let (x0, y0, x1, y1) = (84.0, 140.0, 1196.0, 400.0);
        let label_w = 44.0;
        let gx0 = x0 + label_w;
        let rows: Vec<DrumLane> = ROWS
            .into_iter()
            .filter(|&r| shown.iter().any(|&i| row_of(notes[i].lane) == r))
            .collect();
        let span = (t1 - t0).max(1.0);
        let px = (x1 - gx0 - 20.0) / span as f32;
        // 小節の頭より早く叩いた線が名前に重ならないように、左を少しあける
        let x_of = |ms: f64| gx0 + 14.0 + (ms - t0) as f32 * px;
        let rh = (y1 - y0) / rows.len().max(1) as f32;
        for (i, &lane) in rows.iter().enumerate() {
            let ry = y0 + i as f32 * rh;
            if i % 2 == 0 {
                c.fill(gx0, ry, x1 - gx0, rh, Color32::from_rgb(18, 18, 30));
            }
            let style = ui::bold(13.0, ui::lane_color(lane));
            ui::text_center(
                c,
                g,
                lane.name(),
                x0 + label_w / 2.0,
                ry + rh / 2.0 - 9.0,
                &style,
                label_w,
            );
        }
        // 拍と小節の線、小節の番号
        for l in self
            .play
            .lines
            .iter()
            .filter(|l| l.ms >= t0 - 1.0 && l.ms <= t1 + 1.0)
        {
            let x = x_of(l.ms);
            if l.bar {
                c.fill(x - 1.0, y0, 2.0, y1 - y0, Color32::from_rgb(110, 110, 130));
            } else {
                c.fill(x - 0.5, y0, 1.0, y1 - y0, Color32::from_rgb(45, 45, 60));
            }
        }
        for m in first..=last {
            ui::text(
                c,
                g,
                &m.to_string(),
                x_of(self.bar_ms(m)) + 4.0,
                y0 - 20.0,
                &ui::plain(12.0, ui::MUTED),
                60.0,
            );
        }
        if rows.is_empty() {
            ui::text_center(
                c,
                g,
                t("No notes in these measures", "この小節には音符がありません"),
                (x0 + x1) / 2.0,
                (y0 + y1) / 2.0 - 10.0,
                &ui::plain(16.0, ui::MUTED),
                600.0,
            );
        }
        let row_y = |lane: DrumLane| {
            rows.iter()
                .position(|&r| r == row_of(lane))
                .map(|r| y0 + (r as f32 + 0.5) * rh)
        };
        let (hw, hh) = (5.0, (rh * 0.36).min(11.0));
        for &i in &shown {
            let n = &notes[i];
            let Some(cy) = row_y(n.lane) else {
                continue;
            };
            let x = x_of(n.ms);
            match f.judged.get(i).copied().flatten() {
                Some((Judgement::Miss, _)) => {
                    ui::round_frame(c, x - hw, cy - hh, hw * 2.0, hh * 2.0, 3.0, 2.0, MISS_RED);
                    cross(c, x, cy, 4.0, MISS_RED);
                }
                Some((j, o)) => {
                    let color = if o.is_some() {
                        judgement_color(j)
                    } else {
                        // 自動で叩いた音符
                        Color32::from_rgb(120, 120, 140)
                    };
                    ui::round_fill(c, x - hw, cy - hh, hw * 2.0, hh * 2.0, 3.0, color);
                    if let Some(o) = o {
                        let hx = x_of(n.ms + o);
                        c.fill(hx - 1.0, cy - hh - 4.0, 2.0, hh * 2.0 + 8.0, Color32::WHITE);
                    }
                }
                // 判定する前にやめた所
                None => ui::round_frame(
                    c,
                    x - hw,
                    cy - hh,
                    hw * 2.0,
                    hh * 2.0,
                    3.0,
                    1.5,
                    Color32::from_rgb(120, 120, 140),
                ),
            }
        }
        // 音符の無い所を叩いた（赤い線）
        let mut strays = false;
        for (tk, m) in f.takes.iter().zip(&f.matched) {
            if m.is_some() || tk.ms < t0 || tk.ms >= t1 {
                continue;
            }
            if let Some(cy) = row_y(tk.lane) {
                c.fill(
                    x_of(tk.ms) - 1.0,
                    cy - hh - 4.0,
                    2.0,
                    hh * 2.0 + 8.0,
                    MISS_RED,
                );
                strays = true;
            }
        }
        let head = self.play.replay_ms();
        let head_x = (head >= t0 && head <= t1).then(|| x_of(head));
        if let Some(hx) = head_x {
            c.fill(
                hx - 1.0,
                y0 - 4.0,
                2.0,
                y1 - y0 + 8.0,
                Color32::from_rgb(255, 255, 255),
            );
        }

        // 色の見方
        let small = ui::plain(12.0, ui::DIM);
        let mut lx = 760.0;
        let ly = 410.0;
        for j in [
            Judgement::Perfect,
            Judgement::Great,
            Judgement::Good,
            Judgement::Poor,
            Judgement::Miss,
        ] {
            if j == Judgement::Miss {
                ui::round_frame(c, lx, ly, 10.0, 16.0, 3.0, 2.0, MISS_RED);
            } else {
                ui::round_fill(c, lx, ly, 10.0, 16.0, 3.0, judgement_color(j));
            }
            lx += 16.0;
            lx += ui::text(c, g, j.name(), lx, ly, &small, 100.0) + 14.0;
        }
        c.fill(lx, ly - 2.0, 2.0, 20.0, Color32::WHITE);
        lx += 8.0;
        lx += ui::text(c, g, t("Your Hit", "叩いた所"), lx, ly, &small, 100.0) + 14.0;
        if strays {
            c.fill(lx, ly - 2.0, 2.0, 20.0, MISS_RED);
            ui::text(
                c,
                g,
                t("Extra Hit", "音符の無い所"),
                lx + 8.0,
                ly,
                &small,
                100.0,
            );
        }

        // ずれ（横は時刻、縦はずれ。上が遅い・下が早い）
        ui::text(
            c,
            g,
            t("Offset", "ずれ"),
            84.0,
            410.0,
            &ui::bold(14.0, ui::DIM),
            200.0,
        );
        let (gx0, gy0, gx1, gy1) = (128.0, 456.0, x1, 576.0);
        let w = self.play.judge.windows();
        let cy = (gy0 + gy1) / 2.0;
        let sy = (gy1 - gy0) / 2.0 / w.poor.max(1.0) as f32;
        let band = |ms: f64, color: Color32| {
            let h = (ms as f32 * sy).min((gy1 - gy0) / 2.0);
            c.fill(gx0, cy - h, gx1 - gx0, h * 2.0, color);
        };
        band(w.great, Color32::from_rgb(22, 30, 26));
        band(w.perfect, Color32::from_rgb(36, 40, 22));
        c.fill(
            gx0,
            cy - 0.5,
            gx1 - gx0,
            1.0,
            Color32::from_rgb(120, 120, 120),
        );
        let label = ui::plain(12.0, ui::MUTED);
        ui::text_right(
            c,
            g,
            "MISS",
            gx0 - 6.0,
            gy0 - 21.0,
            &ui::plain(11.0, MISS_RED),
            60.0,
        );
        ui::text_right(c, g, t("Late", "遅い"), gx0 - 6.0, gy0, &label, 60.0);
        ui::text_right(
            c,
            g,
            t("Early", "早い"),
            gx0 - 6.0,
            gy1 - 16.0,
            &label,
            60.0,
        );
        ui::text_right(c, g, "0", gx0 - 6.0, cy - 8.0, &label, 60.0);
        for m in first..=last + 1 {
            let x = x_of(self.bar_ms(m));
            c.fill(x - 0.5, gy0, 1.0, gy1 - gy0, Color32::from_rgb(70, 70, 90));
        }
        for &i in &shown {
            let x = x_of(notes[i].ms);
            match f.judged.get(i).copied().flatten() {
                Some((Judgement::Miss, _)) => cross(c, x, gy0 - 14.0, 4.0, MISS_RED),
                Some((j, Some(o))) => {
                    let y = (cy - o as f32 * sy).clamp(gy0, gy1);
                    ui::dot(c, x, y, 4.0, judgement_color(j));
                }
                _ => {}
            }
        }
        if let Some(hx) = head_x {
            c.fill(hx - 1.0, gy0, 2.0, gy1 - gy0, Color32::WHITE);
        }
    }
}

/// × の印
fn cross(c: &Canvas, x: f32, y: f32, r: f32, color: Color32) {
    let p = c.painter();
    let stroke = egui::Stroke::new(2.0 * c.scale(), color);
    p.line_segment([c.pos(x - r, y - r), c.pos(x + r, y + r)], stroke);
    p.line_segment([c.pos(x - r, y + r), c.pos(x + r, y - r)], stroke);
}

/// 一覧の操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListInput {
    Up,
    Down,
    Decide,
    Cancel,
}

/// 一覧で決めたこと
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListDone {
    /// このプレイ（セッションの番号）を再生する
    Play(i64),
    Close,
}

/// 選曲の簡易設定から開く、選んでいる曲のプレイの一覧（新しい順）
pub struct ReplayList {
    title: String,
    entries: Vec<PlayEntry>,
    /// 達成率が一番のプレイ
    best: Option<i64>,
    /// 今の譜面と違う譜面で遊んだプレイ（譜面を書き換えた）
    changed: Vec<bool>,
    cursor: usize,
}

/// 一覧に並べる行の数
const LIST_ROWS: usize = 7;

impl ReplayList {
    /// `chart_hash` は今の譜面の中身（読めなければ None）
    pub fn new(title: String, entries: Vec<PlayEntry>, chart_hash: Option<u64>) -> Self {
        let best = entries
            .iter()
            .filter_map(|e| Some((e.session_id, e.score.as_ref()?.rate)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id);
        let changed = entries
            .iter()
            .map(|e| matches!((e.chart_hash, chart_hash), (Some(a), Some(b)) if a != b))
            .collect();
        Self {
            title,
            entries,
            best,
            changed,
            cursor: 0,
        }
    }

    pub fn input(&mut self, input: ListInput) -> (Option<Se>, Option<ListDone>) {
        let n = self.entries.len();
        match input {
            ListInput::Up | ListInput::Down if n > 0 => {
                self.cursor = if input == ListInput::Up {
                    (self.cursor + n - 1) % n
                } else {
                    (self.cursor + 1) % n
                };
                (Some(Se::Move), None)
            }
            ListInput::Up | ListInput::Down => (None, None),
            ListInput::Decide => match self.entries.get(self.cursor) {
                Some(e) => (Some(Se::Decide), Some(ListDone::Play(e.session_id))),
                None => (None, None),
            },
            ListInput::Cancel => (Some(Se::Cancel), Some(ListDone::Close)),
        }
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        c.cover(Color32::from_black_alpha(185));
        ui::text(
            c,
            g,
            t("Replay", "リプレイ"),
            40.0,
            18.0,
            &ui::bold(22.0, ui::TEXT),
            300.0,
        );
        ui::text_right(
            c,
            g,
            &self.title,
            1240.0,
            22.0,
            &ui::plain(16.0, ui::DIM),
            800.0,
        );
        ui::round_fill(c, 300.0, 70.0, 680.0, 590.0, 6.0, PANEL_FILL);
        ui::round_frame(c, 300.0, 70.0, 680.0, 590.0, 6.0, 1.0, PANEL_LINE);
        ui::text(
            c,
            g,
            t("Plays of This Song", "この曲のプレイ"),
            324.0,
            84.0,
            &ui::bold(17.0, ui::TEXT),
            400.0,
        );
        let n = self.entries.len();
        ui::text_right(
            c,
            g,
            &tf!("Newest first · {n} plays", "新しい順・{n} 回"),
            956.0,
            88.0,
            &ui::plain(13.0, ui::MUTED),
            300.0,
        );
        if n == 0 {
            ui::paragraph(
                c,
                g,
                t(
                    "No plays yet. Play this song once and you can watch it again here.",
                    "まだプレイがありません。一度遊ぶと、ここから見直せます。",
                ),
                336.0,
                140.0,
                610.0,
                26.0,
                600.0,
                &ui::plain(16.0, ui::DIM),
            );
        }
        let top = self
            .cursor
            .saturating_sub(LIST_ROWS - 1)
            .min(n.saturating_sub(LIST_ROWS));
        for (row, (i, e)) in self
            .entries
            .iter()
            .enumerate()
            .skip(top)
            .take(LIST_ROWS)
            .enumerate()
        {
            let y = 124.0 + row as f32 * 70.0;
            if i == self.cursor {
                ui::round_fill(c, 316.0, y, 648.0, 62.0, 6.0, Color32::from_rgb(30, 28, 44));
                ui::round_frame(c, 316.0, y, 648.0, 62.0, 6.0, 2.0, ORANGE);
            } else {
                ui::round_fill(c, 316.0, y, 648.0, 62.0, 6.0, Color32::from_rgb(16, 16, 26));
            }
            let mid = y + 31.0;
            ui::text(
                c,
                g,
                &when_label(e.started_at),
                336.0,
                mid - 12.0,
                &ui::plain(17.0, ui::TEXT),
                180.0,
            );
            let rank = Rank::of(&e.tally);
            let rank_color = if matches!(rank, Rank::SS | Rank::S) {
                ORANGE
            } else {
                ui::TEXT
            };
            ui::text_center(
                c,
                g,
                &format!("{rank:?}"),
                540.0,
                mid - 17.0,
                &ui::bold(26.0, rank_color),
                60.0,
            );
            let rate = e
                .score
                .as_ref()
                .map_or_else(|| "—".to_owned(), |s| format!("{:.2}%", s.rate));
            ui::text(
                c,
                g,
                &rate,
                600.0,
                mid - 13.0,
                &ui::bold(19.0, ui::TEXT),
                110.0,
            );
            if let Some(s) = &e.score {
                ui::text(
                    c,
                    g,
                    &grouped(s.score),
                    720.0,
                    mid - 10.0,
                    &ui::plain(15.0, ui::DIM),
                    90.0,
                );
            }
            ui::text(
                c,
                g,
                &format!("MISS {}", e.tally.miss),
                820.0,
                mid - 10.0,
                &ui::plain(15.0, MISS_RED),
                80.0,
            );
            let tag = if self.changed[i] {
                Some((t("Edited", "譜面変更"), Color32::from_rgb(255, 154, 31)))
            } else if self.best == Some(e.session_id) {
                Some((t("Best", "ベスト"), Color32::from_rgb(255, 210, 58)))
            } else if !e.completed {
                Some((
                    t("Unfinished", "途中まで"),
                    Color32::from_rgb(150, 143, 172),
                ))
            } else {
                None
            };
            if let Some((tag, color)) = tag {
                let style = ui::bold(12.0, Color32::from_rgb(40, 20, 0));
                let w = ui::text_width(g, tag, &style) + 16.0;
                ui::round_fill(c, 952.0 - w, mid - 11.0, w, 22.0, 11.0, color);
                ui::text_center(c, g, tag, 952.0 - w / 2.0, mid - 9.0, &style, w);
            }
        }
        if top > 0 {
            ui::triangle(c, 640.0, 116.0, 10.0, -2, ui::MUTED);
        }
        if top + LIST_ROWS < n {
            ui::triangle(c, 640.0, 616.0, 10.0, 2, ui::MUTED);
        }
        ui::text(
            c,
            g,
            t(
                "Every play is saved for replay automatically, including earlier ones.",
                "リプレイはプレイのたびに自動で残ります（これまでのプレイも見られます）",
            ),
            324.0,
            628.0,
            &ui::plain(13.0, ui::MUTED),
            640.0,
        );
        ui::legend(
            c,
            g,
            &[
                (&[Pad::HT], t("Up", "上")),
                (&[Pad::LT], t("Down", "下")),
                (&[Pad::CY, Pad::RD], t("Play (Enter)", "再生する（Enter）")),
                (&[Pad::LC], t("Song Select (Esc)", "選曲へ（Esc）")),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::judge::Tally;

    fn entry(id: i64, rate: Option<f64>, hash: Option<u64>) -> PlayEntry {
        PlayEntry {
            session_id: id,
            started_at: 0,
            chart_hash: hash,
            tally: Tally::default(),
            completed: true,
            score: rate.map(|rate| records::PlayScore {
                score: 0,
                rate,
                progress: String::new(),
                cleared: true,
            }),
        }
    }

    #[test]
    fn the_list_marks_the_best_play_and_edited_charts() {
        let l = ReplayList::new(
            String::new(),
            vec![
                entry(3, Some(70.0), Some(1)),
                entry(2, Some(88.0), Some(1)),
                entry(1, None, Some(9)),
            ],
            Some(1),
        );
        assert_eq!(l.best, Some(2));
        assert_eq!(l.changed, vec![false, false, true]);
    }

    #[test]
    fn the_list_wraps_and_picks_the_play_under_the_cursor() {
        let mut l = ReplayList::new(
            String::new(),
            vec![entry(3, None, None), entry(2, None, None)],
            None,
        );
        assert_eq!(l.input(ListInput::Up).1, None);
        assert_eq!(l.input(ListInput::Decide).1, Some(ListDone::Play(2)));
        l.input(ListInput::Down);
        assert_eq!(l.input(ListInput::Decide).1, Some(ListDone::Play(3)));
        assert_eq!(l.input(ListInput::Cancel).1, Some(ListDone::Close));
        // 空の一覧では決められない
        let mut empty = ReplayList::new(String::new(), Vec::new(), None);
        assert_eq!(empty.input(ListInput::Decide), (None, None));
    }

    #[test]
    fn numbers_and_times_are_written_like_the_result_screen() {
        assert_eq!(grouped(629_648), "629,648");
        assert_eq!(grouped(1_000_000), "1,000,000");
        assert_eq!(grouped(12), "12");
        assert_eq!(clock(65_400.0), "1:05");
        assert_eq!(clock(-10.0), "0:00");
    }
}
