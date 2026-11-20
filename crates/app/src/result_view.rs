//! 結果の画面（上流 `CStageResult`）。
//! 1 ページ目は NX のスキンの画像で、ランク、判定の数と割合、スコア、達成率とスキル、
//! 進み具合のバー（今回と前回までのベスト）、ジャケットと曲名を上流と同じ配置で出す。
//! 2 ページ目（←→ で切り替え）は、このアプリで足した分析（苦手な小節、叩き方のくせ、練習の周ごとの成績）。

use std::f64::consts::FRAC_PI_2;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::Color32;
use engine::judge::Tally;
use records::{BestScore, PROGRESS_SECTIONS};

use crate::gfx::{Canvas, Gfx, Opt, fit_scale, src};
use crate::play_view::{
    Finish, LARGE_RESULT, PanelNumbers, draw_score_digits, draw_skill_panel, rotate_about_center,
};
use crate::sfx::Se;
use crate::text::Style;

/// 入ってきたときのフェード（上流 `CActFIFOBlack`）
const FADE_IN: Duration = Duration::from_millis(500);
/// 結果の音から New Record の音までの間（上流は 10 ミリ秒 × 150）
const NEW_RECORD_DELAY: Duration = Duration::from_millis(1500);

/// 見ているページ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// NX の結果の画面
    Summary,
    /// このアプリで足した分析
    Analysis,
}

/// ランク（上流 `CScoreIni.ERANK`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rank {
    SS,
    S,
    A,
    B,
    C,
    D,
    E,
}

impl Rank {
    /// 上流 `tCalculateRank`：判定した数に対する PERFECT・GREAT・最大コンボの割合から出す。
    /// 判定が 1 つも無ければ E（上流の UNKNOWN も E の画像を出す）
    pub fn of(t: &Tally) -> Self {
        let judged = t.perfect + t.great + t.good + t.poor + t.miss;
        if judged == 0 {
            return Self::E;
        }
        let n = f64::from(judged);
        let rate = 100.0 * f64::from(t.perfect) / n * 0.85
            + 100.0 * f64::from(t.great) / n * 0.35
            + 100.0 * f64::from(t.max_combo) / n * 0.15;
        match rate {
            r if r >= 95.0 => Self::SS,
            r if r >= 80.0 => Self::S,
            r if r >= 73.0 => Self::A,
            r if r >= 63.0 => Self::B,
            r if r >= 53.0 => Self::C,
            r if r >= 45.0 => Self::D,
            _ => Self::E,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::SS => "SS",
            Self::S => "S",
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
        }
    }
}

/// 終わり方（上流の Excellent・フルコンボ・ステージクリア）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clear {
    /// 全部 PERFECT
    Excellent,
    /// POOR と MISS が無い
    FullCombo,
    Cleared,
}

impl Clear {
    pub fn of(t: &Tally, notes: usize) -> Self {
        let judged = t.perfect + t.great + t.good + t.poor + t.miss;
        if notes > 0 && t.perfect as usize >= notes {
            Self::Excellent
        } else if judged > 0 && t.poor == 0 && t.miss == 0 {
            Self::FullCombo
        } else {
            Self::Cleared
        }
    }

    /// 上流 `CStageResult`：入ってきたときに鳴らす音
    fn sound(self) -> Se {
        match self {
            Self::Excellent => Se::Excellent,
            Self::FullCombo => Se::FullCombo,
            Self::Cleared => Se::StageClear,
        }
    }

    fn image(self) -> &'static str {
        match self {
            Self::Excellent => "Graphics/ScreenResult Excellent.png",
            Self::FullCombo => "Graphics/ScreenResult fullcombo.png",
            Self::Cleared => "Graphics/ScreenResult StageCleared.png",
        }
    }
}

/// 結果の画面に出すもの
pub struct ResultInfo {
    pub title: String,
    pub artist: String,
    pub preimage: Option<PathBuf>,
    pub level: Option<f64>,
    pub tally: Tally,
    pub notes: usize,
    pub practice: bool,
    /// 普通のプレイを最後まで遊んだ（途中でやめたときと練習は false）
    pub cleared: bool,
    pub finish: Finish,
    /// これまでのベスト（今回の前まで）。記録が読めなければ None
    pub best: Option<BestScore>,
    /// 早かった数と遅かった数を出す（NX の `ShowLagHitCount`）。`Some(true)` なら色を入れ替える
    pub lag_counts: Option<bool>,
}

pub struct ResultView {
    info: ResultInfo,
    entered: Instant,
    pub page: Page,
    rank: Rank,
    /// 最後まで遊んだときだけ（上流は途中でやめると結果の画面を出さない）
    clear: Option<Clear>,
    new_record: bool,
    /// まだ鳴らしていない音と、鳴らす時刻（入ってきてから）
    sounds: Vec<(Duration, Se)>,
}

impl ResultView {
    pub fn new(info: ResultInfo) -> Self {
        let rank = Rank::of(&info.tally);
        let clear = info.cleared.then(|| Clear::of(&info.tally, info.notes));
        // 上流：今回の達成率が前の一番より高ければ New Record（前が無ければ 0 と比べる）。練習は記録しない
        let new_record = info.cleared
            && !info.practice
            && info
                .best
                .as_ref()
                .is_some_and(|b| info.finish.rate > b.rate.unwrap_or(0.0));
        let mut sounds: Vec<(Duration, Se)> =
            clear.iter().map(|c| (Duration::ZERO, c.sound())).collect();
        if new_record {
            sounds.push((NEW_RECORD_DELAY, Se::NewRecord));
        }
        Self {
            // 練習は周ごとの成績を先に見せる
            page: if info.practice {
                Page::Analysis
            } else {
                Page::Summary
            },
            info,
            entered: Instant::now(),
            rank,
            clear,
            new_record,
            sounds,
        }
    }

    /// 鳴らす時刻になった音
    pub fn due_sounds(&mut self) -> Vec<Se> {
        let elapsed = self.entered.elapsed();
        let (due, later): (Vec<_>, Vec<_>) =
            self.sounds.drain(..).partition(|(at, _)| *at <= elapsed);
        self.sounds = later;
        due.into_iter().map(|(_, se)| se).collect()
    }

    /// ページを切り替える（←→）
    pub fn turn(&mut self) {
        self.page = match self.page {
            Page::Summary => Page::Analysis,
            Page::Analysis => Page::Summary,
        };
    }

    /// 背景（上流：ランクごとの背景があればそれ、無ければ 8_background.jpg）
    pub fn draw_background(&self, c: &Canvas, g: &mut Gfx) {
        let ranked = format!("Graphics/8_background rank{}.png", self.rank.name());
        let bg = g
            .skin
            .tex_optional(&ranked)
            .or_else(|| g.skin.tex("Graphics/8_background.jpg"));
        match bg {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(8, 8, 14)),
        }
    }

    /// 1 ページ目（背景は [`Self::draw_background`] で先に描く）
    pub fn draw_summary(&self, c: &Canvas, g: &mut Gfx) {
        let elapsed = self.entered.elapsed();
        self.draw_panels(c, g, elapsed);
        self.draw_image(c, g);
        self.draw_parameters(c, g);
        self.draw_rank(c, g, elapsed);
        let hint = if self.info.practice {
            "→ 周ごとの成績"
        } else {
            "→ 詳しい結果（苦手な小節、叩き方のくせ）"
        };
        self.draw_page_hint(c, g, hint);
    }

    /// 2 ページ目の上に出す、ページの切り替えの案内
    pub fn draw_analysis_hint(&self, c: &Canvas, g: &mut Gfx) {
        self.draw_page_hint(c, g, "← NX の結果の画面");
    }

    /// 入ってきたときのフェード。いちばん上に描く
    pub fn draw_fade(&self, c: &Canvas) {
        let t = self.entered.elapsed().as_secs_f32() / FADE_IN.as_secs_f32();
        if t < 1.0 {
            c.cover(Color32::from_black_alpha(((1.0 - t) * 255.0) as u8));
        }
    }

    fn draw_page_hint(&self, c: &Canvas, g: &mut Gfx, text: &str) {
        let style = Style {
            px: 16.0,
            color: Color32::from_gray(230),
            bold: false,
            shadow: None,
            edge: Some((Color32::BLACK, 1.5)),
            gradient: None,
        };
        if let Some(t) = g.texts.get(text, &style) {
            t.draw(c, 1270.0 - t.w, 696.0, 1.0, 1.0);
        }
    }

    /// 上と下の帯（上流は 8_header panel.png が上から降りてくる。Default のスキンには無い）
    fn draw_panels(&self, c: &Canvas, g: &mut Gfx, elapsed: Duration) {
        if let Some(t) = g.skin.tex_optional("Graphics/8_header panel.png") {
            let v = (elapsed.as_secs_f64() / 0.5).min(1.0);
            let y = (f64::from(t.h) * (FRAC_PI_2 * v).sin()) as f32 - t.h;
            c.draw(&t, 0.0, y);
        }
        if let Some(t) = g.skin.tex_optional("Graphics/8_footer panel.png") {
            c.draw(&t, 0.0, 720.0 - t.h);
        }
    }

    /// ジャケットと曲名と注意書き（上流 `CActResultImage`）
    fn draw_image(&self, c: &Canvas, g: &mut Gfx) {
        if let Some(t) = g.skin.tex("Graphics/7_JacketPanel.png") {
            c.draw(&t, 467.0, 287.0);
        }
        let default = "Graphics/5_preimage default.png";
        let jacket = match &self.info.preimage {
            Some(p) => g
                .images
                .get(p)
                .or_else(|| g.images.failed(p).then(|| g.skin.tex(default)).flatten()),
            None => g.skin.tex(default),
        };
        if let Some(t) = jacket {
            // 上流は 245 に縮めて (-28, -94.5) へ動かし、画面の中心のまわりに 0.3 ラジアン回す
            let f = fit_scale(&t, 245.0);
            let (cx, cy) = rotate_about_center(-28.0, -94.5, 0.3);
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
        if let Some(t) = g.texts.get(&self.info.title, &edge(20.0)) {
            t.draw_fit(c, 500.0, 630.0, 320.0);
        }
        if let Some(t) = g.texts.get(&self.info.artist, &edge(15.0)) {
            t.draw_fit(c, 500.0, 665.0, 320.0);
        }
        // 上流の「Training feature used」「Score will not be saved」
        if self.info.practice {
            let mut y = 360.0;
            for line in ["練習モード", "スコアと New Record には入りません"] {
                if let Some(t) = g.texts.get(line, &edge(18.0)) {
                    t.draw(c, 840.0, y, 1.0, 1.0);
                }
                y += 25.0;
            }
        }
    }

    /// 左のパネル、進み具合のバー、スコア、New Record、早い遅いの数（上流 `CActResultParameterPanel`）
    fn draw_parameters(&self, c: &Canvas, g: &mut Gfx) {
        const X: f32 = 180.0;
        const Y: f32 = 260.0;
        let f = &self.info.finish;
        draw_skill_panel(
            c,
            g,
            (X, Y),
            &PanelNumbers {
                tally: &self.info.tally,
                rate: f.rate,
                level: self.info.level.unwrap_or(0.0),
                diff_panel: f.diff_panel,
            },
            &LARGE_RESULT,
        );
        if let Some(t) = g.skin.tex("Graphics/8_progress_bar_panel.png") {
            c.draw(&t, X + 255.0, Y + 1.0);
        }
        draw_progress_bar(c, X + 256.0, Y + 2.0, 12.0, &f.progress);
        let best = self
            .info
            .best
            .as_ref()
            .and_then(|b| b.progress.as_deref())
            .unwrap_or("");
        draw_progress_bar(c, X + 270.0, Y + 2.0, 4.0, best);
        if let Some(t) = g.skin.tex("Graphics/7_score numbersGD.png") {
            draw_score_digits(c, &t, 30.0, 58.0, 30.0, f.score);
        }
        if self.new_record
            && let Some(t) = g.skin.tex("Graphics/8_New Record.png")
        {
            c.draw(&t, X + 118.0, Y + 322.0);
        }
        if let Some(swapped) = self.info.lag_counts
            && let Some(t) = g.skin.tex("Graphics/7_lag numbers.png")
        {
            // Type-A（既定）は早いが青、遅いが赤
            for (x, n, red) in [(170.0, f.early, swapped), (245.0, f.late, !swapped)] {
                let mut right = X + x;
                for ch in format!("{n:>4}").chars().rev() {
                    if let Some(d) = ch.to_digit(10) {
                        let d = d as f32;
                        let (ox, oy) = if red { (64.0, 64.0) } else { (0.0, 0.0) };
                        let s = src(
                            ox + (d % 4.0) * 15.0,
                            oy + (d / 4.0).floor() * 19.0,
                            15.0,
                            19.0,
                        );
                        c.part(&t, right - 15.0, Y + 335.0, s);
                    }
                    right -= 15.0;
                }
            }
        }
    }

    /// ランクの文字（下から現れる）と、Excellent・フルコンボ・ステージクリア（上流 `CActResultRank`）
    fn draw_rank(&self, c: &Canvas, g: &mut Gfx, elapsed: Duration) {
        const X: f32 = 480.0;
        const Y: f32 = 0.0;
        let v = elapsed.as_secs_f64() * 1000.0;
        if v >= 200.0
            && let Some(t) = g
                .skin
                .tex(&format!("Graphics/8_rank{}.png", self.rank.name()))
        {
            let shown = ((v - 200.0) / 300.0).min(1.0) as f32;
            let h = (t.h * shown).floor();
            c.part(&t, X, Y + t.h - h, src(0.0, 0.0, t.w, h));
        }
        if let Some(t) = self.clear.and_then(|k| g.skin.tex(k.image())) {
            c.draw(&t, X - 165.0, Y + 100.0);
        }
    }
}

/// 進み具合のバー（上流 `txGenerateProgressBarHelper`：64 区間を下から、425 ドットの高さに）
fn draw_progress_bar(c: &Canvas, x: f32, y: f32, w: f32, progress: &str) {
    const H: f32 = 425.0;
    let chars: Vec<char> = progress.chars().collect();
    if chars.len() != PROGRESS_SECTIONS {
        c.fill(x, y, w, H, Color32::BLACK);
        return;
    }
    let mut below = H;
    for (i, ch) in chars.into_iter().enumerate() {
        let top = (H - (i as f32 + 1.0) * H / PROGRESS_SECTIONS as f32).round();
        let color = match ch {
            '1' => Color32::from_rgb(0, 191, 255),
            '2' | '3' => Color32::from_rgb(255, 255, 0),
            _ => Color32::BLACK,
        };
        c.fill(x, y + top, w, below - top, color);
        below = top;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(perfect: u32, great: u32, good: u32, poor: u32, miss: u32, max_combo: u32) -> Tally {
        Tally {
            perfect,
            great,
            good,
            poor,
            miss,
            max_combo,
            ..Tally::default()
        }
    }

    #[test]
    fn rank_follows_the_upstream_thresholds() {
        // 全部 PERFECT：85 + 15 = 100
        assert_eq!(Rank::of(&tally(100, 0, 0, 0, 0, 100)), Rank::SS);
        // PERFECT 90、GREAT 10、最大コンボ 100：76.5 + 3.5 + 15 = 95
        assert_eq!(Rank::of(&tally(90, 10, 0, 0, 0, 100)), Rank::SS);
        // PERFECT 80、GREAT 20、最大コンボ 50：68 + 7 + 7.5 = 82.5
        assert_eq!(Rank::of(&tally(80, 20, 0, 0, 0, 50)), Rank::S);
        // PERFECT 50、GOOD 50、最大コンボ 10：42.5 + 1.5 = 44
        assert_eq!(Rank::of(&tally(50, 0, 50, 0, 0, 10)), Rank::E);
        // PERFECT 50、GREAT 10、MISS 40、最大コンボ 30：42.5 + 3.5 + 4.5 = 50.5
        assert_eq!(Rank::of(&tally(50, 10, 0, 0, 40, 30)), Rank::D);
        assert_eq!(Rank::of(&Tally::default()), Rank::E);
    }

    #[test]
    fn clear_kinds() {
        assert_eq!(Clear::of(&tally(10, 0, 0, 0, 0, 10), 10), Clear::Excellent);
        assert_eq!(Clear::of(&tally(8, 1, 1, 0, 0, 10), 10), Clear::FullCombo);
        assert_eq!(Clear::of(&tally(9, 0, 0, 0, 1, 9), 10), Clear::Cleared);
        assert_eq!(Clear::of(&Tally::default(), 0), Clear::Cleared);
    }

    fn info(rate: f64, best: Option<BestScore>, practice: bool) -> ResultInfo {
        ResultInfo {
            title: "t".into(),
            artist: "a".into(),
            preimage: None,
            level: Some(6.5),
            tally: tally(9, 0, 0, 0, 1, 9),
            notes: 10,
            practice,
            cleared: !practice,
            finish: Finish {
                score: 1000,
                rate,
                progress: "2".repeat(64),
                early: 3,
                late: 4,
                diff_panel: (0.0, 0.0),
            },
            best,
            lag_counts: None,
        }
    }

    fn best(rate: Option<f64>) -> BestScore {
        BestScore {
            rate,
            score: None,
            progress: None,
        }
    }

    #[test]
    fn new_record_beats_the_previous_best_rate() {
        assert!(ResultView::new(info(70.0, Some(best(Some(60.0))), false)).new_record);
        // 同じなら新記録ではない
        assert!(!ResultView::new(info(60.0, Some(best(Some(60.0))), false)).new_record);
        // 初めて最後まで遊んだ
        assert!(ResultView::new(info(10.0, Some(best(None)), false)).new_record);
        // 記録が読めない、練習
        assert!(!ResultView::new(info(70.0, None, false)).new_record);
        assert!(!ResultView::new(info(70.0, Some(best(None)), true)).new_record);
    }

    #[test]
    fn quitting_midway_is_not_a_record() {
        let mut i = info(70.0, Some(best(None)), false);
        i.cleared = false;
        let mut v = ResultView::new(i);
        assert!(!v.new_record);
        assert!(v.due_sounds().is_empty());
    }

    #[test]
    fn result_sound_first_then_new_record_later() {
        let mut v = ResultView::new(info(70.0, Some(best(None)), false));
        assert_eq!(v.due_sounds(), vec![Se::StageClear]);
        assert!(v.due_sounds().is_empty());
        v.entered -= NEW_RECORD_DELAY;
        assert_eq!(v.due_sounds(), vec![Se::NewRecord]);
    }

    #[test]
    fn practice_opens_on_the_analysis_page() {
        assert_eq!(ResultView::new(info(0.0, None, true)).page, Page::Analysis);
        let mut v = ResultView::new(info(70.0, None, false));
        assert_eq!(v.page, Page::Summary);
        v.turn();
        assert_eq!(v.page, Page::Analysis);
    }
}
