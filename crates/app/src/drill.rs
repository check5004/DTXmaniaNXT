//! 基礎練習（ドリル）を演奏につなぐ部分。型と段とまとめ方は `engine::drill` が決め、
//! ここでは画面に出す名前、演奏中のクリックと手順、記録の読み方を受け持つ。
//!
//! ドリルは作った譜面（DTX の文字）をふつうの演奏と同じ判定に流す。記録の譜面のパスは `drill:<id>` で、
//! 曲のフォルダのファイルとは重ならない。スコアとスキルには入れない。

use std::path::PathBuf;

use dtx::{DrumLane, TempoMap};
use engine::drill::{Category, Drill, Hand, MANY_MISSES, Run, Summary, Way};
use engine::judge::Judgement;
use i18n::{t, tf};

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
        // タイミング
        "quarter" => t("Quarter Notes", "4 分音符"),
        "eighth" => t("8th Notes", "8 分音符"),
        "sixteenth" => t("16th Notes", "16 分音符"),
        "triplet" => t("Triplets", "3 連符"),
        "upbeat" => t("Upbeats", "裏拍"),
        "sixteenth_e" => t("16th Offbeats (e)", "16 分の 2 つ目だけ"),
        "sixteenth_a" => t("16th Offbeats (a)", "16 分の 4 つ目だけ"),
        "dotted_sixteenth" => t("Dotted 8th + 16th", "付点 8 分と 16 分"),
        "sixteenth_dotted" => t("16th + Dotted 8th", "16 分と付点 8 分"),
        "dotted_eighths" => t("Dotted 8ths Across the Bar", "付点 8 分の連続"),
        "quarter_triplet" => t("Quarter-Note Triplets", "2 拍 3 連"),
        "swung_eighths" => t("Swung 8ths", "ハネた 8 分"),
        "quintuplet" => t("Quintuplets", "5 連符"),
        "sextuplet" => t("Sextuplets", "6 連符"),
        "subdivisions" => t("Subdivision Ladder", "4・8・3 連・16 の切り替え"),
        "eighth_sixteenth" => t("8ths to 16ths", "8 分と 16 分の切り替え"),
        "triplet_sixteenth" => t("Triplets to 16ths", "3 連と 16 分の切り替え"),
        // ルーディメンツ
        "single_stroke" => t("Single Stroke Roll", "シングルストローク"),
        "double_stroke" => t("Double Stroke Roll", "ダブルストローク"),
        "triple_stroke" => t("Triple Stroke Roll", "トリプルストローク"),
        "five_stroke" => t("Five Stroke Roll", "5 ストロークロール"),
        "six_stroke" => t("Six Stroke Roll", "シックスストローク"),
        "seven_stroke" => t("Seven Stroke Roll", "7 ストロークロール"),
        "nine_stroke" => t("Nine Stroke Roll", "9 ストロークロール"),
        "paradiddle" => t("Paradiddle", "パラディドル"),
        "inverted_paradiddle" => t("Inverted Paradiddle", "インバーテッドパラディドル"),
        "reverse_paradiddle" => t("Reverse Paradiddle", "リバースパラディドル"),
        "double_paradiddle" => t("Double Paradiddle", "ダブルパラディドル"),
        "triple_paradiddle" => t("Triple Paradiddle", "トリプルパラディドル"),
        "paradiddle_diddle" => t("Paradiddle-diddle", "パラディドルディドル"),
        "paradiddle_triplets" => t("Paradiddle Triplets", "3 連のパラディドル"),
        "single_stroke_four" => t("Single Stroke Four", "シングルストローク 4"),
        "single_stroke_seven" => t("Single Stroke Seven", "シングルストローク 7"),
        "double_triplets" => t("Double Strokes in Triplets", "3 連のダブルストローク"),
        // ビート
        "eight_beat_basic" => t("Basic 8-Beat", "8 ビート（基本）"),
        "eight_beat" => t("8-Beat: Kick on 3&", "8 ビート：3 拍目の裏にバスドラ"),
        "kick_and_of_2" => t("8-Beat: Kick on 2&", "8 ビート：2 拍目の裏にバスドラ"),
        "kick_and_of_4" => t("8-Beat: Kick on 4&", "8 ビート：4 拍目の裏にバスドラ"),
        "kick_sixteenth" => t("8-Beat: 16th Kicks", "8 ビート：16 分のバスドラ"),
        "quarter_hats" => t("Quarter-Note Hi-Hat", "4 分のハイハット"),
        "sixteen_beat" => t("16-Beat", "16 ビート"),
        "sixteen_beat_kick" => t("16-Beat: Busy Kick", "16 ビート：バスドラ多め"),
        "sixteen_one_hand" => t("One-Hand 16-Beat", "片手の 16 ビート"),
        "half_time" => t("Half-Time", "ハーフタイム"),
        "two_beat" => t("Two-Beat", "2 ビート"),
        "snare_every_beat" => t("Snare on Every Beat", "スネアの 4 つ打ち"),
        "four_on_the_floor" => t("Four on the Floor", "4 つ打ち"),
        "shuffle" => t("Shuffle", "シャッフル"),
        "hand_foot_triplet" => t("Hand-Foot Triplets", "手足の 3 連"),
        // ジャンル
        "funk_ghost" => t("Funk with Ghost Notes", "ファンク（ゴーストノート）"),
        "linear" => t("Linear Groove", "リニア"),
        "disco" => t("Disco", "ディスコ"),
        "hip_hop" => t("Hip-Hop", "ヒップホップ"),
        "half_time_shuffle" => t("Half-Time Shuffle", "ハーフタイムシャッフル"),
        "twelve_eight" => t("12/8 Ballad", "12/8 のバラード"),
        "one_drop" => t("One Drop", "ワンドロップ（レゲエ）"),
        "bossa_nova" => t("Bossa Nova", "ボサノバ"),
        "samba" => t("Samba", "サンバ"),
        "jazz_ride" => t("Jazz Ride", "ジャズのライド"),
        "jazz_comping" => t("Jazz Comping", "ジャズのコンピング"),
        "drum_and_bass" => t("Drum and Bass", "ドラムンベース"),
        "blast_beat" => t("Blast Beat", "ブラストビート"),
        // タム回しとフィル
        "toms_eighths" => t("Around the Toms: 8ths", "タム回し（8 分）"),
        "toms_sixteenths" => t("Around the Toms: 16ths", "タム回し（16 分）"),
        "toms_upward" => t("Toms Going Up", "タム回し（下から上へ）"),
        "toms_triplets" => t("Around the Toms: Triplets", "タム回し（3 連）"),
        "toms_sextuplets" => t("Around the Toms: Sextuplets", "タム回し（6 連）"),
        "snare_and_toms" => t("Snare and Toms", "スネアとタムの行き来"),
        "rll_fill" => t("RLL Fill", "RLL のフィル"),
        "hand_hand_foot" => t("Hand-Hand-Foot Fill", "手・手・足のフィル"),
        "fill_one_beat" => t("Groove + 1-Beat Fill", "ビートと 1 拍のフィル"),
        "fill_two_beats" => t("Groove + 2-Beat Fill", "ビートと 2 拍のフィル"),
        "fill_four_bars" => t("3 Bars + 1-Bar Fill", "3 小節のビートと 1 小節のフィル"),
        // 足
        "kick_eighths" => t("8th-Note Kicks", "バスドラの 8 分"),
        "kick_upbeats" => t("Kick on the Upbeats", "バスドラの裏拍"),
        "kick_doubles" => t("Kick Doubles", "バスドラの 16 分 2 つ"),
        "hihat_foot" => t("Hi-Hat Foot on the Beat", "ハイハットペダルで 4 分"),
        "hihat_foot_2_4" => t("Hi-Hat Foot on 2 and 4", "ハイハットペダルを 2・4 拍で"),
        "kick_and_hihat_foot" => t("Kick and Hi-Hat Foot", "バスドラとハイハットペダル"),
        "double_bass_eighths" => t("Double Bass: 8ths", "ツーバスの 8 分"),
        "double_bass_triplets" => t("Double Bass: Triplets", "ツーバスの 3 連"),
        "double_bass_sixteenths" => t("Double Bass: 16ths", "ツーバスの 16 分"),
        "gallop" => t("Gallop", "ギャロップ"),
        // シンバルとハイハット
        "open_hihat_four" => t("Open Hi-Hat on 4&", "4 拍目の裏でオープン"),
        "open_hihat_upbeats" => t("Open Hi-Hat on the Upbeats", "裏拍でオープン"),
        "ride_and_crash" => t("Ride with a Crash on 1", "ライドと頭のクラッシュ"),
        "hats_to_ride" => t("Hi-Hat to Ride", "ハイハットとライドの持ち替え"),
        "left_right_crash" => t("Left and Right Crash", "左右のクラッシュ"),
        "crash_every_beat" => t("Crash on Every Beat", "クラッシュの 4 つ打ち"),
        "kicks_and_crashes" => t("Hits with Crash and Kick", "キメ（クラッシュとバスドラ）"),
        // 拍子とポリリズム
        "waltz" => t("Waltz (3/4)", "ワルツ（3/4）"),
        "three_four" => t("3/4 Groove", "3/4 のビート"),
        "six_eight" => t("6/8 Groove", "6/8 のビート"),
        "five_four" => t("5/4 Groove", "5/4 のビート"),
        "seven_eight" => t("7/8 Groove (2+2+3)", "7/8 のビート（2+2+3）"),
        "three_against_two" => t("3 Against 2", "3 対 2"),
        "four_against_three" => t("4 Against 3", "4 対 3"),
        "triplets_over_kick" => t("Triplets over Quarter Kicks", "4 分のバスドラに 3 連"),
        "dotted_quarter_kick" => t("Dotted-Quarter Kicks", "付点 4 分のバスドラ"),
        // 体内時計
        "gap_1" => t(
            "Gap Click (Every Other Bar)",
            "1 小節おきにクリックが消える",
        ),
        "gap_2" => t("Gap Click (2 Bars)", "2 小節ごとにクリックが消える"),
        "gap_4" => t("Gap Click (4 Bars)", "4 小節ごとにクリックが消える"),
        "gap_long" => t("Gap Click (6 Bars)", "クリックが 6 小節消える"),
        "groove_gap" => t("8-Beat with Gap Click", "8 ビートでクリックが消える"),
        "click_two_four" => t("Click on 2 and 4", "クリックは 2・4 拍目だけ"),
        "click_one" => t("Click on 1 Only", "クリックは 1 拍目だけ"),
        "click_upbeats" => t("Click on the Upbeats", "クリックは裏拍"),
        _ => d.id,
    }
}

/// 分け方の名前
pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Timing => t("Timing", "タイミング"),
        Category::Rudiments => t("Rudiments", "ルーディメンツ"),
        Category::Grooves => t("Beats", "ビート"),
        Category::Styles => t("Styles", "ジャンル"),
        Category::Fills => t("Fills and Toms", "タム回しとフィル"),
        Category::Feet => t("Feet", "足"),
        Category::Cymbals => t("Cymbals and Hi-Hat", "シンバルとハイハット"),
        Category::Meters => t("Meters and Polyrhythms", "拍子とポリリズム"),
        Category::InnerClock => t("Inner Clock", "体内時計"),
    }
}

/// やり方の名前（基礎練習の画面の札、結果の見出し）
pub fn way_name(w: Way) -> String {
    match w {
        Way::Climb => t("Speed Up", "テンポを上げる").to_owned(),
        Way::Limit => t("Limit Test", "限界チャレンジ").to_owned(),
        Way::Endurance(m) => tf!("Endurance {m} min", "持久 {m} 分"),
        Way::Blind => t("Eyes Off", "見ないで").to_owned(),
        Way::Echo => t("Call and Response", "聞いてまねる").to_owned(),
    }
}

/// 手順の名前（右から・左から）
pub fn lead_name(lefty: bool) -> &'static str {
    if lefty {
        t("Left Lead", "左から")
    } else {
        t("Right Lead", "右から")
    }
}

/// 手足の名前（結果の行）
pub fn hand_name(h: Hand) -> &'static str {
    match h {
        Hand::Right => t("Right", "右手"),
        Hand::Left => t("Left", "左手"),
        Hand::Foot => t("Right Foot", "右足"),
        Hand::LeftFoot => t("Left Foot", "左足"),
    }
}

/// 手足の文字（音符の横に出す）。足は英語でも日本語でも F でなく、分かりやすい字に
pub fn hand_letter(h: Hand) -> &'static str {
    match h {
        Hand::Right => "R",
        Hand::Left => "L",
        Hand::Foot => t("F", "足"),
        Hand::LeftFoot => t("LF", "左足"),
    }
}

/// 型の説明の行（「16 分音符 · スネア」「3/4 · 8 分音符 · HH・SD・BD」）
pub fn pattern_line(d: &Drill) -> String {
    let lanes = d.lanes();
    let feel = if d.measures() > 1 {
        t("Mixed", "ミックス")
    } else {
        match finest_division(d) {
            Some(1) => t("Quarter Notes", "4 分音符"),
            Some(2) => t("8th Notes", "8 分音符"),
            Some(3) => t("Triplets", "3 連符"),
            Some(4) => t("16th Notes", "16 分音符"),
            Some(5) => t("Quintuplets", "5 連符"),
            Some(6) => t("Sextuplets", "6 連符"),
            _ => t("Mixed", "ミックス"),
        }
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
    if d.meter == (4, 4) {
        format!("{feel} · {pads}")
    } else {
        format!("{}/{} · {feel} · {pads}", d.meter.0, d.meter.1)
    }
}

/// 4 分音符ひとつをいくつに分ければ型の音符がみな乗るか（1・2・3・4・5・6 のうち一番少ないもの）
fn finest_division(d: &Drill) -> Option<u32> {
    let quarters = f64::from(d.meter.0) * 4.0 / f64::from(d.meter.1);
    let notes = d.notes();
    [1u32, 2, 4, 3, 6, 5].into_iter().find(|&k| {
        notes.iter().all(|n| {
            let x = n.pos() * quarters * f64::from(k);
            (x - x.round()).abs() < 1e-6
        })
    })
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
    /// 段ごとの落とした数（POOR と MISS。限界チャレンジで終わるかを見る）
    misses: Vec<u32>,
    /// 限界チャレンジで終えた段と、その時刻（判定の時刻）
    pub stopped: Option<(usize, f64)>,
}

/// これだけ叩かなかったら止める（ミリ秒）。1 小節より短いときは 1 小節
const REST_MS: f64 = 4000.0;

impl DrillPlay {
    pub fn new(run: Run, tempo: &TempoMap, notes: &[dtx::DrumNote]) -> Self {
        let measure_ms = (0..=run.measure_count() as usize)
            .map(|m| tempo.measure_start_ms(m))
            .collect();
        let hands = notes.iter().map(|n| run.hand_at(n.at, n.lane)).collect();
        let misses = vec![0; run.steps()];
        Self {
            run,
            measure_ms,
            hands,
            last_hit_ms: None,
            rest_sent: false,
            misses,
            stopped: None,
        }
    }

    /// お手本の音の時刻（譜面の時刻）とレーン。聞いてまねるだけ
    pub fn demo_hits(&self) -> Vec<(f64, DrumLane)> {
        self.run
            .demo_notes()
            .into_iter()
            .map(|(m, pos, lane)| {
                let (from, to) = (self.measure_start_ms(m), self.measure_start_ms(m + 1));
                (from + pos * (to - from), lane)
            })
            .collect()
    }

    /// 譜面の小節 `m` の音符を、譜面の時刻 `now_ms` に描く濃さ。見ないでは段ごとに薄く、
    /// 聞いてまねるはお手本を聞いている間は先の小節を出さない（自分の小節が始まってから出す）
    pub fn note_alpha(&self, m: u32, now_ms: f64) -> f32 {
        if self.run.way == Way::Echo && self.measure_at(now_ms) < m {
            return 0.0;
        }
        self.run.note_alpha(m)
    }

    /// 音符を判定した（自動で叩いた音符は呼ばない）。限界チャレンジで同じ段に 3 つ落としたら、
    /// そのときだけ true（そこで終える）
    pub fn on_judged(&mut self, measure: u32, judgement: Judgement, ms: f64) -> bool {
        if self.run.way != Way::Limit
            || self.stopped.is_some()
            || !matches!(judgement, Judgement::Poor | Judgement::Miss)
        {
            return false;
        }
        let Some(s) = self.run.step_of(measure) else {
            return false;
        };
        self.misses[s] += 1;
        if self.misses[s] >= MANY_MISSES {
            self.stopped = Some((s, ms));
            return true;
        }
        false
    }

    /// 段 `step` で落とした数
    pub fn misses_in(&self, step: usize) -> u32 {
        self.misses.get(step).copied().unwrap_or(0)
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

    /// 譜面の時刻 `ms` でクリックを鳴らすか（最後の小節が終わったら、限界チャレンジで終えたら鳴らさない）
    pub fn clicks_at(&self, ms: f64) -> bool {
        if self.stopped.is_some_and(|(_, at)| ms > at) {
            return false;
        }
        let m = self.measure_at(ms + 0.5);
        m < self.run.measure_count() && self.run.clicks(m)
    }

    /// カウントの小節の何拍目か（1 から。カウントの外は None）
    /// と、その拍の頭の時刻。拍はカウントのクリック（3/4 なら 3 拍、7/8 なら 2+2+3 の 3 つ）
    pub fn count_in_beat(&self, ms: f64) -> Option<(u32, f64)> {
        let (from, to) = (self.measure_ms[0], self.measure_ms[1]);
        if ms < from || ms >= to {
            return None;
        }
        let starts: Vec<f64> = self
            .run
            .clicks_in(0)
            .iter()
            .map(|&(pos, _)| from + pos * (to - from))
            .collect();
        let i = starts.partition_point(|&s| s <= ms).max(1);
        Some((i as u32, starts[i - 1]))
    }

    /// クリックを鳴らす時刻と強さ（譜面の時刻の順）。拍子・クリックの型・消える小節を入れて、終わりの小節まで
    pub fn clicks(&self) -> Vec<(f64, bool)> {
        let mut v = Vec::new();
        for m in 0..self.run.measure_count() {
            let (from, to) = (self.measure_start_ms(m), self.measure_start_ms(m + 1));
            for (pos, accent) in self.run.clicks_in(m) {
                v.push((from + pos * (to - from), accent));
            }
        }
        v
    }

    /// 段 `step` の小節の範囲（譜面の小節）
    pub fn step_measures(&self, step: usize) -> std::ops::Range<u32> {
        self.run.step_range(step)
    }

    /// 叩いた（判定の時刻）
    pub fn on_hit(&mut self, ms: f64) {
        self.last_hit_ms = Some(ms);
        self.rest_sent = false;
    }

    /// 叩くのをやめたか。最初の音符から 1 小節（4 秒より短ければ 4 秒）叩かなければ 1 回だけ true。
    /// 聞いてまねるのお手本の小節は叩かないので、自分の小節の頭から数える
    pub fn check_rest(&mut self, now_ms: f64) -> bool {
        if self.rest_sent || self.stopped.is_some() {
            return false;
        }
        let first = self.measure_ms[1];
        let end = *self.measure_ms.last().unwrap_or(&first);
        if now_ms >= end {
            return false;
        }
        let m = self.measure_at(now_ms).max(1);
        if self.run.is_demo(m) {
            return false;
        }
        let bar = self.measure_start_ms(m + 1) - self.measure_start_ms(m);
        let mut since = self.last_hit_ms.unwrap_or(first).max(first);
        if self.run.is_demo(m - 1) {
            since = since.max(self.measure_start_ms(m));
        }
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

/// ドリルの記録の引き方（ドリルの id と、手順を左からにしたか）。右からと左からは別に数える
pub type RecordKey = (&'static str, bool);

/// ドリルごとのこれまでの記録
pub type DrillRecords = std::collections::HashMap<RecordKey, DrillRecord>;

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
    use engine::drill::Way;

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
        assert_eq!(line("waltz"), "3/4 · 8 分音符 · HH・SD・BD");
        assert_eq!(line("sextuplet"), "6 連符 · スネア");
        assert_eq!(line("three_against_two"), "3 連符 · HH・SD");
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
        assert_eq!(p.count_in_beat(0.0), Some((1, 0.0)));
        assert_eq!(p.count_in_beat(1600.0), Some((4, 1500.0)));
        assert_eq!(p.count_in_beat(2000.0), None);
        assert_eq!(p.measure_at(2000.0), 1);
        assert_eq!(p.step_measures(1), 5..9);
    }

    #[test]
    fn call_and_response_shows_your_bar_only_once_it_starts() {
        let run = Run::with(Drill::find("eighth").unwrap(), Way::Echo, 120, 10, false);
        let chart = dtx::parse(&run.to_dtx("echo"));
        let tempo = TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        let p = DrillPlay::new(run, &tempo, &notes);
        // 120 BPM で 1 小節 2 秒。小節 1 はお手本、小節 2 が自分
        assert_eq!(p.note_alpha(2, 3000.0), 0.0);
        assert_eq!(p.note_alpha(2, 4000.0), 1.0);
        assert_eq!(p.note_alpha(2, 5000.0), 1.0);
        // お手本の音は小節 1 と 3 に 8 つずつ
        let heard = p.demo_hits();
        assert_eq!(heard.len(), 8 * 8);
        assert_eq!(heard[0], (2000.0, DrumLane::SD));
        assert_eq!(heard[8].0, 6000.0);
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
