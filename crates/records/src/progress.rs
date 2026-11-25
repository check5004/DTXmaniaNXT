//! 上達の記録に使う読み書き。遊んだ時間（日ごとの練習時間）と、譜面ごとの達成率の移り変わり。
//!
//! - 遊んだ時間は `play_time` の表に 1 回ずつ残す。曲・区間練習のほか、ほかの遊び（基礎練習など）も種類を分けて入れられる
//! - 表を足す前の記録には時間が無いので、アプリが譜面から長さを見積もって後から入れる（[`Records::untimed_sessions`]）

use rusqlite::params;

use crate::{ClearRecord, Records, Result, SessionKind};

/// 遊んだ時間の種類（`play_time.kind`）
pub mod time_kind {
    /// 曲を通して遊んだ（途中でやめたときも）
    pub const PLAY: &str = "play";
    /// 区間練習
    pub const PRACTICE: &str = "practice";
    /// 基礎練習
    pub const DRILL: &str = "drill";
}

/// 遊んだ時間 1 回ぶん
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayTime {
    /// [`time_kind`] のどれか
    pub kind: String,
    /// 始めた時刻（UNIX 秒）
    pub started_at: i64,
    /// 叩いていた時間（ミリ秒）
    pub played_ms: i64,
}

/// 時間の記録が無いセッション（表を足す前の記録）。譜面から長さを見積もるのに使う
#[derive(Debug, Clone, PartialEq)]
pub struct UntimedSession {
    pub id: i64,
    pub kind: SessionKind,
    pub started_at: i64,
    pub chart_path: String,
    pub laps: Vec<UntimedLap>,
}

/// 時間の記録が無いセッションの 1 周
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UntimedLap {
    pub speed: f64,
    /// 判定した音符の、最初と最後の位置（小節の番号に小節の中の位置を足したもの）。音符が無ければ None
    pub span: Option<(f64, f64)>,
}

/// 譜面 1 つの、これまでの記録
#[derive(Debug, Clone, PartialEq)]
pub struct ChartProgress {
    pub path: String,
    pub title: String,
    pub artist: String,
    /// 最後まで遊んだ普通のプレイ（NX から引き継いだものも）。古い順
    pub plays: Vec<ProgressPlay>,
    /// 区間練習でクリアした所。古い順
    pub clears: Vec<ClearRecord>,
    /// 最後に遊んだ時刻（UNIX 秒。練習も含む）
    pub last_played: i64,
    /// この譜面で遊んだ時間の合計（ミリ秒。曲と区間練習）
    pub played_ms: i64,
}

/// 最後まで遊んだプレイ 1 回
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressPlay {
    pub session_id: i64,
    pub started_at: i64,
    /// 達成率（0〜100）
    pub rate: f64,
    /// 叩けた音符のずれのばらつき（標準偏差、ミリ秒）。引き継いだ記録には無い
    pub offset_sd_ms: Option<f64>,
    /// NX の score.ini から引き継いだ
    pub imported: bool,
}

impl Records {
    /// 遊んだ時間を 1 回ぶん残す。`session` は一緒に残したセッションの番号（無ければ None）
    pub fn add_play_time(
        &mut self,
        kind: &str,
        started_at: i64,
        played_ms: i64,
        session: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO play_time (kind, started_at, played_ms, session_id) VALUES (?1, ?2, ?3, ?4)",
            params![kind, started_at, played_ms.max(0), session],
        )?;
        Ok(())
    }

    /// 遊んだ時間をまとめて残す（[`untimed_sessions`](Self::untimed_sessions) の見積もりを入れるのに使う）。
    /// 並びは（種類、始めた時刻、ミリ秒、セッションの番号）
    pub fn add_play_times(&mut self, rows: &[(&str, i64, i64, Option<i64>)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut q = tx.prepare(
                "INSERT INTO play_time (kind, started_at, played_ms, session_id) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (kind, started_at, ms, session) in rows {
                q.execute(params![kind, started_at, (*ms).max(0), session])?;
            }
        }
        tx.commit()
    }

    /// 遊んだ時間。`since`（UNIX 秒）があればそれ以後に始めたものだけ。古い順
    pub fn play_times(&self, since: Option<i64>) -> Result<Vec<PlayTime>> {
        let mut q = self.conn.prepare(
            "SELECT kind, started_at, played_ms FROM play_time
             WHERE ?1 IS NULL OR started_at >= ?1
             ORDER BY started_at, id",
        )?;
        q.query_map(params![since], |row| {
            Ok(PlayTime {
                kind: row.get(0)?,
                started_at: row.get(1)?,
                played_ms: row.get(2)?,
            })
        })?
        .collect()
    }

    /// 遊んだ時間の記録が無い、普通のプレイと練習のセッション（表を足す前に遊んだもの）
    pub fn untimed_sessions(&self) -> Result<Vec<UntimedSession>> {
        let mut q = self.conn.prepare(
            "SELECT s.id, s.kind, s.started_at, c.path
             FROM sessions s JOIN charts c ON c.id = s.chart_id
             WHERE s.kind IN ('play', 'practice')
                 AND NOT EXISTS (SELECT 1 FROM play_time t WHERE t.session_id = s.id)
             ORDER BY s.started_at, s.id",
        )?;
        let rows = q
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        let mut laps = self.conn.prepare(
            "SELECT l.speed,
                 MIN(h.measure + CAST(h.pos_num AS REAL) / h.pos_den),
                 MAX(h.measure + CAST(h.pos_num AS REAL) / h.pos_den)
             FROM laps l LEFT JOIN hits h ON h.lap_id = l.id
             WHERE l.session_id = ?1
             GROUP BY l.id
             ORDER BY l.lap",
        )?;
        rows.into_iter()
            .filter_map(|(id, kind, started_at, chart_path)| {
                let kind = SessionKind::from_name(&kind)?;
                let laps = laps
                    .query_map(params![id], |row| {
                        let first: Option<f64> = row.get(1)?;
                        let last: Option<f64> = row.get(2)?;
                        Ok(UntimedLap {
                            speed: row.get(0)?,
                            span: first.zip(last),
                        })
                    })
                    .and_then(|r| r.collect::<Result<Vec<_>>>());
                Some(laps.map(|laps| UntimedSession {
                    id,
                    kind,
                    started_at,
                    chart_path,
                    laps,
                }))
            })
            .collect()
    }

    /// 遊んだことのある譜面すべての、これまでの記録。最後に遊んだのが新しい順。
    /// 基礎練習の譜面（その場で作るもの）は曲ではないので入れない
    pub fn chart_progress(&self) -> Result<Vec<ChartProgress>> {
        let mut charts = self.conn.prepare(
            "SELECT c.id, c.path, c.title, c.artist, MAX(s.started_at),
                 (SELECT COALESCE(SUM(t.played_ms), 0) FROM play_time t
                  JOIN sessions u ON u.id = t.session_id WHERE u.chart_id = c.id)
             FROM charts c JOIN sessions s ON s.chart_id = c.id
             WHERE s.kind <> 'drill'
             GROUP BY c.id
             ORDER BY MAX(s.started_at) DESC, c.id DESC",
        )?;
        let mut plays = self.conn.prepare(
            "SELECT s.id, s.started_at, p.rate, l.offset_sd_ms, s.kind
             FROM play_scores p
             JOIN sessions s ON s.id = p.session_id
             LEFT JOIN laps l ON l.session_id = s.id AND l.lap = 0
             WHERE s.chart_id = ?1 AND p.cleared
             ORDER BY s.started_at, s.id",
        )?;
        let rows = charts
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    ChartProgress {
                        path: row.get(1)?,
                        title: row.get(2)?,
                        artist: row.get(3)?,
                        plays: Vec::new(),
                        clears: Vec::new(),
                        last_played: row.get(4)?,
                        played_ms: row.get(5)?,
                    },
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, mut chart)| {
                chart.plays = plays
                    .query_map(params![id], |row| {
                        let kind: String = row.get(4)?;
                        Ok(ProgressPlay {
                            session_id: row.get(0)?,
                            started_at: row.get(1)?,
                            rate: row.get(2)?,
                            offset_sd_ms: row.get(3)?,
                            imported: kind == SessionKind::Imported.name(),
                        })
                    })?
                    .collect::<Result<Vec<_>>>()?;
                chart.clears = self.clears(&chart.path)?;
                Ok(chart)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use dtx::{DrumLane, NotePos};
    use engine::judge::{Judgement, Tally};

    use super::*;
    use crate::{ChartKey, Lap, NoteHit, PlayScore, SectionClear, Session};

    fn chart(path: &str) -> ChartKey<'_> {
        ChartKey {
            path,
            title: "A",
            artist: "someone",
            hash: 1,
        }
    }

    fn hit(measure: u32, num: u32) -> NoteHit {
        NoteHit {
            at: NotePos::new(measure, num, 4),
            lane: DrumLane::SD,
            judgement: Judgement::Perfect,
            offset_ms: Some(1.0),
            velocity: Some(100),
            articulation: None,
            hh_pedal: None,
            beat_pos: Some(0.0),
        }
    }

    fn lap(speed: f64, hits: Vec<NoteHit>, sd: Option<f64>) -> Lap {
        Lap {
            speed,
            completed: true,
            passed: None,
            tally: Tally::default(),
            offset_mean_ms: sd.map(|_| 0.0),
            offset_sd_ms: sd,
            hits,
            strays: Vec::new(),
        }
    }

    fn played(started_at: i64, rate: f64, cleared: bool, sd: f64) -> Session {
        Session {
            kind: SessionKind::Play,
            started_at,
            settings: "{}".into(),
            laps: vec![lap(1.0, vec![hit(0, 0), hit(12, 3)], Some(sd))],
            clear: None,
            score: Some(PlayScore {
                score: 0,
                rate,
                progress: "2".repeat(64),
                cleared,
            }),
        }
    }

    #[test]
    fn play_times_are_kept_in_order() {
        let mut r = Records::open_in_memory().unwrap();
        r.add_play_time(time_kind::PRACTICE, 200, 60_000, None)
            .unwrap();
        r.add_play_time(time_kind::PLAY, 100, 90_000, None).unwrap();
        r.add_play_time(time_kind::DRILL, 300, -5, None).unwrap();
        let all = r.play_times(None).unwrap();
        assert_eq!(
            all.iter().map(|t| t.started_at).collect::<Vec<_>>(),
            [100, 200, 300]
        );
        assert_eq!(all[0].kind, "play");
        // マイナスの時間は 0 にする
        assert_eq!(all[2].played_ms, 0);
        assert_eq!(r.play_times(Some(200)).unwrap().len(), 2);
    }

    #[test]
    fn untimed_sessions_give_the_measures_each_lap_reached() {
        let mut r = Records::open_in_memory().unwrap();
        let old = r
            .save(&chart("a.dtx"), &played(100, 80.0, true, 20.0))
            .unwrap();
        let practice = Session {
            kind: SessionKind::Practice,
            started_at: 200,
            settings: "{}".into(),
            laps: vec![
                lap(0.7, vec![hit(8, 0), hit(9, 2)], None),
                lap(0.8, Vec::new(), None),
            ],
            clear: None,
            score: None,
        };
        r.save(&chart("a.dtx"), &practice).unwrap();
        // 時間を残したセッションと、引き継いだ記録は入れない
        let timed = r
            .save(&chart("a.dtx"), &played(300, 81.0, true, 19.0))
            .unwrap();
        r.add_play_time(time_kind::PLAY, 300, 1000, Some(timed))
            .unwrap();
        let mut imported = played(50, 70.0, true, 0.0);
        imported.kind = SessionKind::Imported;
        r.save(&chart("a.dtx"), &imported).unwrap();

        let got = r.untimed_sessions().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, old);
        assert_eq!(got[0].kind, SessionKind::Play);
        assert_eq!(got[0].chart_path, "a.dtx");
        assert_eq!(
            got[0].laps,
            [UntimedLap {
                speed: 1.0,
                span: Some((0.0, 12.75))
            }]
        );
        assert_eq!(got[1].kind, SessionKind::Practice);
        assert_eq!(
            got[1].laps,
            [
                UntimedLap {
                    speed: 0.7,
                    span: Some((8.0, 9.5))
                },
                UntimedLap {
                    speed: 0.8,
                    span: None
                }
            ]
        );
        // 見積もりを入れたら、もう出てこない
        let rows: Vec<_> = got
            .iter()
            .map(|s| (time_kind::PLAY, s.started_at, 500, Some(s.id)))
            .collect();
        r.add_play_times(&rows).unwrap();
        assert!(r.untimed_sessions().unwrap().is_empty());
        assert_eq!(r.play_times(None).unwrap().len(), 3);
    }

    #[test]
    fn chart_progress_lists_cleared_plays_and_practice_clears() {
        let mut r = Records::open_in_memory().unwrap();
        let first = r
            .save(&chart("a.dtx"), &played(100, 70.0, true, 22.0))
            .unwrap();
        r.add_play_time(time_kind::PLAY, 100, 120_000, Some(first))
            .unwrap();
        // 途中でやめたプレイは線に入れない
        r.save(&chart("a.dtx"), &played(150, 30.0, false, 30.0))
            .unwrap();
        let practice = Session {
            kind: SessionKind::Practice,
            started_at: 200,
            settings: "{}".into(),
            laps: vec![lap(1.0, vec![hit(4, 0)], None)],
            clear: Some(SectionClear {
                first: 4,
                last: 7,
                speed: 1.0,
            }),
            score: None,
        };
        let p = r.save(&chart("a.dtx"), &practice).unwrap();
        r.add_play_time(time_kind::PRACTICE, 200, 300_000, Some(p))
            .unwrap();
        r.save(&chart("a.dtx"), &played(300, 76.5, true, 18.0))
            .unwrap();
        r.save(&chart("b.dtx"), &played(250, 90.0, true, 15.0))
            .unwrap();
        // 基礎練習の時間は譜面の時間に入れない
        r.add_play_time(time_kind::DRILL, 260, 999_000, None)
            .unwrap();
        // 基礎練習の譜面は曲の一覧に出さない（いちばん新しくても）
        let drill = Session {
            kind: SessionKind::Drill,
            started_at: 400,
            settings: "{}".into(),
            laps: vec![lap(0.7, vec![hit(1, 0)], None)],
            clear: None,
            score: None,
        };
        let d = r.save(&chart("drill:paradiddle"), &drill).unwrap();
        r.add_play_time(time_kind::DRILL, 400, 45_000, Some(d))
            .unwrap();

        let got = r.chart_progress().unwrap();
        assert_eq!(
            got.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            ["a.dtx", "b.dtx"]
        );
        let a = &got[0];
        assert_eq!(a.last_played, 300);
        assert_eq!(a.played_ms, 420_000);
        assert_eq!(
            a.plays
                .iter()
                .map(|p| (p.started_at, p.rate, p.offset_sd_ms))
                .collect::<Vec<_>>(),
            [(100, 70.0, Some(22.0)), (300, 76.5, Some(18.0))]
        );
        assert_eq!(a.plays[0].session_id, first);
        assert!(!a.plays[0].imported);
        assert_eq!(a.clears.len(), 1);
        assert_eq!((a.clears[0].first, a.clears[0].last), (4, 7));
        assert_eq!(got[1].played_ms, 0);
    }
}
