//! NX が譜面の隣に書く `譜面の名前.score.ini`（上流 `CScoreIni`）のうち、ドラムの記録を読む。
//!
//! NXT の記録に引き継ぐのに使う。ギター・ベースと、遊んだときの設定（Risky や HID-SUD など）は読まない。

/// 1 回のプレイの記録（`[HiScore.Drums]`、`[HiSkill.Drums]` など）
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreRecord {
    pub score: i64,
    /// 達成率（上流の演奏型スキル、0〜100）
    pub rate: f64,
    pub perfect: u32,
    pub great: u32,
    pub good: u32,
    pub poor: u32,
    pub miss: u32,
    pub max_combo: u32,
    pub total_chips: u32,
    /// 自動で叩いたレーンがあった（上流の `AutoPlay` に 1 がある）
    pub auto_play: bool,
    /// 遊んだ日時（年、月、日、時、分、秒。この PC の時刻）。書いていなければ None
    pub date_time: Option<[u32; 6]>,
    /// 進み具合のバー（64 文字。'0' はたどり着いていない、'1' は取りこぼしあり、'2' は全部叩けた、'3' は音符なし）
    pub progress: String,
}

impl ScoreRecord {
    /// 遊んだ形跡がある（達成率か進み具合がある。スコアだけの記録は、途中でやめたものなどで当てにならない）
    pub fn is_played(&self) -> bool {
        self.rate > 0.0 || self.progress.chars().any(|c| c != '0')
    }
}

/// score.ini のドラムの部分
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScoreIni {
    /// 遊んだ回数（`PlayCountDrums`）
    pub play_count: u32,
    /// 最後まで遊んだ回数（`ClearCountDrums`）
    pub clear_count: u32,
    /// スコアが一番高かったプレイ
    pub hi_score: Option<ScoreRecord>,
    /// 達成率が一番高かったプレイ
    pub hi_skill: Option<ScoreRecord>,
}

impl ScoreIni {
    /// 中身（文字に直したもの）を読む。読めない行は飛ばす
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut section = String::new();
        let mut fields: Vec<(String, String)> = Vec::new();
        let finish = |section: &str, fields: &mut Vec<(String, String)>, out: &mut Self| {
            let record = || Some(record(fields));
            match section {
                "HiScore.Drums" => out.hi_score = record(),
                "HiSkill.Drums" => out.hi_skill = record(),
                _ => {}
            }
            fields.clear();
        };
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                finish(&section, &mut fields, &mut out);
                section = name.trim().to_owned();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if section == "File" {
                match key {
                    "PlayCountDrums" => out.play_count = value.parse().unwrap_or(0),
                    "ClearCountDrums" => out.clear_count = value.parse().unwrap_or(0),
                    _ => {}
                }
            } else {
                fields.push((key.to_owned(), value.to_owned()));
            }
        }
        finish(&section, &mut fields, &mut out);
        out
    }
}

fn record(fields: &[(String, String)]) -> ScoreRecord {
    let get = |k: &str| {
        fields
            .iter()
            .find(|(key, _)| key == k)
            .map_or("", |(_, v)| v.as_str())
    };
    let count = |k: &str| get(k).parse::<u32>().unwrap_or(0);
    ScoreRecord {
        score: get("Score").parse().unwrap_or(0),
        rate: get("PlaySkill")
            .parse::<f64>()
            .ok()
            .filter(|r| r.is_finite())
            .unwrap_or(0.0)
            .clamp(0.0, 100.0),
        perfect: count("Perfect"),
        great: count("Great"),
        good: count("Good"),
        poor: count("Poor"),
        miss: count("Miss"),
        max_combo: count("MaxCombo"),
        total_chips: count("TotalChips"),
        auto_play: get("AutoPlay").contains('1'),
        date_time: date_time(get("DateTime")),
        progress: get("Progress").to_owned(),
    }
}

/// `2026/05/30 0:13:58` の形（上流は .NET の日時をその PC の書き方で書くので、
/// 英語の Windows の `1/2/2025 3:11:58 PM`（月/日/年と午前・午後）も読む）
fn date_time(s: &str) -> Option<[u32; 6]> {
    let mut words = s.split_whitespace();
    let (date, time) = (words.next()?, words.next()?);
    let half = words.next();
    let num = |p: &str| p.trim().parse::<u32>().ok();
    let d: Vec<&str> = date.split(['/', '-']).collect();
    let t: Vec<&str> = time.split(':').collect();
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    let [year, month, day] = if d[0].len() == 4 {
        [num(d[0])?, num(d[1])?, num(d[2])?]
    } else {
        [num(d[2])?, num(d[0])?, num(d[1])?]
    };
    let mut hour = num(t[0])?;
    match half.map(str::to_ascii_uppercase).as_deref() {
        Some("PM") if hour < 12 => hour += 12,
        Some("AM") if hour == 12 => hour = 0,
        _ => {}
    }
    let out = [year, month, day, hour, num(t[1])?, num(t[2])?];
    let valid = (1..=12).contains(&month) && (1..=31).contains(&day) && hour < 24;
    valid.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEARED: &str = "[File]\nTitle=t\nPlayCountDrums=8\nClearCountDrums=6\nBestRankDrums=2\n\n\
        [HiScore.Drums]\nScore=620000\nPlaySkill=77.5\nPerfect=1280\nGreat=200\nGood=13\nPoor=10\nMiss=40\n\
        MaxCombo=300\nTotalChips=1543\nAutoPlay=000000000000000000000000000\nDateTime=2026/01/09 21:05:07\n\
        Progress=1222\n\
        [HiSkill.Drums]\nScore=617188\nPlaySkill=78.4251458198315\nPerfect=1289\nGreat=192\nGood=12\nPoor=11\n\
        Miss=39\nMaxCombo=315\nTotalChips=1543\nAutoPlay=000000000000000000000000000\n\
        DateTime=2026/05/30 0:13:58\nProgress=1222122\n\
        [HiScore.Guitar]\nScore=999\n";

    #[test]
    fn reads_the_drum_records() {
        let s = ScoreIni::parse(CLEARED);
        assert_eq!((s.play_count, s.clear_count), (8, 6));
        let skill = s.hi_skill.unwrap();
        assert_eq!(skill.score, 617188);
        assert!((skill.rate - 78.4251458198315).abs() < 1e-9);
        assert_eq!(
            (
                skill.perfect,
                skill.great,
                skill.good,
                skill.poor,
                skill.miss
            ),
            (1289, 192, 12, 11, 39)
        );
        assert_eq!((skill.max_combo, skill.total_chips), (315, 1543));
        assert!(!skill.auto_play);
        assert_eq!(skill.date_time, Some([2026, 5, 30, 0, 13, 58]));
        assert_eq!(skill.progress, "1222122");
        // ギターの記録は読まない
        assert_eq!(s.hi_score.unwrap().score, 620000);
    }

    #[test]
    fn failed_plays_keep_only_the_progress() {
        let text = "[File]\nPlayCountDrums=1\nClearCountDrums=0\n[HiSkill.Drums]\nScore=0\nPlaySkill=0\n\
            AutoPlay=000000000000000000000000001\nDateTime=\nProgress=3111100000\n";
        let s = ScoreIni::parse(text);
        let skill = s.hi_skill.unwrap();
        assert!(skill.is_played());
        assert!(skill.auto_play);
        assert_eq!(skill.date_time, None);
        assert_eq!(s.hi_score, None);
        // スコアだけで達成率も進み具合も無い記録（古い版の途中でやめたものなど）は遊んだことにしない
        let odd = ScoreIni::parse(
            "[HiScore.Drums]\nScore=30000\nPlaySkill=0\nPerfect=1690\nProgress=0000\n",
        );
        assert!(!odd.hi_score.unwrap().is_played());
    }

    #[test]
    fn odd_dates_are_ignored() {
        assert_eq!(date_time("2026/13/01 0:00:00"), None);
        assert_eq!(date_time("2026/1/9"), None);
        assert_eq!(
            date_time("2026-01-09 07:08:09"),
            Some([2026, 1, 9, 7, 8, 9])
        );
        // 英語の Windows の書き方
        assert_eq!(
            date_time("1/2/2025 3:11:58 PM"),
            Some([2025, 1, 2, 15, 11, 58])
        );
        assert_eq!(
            date_time("12/31/2024 12:00:01 AM"),
            Some([2024, 12, 31, 0, 0, 1])
        );
    }
}
