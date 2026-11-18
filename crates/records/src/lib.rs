//! 記録。普通のプレイと練習の、音符 1 つごとの結果を SQLite の 1 ファイルに残し、苦手の分析に渡す。
//!
//! - 普通のプレイは「1 周だけの練習」と同じ形で残す（セッション → 周 → 音符）
//! - 音符は「小節・小節の中の位置・レーン」で見分ける。譜面の一部を直しても、ほかの音符の記録はつながる
//! - 表の形は `PRAGMA user_version` の番号で管理し、開くときに足りない分だけ書き換える

use std::path::Path;

use dtx::{DrumLane, NotePos};
use engine::analysis::NoteResult;
use engine::input::Articulation;
use engine::judge::{Judgement, Tally};
use rusqlite::{Connection, Transaction, params};

pub use rusqlite::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// 表の形の番号と、その番号にするための SQL（順に当てる）
const MIGRATIONS: &[&str] = &[
    // 1: 譜面・セッション・周・音符・空打ち
    "
    CREATE TABLE charts (
        id INTEGER PRIMARY KEY,
        path TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        artist TEXT NOT NULL,
        -- ファイルの中身のハッシュ（16 進）。エディタで直すと変わる
        hash TEXT NOT NULL,
        -- ハッシュが最後に変わった時刻（UNIX 秒）
        hash_changed_at INTEGER NOT NULL
    );
    CREATE TABLE sessions (
        id INTEGER PRIMARY KEY,
        chart_id INTEGER NOT NULL REFERENCES charts(id),
        -- 'play'（普通のプレイ）か 'practice'（練習）
        kind TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        -- 遊んだときの譜面のハッシュ
        chart_hash TEXT NOT NULL,
        -- 遊んだときの設定（JSON）
        settings TEXT NOT NULL
    );
    CREATE INDEX sessions_by_chart ON sessions(chart_id, started_at);
    CREATE TABLE laps (
        id INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL REFERENCES sessions(id),
        lap INTEGER NOT NULL,
        speed REAL NOT NULL,
        -- 最後まで遊んだか
        completed INTEGER NOT NULL,
        -- 練習の合格（普通のプレイは NULL）
        passed INTEGER,
        perfect INTEGER NOT NULL,
        great INTEGER NOT NULL,
        good INTEGER NOT NULL,
        poor INTEGER NOT NULL,
        miss INTEGER NOT NULL,
        stray INTEGER NOT NULL,
        max_combo INTEGER NOT NULL,
        offset_mean_ms REAL,
        offset_sd_ms REAL
    );
    CREATE INDEX laps_by_session ON laps(session_id);
    CREATE TABLE hits (
        lap_id INTEGER NOT NULL REFERENCES laps(id),
        measure INTEGER NOT NULL,
        pos_num INTEGER NOT NULL,
        pos_den INTEGER NOT NULL,
        lane TEXT NOT NULL,
        judgement TEXT NOT NULL,
        -- 叩いた時刻 − 音符の時刻（実時間のミリ秒、マイナスは早い）。見逃しは NULL
        offset_ms REAL,
        velocity INTEGER,
        articulation TEXT,
        hh_pedal INTEGER
    );
    CREATE INDEX hits_by_lap ON hits(lap_id);
    CREATE TABLE strays (
        lap_id INTEGER NOT NULL REFERENCES laps(id),
        -- 曲の時刻（ミリ秒）
        at_ms REAL NOT NULL,
        lane TEXT NOT NULL,
        velocity INTEGER NOT NULL,
        articulation TEXT,
        hh_pedal INTEGER
    );
    CREATE INDEX strays_by_lap ON strays(lap_id);
    ",
];

/// 譜面 1 つの見分け方
#[derive(Debug, Clone, Copy)]
pub struct ChartKey<'a> {
    pub path: &'a str,
    pub title: &'a str,
    pub artist: &'a str,
    /// ファイルの中身のハッシュ（[`hash_bytes`]）
    pub hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Play,
    Practice,
}

impl SessionKind {
    fn name(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Practice => "practice",
        }
    }
}

/// 普通のプレイ 1 回、または練習 1 回
#[derive(Debug, Clone)]
pub struct Session {
    pub kind: SessionKind,
    /// 始めた時刻（UNIX 秒）
    pub started_at: i64,
    /// 遊んだときの設定（JSON）
    pub settings: String,
    pub laps: Vec<Lap>,
}

/// 1 周（普通のプレイは 1 周だけ）
#[derive(Debug, Clone)]
pub struct Lap {
    pub speed: f64,
    pub completed: bool,
    pub passed: Option<bool>,
    pub tally: Tally,
    pub offset_mean_ms: Option<f64>,
    pub offset_sd_ms: Option<f64>,
    pub hits: Vec<NoteHit>,
    pub strays: Vec<StrayHit>,
}

/// 音符 1 つの結果
#[derive(Debug, Clone, PartialEq)]
pub struct NoteHit {
    pub at: NotePos,
    pub lane: DrumLane,
    pub judgement: Judgement,
    pub offset_ms: Option<f64>,
    pub velocity: Option<u8>,
    pub articulation: Option<Articulation>,
    pub hh_pedal: Option<u8>,
}

/// 空打ち 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct StrayHit {
    pub at_ms: f64,
    pub lane: DrumLane,
    pub velocity: u8,
    pub articulation: Option<Articulation>,
    pub hh_pedal: Option<u8>,
}

pub struct Records {
    conn: Connection,
}

impl Records {
    /// ファイルを開く（無ければ作る）。フォルダは呼ぶ側で作っておく
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// メモリの中だけに作る（テスト用）
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let mut r = Self { conn };
        r.migrate()?;
        Ok(r)
    }

    /// 表の形の番号
    pub fn schema_version(&self) -> Result<usize> {
        let v: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok(usize::try_from(v).unwrap_or(0))
    }

    fn migrate(&mut self) -> Result<()> {
        let current = self.schema_version()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    /// セッションを 1 つ残す。返すのはセッションの番号
    pub fn save(&mut self, chart: &ChartKey, session: &Session) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let chart_id = upsert_chart(&tx, chart, session.started_at)?;
        tx.execute(
            "INSERT INTO sessions (chart_id, kind, started_at, chart_hash, settings)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                chart_id,
                session.kind.name(),
                session.started_at,
                hash_text(chart.hash),
                session.settings
            ],
        )?;
        let session_id = tx.last_insert_rowid();
        for (i, lap) in session.laps.iter().enumerate() {
            insert_lap(&tx, session_id, i, lap)?;
        }
        tx.commit()?;
        Ok(session_id)
    }

    /// 等速（100%）で叩いた直近 `limit` 周の、音符ごとの判定。新しい順
    pub fn recent_results(&self, chart_path: &str, limit: usize) -> Result<Vec<Vec<NoteResult>>> {
        let mut laps = self.conn.prepare(
            "SELECT l.id FROM laps l
             JOIN sessions s ON s.id = l.session_id
             JOIN charts c ON c.id = s.chart_id
             WHERE c.path = ?1 AND abs(l.speed - 1.0) < 1e-6
             ORDER BY s.started_at DESC, s.id DESC, l.lap DESC
             LIMIT ?2",
        )?;
        let ids = laps
            .query_map(params![chart_path, limit as i64], |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<Result<Vec<_>>>()?;
        let mut hits = self
            .conn
            .prepare("SELECT measure, judgement FROM hits WHERE lap_id = ?1")?;
        ids.into_iter()
            .map(|id| {
                hits.query_map(params![id], |row| {
                    let measure: u32 = row.get(0)?;
                    let name: String = row.get(1)?;
                    Ok((measure, name))
                })?
                .filter_map(|r| match r {
                    Ok((measure, name)) => Judgement::from_name(&name)
                        .map(|judgement| Ok(NoteResult { measure, judgement })),
                    Err(e) => Some(Err(e)),
                })
                .collect()
            })
            .collect()
    }
}

fn upsert_chart(tx: &Transaction, chart: &ChartKey, now: i64) -> Result<i64> {
    tx.query_row(
        "INSERT INTO charts (path, title, artist, hash, hash_changed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET
             title = excluded.title,
             artist = excluded.artist,
             hash_changed_at = CASE WHEN charts.hash = excluded.hash
                 THEN charts.hash_changed_at ELSE excluded.hash_changed_at END,
             hash = excluded.hash
         RETURNING id",
        params![
            chart.path,
            chart.title,
            chart.artist,
            hash_text(chart.hash),
            now
        ],
        |row| row.get(0),
    )
}

fn insert_lap(tx: &Transaction, session_id: i64, index: usize, lap: &Lap) -> Result<()> {
    let t = &lap.tally;
    tx.execute(
        "INSERT INTO laps (session_id, lap, speed, completed, passed,
             perfect, great, good, poor, miss, stray, max_combo, offset_mean_ms, offset_sd_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            session_id,
            index as i64,
            lap.speed,
            lap.completed,
            lap.passed,
            t.perfect,
            t.great,
            t.good,
            t.poor,
            t.miss,
            t.stray,
            t.max_combo,
            lap.offset_mean_ms,
            lap.offset_sd_ms
        ],
    )?;
    let lap_id = tx.last_insert_rowid();
    let mut hit = tx.prepare_cached(
        "INSERT INTO hits (lap_id, measure, pos_num, pos_den, lane, judgement,
             offset_ms, velocity, articulation, hh_pedal)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    for h in &lap.hits {
        hit.execute(params![
            lap_id,
            h.at.measure,
            h.at.num,
            h.at.den,
            h.lane.name(),
            h.judgement.name(),
            h.offset_ms,
            h.velocity,
            h.articulation.map(Articulation::name),
            h.hh_pedal
        ])?;
    }
    let mut stray = tx.prepare_cached(
        "INSERT INTO strays (lap_id, at_ms, lane, velocity, articulation, hh_pedal)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for s in &lap.strays {
        stray.execute(params![
            lap_id,
            s.at_ms,
            s.lane.name(),
            s.velocity,
            s.articulation.map(Articulation::name),
            s.hh_pedal
        ])?;
    }
    Ok(())
}

fn hash_text(hash: u64) -> String {
    format!("{hash:016x}")
}

/// ファイルの中身のハッシュ（FNV-1a 64 ビット）。Rust の版が変わっても値が変わらないものを使う
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes
        .iter()
        .fold(OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart(hash: u64) -> ChartKey<'static> {
        ChartKey {
            path: "songs/a/master.dtx",
            title: "A",
            artist: "someone",
            hash,
        }
    }

    fn hit(measure: u32, judgement: Judgement) -> NoteHit {
        NoteHit {
            at: NotePos::new(measure, 1, 4),
            lane: DrumLane::SD,
            judgement,
            offset_ms: (judgement != Judgement::Miss).then_some(-3.5),
            velocity: (judgement != Judgement::Miss).then_some(90),
            articulation: Some(Articulation::Rim),
            hh_pedal: None,
        }
    }

    fn play(started_at: i64, speed: f64, hits: Vec<NoteHit>) -> Session {
        Session {
            kind: SessionKind::Play,
            started_at,
            settings: "{}".into(),
            laps: vec![Lap {
                speed,
                completed: true,
                passed: None,
                tally: Tally::default(),
                offset_mean_ms: None,
                offset_sd_ms: None,
                hits,
                strays: vec![StrayHit {
                    at_ms: 1234.5,
                    lane: DrumLane::HH,
                    velocity: 40,
                    articulation: Some(Articulation::Bow),
                    hh_pedal: Some(10),
                }],
            }],
        }
    }

    #[test]
    fn known_hashes() {
        assert_eq!(hash_bytes(b""), 0xcbf29ce484222325);
        assert_eq!(hash_bytes(b"a"), 0xaf63dc4c8601ec8c);
    }

    #[test]
    fn new_database_has_the_latest_schema() {
        let r = Records::open_in_memory().unwrap();
        assert_eq!(r.schema_version().unwrap(), MIGRATIONS.len());
    }

    #[test]
    fn recent_results_are_newest_first_at_normal_speed() {
        let mut r = Records::open_in_memory().unwrap();
        r.save(&chart(1), &play(100, 1.0, vec![hit(1, Judgement::Miss)]))
            .unwrap();
        r.save(&chart(1), &play(200, 0.7, vec![hit(2, Judgement::Good)]))
            .unwrap();
        r.save(
            &chart(1),
            &play(
                300,
                1.0,
                vec![hit(3, Judgement::Perfect), hit(4, Judgement::Great)],
            ),
        )
        .unwrap();
        let got = r.recent_results("songs/a/master.dtx", 10).unwrap();
        // 70% の周は数えない
        assert_eq!(
            got,
            vec![
                vec![
                    NoteResult {
                        measure: 3,
                        judgement: Judgement::Perfect
                    },
                    NoteResult {
                        measure: 4,
                        judgement: Judgement::Great
                    },
                ],
                vec![NoteResult {
                    measure: 1,
                    judgement: Judgement::Miss
                }],
            ]
        );
        assert_eq!(r.recent_results("songs/a/master.dtx", 1).unwrap().len(), 1);
        assert!(r.recent_results("other.dtx", 10).unwrap().is_empty());
    }

    #[test]
    fn hits_and_strays_keep_their_details() {
        let mut r = Records::open_in_memory().unwrap();
        r.save(&chart(1), &play(100, 1.0, vec![hit(5, Judgement::Good)]))
            .unwrap();
        let row: (u32, u32, u32, String, String, f64, u8, String) = r
            .conn
            .query_row(
                "SELECT measure, pos_num, pos_den, lane, judgement, offset_ms, velocity, articulation
                 FROM hits",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (5, 1, 4, "SD".into(), "GOOD".into(), -3.5, 90, "rim".into())
        );
        let stray: (f64, String, Option<u8>) = r
            .conn
            .query_row("SELECT at_ms, lane, hh_pedal FROM strays", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(stray, (1234.5, "HH".into(), Some(10)));
    }

    #[test]
    fn edited_chart_updates_its_hash() {
        let mut r = Records::open_in_memory().unwrap();
        r.save(&chart(1), &play(100, 1.0, vec![])).unwrap();
        r.save(&chart(1), &play(200, 1.0, vec![])).unwrap();
        r.save(&chart(2), &play(300, 1.0, vec![])).unwrap();
        let (count, hash, changed): (i64, String, i64) = r
            .conn
            .query_row(
                "SELECT count(*), hash, hash_changed_at FROM charts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (count, hash.as_str(), changed),
            (1, "0000000000000002", 300)
        );
        // セッションには遊んだときのハッシュが残る
        let hashes: Vec<String> = r
            .conn
            .prepare("SELECT chart_hash FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(
            hashes,
            vec!["0000000000000001", "0000000000000001", "0000000000000002"]
        );
    }

    #[test]
    fn reopening_a_file_keeps_the_records() {
        let path = std::env::temp_dir().join(format!(
            "dtxmanianxt-records-test-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let mut r = Records::open(&path).unwrap();
            r.save(&chart(1), &play(100, 1.0, vec![hit(1, Judgement::Miss)]))
                .unwrap();
        }
        let r = Records::open(&path).unwrap();
        assert_eq!(r.schema_version().unwrap(), MIGRATIONS.len());
        assert_eq!(r.recent_results("songs/a/master.dtx", 10).unwrap().len(), 1);
        drop(r);
        let _ = std::fs::remove_file(&path);
    }
}
