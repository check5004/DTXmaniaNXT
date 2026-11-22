//! 選曲の並べ替え。NX v2.0（BocuD 版）の並べ替えメニューと同じ 9 種類で、曲の木を作り直す。
//!
//! 曲名やアーティストの頭文字、レベル、難易度、遊んだ記録で BOX に分ける。レベルや難易度で
//! 分けるときは、曲をその譜面 1 つだけにした写しを入れる（選ぶとその譜面を遊ぶ）。
//! 中身の無い BOX は作らない。

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use i18n::{t, tf};

use crate::{LEVELS, Library, Node, Song, SongBox};

/// 並べ替えの種類（並べ替えメニューの並び順）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SortKind {
    /// フォルダのまま
    #[default]
    Default,
    /// 一番上の BOX だけを並べ、BOX に入っていない曲は「BOX の外の曲」に
    Box,
    Title,
    Artist,
    /// 難易度（BASIC〜DTX）ごと
    Difficulty,
    /// レベル 0.5 刻み
    Level,
    /// 最後に遊んだ日とランク
    Player,
    /// すべての曲を 1 列に
    AllSongs,
    /// スキルの高い 50 曲
    Skill,
}

impl SortKind {
    pub const ALL: [SortKind; 9] = [
        SortKind::Default,
        SortKind::Box,
        SortKind::Title,
        SortKind::Artist,
        SortKind::Difficulty,
        SortKind::Level,
        SortKind::Player,
        SortKind::AllSongs,
        SortKind::Skill,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&k| k == self).unwrap_or(0)
    }

    /// メニューで `delta` 個となり（端から端へ回る）
    pub fn step(self, delta: isize) -> Self {
        let n = Self::ALL.len() as isize;
        Self::ALL[(self.index() as isize + delta).rem_euclid(n) as usize]
    }

    pub fn name(self) -> &'static str {
        match self {
            SortKind::Default => "Default",
            SortKind::Box => "BOX",
            SortKind::Title => "Title",
            SortKind::Artist => "Artist",
            SortKind::Difficulty => "Difficulty",
            SortKind::Level => "Level",
            SortKind::Player => "Player",
            SortKind::AllSongs => "All Songs",
            SortKind::Skill => "Skill",
        }
    }

    /// スキンの `Graphics/Sorting/` の画像と音の名前
    pub fn icon(self) -> &'static str {
        match self {
            SortKind::Default | SortKind::Box => "recommended",
            SortKind::Title => "title",
            SortKind::Artist => "artist",
            SortKind::Difficulty => "difficulty",
            SortKind::Level => "level",
            SortKind::Player => "player",
            SortKind::AllSongs => "allmusic",
            SortKind::Skill => "skill",
        }
    }

    /// 遊んだ記録で分ける（記録が変わったら作り直す）
    pub fn uses_records(self) -> bool {
        matches!(self, SortKind::Player | SortKind::Skill)
    }
}

/// 譜面ごとの記録（並べ替えと、選曲画面のランプ・ランク・スキルに使う）
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ChartStats {
    /// 最後まで遊んだプレイの一番高い達成率（0〜100）
    pub best_rate: Option<f64>,
    /// 最後まで遊んだプレイの一番よいランク（0 が SS、6 が E）
    pub best_rank: Option<u8>,
    /// 最後に遊んだ時刻（UNIX 秒。練習も含む）
    pub last_played: Option<i64>,
}

/// 譜面のファイルごとの記録
pub type Stats = HashMap<PathBuf, ChartStats>;

/// 難易度の名前（上流 v2 の難易度での並べ替え）
pub const DIFFICULTY_NAMES: [&str; LEVELS] = ["BASIC", "ADVANCED", "EXTREME", "MASTER", "DTX"];

/// ランクの名前（0 が SS）
pub const RANK_NAMES: [&str; 7] = ["SS", "S", "A", "B", "C", "D", "E"];

/// スキルの曲の数
pub const SKILL_SONGS: usize = 50;

/// 譜面のスキル（上流：レベル × 達成率 ÷ 100 × 20。最大はレベル × 20）
pub fn skill(level: f64, rate: f64) -> f64 {
    level * rate * 0.2
}

/// 曲の一番高いスキルと、その難易度
pub fn top_skill(song: &Song, stats: &Stats) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (i, c) in song.charts.iter().enumerate() {
        let Some(c) = c else { continue };
        let (Some(level), Some(rate)) =
            (c.head.level, stats.get(&c.path).and_then(|s| s.best_rate))
        else {
            continue;
        };
        let s = skill(level, rate);
        if s > 0.0 && best.is_none_or(|(_, b)| s > b) {
            best = Some((i, s));
        }
    }
    best
}

/// 並べ替えた木。`now` は今の時刻（UNIX 秒。最後に遊んだ日で分けるのに使う）
pub fn sorted(lib: &Library, kind: SortKind, stats: &Stats, now: i64) -> Vec<Node> {
    match kind {
        SortKind::Default => lib.nodes.clone(),
        SortKind::Box => by_box(lib),
        SortKind::Title => by_initial(lib, |s| &s.title, t("No Title", "曲名なし")),
        SortKind::Artist => by_initial(lib, |s| &s.artist, t("No Artist", "アーティスト名なし")),
        SortKind::Difficulty => by_difficulty(lib),
        SortKind::Level => by_level(lib),
        SortKind::Player => by_player(lib, stats, now),
        SortKind::AllSongs => lib
            .all_songs()
            .into_iter()
            .map(|s| Node::Song(Box::new(s.clone())))
            .collect(),
        SortKind::Skill => by_skill(lib, stats),
    }
}

/// 名前だけの BOX（中身が無ければ作らない）
fn group(title: &str, nodes: Vec<Node>) -> Option<Node> {
    (!nodes.is_empty()).then(|| {
        Node::Box(SongBox {
            title: title.to_owned(),
            nodes,
            ..Default::default()
        })
    })
}

/// 譜面 `i` だけにした曲の写し
fn only_chart(song: &Song, i: usize) -> Node {
    let mut s = song.clone();
    for (j, c) in s.charts.iter_mut().enumerate() {
        if j != i {
            *c = None;
        }
    }
    Node::Song(Box::new(s))
}

fn by_box(lib: &Library) -> Vec<Node> {
    let mut boxes: Vec<Node> = lib
        .nodes
        .iter()
        .filter(|n| matches!(n, Node::Box(_)))
        .cloned()
        .collect();
    boxes.sort_by_key(|n| match n {
        Node::Box(b) => b.dir.to_string_lossy().to_lowercase(),
        Node::Song(_) => String::new(),
    });
    let loose: Vec<Node> = lib
        .nodes
        .iter()
        .filter(|n| matches!(n, Node::Song(_)))
        .cloned()
        .collect();
    boxes.extend(group(t("Songs Outside Boxes", "BOX の外の曲"), loose));
    boxes
}

/// 頭文字の分け方
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Initial {
    /// A〜Z（0〜25）
    Letter(u8),
    /// あ・か・さ・た・な・は・ま・や・ら・わ（0〜9）
    Kana(u8),
    Empty,
    Other,
}

const KANA_ROWS: [&str; 10] = ["あ", "か", "さ", "た", "な", "は", "ま", "や", "ら", "わ"];

/// カタカナはひらがなに、全角の英字は半角に
fn fold(c: char) -> char {
    match c {
        'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
        'Ａ'..='Ｚ' | 'ａ'..='ｚ' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        _ => c,
    }
}

/// 頭文字の行（上流は漢字を読みに直すが、ここでは「その他」に入れる）
fn initial(text: &str) -> Initial {
    let Some(c) = text.trim().chars().next() else {
        return Initial::Empty;
    };
    let c = fold(c);
    if c.is_ascii_alphabetic() {
        return Initial::Letter(c.to_ascii_uppercase() as u8 - b'A');
    }
    let row = match c {
        'ぁ'..='お' | 'ゔ' => 0,
        'か'..='ご' | 'ゕ' | 'ゖ' => 1,
        'さ'..='ぞ' => 2,
        'た'..='ど' => 3,
        'な'..='の' => 4,
        'は'..='ぽ' => 5,
        'ま'..='も' => 6,
        'ゃ'..='よ' => 7,
        'ら'..='ろ' => 8,
        'ゎ'..='ん' => 9,
        _ => return Initial::Other,
    };
    Initial::Kana(row)
}

/// 名前の比べ方（大文字・小文字とカタカナ・ひらがなを区別しない）
fn name_key(text: &str) -> String {
    text.trim()
        .chars()
        .map(fold)
        .collect::<String>()
        .to_lowercase()
}

fn by_initial(lib: &Library, field: fn(&Song) -> &String, empty: &str) -> Vec<Node> {
    let mut groups: Vec<(Initial, Vec<&Song>)> = Vec::new();
    for s in lib.all_songs() {
        let k = initial(field(s));
        match groups.iter_mut().find(|(g, _)| *g == k) {
            Some((_, v)) => v.push(s),
            None => groups.push((k, vec![s])),
        }
    }
    groups.sort_by_key(|(k, _)| *k);
    groups
        .into_iter()
        .filter_map(|(k, mut songs)| {
            songs.sort_by_cached_key(|s| (name_key(field(s)), name_key(&s.title)));
            let title = match k {
                Initial::Letter(i) => char::from(b'A' + i).to_string(),
                Initial::Kana(i) => KANA_ROWS[i as usize].to_owned(),
                Initial::Empty => empty.to_owned(),
                Initial::Other => t("Other", "その他").to_owned(),
            };
            group(
                &title,
                songs
                    .into_iter()
                    .map(|s| Node::Song(Box::new(s.clone())))
                    .collect(),
            )
        })
        .collect()
}

/// 譜面のレベルの低い順（レベルの無い譜面は後ろ）、同じなら曲名の順
fn by_chart_level(a: &(f64, &Song), b: &(f64, &Song)) -> Ordering {
    a.0.total_cmp(&b.0)
        .then_with(|| name_key(&a.1.title).cmp(&name_key(&b.1.title)))
}

fn by_difficulty(lib: &Library) -> Vec<Node> {
    let songs = lib.all_songs();
    (0..LEVELS)
        .filter_map(|i| {
            let mut charts: Vec<(f64, &Song)> = songs
                .iter()
                .filter_map(|s| {
                    let c = s.charts[i].as_ref()?;
                    Some((c.head.level.unwrap_or(f64::INFINITY), *s))
                })
                .collect();
            charts.sort_by(by_chart_level);
            group(
                DIFFICULTY_NAMES[i],
                charts.into_iter().map(|(_, s)| only_chart(s, i)).collect(),
            )
        })
        .collect()
}

/// レベルの BOX の番号（0.5 刻み。上流と同じく 1.00 未満は 1.00〜1.49 に、9.50 以上は 9.50〜9.99 に）
fn level_bucket(level: f64) -> usize {
    ((level * 2.0).floor() as i64).clamp(2, 19) as usize
}

fn by_level(lib: &Library) -> Vec<Node> {
    let mut buckets: Vec<Vec<(f64, &Song, usize)>> = vec![Vec::new(); 21];
    for s in lib.all_songs() {
        for (i, c) in s.charts.iter().enumerate() {
            let Some(c) = c else { continue };
            match c.head.level {
                Some(l) => buckets[level_bucket(l)].push((l, s, i)),
                // レベルの無い譜面は最後の「レベルなし」に
                None => buckets[20].push((f64::INFINITY, s, i)),
            }
        }
    }
    buckets
        .into_iter()
        .enumerate()
        .filter_map(|(k, mut charts)| {
            charts.sort_by(|a, b| by_chart_level(&(a.0, a.1), &(b.0, b.1)));
            let title = if k == 20 {
                t("No Level", "レベルなし").to_owned()
            } else {
                let low = k as f64 / 2.0;
                tf!("Level {low:.2}–{:.2}", "レベル {low:.2}〜{:.2}", low + 0.49)
            };
            group(
                &title,
                charts
                    .into_iter()
                    .map(|(_, s, i)| only_chart(s, i))
                    .collect(),
            )
        })
        .collect()
}

fn by_player(lib: &Library, stats: &Stats, now: i64) -> Vec<Node> {
    let songs = lib.all_songs();
    // 最後に遊んだ日
    const DAY: f64 = 86_400.0;
    let names = [
        t("Today", "今日"),
        t("Yesterday", "昨日"),
        t("Past Week", "1 週間以内"),
        t("Past Month", "1 か月以内"),
        t("Past Year", "1 年以内"),
        t("Over a Year Ago", "1 年より前"),
        t("Not Played Yet", "まだ遊んでいない"),
    ];
    let mut recent: Vec<Vec<(i64, &Song)>> = vec![Vec::new(); names.len()];
    for s in &songs {
        let last = s
            .charts
            .iter()
            .flatten()
            .filter_map(|c| stats.get(&c.path)?.last_played)
            .max();
        let Some(last) = last else {
            recent[6].push((i64::MIN, s));
            continue;
        };
        let days = (now - last) as f64 / DAY;
        let k = match days {
            d if d < 1.0 => 0,
            d if d < 2.0 => 1,
            d if d < 7.0 => 2,
            d if d < 30.0 => 3,
            d if d < 365.0 => 4,
            _ => 5,
        };
        recent[k].push((last, s));
    }
    let recent: Vec<Node> = recent
        .into_iter()
        .zip(names)
        .filter_map(|(mut v, name)| {
            // 新しく遊んだ順。遊んでいない曲は曲名の順
            v.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| name_key(&a.1.title).cmp(&name_key(&b.1.title)))
            });
            group(
                name,
                v.into_iter()
                    .map(|(_, s)| Node::Song(Box::new(s.clone())))
                    .collect(),
            )
        })
        .collect();
    // 一番よいランク（譜面ごと）
    let mut ranks: Vec<Vec<(&Song, usize)>> = vec![Vec::new(); RANK_NAMES.len() + 1];
    for s in &songs {
        for (i, c) in s.charts.iter().enumerate() {
            let Some(c) = c else { continue };
            let k = stats
                .get(&c.path)
                .and_then(|st| st.best_rank)
                .map_or(RANK_NAMES.len(), |r| {
                    usize::from(r).min(RANK_NAMES.len() - 1)
                });
            ranks[k].push((s, i));
        }
    }
    let ranks: Vec<Node> = ranks
        .into_iter()
        .enumerate()
        .filter_map(|(k, mut v)| {
            v.sort_by_cached_key(|(s, i)| (name_key(&s.title), *i));
            group(
                RANK_NAMES
                    .get(k)
                    .copied()
                    .unwrap_or(t("No Rank", "ランクなし")),
                v.into_iter().map(|(s, i)| only_chart(s, i)).collect(),
            )
        })
        .collect();
    group(t("Last Played", "最後に遊んだ日"), recent)
        .into_iter()
        .chain(group(t("Best Rank", "一番よいランク"), ranks))
        .collect()
}

/// スキルの高い `SKILL_SONGS` 曲（曲ごとに一番高い譜面）。高い順
fn top_songs<'a>(lib: &'a Library, stats: &Stats) -> Vec<(f64, &'a Song, usize)> {
    let mut top: Vec<(f64, &Song, usize)> = lib
        .all_songs()
        .into_iter()
        .filter_map(|s| top_skill(s, stats).map(|(i, v)| (v, s, i)))
        .collect();
    top.sort_by(|a, b| b.0.total_cmp(&a.0));
    top.truncate(SKILL_SONGS);
    top
}

/// スキルに数える譜面（上流 v2 `countSkill`）
pub fn skill_charts(lib: &Library, stats: &Stats) -> HashSet<PathBuf> {
    top_songs(lib, stats)
        .into_iter()
        .filter_map(|(_, s, i)| Some(s.charts[i].as_ref()?.path.clone()))
        .collect()
}

fn by_skill(lib: &Library, stats: &Stats) -> Vec<Node> {
    let top = top_songs(lib, stats);
    group(
        &tf!(
            "Top {SKILL_SONGS} Skill Songs",
            "スキルの上位 {SKILL_SONGS} 曲"
        ),
        top.into_iter().map(|(_, s, i)| only_chart(s, i)).collect(),
    )
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChartHead, SongChart};
    use i18n::Lang;

    fn song(title: &str, artist: &str, levels: &[(usize, Option<f64>)]) -> Song {
        let mut charts: [Option<SongChart>; LEVELS] = Default::default();
        for &(i, level) in levels {
            charts[i] = Some(SongChart {
                label: String::new(),
                path: PathBuf::from(format!("{title}/{i}.dtx")),
                head: ChartHead {
                    level,
                    ..Default::default()
                },
            });
        }
        Song {
            title: title.into(),
            artist: artist.into(),
            genre: String::new(),
            dir: PathBuf::from(title),
            set_def_block: None,
            charts,
        }
    }

    fn lib() -> Library {
        Library {
            nodes: vec![
                Node::Box(SongBox {
                    title: "Rock".into(),
                    dir: PathBuf::from("b/rock"),
                    nodes: vec![
                        Node::Song(Box::new(song(
                            "apple",
                            "Zed",
                            &[(0, Some(2.3)), (2, Some(7.6))],
                        ))),
                        Node::Song(Box::new(song("バラード", "", &[(1, Some(5.0))]))),
                    ],
                    ..Default::default()
                }),
                Node::Song(Box::new(song("Banana", "あいう", &[(0, None)]))),
                Node::Song(Box::new(song("夜明け", "Zack", &[(4, Some(9.9))]))),
            ],
            ..Default::default()
        }
    }

    fn titles(nodes: &[Node]) -> Vec<&str> {
        nodes.iter().map(Node::title).collect()
    }

    fn inner(nodes: &[Node], title: &str) -> Vec<Node> {
        nodes
            .iter()
            .find_map(|n| match n {
                Node::Box(b) if b.title == title => Some(b.nodes.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn step_wraps_around_the_menu() {
        assert_eq!(SortKind::Default.step(-1), SortKind::Skill);
        assert_eq!(SortKind::Skill.step(1), SortKind::Default);
        assert_eq!(SortKind::Title.step(2), SortKind::Difficulty);
    }

    #[test]
    fn initials_fold_kana_and_full_width() {
        assert_eq!(initial("apple"), Initial::Letter(0));
        assert_eq!(initial("Ｚｅｒｏ"), Initial::Letter(25));
        assert_eq!(initial("バラード"), Initial::Kana(5));
        assert_eq!(initial("ぎんいろ"), Initial::Kana(1));
        assert_eq!(initial("ンジャメナ"), Initial::Kana(9));
        assert_eq!(initial("夜明け"), Initial::Other);
        assert_eq!(initial("123"), Initial::Other);
        assert_eq!(initial("  "), Initial::Empty);
    }

    #[test]
    fn title_groups_by_initial_and_skips_empty_boxes() {
        let _ja = i18n::scoped(Lang::Ja);
        let t = sorted(&lib(), SortKind::Title, &Stats::new(), 0);
        assert_eq!(titles(&t), ["A", "B", "は", "その他"]);
        assert_eq!(titles(&inner(&t, "その他")), ["夜明け"]);
        let a = sorted(&lib(), SortKind::Artist, &Stats::new(), 0);
        assert_eq!(titles(&a), ["Z", "あ", "アーティスト名なし"]);
        // 同じ頭文字は名前の順
        assert_eq!(titles(&inner(&a, "Z")), ["夜明け", "apple"]);
    }

    #[test]
    fn level_and_difficulty_hold_single_charts() {
        let _ja = i18n::scoped(Lang::Ja);
        let l = sorted(&lib(), SortKind::Level, &Stats::new(), 0);
        assert_eq!(
            titles(&l),
            [
                "レベル 2.00〜2.49",
                "レベル 5.00〜5.49",
                "レベル 7.50〜7.99",
                "レベル 9.50〜9.99",
                "レベルなし"
            ]
        );
        let Node::Song(s) = &inner(&l, "レベル 7.50〜7.99")[0] else {
            panic!()
        };
        assert_eq!(s.levels().collect::<Vec<_>>(), [2]);
        let d = sorted(&lib(), SortKind::Difficulty, &Stats::new(), 0);
        assert_eq!(titles(&d), ["BASIC", "ADVANCED", "EXTREME", "DTX"]);
        // レベルの低い順、レベルの無い譜面は後ろ
        assert_eq!(titles(&inner(&d, "BASIC")), ["apple", "Banana"]);
    }

    #[test]
    fn box_sort_keeps_top_boxes_and_loose_songs() {
        let _ja = i18n::scoped(Lang::Ja);
        let b = sorted(&lib(), SortKind::Box, &Stats::new(), 0);
        assert_eq!(titles(&b), ["Rock", "BOX の外の曲"]);
        assert_eq!(titles(&inner(&b, "BOX の外の曲")), ["Banana", "夜明け"]);
        let all = sorted(&lib(), SortKind::AllSongs, &Stats::new(), 0);
        assert_eq!(titles(&all), ["apple", "バラード", "Banana", "夜明け"]);
        // 英語の画面では BOX の名前も英語
        let _en = i18n::scoped(Lang::En);
        let b = sorted(&lib(), SortKind::Box, &Stats::new(), 0);
        assert_eq!(titles(&b), ["Rock", "Songs Outside Boxes"]);
    }

    #[test]
    fn player_and_skill_use_the_records() {
        let _ja = i18n::scoped(Lang::Ja);
        let now = 1_000_000_000;
        let mut stats = Stats::new();
        stats.insert(
            PathBuf::from("apple/2.dtx"),
            ChartStats {
                best_rate: Some(90.0),
                best_rank: Some(1),
                last_played: Some(now - 3600),
            },
        );
        stats.insert(
            PathBuf::from("夜明け/4.dtx"),
            ChartStats {
                best_rate: Some(50.0),
                best_rank: Some(4),
                last_played: Some(now - 40 * 86_400),
            },
        );
        let p = sorted(&lib(), SortKind::Player, &stats, now);
        assert_eq!(titles(&p), ["最後に遊んだ日", "一番よいランク"]);
        let recent = inner(&p, "最後に遊んだ日");
        assert_eq!(titles(&recent), ["今日", "1 年以内", "まだ遊んでいない"]);
        assert_eq!(titles(&inner(&recent, "今日")), ["apple"]);
        let ranks = inner(&p, "一番よいランク");
        assert_eq!(titles(&ranks), ["S", "C", "ランクなし"]);
        // apple の 0、バラード、Banana
        assert_eq!(inner(&ranks, "ランクなし").len(), 3);
        let s = sorted(&lib(), SortKind::Skill, &stats, now);
        let top = inner(&s, "スキルの上位 50 曲");
        // 7.6 × 90 × 0.2 = 136.8、9.9 × 50 × 0.2 = 99
        assert_eq!(titles(&top), ["apple", "夜明け"]);
        let charts = skill_charts(&lib(), &stats);
        assert_eq!(charts.len(), 2);
        assert!(charts.contains(&PathBuf::from("apple/2.dtx")));
        let first = lib().all_songs()[0].clone();
        let (i, v) = top_skill(&first, &stats).unwrap();
        assert_eq!(i, 2);
        assert!((v - 136.8).abs() < 1e-9);
    }
}
