//! 基礎練習（ドリル）を演奏につなぐ部分。型と段とまとめ方は `engine::drill` が決め、
//! ここでは画面に出す名前、演奏中のクリックと手順、記録の読み方を受け持つ。
//!
//! ドリルは作った譜面（DTX の文字）をふつうの演奏と同じ判定に流す。記録の譜面のパスは `drill:<id>` で、
//! 曲のフォルダのファイルとは重ならない。スコアとスキルには入れない。

use std::path::PathBuf;

use dtx::{DrumLane, TempoMap};
use engine::drill::{Category, Drill, Hand, Run, STEP_MEASURES, Summary};
use i18n::t;

/// 右手と左手の色（音符の横の文字、結果のずれ）
pub const RIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x96, 0x46);
pub const LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(0x5A, 0xBE, 0xFF);

/// 記録に使う譜面のパスの頭
pub const PATH_PREFIX: &str = "drill:";

/// ドリルの記録の譜面のパス
pub fn chart_path(drill: &Drill) -> PathBuf {
    PathBuf::from(format!("{PATH_PREFIX}{}", drill.id))
}

/// 記録の譜面のパスからドリルを引く（ドリルでなければ None）
pub fn of_path(path: &std::path::Path) -> Option<&'static Drill> {
    let s = path.to_str()?;
    Drill::find(s.strip_prefix(PATH_PREFIX)?)
}

/// ドリルの名前
pub fn name(d: &Drill) -> &'static str {
    match d.id {
        "quarter" => t("Quarter Notes", "4 分音符"),
        "eighth" => t("8th Notes", "8 分音符"),
        "sixteenth" => t("16th Notes", "16 分音符"),
        "triplet" => t("Triplets", "3 連符"),
        "upbeat" => t("Upbeats", "裏拍"),
        "subdivisions" => t("Subdivision Ladder", "4・8・3 連・16 の切り替え"),
        "single_stroke" => t("Single Stroke Roll", "シングルストローク"),
        "double_stroke" => t("Double Stroke Roll", "ダブルストローク"),
        "paradiddle" => t("Paradiddle", "パラディドル"),
        "double_paradiddle" => t("Double Paradiddle", "ダブルパラディドル"),
        "paradiddle_diddle" => t("Paradiddle-diddle", "パラディドルディドル"),
        "six_stroke" => t("Six Stroke Roll", "シックスストローク"),
        "eight_beat" => t("8-Beat", "8 ビート"),
        "sixteen_beat" => t("16-Beat", "16 ビート"),
        "four_on_the_floor" => t("Four on the Floor", "4 つ打ち"),
        "shuffle" => t("Shuffle", "シャッフル"),
        "hand_foot_triplet" => t("Hand-Foot Triplets", "手足の 3 連"),
        "gap_2" => t("Gap Click (2 Bars)", "2 小節ごとにクリックが消える"),
        "gap_4" => t("Gap Click (4 Bars)", "4 小節ごとにクリックが消える"),
        _ => d.id,
    }
}

/// 分け方の名前
pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Timing => t("Timing", "タイミング"),
        Category::Rudiments => t("Rudiments", "ルーディメンツ"),
        Category::Grooves => t("Hands and Feet", "手と足"),
        Category::InnerClock => t("Inner Clock", "体内時計"),
    }
}

/// 手足の名前（結果の行）
pub fn hand_name(h: Hand) -> &'static str {
    match h {
        Hand::Right => t("Right", "右手"),
        Hand::Left => t("Left", "左手"),
        Hand::Foot => t("Foot", "足"),
    }
}

/// 手足の文字（音符の横に出す）。足は英語でも日本語でも F でなく、分かりやすい字に
pub fn hand_letter(h: Hand) -> &'static str {
    match h {
        Hand::Right => "R",
        Hand::Left => "L",
        Hand::Foot => t("F", "足"),
    }
}

/// 型の説明の行（「16 分音符 · スネア」）
pub fn pattern_line(d: &Drill) -> String {
    let notes = d.notes();
    let cells: Vec<u32> = notes.iter().map(|n| n.cells).collect();
    let lanes = d.lanes();
    let feel = if d.measures() > 1 {
        t("Mixed", "いろいろ")
    } else if cells.contains(&12) {
        t("Triplets", "3 連符")
    } else if notes.iter().all(|n| n.cell % 4 == 0) {
        t("Quarter Notes", "4 分音符")
    } else if notes.iter().all(|n| n.cell % 2 == 0) {
        t("8th Notes", "8 分音符")
    } else {
        t("16th Notes", "16 分音符")
    };
    let pads = if lanes == [DrumLane::SD] {
        t("Snare", "スネア").to_owned()
    } else {
        lanes
            .iter()
            .map(|l| l.name())
            .collect::<Vec<_>>()
            .join("・")
    };
    format!("{feel} · {pads}")
}

/// 演奏中のドリル（`Play` が持つ）
#[derive(Debug, Clone)]
pub struct DrillPlay {
    pub run: Run,
    /// 小節の頭の時刻（譜面の時刻。小節の数 + 1）
    measure_ms: Vec<f64>,
    /// 判定の音符ごとの手足（判定の音符と同じ並び）
    pub hands: Vec<Option<Hand>>,
    /// 最後に叩いた時刻（判定の時刻）。休みを見つけるのに使う
    last_hit_ms: Option<f64>,
    /// 休みを知らせた
    rest_sent: bool,
}

/// これだけ叩かなかったら止める（ミリ秒）。1 小節より短いときは 1 小節
const REST_MS: f64 = 4000.0;

impl DrillPlay {
    pub fn new(run: Run, tempo: &TempoMap, notes: &[dtx::DrumNote]) -> Self {
        let measure_ms = (0..=run.measure_count() as usize)
            .map(|m| tempo.measure_start_ms(m))
            .collect();
        let hands = notes.iter().map(|n| run.hand_at(n.at, n.lane)).collect();
        Self {
            run,
            measure_ms,
            hands,
            last_hit_ms: None,
            rest_sent: false,
        }
    }

    /// 譜面の時刻 `ms` の小節（カウントは 0。終わりを過ぎたら最後の次）
    pub fn measure_at(&self, ms: f64) -> u32 {
        let i = self.measure_ms.partition_point(|&s| s <= ms);
        i.saturating_sub(1) as u32
    }

    /// 小節の頭の時刻
    pub fn measure_start_ms(&self, m: u32) -> f64 {
        let last = self.measure_ms.len() - 1;
        self.measure_ms[(m as usize).min(last)]
    }

    /// 譜面の時刻 `ms` でクリックを鳴らすか（最後の小節が終わったら鳴らさない）
    pub fn clicks_at(&self, ms: f64) -> bool {
        let m = self.measure_at(ms + 0.5);
        m < self.run.measure_count() && self.run.clicks(m)
    }

    /// カウントの小節の何拍目か（1 から。カウントの外は None）
    pub fn count_in_beat(&self, ms: f64) -> Option<u32> {
        let (from, to) = (self.measure_ms[0], self.measure_ms[1]);
        if ms < from || ms >= to {
            return None;
        }
        Some(((ms - from) / ((to - from) / 4.0)) as u32 + 1)
    }

    /// 段 `step` の小節の範囲（譜面の小節）
    pub fn step_measures(&self, step: usize) -> std::ops::Range<u32> {
        let first = engine::drill::COUNT_IN_MEASURES + step as u32 * STEP_MEASURES;
        first..first + STEP_MEASURES
    }

    /// 叩いた（判定の時刻）
    pub fn on_hit(&mut self, ms: f64) {
        self.last_hit_ms = Some(ms);
        self.rest_sent = false;
    }

    /// 叩くのをやめたか。最初の音符から 1 小節（4 秒より短ければ 4 秒）叩かなければ 1 回だけ true
    pub fn check_rest(&mut self, now_ms: f64) -> bool {
        if self.rest_sent {
            return false;
        }
        let first = self.measure_ms[1];
        let end = *self.measure_ms.last().unwrap_or(&first);
        if now_ms >= end {
            return false;
        }
        let m = self.measure_at(now_ms).max(1);
        let bar = self.measure_start_ms(m + 1) - self.measure_start_ms(m);
        let since = self.last_hit_ms.unwrap_or(first).max(first);
        if now_ms - since >= bar.max(REST_MS) {
            self.rest_sent = true;
            return true;
        }
        false
    }

    /// 止めたあと、もう一度数え始める（つづけたとき）
    pub fn resume_at(&mut self, ms: f64) {
        self.last_hit_ms = Some(ms);
        self.rest_sent = false;
    }
}

/// 線に出す、これまでの回の数
pub const TRAIL_RUNS: usize = 8;

/// ドリル 1 つのこれまでの記録（基礎練習の画面に出す）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrillRecord {
    /// ミスなしで叩けた一番速いテンポ
    pub best: Option<u32>,
    /// 叩いた回数
    pub runs: u32,
    /// 最後に叩いた時刻（UNIX 秒）
    pub last_at: Option<i64>,
    /// 最近の回の、ミスなしで叩けた一番速いテンポ（古い順、ミスなしの段が無かった回は入れない）
    pub trail: Vec<u32>,
}

impl DrillRecord {
    /// 1 回を足す（古い順に）
    pub fn add(&mut self, started_at: i64, best: Option<u32>) {
        self.runs += 1;
        self.last_at = Some(self.last_at.map_or(started_at, |t| t.max(started_at)));
        if let Some(b) = best {
            self.best = Some(self.best.map_or(b, |x| x.max(b)));
            self.trail.push(b);
            if self.trail.len() > TRAIL_RUNS {
                self.trail.remove(0);
            }
        }
    }
}

/// ドリル 1 回の成績（結果の画面へ）
#[derive(Debug, Clone)]
pub struct DrillOutcome {
    pub run: Run,
    pub summary: Summary,
    /// 叩いた時間（カウントの頭から、やめるまで。ミリ秒）
    pub elapsed_ms: f64,
    /// 今回より前の、ミスなしで叩けた一番速いテンポ（記録を読んでから入れる）
    pub best_before: Option<u32>,
}

impl DrillOutcome {
    /// 今回の最高（ミスなしで叩けた一番速いテンポ）
    pub fn best_now(&self) -> Option<u32> {
        self.summary.best_clean_bpm()
    }

    /// 最高を更新した（前が無ければ、ミスなしの段があれば更新）
    pub fn new_best(&self) -> bool {
        match (self.best_now(), self.best_before) {
            (Some(now), Some(before)) => now > before,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(id: &str, target: u32, rise: u32) -> DrillPlay {
        let run = Run::new(Drill::find(id).unwrap(), target, rise);
        let chart = dtx::parse(&run.to_dtx(id));
        let tempo = TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        DrillPlay::new(run, &tempo, &notes)
    }

    #[test]
    fn paths_round_trip() {
        let d = Drill::find("paradiddle").unwrap();
        assert_eq!(of_path(&chart_path(d)), Some(d));
        assert_eq!(of_path(std::path::Path::new("/songs/a/b.dtx")), None);
    }

    #[test]
    fn every_drill_has_a_name_in_both_languages() {
        for d in engine::drill::DRILLS {
            let en = {
                let _en = i18n::scoped(i18n::Lang::En);
                name(d)
            };
            let ja = {
                let _ja = i18n::scoped(i18n::Lang::Ja);
                name(d)
            };
            assert_ne!(en, d.id, "{}", d.id);
            assert_ne!(en, ja, "{}", d.id);
        }
    }

    #[test]
    fn pattern_lines_describe_the_feel_and_pads() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        let line = |id| pattern_line(Drill::find(id).unwrap());
        assert_eq!(line("paradiddle"), "16 分音符 · スネア");
        assert_eq!(line("double_paradiddle"), "3 連符 · スネア");
        assert_eq!(line("eight_beat"), "8 分音符 · HH・SD・BD");
        assert_eq!(line("quarter"), "4 分音符 · スネア");
    }

    #[test]
    fn clicks_stop_in_the_gap_bars_and_count_in_is_beats() {
        let p = play("gap_2", 120, 0);
        // 120 BPM で 1 小節 2 秒。カウント、鳴る 2 小節、消える 2 小節
        assert!(p.clicks_at(0.0));
        assert!(p.clicks_at(2000.0));
        assert!(p.clicks_at(5999.0));
        assert!(!p.clicks_at(6000.0));
        assert!(!p.clicks_at(9500.0));
        assert!(p.clicks_at(10000.0));
        // 16 小節を叩き終えたら鳴らさない（60 BPM で 1 小節 4 秒、カウントの 1 小節 + 16 小節 = 68 秒）
        let e = play("eighth", 60, 0);
        assert!(e.clicks_at(67_000.0));
        assert!(!e.clicks_at(68_000.0));
        assert_eq!(p.count_in_beat(0.0), Some(1));
        assert_eq!(p.count_in_beat(1600.0), Some(4));
        assert_eq!(p.count_in_beat(2000.0), None);
        assert_eq!(p.measure_at(2000.0), 1);
        assert_eq!(p.step_measures(1), 5..9);
    }

    #[test]
    fn records_keep_the_best_and_a_short_trail() {
        let mut r = DrillRecord::default();
        for (i, b) in [Some(70), None, Some(80), Some(75)].into_iter().enumerate() {
            r.add(100 + i as i64, b);
        }
        assert_eq!(r.best, Some(80));
        assert_eq!(r.runs, 4);
        assert_eq!(r.last_at, Some(103));
        assert_eq!(r.trail, vec![70, 80, 75]);
        for i in 0..10 {
            r.add(200 + i, Some(90 + i as u32));
        }
        assert_eq!(r.trail.len(), TRAIL_RUNS);
        assert_eq!(r.trail.last(), Some(&99));
    }

    #[test]
    fn hands_follow_the_judge_notes() {
        let p = play("paradiddle", 100, 10);
        let s: String = p.hands[..8].iter().map(|h| h.unwrap().letter()).collect();
        assert_eq!(s, "RLRRLRLL");
    }

    #[test]
    fn stopping_for_a_bar_is_a_rest_once() {
        let mut p = play("eighth", 60, 0);
        // 60 BPM で 1 小節 4 秒。最初の音符は 4000ms
        assert!(!p.check_rest(7000.0));
        assert!(p.check_rest(8000.0));
        assert!(!p.check_rest(9000.0));
        p.resume_at(9000.0);
        p.on_hit(9100.0);
        assert!(!p.check_rest(12000.0));
        assert!(p.check_rest(13200.0));
    }
}
