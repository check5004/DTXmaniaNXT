//! 今日のメニュー（基礎練習の画面の左の列の一番上。2026-11-27 カタログの「今日のメニュー」）。
//!
//! 5・10・15 分のおまかせ。準備（持久）→ 記録から選んだ苦手な型 → ビート → 限界チャレンジ → 曲の苦手な所を
//! 順に 1 回ずつ叩く。中身は日付と記録から決め、毎日少しずつ変わる。始めたらその日のうちは中身を変えない。
//! 1 つ終えるたびに結果の画面で止まり、CY で次へ（勝手に次へ進まない）。
//! 記録の無い人には、苦手な型の代わりにまだ叩いていない型を出し、曲の記録が無ければ曲の代わりに型を足す。

use std::path::PathBuf;

use chrono::{Datelike, NaiveDate};
use engine::drill::{BPM_MIN, Category, DRILLS, Drill, Run, Way};
use engine::practice::Section;
use i18n::t;

use crate::drill::{self, DrillRecords};
use crate::drill_menu::run_seconds;

/// 選べる長さ（分）と既定
pub const LENGTHS: [u32; 3] = [5, 10, 15];
pub const DEFAULT_LENGTH: u32 = 10;

/// 1 つごとの結果の画面と切り替えの時間の見込み（秒）
const BETWEEN_S: f64 = 10.0;
/// 限界チャレンジの長さの見込み（はじめの何段で終わるか）
const LIMIT_EXPECTED_STEPS: usize = 8;
/// 準備と持久のビートは、目標のテンポからこれだけ下げる
const EASY_DOWN: u32 = 10;
/// 苦手な型を選ぶ候補の数（日ごとにこの中で回す）
const WEAK_POOL: usize = 4;
/// 中身を足していくときに、残りがこれより短ければ足さない（秒）
const FILL_MARGIN_S: f64 = 30.0;
/// 並べる数の上限（真ん中の列に収まる数）
pub const MAX_STEPS: usize = 12;

/// 準備に使う型（手だけで、ゆっくり体をほぐせるもの）
const WARM_UP: [&str; 7] = [
    "eighth",
    "single_stroke",
    "sixteenth",
    "double_stroke",
    "triplet",
    "upbeat",
    "single_stroke_four",
];
/// 限界チャレンジに使う型（速さを競いやすい、くり返しの型）
const CHALLENGE: [&str; 8] = [
    "single_stroke",
    "sixteenth",
    "eight_beat",
    "double_stroke",
    "sixteen_beat",
    "paradiddle",
    "four_on_the_floor",
    "kick_eighths",
];

/// 1 つの役目（真ん中の列の札と色）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 準備（持久でゆっくり）
    WarmUp,
    /// 記録から選んだ苦手な型（ミスなしのテンポが目標に届いていない）
    Weak,
    /// まだ叩いていない型（記録が少ないとき）
    New,
    /// しばらく叩いていない型（苦手な型が無いとき）
    Review,
    /// ビート（基本のビートとジャンル）
    Groove,
    /// 限界チャレンジ
    Challenge,
    /// 曲の苦手な所（区間練習）
    Song,
}

impl Role {
    /// 札の文字
    pub fn label(self) -> &'static str {
        match self {
            Self::WarmUp => t("Warm-Up", "準備"),
            Self::Weak => t("Weak Spot", "苦手"),
            Self::New => t("New", "初めて"),
            Self::Review => t("Review", "おさらい"),
            Self::Groove => t("Groove", "ビート"),
            Self::Challenge => t("Challenge", "挑戦"),
            Self::Song => t("Song", "曲"),
        }
    }

    /// 選んだわけ（右のパネルの一言）
    pub fn reason(self) -> &'static str {
        match self {
            Self::WarmUp => t(
                "Loosen up at an easy tempo.",
                "ゆっくりのテンポで手をほぐします。",
            ),
            Self::Weak => t(
                "Your clean tempo hasn't reached your target yet.",
                "ミスなしのテンポが、まだ目標に届いていない型です。",
            ),
            Self::New => t(
                "A pattern you haven't tried yet.",
                "まだ叩いたことのない型です。",
            ),
            Self::Review => t(
                "A pattern you haven't played for a while.",
                "しばらく叩いていない型です。",
            ),
            Self::Groove => t(
                "Keep the beat steady from start to finish.",
                "最初から最後まで同じノリで叩きます。",
            ),
            Self::Challenge => t(
                "See how fast you can go today.",
                "今日はどこまで速くできるか試します。",
            ),
            Self::Song => t(
                "The part you missed most in your recent plays.",
                "最近のプレイでミスが多かった所です。",
            ),
        }
    }

    /// 色（帯と札）
    pub fn color(self) -> egui::Color32 {
        use egui::Color32 as C;
        match self {
            Self::WarmUp => C::from_rgb(0x70, 0xE0, 0x90),
            Self::Weak => C::from_rgb(0xFF, 0x70, 0x50),
            Self::New | Self::Review => C::from_rgb(0xB4, 0x8C, 0xFF),
            Self::Groove => C::from_rgb(0x38, 0xA6, 0xFF),
            Self::Challenge => C::from_rgb(0xFF, 0x5A, 0xA0),
            Self::Song => C::from_rgb(0xFF, 0xB8, 0x40),
        }
    }
}

/// 曲の苦手な所（記録から選んだ曲と区間）
#[derive(Debug, Clone, PartialEq)]
pub struct SongPick {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub section: Section,
    /// 小節ごとの苦手度（帯に出す）
    pub weakness: Vec<Option<f64>>,
}

/// 1 つの中身
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    Drill(Run),
    /// 曲の区間練習。`seconds` を過ぎたら、その段の終わりで結果へ
    Song {
        pick: SongPick,
        seconds: u32,
    },
}

/// メニューの 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub role: Role,
    pub task: Task,
}

impl Step {
    /// 長さの見込み（秒。結果の画面は入れない）
    pub fn seconds(&self) -> f64 {
        match &self.task {
            Task::Drill(run) => expected_seconds(run),
            Task::Song { seconds, .. } => f64::from(*seconds),
        }
    }

    /// 名前（ドリルの名前か曲名）
    pub fn name(&self) -> String {
        match &self.task {
            Task::Drill(run) => drill::name(run.drill).to_owned(),
            Task::Song { pick, .. } => pick.title.clone(),
        }
    }

    /// ドリルの 1 回（曲なら None）
    pub fn run(&self) -> Option<&Run> {
        match &self.task {
            Task::Drill(run) => Some(run),
            Task::Song { .. } => None,
        }
    }
}

/// 1 回のドリルの長さの見込み（秒）。限界チャレンジは、はじめの何段かで終わる見込みで
pub fn expected_seconds(run: &Run) -> f64 {
    if run.way != Way::Limit || run.steps() <= LIMIT_EXPECTED_STEPS {
        return run_seconds(run);
    }
    let mut r = run.clone();
    r.bpms.truncate(LIMIT_EXPECTED_STEPS);
    run_seconds(&r)
}

/// 今日のメニュー
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub day: NaiveDate,
    /// 選んだ長さ（分）
    pub minutes: u32,
    pub steps: Vec<Step>,
}

impl Plan {
    /// 全体の長さの見込み（秒。結果の画面と切り替えも入れて）
    pub fn seconds(&self) -> f64 {
        self.steps.iter().map(|s| s.seconds() + BETWEEN_S).sum()
    }
}

/// 進み具合（始めたメニュー。その日のうちは中身を変えない）
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub plan: Plan,
    /// 終えたもの
    pub done: Vec<bool>,
    /// いま叩いているもの（結果の画面まで。メニューを離れたら None）
    pub current: Option<usize>,
}

impl Progress {
    pub fn new(plan: Plan) -> Self {
        let n = plan.steps.len();
        Self {
            plan,
            done: vec![false; n],
            current: None,
        }
    }

    /// 1 つでも終えた
    pub fn started(&self) -> bool {
        self.done.iter().any(|&d| d)
    }

    /// 全部終えた
    pub fn complete(&self) -> bool {
        !self.done.is_empty() && self.done.iter().all(|&d| d)
    }

    /// 終えた数
    pub fn done_count(&self) -> usize {
        self.done.iter().filter(|&&d| d).count()
    }

    /// `i` の次に叩くもの（`i` より後のまだのもの。無ければ前のまだのもの）
    pub fn next_after(&self, i: usize) -> Option<usize> {
        let n = self.done.len();
        (i + 1..n).chain(0..i.min(n)).find(|&k| !self.done[k])
    }

    /// はじめに選んでおくもの（最初のまだのもの。全部終えていたら最初）
    pub fn first_open(&self) -> usize {
        self.done.iter().position(|&d| !d).unwrap_or(0)
    }
}

impl Progress {
    /// 結果の画面に出す進み具合（`index` を叩き終えたところ）
    pub fn mark(&self, index: usize) -> Mark {
        Mark {
            index,
            steps: self
                .plan
                .steps
                .iter()
                .zip(&self.done)
                .map(|(s, &d)| (s.role, s.seconds(), d))
                .collect(),
            next: self
                .next_after(index)
                .map(|k| (self.plan.steps[k].role, self.plan.steps[k].name())),
        }
    }
}

/// 結果の画面に出す、今日のメニューの進み具合
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// いま叩いたもの
    pub index: usize,
    /// 役目・長さ（秒）・終えたか
    pub steps: Vec<(Role, f64, bool)>,
    /// 次に叩くもの（無ければ全部終えた）
    pub next: Option<(Role, String)>,
}

/// メニューを決めるのに使うもの
pub struct Inputs<'a> {
    pub day: NaiveDate,
    pub records: &'a DrillRecords,
    /// ドリルの目標のテンポ（基礎練習の画面で選んだもの）
    pub tempo: &'a dyn Fn(&Drill) -> u32,
    pub rise: u32,
    pub lefty: bool,
    pub song: Option<&'a SongPick>,
}

/// 長さごとの組み立て
struct Shape {
    warm_up_minutes: u32,
    weak: usize,
    grooves: usize,
    /// 持久のビート（2 分）
    endurance_groove: bool,
    challenge: bool,
    song_seconds: u32,
}

fn shape(minutes: u32) -> Shape {
    match minutes {
        ..=5 => Shape {
            warm_up_minutes: 1,
            weak: 1,
            grooves: 1,
            endurance_groove: false,
            challenge: false,
            song_seconds: 120,
        },
        6..=10 => Shape {
            warm_up_minutes: 2,
            weak: 2,
            grooves: 2,
            endurance_groove: false,
            challenge: true,
            song_seconds: 180,
        },
        _ => Shape {
            warm_up_minutes: 2,
            weak: 3,
            grooves: 3,
            endurance_groove: true,
            challenge: true,
            song_seconds: 240,
        },
    }
}

/// ビートのやり方（日ごとに回す。テンポを上げる・聞いてまねる・見ないで）
const GROOVE_WAYS: [Way; 3] = [Way::Climb, Way::Echo, Way::Blind];

/// 今日のメニューを決める。同じ日・同じ記録なら同じ中身
pub fn plan(minutes: u32, input: &Inputs) -> Plan {
    let seed = input.day.num_days_from_ce().unsigned_abs() as usize;
    let sh = shape(minutes);
    let tempo = |d: &Drill| (input.tempo)(d);
    let easy = |d: &Drill| tempo(d).saturating_sub(EASY_DOWN).max(BPM_MIN);
    let run =
        |d: &'static Drill, way: Way, bpm: u32| Run::with(d, way, bpm, input.rise, input.lefty);
    let mut used: Vec<&'static str> = Vec::new();

    // 準備
    let mut warm = Vec::new();
    if let Some(d) = rotate(&ids(&WARM_UP), seed, 0, &used) {
        used.push(d.id);
        warm.push(Step {
            role: Role::WarmUp,
            task: Task::Drill(run(d, Way::Endurance(sh.warm_up_minutes), easy(d))),
        });
    }

    // 苦手な型（無ければ、まだの型・しばらくやっていない型）
    let mut picks = Picker::new(input, seed);
    let mut weak = Vec::new();
    for k in 0..sh.weak {
        if let Some((role, d)) = picks.next(k, &used) {
            used.push(d.id);
            weak.push(Step {
                role,
                task: Task::Drill(run(d, Way::Climb, tempo(d))),
            });
        }
    }

    // ビート
    let grooves_pool: Vec<&'static Drill> = DRILLS
        .iter()
        .filter(|d| matches!(d.category, Category::Grooves | Category::Styles))
        .collect();
    let mut grooves = Vec::new();
    for k in 0..sh.grooves {
        if let Some(d) = rotate(&grooves_pool, seed, k, &used) {
            used.push(d.id);
            let way = GROOVE_WAYS[(seed + k) % GROOVE_WAYS.len()];
            grooves.push(Step {
                role: Role::Groove,
                task: Task::Drill(run(d, way, tempo(d))),
            });
        }
    }
    if sh.endurance_groove
        && let Some(d) = rotate(&grooves_pool, seed, sh.grooves, &used)
    {
        used.push(d.id);
        grooves.push(Step {
            role: Role::Groove,
            task: Task::Drill(run(d, Way::Endurance(2), easy(d))),
        });
    }

    // 限界チャレンジ
    let mut challenge = Vec::new();
    let add_challenge = sh.challenge || input.song.is_none();
    if add_challenge && let Some(d) = rotate(&ids(&CHALLENGE), seed, 3, &used) {
        used.push(d.id);
        challenge.push(Step {
            role: Role::Challenge,
            task: Task::Drill(run(d, Way::Limit, easy(d))),
        });
    }

    // 曲
    let song: Vec<Step> = input
        .song
        .map(|p| Step {
            role: Role::Song,
            task: Task::Song {
                pick: p.clone(),
                seconds: sh.song_seconds,
            },
        })
        .into_iter()
        .collect();

    let total = |w: &[Step], a: &[Step], g: &[Step], c: &[Step], s: &[Step]| -> f64 {
        w.iter()
            .chain(a)
            .chain(g)
            .chain(c)
            .chain(s)
            .map(|x| x.seconds() + BETWEEN_S)
            .sum()
    };
    // 曲が無いなどで短ければ、苦手な型とビートを交互に足す
    let budget = f64::from(minutes * 60);
    let mut turn = 0;
    while weak.len() + grooves.len() + warm.len() + challenge.len() + song.len() < MAX_STEPS {
        let now = total(&warm, &weak, &grooves, &challenge, &song);
        let step = if turn % 2 == 0 {
            picks
                .next(weak.len(), &used)
                .map(|(role, d)| (role, run(d, Way::Climb, tempo(d))))
        } else {
            let k = grooves.len();
            rotate(&grooves_pool, seed, k, &used).map(|d| {
                let way = GROOVE_WAYS[(seed + k) % GROOVE_WAYS.len()];
                (Role::Groove, run(d, way, tempo(d)))
            })
        };
        turn += 1;
        let Some((role, r)) = step else {
            if turn > 4 {
                break;
            }
            continue;
        };
        let s = Step {
            role,
            task: Task::Drill(r),
        };
        if now + s.seconds() + BETWEEN_S > budget + FILL_MARGIN_S {
            break;
        }
        used.push(s.run().map_or("", |r| r.drill.id));
        if role == Role::Groove {
            grooves.push(s);
        } else {
            weak.push(s);
        }
    }

    let mut steps = warm;
    steps.extend(weak);
    steps.extend(grooves);
    steps.extend(challenge);
    steps.extend(song);
    Plan {
        day: input.day,
        minutes,
        steps,
    }
}

/// id の並びをドリルにする
fn ids(list: &[&str]) -> Vec<&'static Drill> {
    list.iter().filter_map(|id| Drill::find(id)).collect()
}

/// 日ごとに回して、まだ使っていないものを 1 つ選ぶ。`k` はその日の何番目か
fn rotate(
    pool: &[&'static Drill],
    seed: usize,
    k: usize,
    used: &[&'static str],
) -> Option<&'static Drill> {
    if pool.is_empty() {
        return None;
    }
    let n = pool.len();
    let start = (seed.wrapping_mul(17) + k.wrapping_mul(11)) % n;
    (0..n)
        .map(|i| pool[(start + i) % n])
        .find(|d| !used.contains(&d.id))
}

/// 苦手な型を選ぶ（記録から）。足りなければまだの型、それも無ければしばらくやっていない型
struct Picker {
    weak: Vec<&'static Drill>,
    fresh: Vec<&'static Drill>,
    stale: Vec<&'static Drill>,
    seed: usize,
}

impl Picker {
    fn new(input: &Inputs, seed: usize) -> Self {
        let mut weak: Vec<(f64, i64, &'static Drill)> = Vec::new();
        let mut stale: Vec<(i64, &'static Drill)> = Vec::new();
        let mut fresh = Vec::new();
        for d in DRILLS {
            let own = input.records.get(&(d.id, input.lefty));
            let other = input.records.get(&(d.id, !input.lefty));
            if own.is_none_or(|r| r.runs == 0) && other.is_none_or(|r| r.runs == 0) {
                // ビートはビートの役目で出すので、まだの型には入れない
                if !matches!(d.category, Category::Grooves | Category::Styles) {
                    fresh.push(d);
                }
                continue;
            }
            let Some(r) = own.filter(|r| r.runs > 0) else {
                continue;
            };
            let target = f64::from((input.tempo)(d).max(1));
            let ratio = r.best.map_or(0.0, |b| f64::from(b) / target);
            let last = r.last_at.unwrap_or(0);
            if ratio < 1.0 {
                weak.push((ratio, last, d));
            } else {
                stale.push((last, d));
            }
        }
        // 苦手な順（同じなら、前に叩いたのが古い順）
        weak.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        stale.sort_by_key(|s| s.0);
        let mut weak: Vec<&'static Drill> = weak.into_iter().map(|w| w.2).collect();
        // 候補の中を日ごとに回す（毎日同じ型にならないように）
        let pool = weak.len().min(WEAK_POOL);
        if pool > 1 {
            weak[..pool].rotate_left(seed % pool);
        }
        Self {
            weak,
            fresh,
            stale: stale.into_iter().map(|s| s.1).collect(),
            seed,
        }
    }

    /// `k` 番目の苦手な型（使っていないもの）
    fn next(&mut self, k: usize, used: &[&'static str]) -> Option<(Role, &'static Drill)> {
        if let Some(d) = self.weak.iter().find(|d| !used.contains(&d.id)) {
            return Some((Role::Weak, d));
        }
        if let Some(d) = rotate(&self.fresh, self.seed, k + 5, used) {
            return Some((Role::New, d));
        }
        self.stale
            .iter()
            .find(|d| !used.contains(&d.id))
            .map(|&d| (Role::Review, d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drill::DrillRecord;
    use std::collections::HashMap;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn default_tempo(d: &Drill) -> u32 {
        d.default_bpm
    }

    fn song() -> SongPick {
        SongPick {
            path: PathBuf::from("/songs/a/a.dtx"),
            title: "Song A".to_owned(),
            artist: String::new(),
            section: Section::new(8, 11),
            weakness: vec![None; 20],
        }
    }

    fn make(minutes: u32, d: u32, records: &DrillRecords, song: Option<&SongPick>) -> Plan {
        plan(
            minutes,
            &Inputs {
                day: day(d),
                records,
                tempo: &default_tempo,
                rise: 10,
                lefty: false,
                song,
            },
        )
    }

    #[test]
    fn each_length_fits_its_minutes_and_ends_with_the_song() {
        let records = HashMap::new();
        let s = song();
        for minutes in LENGTHS {
            for d in 1..=28 {
                for song in [Some(&s), None] {
                    let p = make(minutes, d, &records, song);
                    let secs = p.seconds();
                    let want = f64::from(minutes * 60);
                    assert!(
                        secs > want * 0.75 && secs < want * 1.15,
                        "{minutes} min on day {d} (song {}): {secs:.0} s",
                        song.is_some()
                    );
                    assert!(p.steps.len() <= MAX_STEPS);
                    assert_eq!(p.steps[0].role, Role::WarmUp);
                    assert_eq!(p.steps.last().unwrap().role == Role::Song, song.is_some());
                    // 同じ型は 2 度出さない
                    let mut ids: Vec<&str> = p
                        .steps
                        .iter()
                        .filter_map(|s| s.run().map(|r| r.drill.id))
                        .collect();
                    let n = ids.len();
                    ids.sort_unstable();
                    ids.dedup();
                    assert_eq!(ids.len(), n);
                }
            }
        }
    }

    #[test]
    fn the_order_is_warm_up_weak_groove_challenge_song() {
        let records = HashMap::new();
        let s = song();
        let p = make(15, 3, &records, Some(&s));
        let order = |r: Role| match r {
            Role::WarmUp => 0,
            Role::Weak | Role::New | Role::Review => 1,
            Role::Groove => 2,
            Role::Challenge => 3,
            Role::Song => 4,
        };
        let roles: Vec<usize> = p.steps.iter().map(|s| order(s.role)).collect();
        let mut sorted = roles.clone();
        sorted.sort_unstable();
        assert_eq!(roles, sorted);
        assert!(p.steps.iter().any(|s| s.role == Role::Challenge));
        // 準備は持久で、目標より 10 ゆっくり
        let w = p.steps[0].run().unwrap();
        assert!(matches!(w.way, Way::Endurance(2)));
        assert_eq!(w.target(), w.drill.default_bpm - EASY_DOWN);
    }

    #[test]
    fn the_same_day_gives_the_same_menu_and_days_differ() {
        let records = HashMap::new();
        let a = make(10, 5, &records, None);
        assert_eq!(a, make(10, 5, &records, None));
        let names = |p: &Plan| p.steps.iter().map(Step::name).collect::<Vec<_>>();
        let differ = (6..13).filter(|&d| names(&make(10, d, &records, None)) != names(&a));
        assert!(differ.count() >= 6);
    }

    #[test]
    fn weak_drills_come_from_the_records() {
        let mut records: DrillRecords = HashMap::new();
        let rec = |best: Option<u32>, at: i64| {
            let mut r = DrillRecord::default();
            r.add(at, best);
            r
        };
        // パラディドル（目標 100）は 80 まで、ダブル（目標 90）はミスなしなし、シングルは目標を越えた
        records.insert(("paradiddle", false), rec(Some(80), 10));
        records.insert(("double_stroke", false), rec(None, 20));
        records.insert(("single_stroke", false), rec(Some(120), 30));
        let p = make(10, 1, &records, None);
        let weak: Vec<&str> = p
            .steps
            .iter()
            .filter(|s| s.role == Role::Weak)
            .filter_map(|s| s.run().map(|r| r.drill.id))
            .collect();
        assert!(weak.contains(&"paradiddle") && weak.contains(&"double_stroke"));
        assert!(!weak.contains(&"single_stroke"));
        // 苦手な型は目標のテンポでのぼる
        let s = p
            .steps
            .iter()
            .find(|s| s.run().is_some_and(|r| r.drill.id == "paradiddle"))
            .unwrap();
        assert_eq!(s.run().unwrap().way, Way::Climb);
        assert_eq!(s.run().unwrap().target(), 100);
        // 記録の無い人には、まだの型
        let fresh = make(10, 1, &HashMap::new(), None);
        assert!(fresh.steps.iter().any(|s| s.role == Role::New));
        assert!(!fresh.steps.iter().any(|s| s.role == Role::Weak));
    }

    #[test]
    fn progress_moves_on_to_the_next_open_step() {
        let p = make(5, 1, &HashMap::new(), Some(&song()));
        let mut g = Progress::new(p);
        let n = g.done.len();
        assert!(!g.started());
        assert_eq!(g.first_open(), 0);
        g.done[0] = true;
        g.done[2] = true;
        assert_eq!(g.next_after(0), Some(1));
        assert_eq!(g.next_after(1), Some(3));
        assert_eq!(g.next_after(n - 1), Some(1));
        assert_eq!(g.first_open(), 1);
        g.done.fill(true);
        assert!(g.complete());
        assert_eq!(g.next_after(0), None);
        assert_eq!(g.done_count(), n);
    }
}
