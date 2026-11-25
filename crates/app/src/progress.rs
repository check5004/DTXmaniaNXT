//! 上達の記録の集計。遊んだ時間を日ごと・週ごとにまとめ、譜面ごとの達成率の移り変わりを出す。
//!
//! - 日付は朝 4 時で区切る（夜遅くの練習は前の日に数える。日付が変わってすぐ遊んでも「続けて」が切れない）
//! - 週は月曜から
//! - 遊んだ時間の表を足す前の記録には時間が無いので、譜面から長さを見積もって入れる（[`estimate_ms`]）

use std::collections::BTreeMap;

use chrono::{Datelike, Days, Local, NaiveDate, TimeZone};
use dtx::TempoMap;
use records::{ChartProgress, PlayTime, SessionKind, UntimedSession, time_kind};

/// 日付が変わる時刻（朝 4 時）
const DAY_START_HOUR: i64 = 4;

/// UNIX 秒の、この PC の時間帯での日付（朝 4 時で区切る）
pub fn local_day(unix: i64) -> NaiveDate {
    Local
        .timestamp_opt(unix - DAY_START_HOUR * 3600, 0)
        .earliest()
        .map(|t| t.date_naive())
        .unwrap_or_default()
}

/// 今日（朝 4 時で区切る）
pub fn today() -> NaiveDate {
    local_day(Local::now().timestamp())
}

/// 1 日（か 1 週）に遊んだ時間（ミリ秒）。種類ごとに分ける
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spent {
    /// 曲を通して遊んだ
    pub play: i64,
    /// 区間練習
    pub practice: i64,
    /// 基礎練習など、ほかの遊び
    pub other: i64,
}

impl Spent {
    pub fn total(&self) -> i64 {
        self.play + self.practice + self.other
    }

    fn add(&mut self, kind: &str, ms: i64) {
        match kind {
            time_kind::PLAY => self.play += ms,
            time_kind::PRACTICE => self.practice += ms,
            _ => self.other += ms,
        }
    }

    fn sum<'a>(items: impl Iterator<Item = &'a Spent>) -> Spent {
        items.fold(Spent::default(), |a, b| Spent {
            play: a.play + b.play,
            practice: a.practice + b.practice,
            other: a.other + b.other,
        })
    }
}

/// 遊んだ時間を日ごとにまとめたもの
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimeLog {
    days: BTreeMap<NaiveDate, Spent>,
}

impl TimeLog {
    /// 遊んだ時間を `day`（UNIX 秒 → 日付）で日ごとにまとめる
    pub fn new(times: &[PlayTime], day: impl Fn(i64) -> NaiveDate) -> Self {
        let mut days: BTreeMap<NaiveDate, Spent> = BTreeMap::new();
        for t in times.iter().filter(|t| t.played_ms > 0) {
            days.entry(day(t.started_at))
                .or_default()
                .add(&t.kind, t.played_ms);
        }
        Self { days }
    }

    pub fn day(&self, d: NaiveDate) -> Spent {
        self.days.get(&d).copied().unwrap_or_default()
    }

    /// `today` までの `n` 日。古い順
    pub fn last_days(&self, today: NaiveDate, n: usize) -> Vec<(NaiveDate, Spent)> {
        (0..n as u64)
            .rev()
            .filter_map(|back| today.checked_sub_days(Days::new(back)))
            .map(|d| (d, self.day(d)))
            .collect()
    }

    /// 週ごと（月曜の日付と、その週の合計）。最初に遊んだ週から `today` の週まで、古い順
    pub fn weeks(&self, today: NaiveDate) -> Vec<(NaiveDate, Spent)> {
        let last = week_start(today);
        let first = self.days.keys().next().map_or(last, |&d| week_start(d));
        let mut out = Vec::new();
        let mut w = first;
        while w <= last {
            out.push((w, self.week(w)));
            let Some(next) = w.checked_add_days(Days::new(7)) else {
                break;
            };
            w = next;
        }
        out
    }

    /// 月曜 `monday` からの 1 週の合計
    fn week(&self, monday: NaiveDate) -> Spent {
        let end = monday.checked_add_days(Days::new(7)).unwrap_or(monday);
        Spent::sum(self.days.range(monday..end).map(|(_, s)| s))
    }

    /// `today` の週の合計と、遊んだ日の数
    pub fn this_week(&self, today: NaiveDate) -> (Spent, usize) {
        let monday = week_start(today);
        let days = self
            .days
            .range(monday..=today)
            .filter(|(_, s)| s.total() > 0)
            .count();
        (self.week(monday), days)
    }

    /// 続けて遊んだ日の数。今日まだ遊んでいなければ昨日まで（今日これから遊べば続くので、0 にしない）
    pub fn streak(&self, today: NaiveDate) -> usize {
        let played = |d: NaiveDate| self.day(d).total() > 0;
        let mut d = if played(today) {
            today
        } else {
            match today.pred_opt() {
                Some(y) => y,
                None => return 0,
            }
        };
        let mut n = 0;
        while played(d) {
            n += 1;
            match d.pred_opt() {
                Some(p) => d = p,
                None => break,
            }
        }
        n
    }

    /// これまでの合計
    pub fn total(&self) -> Spent {
        Spent::sum(self.days.values())
    }

    /// 遊んだ日の数
    pub fn played_days(&self) -> usize {
        self.days.values().filter(|s| s.total() > 0).count()
    }
}

/// 月曜の日付
pub fn week_start(d: NaiveDate) -> NaiveDate {
    d.checked_sub_days(Days::new(u64::from(d.weekday().num_days_from_monday())))
        .unwrap_or(d)
}

/// 譜面 1 つの、達成率の移り変わり（一覧の 1 行と、右のグラフ）
#[derive(Debug, Clone, PartialEq)]
pub struct SongTrend {
    pub record: ChartProgress,
    /// 難易度の名前（MASTER など。分からなければ空）
    pub label: String,
    /// ドラムのレベル（分からなければ None）
    pub level: Option<f64>,
}

impl SongTrend {
    /// 最後まで遊んだプレイの一番高い達成率
    pub fn best(&self) -> Option<f64> {
        self.record.plays.iter().map(|p| p.rate).reduce(f64::max)
    }

    /// 初めて最後まで遊んだときの達成率
    pub fn first(&self) -> Option<f64> {
        self.record.plays.first().map(|p| p.rate)
    }

    /// 初回からベストまでの伸び（ポイント）。プレイが 2 回以上のときだけ。
    /// 調子の悪い日があっても減らないよう、最新ではなくベストと比べる
    pub fn gain(&self) -> Option<f64> {
        (self.record.plays.len() >= 2)
            .then(|| self.best().zip(self.first()).map(|(b, f)| b - f))
            .flatten()
    }
}

/// 時間の記録が無いセッションの長さの見積もり（ミリ秒）。`tempo` はその譜面のテンポ。
/// 普通のプレイは曲の頭から最後に判定した音符まで、練習は周ごとに区間の小節（判定した音符の小節）の長さを速度で割る
pub fn estimate_ms(session: &UntimedSession, tempo: &TempoMap) -> i64 {
    let ms_at = |pos: f64| -> f64 {
        let last = tempo.measure_count();
        let m = (pos.max(0.0).floor() as usize).min(last);
        let frac = if m < last { pos - m as f64 } else { 0.0 };
        let start = tempo.measure_start_beat(m);
        let len = if m < last {
            tempo.measure_start_beat(m + 1) - start
        } else {
            0.0
        };
        tempo.beat_to_ms(start + frac.clamp(0.0, 1.0) * len)
    };
    let ms: f64 = session
        .laps
        .iter()
        .filter_map(|l| l.span.map(|span| (l.speed.max(0.1), span)))
        .map(|(speed, (first, last))| match session.kind {
            SessionKind::Practice => {
                (ms_at(last.floor() + 1.0) - ms_at(first.floor())).max(0.0) / speed
            }
            _ => ms_at(last).max(0.0) / speed,
        })
        .sum();
    ms.round() as i64
}

/// 時間の長さの文（「1 時間 5 分」「12 分」「1 分未満」）
pub fn duration_text(ms: i64) -> String {
    let min = ms / 60_000;
    if ms > 0 && min == 0 {
        return i18n::t("< 1 min", "1 分未満").to_owned();
    }
    let (h, m) = (min / 60, min % 60);
    match (h, m) {
        (0, m) => i18n::tf!("{m} min", "{m} 分"),
        (h, 0) => i18n::tf!("{h} h", "{h} 時間"),
        (h, m) => i18n::tf!("{h} h {m} min", "{h} 時間 {m} 分"),
    }
}

#[cfg(test)]
mod tests {
    use records::{ProgressPlay, UntimedLap};

    use super::*;

    fn date(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    /// 試験の日付: UNIX 秒を 1 日 = 100 秒として、11 月 1 日からの日にする
    fn day_of(t: i64) -> NaiveDate {
        date(11, 1) + Days::new((t / 100) as u64)
    }

    fn time(kind: &str, day: i64, min: i64) -> PlayTime {
        PlayTime {
            kind: kind.into(),
            started_at: day * 100 + 10,
            played_ms: min * 60_000,
        }
    }

    #[test]
    fn days_and_weeks_add_up_by_kind() {
        // 11/1 は日曜。11/2〜11/8 が 1 週
        let log = TimeLog::new(
            &[
                time(time_kind::PLAY, 0, 10),
                time(time_kind::PLAY, 1, 5),
                time(time_kind::PRACTICE, 1, 20),
                time(time_kind::DRILL, 3, 7),
                time(time_kind::PLAY, 3, 0),
            ],
            day_of,
        );
        assert_eq!(
            log.day(date(11, 2)),
            Spent {
                play: 5 * 60_000,
                practice: 20 * 60_000,
                other: 0
            }
        );
        let days = log.last_days(date(11, 4), 3);
        assert_eq!(
            days.iter()
                .map(|(d, s)| (d.day(), s.total() / 60_000))
                .collect::<Vec<_>>(),
            [(2, 25), (3, 0), (4, 7)]
        );
        let weeks = log.weeks(date(11, 4));
        assert_eq!(
            weeks
                .iter()
                .map(|(d, s)| (d.day(), s.total() / 60_000))
                .collect::<Vec<_>>(),
            [(26, 10), (2, 32)]
        );
        let (week, played) = log.this_week(date(11, 4));
        assert_eq!((week.total() / 60_000, played), (32, 2));
        assert_eq!(log.total().total() / 60_000, 42);
        assert_eq!(log.played_days(), 3);
    }

    #[test]
    fn streak_counts_up_to_yesterday_until_today_is_played() {
        let log = TimeLog::new(
            &[
                time(time_kind::PLAY, 2, 3),
                time(time_kind::PLAY, 3, 3),
                time(time_kind::PLAY, 4, 3),
            ],
            day_of,
        );
        assert_eq!(log.streak(date(11, 5)), 3);
        // 今日まだ遊んでいなくても、昨日までが続いている
        assert_eq!(log.streak(date(11, 6)), 3);
        // 2 日空いたら切れる
        assert_eq!(log.streak(date(11, 7)), 0);
        assert_eq!(TimeLog::default().streak(date(11, 7)), 0);
    }

    #[test]
    fn local_days_start_at_four_in_the_morning() {
        let at = |h: u32, m: u32| {
            Local
                .with_ymd_and_hms(2026, 11, 25, h, m, 0)
                .earliest()
                .unwrap()
                .timestamp()
        };
        assert_eq!(local_day(at(3, 59)), date(11, 24));
        assert_eq!(local_day(at(4, 0)), date(11, 25));
        assert_eq!(local_day(at(23, 30)), date(11, 25));
    }

    #[test]
    fn gain_compares_the_best_with_the_first_play() {
        let play = |rate: f64| ProgressPlay {
            session_id: 0,
            started_at: 0,
            rate,
            offset_sd_ms: None,
            imported: false,
        };
        let mut t = SongTrend {
            record: ChartProgress {
                path: "a.dtx".into(),
                title: "A".into(),
                artist: String::new(),
                plays: vec![play(70.0)],
                clears: Vec::new(),
                last_played: 0,
                played_ms: 0,
            },
            label: String::new(),
            level: None,
        };
        assert_eq!(t.gain(), None);
        t.record.plays.extend([play(80.5), play(75.0)]);
        assert_eq!(t.best(), Some(80.5));
        assert_eq!(t.gain(), Some(10.5));
    }

    #[test]
    fn estimates_play_and_practice_lengths_from_the_chart() {
        // 120 BPM の 4/4（1 小節 2 秒）、小節 0〜9
        let chart = dtx::parse("#BPM 120\n#00911: 01\n");
        let tempo = TempoMap::build(&chart);
        let play = UntimedSession {
            id: 1,
            kind: SessionKind::Play,
            started_at: 0,
            chart_path: String::new(),
            laps: vec![UntimedLap {
                speed: 1.0,
                span: Some((0.0, 5.5)),
            }],
        };
        assert_eq!(estimate_ms(&play, &tempo), 11_000);
        // 練習は小節 2〜3 を 2 周（50% と 100%）。叩かなかった周は数えない
        let practice = UntimedSession {
            kind: SessionKind::Practice,
            laps: vec![
                UntimedLap {
                    speed: 0.5,
                    span: Some((2.0, 3.75)),
                },
                UntimedLap {
                    speed: 1.0,
                    span: Some((2.25, 3.0)),
                },
                UntimedLap {
                    speed: 1.0,
                    span: None,
                },
            ],
            ..play
        };
        assert_eq!(estimate_ms(&practice, &tempo), 8_000 + 4_000);
    }

    #[test]
    fn durations_read_naturally() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        assert_eq!(duration_text(0), "0 分");
        assert_eq!(duration_text(30_000), "1 分未満");
        assert_eq!(duration_text(12 * 60_000), "12 分");
        assert_eq!(duration_text(120 * 60_000), "2 時間");
        assert_eq!(duration_text(125 * 60_000), "2 時間 5 分");
        drop(_ja);
        assert_eq!(duration_text(125 * 60_000), "2 h 5 min");
    }
}
