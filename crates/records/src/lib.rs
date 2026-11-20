//! 記録。普通のプレイと練習の、音符 1 つごとの結果を SQLite の 1 ファイルに残し、苦手の分析に渡す。
//!
//! - 普通のプレイは「1 周だけの練習」と同じ形で残す（セッション → 周 → 音符）
//! - 音符は「小節・小節の中の位置・レーン」で見分ける。譜面の一部を直しても、ほかの音符の記録はつながる
//! - 表の形は `PRAGMA user_version` の番号で管理し、開くときに足りない分だけ書き換える

use std::path::Path;

use dtx::{DrumLane, NotePos};
use engine::analysis::{NoteResult, TimedHit};
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
    // 2: 音符の拍の中の位置（叩き方のくせに使う）と、練習でクリアした区間（克服の印に使う）
    "
    -- 拍の中の位置（0 以上 1 未満、拍の頭が 0）。前の版の記録は NULL
    ALTER TABLE hits ADD COLUMN beat_pos REAL;
    CREATE TABLE clears (
        id INTEGER PRIMARY KEY,
        chart_id INTEGER NOT NULL REFERENCES charts(id),
        session_id INTEGER NOT NULL REFERENCES sessions(id),
        first_measure INTEGER NOT NULL,
        last_measure INTEGER NOT NULL,
        -- 続けて合格した速度（1.0 以上）
        speed REAL NOT NULL,
        -- クリアした練習を始めた時刻（UNIX 秒）
        cleared_at INTEGER NOT NULL,
        -- 普通のプレイで克服した時刻と、そのときの区間の苦手度（練習の前と後）。まだなら NULL
        overcome_at INTEGER,
        weakness_before REAL,
        weakness_after REAL
    );
    CREATE INDEX clears_by_chart ON clears(chart_id);
    ",
    // 3: 普通のプレイのスコアと達成率と進み具合のバー（結果の画面の New Record と前回のベストに使う）
    "
    CREATE TABLE play_scores (
        session_id INTEGER PRIMARY KEY REFERENCES sessions(id),
        -- XG のスコア（0〜100 万）
        score INTEGER NOT NULL,
        -- 達成率（上流の演奏型スキル、0〜100）
        rate REAL NOT NULL,
        -- 進み具合のバー。64 区間を最初から 1 文字ずつ（上流の score.ini と同じ。
        -- '0' はたどり着いていない、'1' は取りこぼしあり、'2' は全部叩けた、'3' は音符なし）
        progress TEXT NOT NULL,
        -- 最後まで遊んだ（STAGE FAILED や途中でやめたら 0）
        cleared INTEGER NOT NULL
    );
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

    fn from_name(name: &str) -> Option<Self> {
        [Self::Play, Self::Practice]
            .into_iter()
            .find(|k| k.name() == name)
    }
}

/// 等速で叩いた周 1 つぶんの、音符ごとの判定
#[derive(Debug, Clone, PartialEq)]
pub struct RecentLap {
    pub kind: SessionKind,
    pub results: Vec<NoteResult>,
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
    /// 練習で区間をクリアした（等速以上で続けて合格した）
    pub clear: Option<SectionClear>,
    /// 普通のプレイのスコアと達成率（練習は None）
    pub score: Option<PlayScore>,
}

/// 普通のプレイ 1 回のスコアと達成率と進み具合
#[derive(Debug, Clone, PartialEq)]
pub struct PlayScore {
    pub score: i64,
    /// 達成率（0〜100）
    pub rate: f64,
    /// 進み具合のバー（64 文字。[`PlayScore::reached`] を参照）
    pub progress: String,
    /// 最後まで遊んだ
    pub cleared: bool,
}

impl PlayScore {
    /// 進み具合のバーで、たどり着いた区間の数（上流 `tProgressBarLength`）。形が違えば 0
    pub fn reached(progress: &str) -> usize {
        if progress.chars().count() != PROGRESS_SECTIONS {
            return 0;
        }
        progress.chars().filter(|&c| c != '0').count()
    }
}

/// 進み具合のバーの区間の数（上流 `nSectionIntervalCount`）
pub const PROGRESS_SECTIONS: usize = 64;

/// 譜面のこれまでのベスト（結果の画面に出す）
#[derive(Debug, Clone, PartialEq)]
pub struct BestScore {
    /// 最後まで遊んだプレイの一番高い達成率。まだ無ければ None
    pub rate: Option<f64>,
    /// 一番高いスコア
    pub score: Option<i64>,
    /// 前回までのベストの進み具合。達成率が一番高かったプレイのもの、
    /// 最後まで遊んだプレイが無ければ一番先までたどり着いたもの（上流と同じ）
    pub progress: Option<String>,
}

/// 練習でクリアした区間
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionClear {
    pub first: u32,
    pub last: u32,
    /// 続けて合格した速度（1.0 以上）
    pub speed: f64,
}

/// 残っているクリアの記録
#[derive(Debug, Clone, PartialEq)]
pub struct ClearRecord {
    pub first: u32,
    pub last: u32,
    pub speed: f64,
    pub cleared_at: i64,
    /// 克服した時刻と、そのときの区間の苦手度（練習の前、後）
    pub overcome: Option<(i64, Option<f64>, Option<f64>)>,
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
    /// 拍の中の位置（0 以上 1 未満、拍の頭が 0）
    pub beat_pos: Option<f64>,
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
        if let Some(p) = &session.score {
            tx.execute(
                "INSERT INTO play_scores (session_id, score, rate, progress, cleared)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, p.score, p.rate, p.progress, p.cleared],
            )?;
        }
        if let Some(c) = session.clear {
            tx.execute(
                "INSERT INTO clears (chart_id, session_id, first_measure, last_measure, speed, cleared_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![chart_id, session_id, c.first, c.last, c.speed, session.started_at],
            )?;
        }
        tx.commit()?;
        Ok(session_id)
    }

    /// 譜面のこれまでのベスト。`before`（セッションの番号）があれば、それより前のプレイだけから
    pub fn best_score(&self, chart_path: &str, before: Option<i64>) -> Result<BestScore> {
        let mut q = self.conn.prepare(
            "SELECT p.score, p.rate, p.progress, p.cleared
             FROM play_scores p
             JOIN sessions s ON s.id = p.session_id
             JOIN charts c ON c.id = s.chart_id
             WHERE c.path = ?1 AND (?2 IS NULL OR s.id < ?2)
             ORDER BY s.started_at, s.id",
        )?;
        let rows = q
            .query_map(params![chart_path, before], |row| {
                Ok(PlayScore {
                    score: row.get(0)?,
                    rate: row.get(1)?,
                    progress: row.get(2)?,
                    cleared: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        let score = rows.iter().map(|p| p.score).max();
        // 同じ達成率なら先に出したもの
        let mut best: Option<&PlayScore> = None;
        for p in rows.iter().filter(|p| p.cleared) {
            if best.is_none_or(|b| p.rate > b.rate) {
                best = Some(p);
            }
        }
        let progress = match best {
            Some(b) => Some(b.progress.clone()),
            None => {
                let mut far: Option<&PlayScore> = None;
                for p in &rows {
                    if far.is_none_or(|f| {
                        PlayScore::reached(&p.progress) >= PlayScore::reached(&f.progress)
                    }) {
                        far = Some(p);
                    }
                }
                far.map(|f| f.progress.clone())
            }
        };
        Ok(BestScore {
            rate: best.map(|b| b.rate),
            score,
            progress,
        })
    }

    /// 譜面のクリアの記録。古い順
    pub fn clears(&self, chart_path: &str) -> Result<Vec<ClearRecord>> {
        let mut q = self.conn.prepare(
            "SELECT k.first_measure, k.last_measure, k.speed, k.cleared_at,
                 k.overcome_at, k.weakness_before, k.weakness_after
             FROM clears k JOIN charts c ON c.id = k.chart_id
             WHERE c.path = ?1
             ORDER BY k.cleared_at, k.id",
        )?;
        q.query_map(params![chart_path], |row| {
            let overcome_at: Option<i64> = row.get(4)?;
            let before: Option<f64> = row.get(5)?;
            let after: Option<f64> = row.get(6)?;
            Ok(ClearRecord {
                first: row.get(0)?,
                last: row.get(1)?,
                speed: row.get(2)?,
                cleared_at: row.get(3)?,
                overcome: overcome_at.map(|at| (at, before, after)),
            })
        })?
        .collect()
    }

    /// まだ克服していない区間（小節 `first`〜`last`）のクリアを、時刻 `at` に克服したことにする。
    /// `before` と `after` は区間の苦手度（練習の前と、克服したプレイのあと）。返すのは書き換えた数
    pub fn mark_overcome(
        &mut self,
        chart_path: &str,
        first: u32,
        last: u32,
        at: i64,
        before: Option<f64>,
        after: Option<f64>,
    ) -> Result<usize> {
        self.conn.execute(
            "UPDATE clears SET overcome_at = ?4, weakness_before = ?5, weakness_after = ?6
             WHERE overcome_at IS NULL AND first_measure = ?2 AND last_measure = ?3
                 AND chart_id = (SELECT id FROM charts WHERE path = ?1)",
            params![chart_path, first, last, at, before, after],
        )
    }

    /// 等速（100%）で叩けた音符のずれと拍の中の位置。すべての譜面の、直近 `sessions` 回のプレイと練習から
    pub fn recent_timed_hits(&self, sessions: usize) -> Result<Vec<TimedHit>> {
        let mut q = self.conn.prepare(
            "SELECT h.lane, h.beat_pos, h.offset_ms
             FROM hits h JOIN laps l ON l.id = h.lap_id
             WHERE h.offset_ms IS NOT NULL AND h.beat_pos IS NOT NULL
                 AND abs(l.speed - 1.0) < 1e-6
                 AND l.session_id IN (SELECT id FROM sessions ORDER BY started_at DESC, id DESC LIMIT ?1)",
        )?;
        q.query_map(params![sessions as i64], |row| {
            let lane: String = row.get(0)?;
            Ok((lane, row.get::<_, f64>(1)?, row.get::<_, f64>(2)?))
        })?
        .filter_map(|r| match r {
            Ok((lane, beat_pos, offset_ms)) => DrumLane::from_name(&lane).map(|lane| {
                Ok(TimedHit {
                    lane,
                    beat_pos,
                    offset_ms,
                })
            }),
            Err(e) => Some(Err(e)),
        })
        .collect()
    }

    /// 等速（100%）で叩いた周の、音符ごとの判定。新しい順。
    /// 普通のプレイは直近 `plays` 回、練習は直近 `practice_laps` 周まで（練習の周は区間しか叩かないので多めに読む）。
    /// `before` があれば、その時刻（UNIX 秒）より前に始めたセッションだけ
    pub fn recent_results(
        &self,
        chart_path: &str,
        plays: usize,
        practice_laps: usize,
        before: Option<i64>,
    ) -> Result<Vec<RecentLap>> {
        let mut laps = self.conn.prepare(
            "SELECT id, kind FROM (
                 SELECT l.id, s.kind, s.started_at, s.id AS session, l.lap,
                     ROW_NUMBER() OVER (
                         PARTITION BY s.kind
                         ORDER BY s.started_at DESC, s.id DESC, l.lap DESC
                     ) AS n
                 FROM laps l
                 JOIN sessions s ON s.id = l.session_id
                 JOIN charts c ON c.id = s.chart_id
                 WHERE c.path = ?1 AND abs(l.speed - 1.0) < 1e-6
                     AND (?4 IS NULL OR s.started_at < ?4)
             )
             WHERE (kind = 'play' AND n <= ?2) OR (kind = 'practice' AND n <= ?3)
             ORDER BY started_at DESC, session DESC, lap DESC",
        )?;
        let ids = laps
            .query_map(
                params![chart_path, plays as i64, practice_laps as i64, before],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<Result<Vec<_>>>()?;
        let mut hits = self
            .conn
            .prepare("SELECT measure, judgement FROM hits WHERE lap_id = ?1")?;
        ids.into_iter()
            .filter_map(|(id, kind)| SessionKind::from_name(&kind).map(|kind| (id, kind)))
            .map(|(id, kind)| {
                let results = hits
                    .query_map(params![id], |row| {
                        let measure: u32 = row.get(0)?;
                        let name: String = row.get(1)?;
                        Ok((measure, name))
                    })?
                    .filter_map(|r| match r {
                        Ok((measure, name)) => Judgement::from_name(&name)
                            .map(|judgement| Ok(NoteResult { measure, judgement })),
                        Err(e) => Some(Err(e)),
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(RecentLap { kind, results })
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
             offset_ms, velocity, articulation, hh_pedal, beat_pos)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            h.hh_pedal,
            h.beat_pos
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
            beat_pos: Some(0.25),
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
            clear: None,
            score: None,
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
        let got: Vec<Vec<NoteResult>> = r
            .recent_results("songs/a/master.dtx", 10, 10, None)
            .unwrap()
            .into_iter()
            .map(|l| l.results)
            .collect();
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
        assert_eq!(
            r.recent_results("songs/a/master.dtx", 1, 0, None)
                .unwrap()
                .len(),
            1
        );
        assert!(
            r.recent_results("other.dtx", 10, 10, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn practice_laps_are_limited_separately() {
        let mut r = Records::open_in_memory().unwrap();
        r.save(&chart(1), &play(100, 1.0, vec![hit(1, Judgement::Miss)]))
            .unwrap();
        let mut practice = play(200, 1.0, vec![hit(7, Judgement::Good)]);
        practice.kind = SessionKind::Practice;
        let lap = practice.laps[0].clone();
        practice.laps.push(Lap {
            hits: vec![hit(8, Judgement::Perfect)],
            ..lap.clone()
        });
        practice.laps.push(Lap {
            speed: 0.7,
            hits: vec![hit(9, Judgement::Perfect)],
            ..lap
        });
        r.save(&chart(1), &practice).unwrap();
        let got: Vec<(SessionKind, u32)> = r
            .recent_results("songs/a/master.dtx", 10, 10, None)
            .unwrap()
            .into_iter()
            .map(|l| (l.kind, l.results[0].measure))
            .collect();
        // 新しい順（同じ練習の中は後の周が先）。70% の周は入らない
        assert_eq!(
            got,
            vec![
                (SessionKind::Practice, 8),
                (SessionKind::Practice, 7),
                (SessionKind::Play, 1)
            ]
        );
        // 練習は 1 周まで、プレイは別に数える
        let got: Vec<SessionKind> = r
            .recent_results("songs/a/master.dtx", 10, 1, None)
            .unwrap()
            .into_iter()
            .map(|l| l.kind)
            .collect();
        assert_eq!(got, vec![SessionKind::Practice, SessionKind::Play]);
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
        assert_eq!(
            r.recent_results("songs/a/master.dtx", 10, 10, None)
                .unwrap()
                .len(),
            1
        );
        drop(r);
        let _ = std::fs::remove_file(&path);
    }
    #[test]
    fn results_before_a_time() {
        let mut r = Records::open_in_memory().unwrap();
        r.save(&chart(1), &play(100, 1.0, vec![hit(1, Judgement::Miss)]))
            .unwrap();
        r.save(&chart(1), &play(200, 1.0, vec![hit(2, Judgement::Miss)]))
            .unwrap();
        let got: Vec<u32> = r
            .recent_results("songs/a/master.dtx", 10, 10, Some(200))
            .unwrap()
            .into_iter()
            .map(|l| l.results[0].measure)
            .collect();
        assert_eq!(got, vec![1]);
    }

    #[test]
    fn clears_and_overcoming() {
        let mut r = Records::open_in_memory().unwrap();
        let mut practice = play(100, 1.0, vec![hit(3, Judgement::Perfect)]);
        practice.kind = SessionKind::Practice;
        practice.clear = Some(SectionClear {
            first: 3,
            last: 4,
            speed: 1.0,
        });
        r.save(&chart(1), &practice).unwrap();
        practice.started_at = 200;
        r.save(&chart(1), &practice).unwrap();
        let c = r.clears("songs/a/master.dtx").unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].first, c[0].last, c[0].cleared_at), (3, 4, 100));
        assert_eq!(c[0].overcome, None);
        // 同じ区間のまだ克服していないクリアをまとめて書き換える
        assert_eq!(
            r.mark_overcome("songs/a/master.dtx", 3, 4, 300, Some(0.5), Some(0.1))
                .unwrap(),
            2
        );
        assert_eq!(
            r.mark_overcome("songs/a/master.dtx", 3, 4, 400, None, None)
                .unwrap(),
            0
        );
        let c = r.clears("songs/a/master.dtx").unwrap();
        assert_eq!(c[1].overcome, Some((300, Some(0.5), Some(0.1))));
        assert!(r.clears("other.dtx").unwrap().is_empty());
    }

    #[test]
    fn timed_hits_across_charts_at_normal_speed() {
        let mut r = Records::open_in_memory().unwrap();
        let other = ChartKey {
            path: "songs/b/master.dtx",
            ..chart(9)
        };
        r.save(&chart(1), &play(100, 1.0, vec![hit(1, Judgement::Good)]))
            .unwrap();
        r.save(&other, &play(200, 1.0, vec![hit(1, Judgement::Great)]))
            .unwrap();
        // 70% の周、見逃し、拍の位置の無い記録は入らない
        r.save(&chart(1), &play(300, 0.7, vec![hit(1, Judgement::Good)]))
            .unwrap();
        let mut old = hit(1, Judgement::Good);
        old.beat_pos = None;
        r.save(
            &chart(1),
            &play(400, 1.0, vec![old, hit(2, Judgement::Miss)]),
        )
        .unwrap();
        let hits = r.recent_timed_hits(10).unwrap();
        assert_eq!(
            hits,
            vec![
                TimedHit {
                    lane: DrumLane::SD,
                    beat_pos: 0.25,
                    offset_ms: -3.5
                };
                2
            ]
        );
        // 直近 3 回（300・400 と 200）だけなら 1 打
        assert_eq!(r.recent_timed_hits(3).unwrap().len(), 1);
    }

    fn scored(started_at: i64, score: i64, rate: f64, progress: &str, cleared: bool) -> Session {
        Session {
            score: Some(PlayScore {
                score,
                rate,
                progress: progress.to_owned(),
                cleared,
            }),
            ..play(started_at, 1.0, vec![hit(1, Judgement::Perfect)])
        }
    }

    #[test]
    fn best_score_takes_the_best_rate_of_cleared_plays() {
        let mut r = Records::open_in_memory().unwrap();
        let path = "songs/a/master.dtx";
        assert_eq!(
            r.best_score(path, None).unwrap(),
            BestScore {
                rate: None,
                score: None,
                progress: None
            }
        );
        let half = format!("{}{}", "1".repeat(32), "0".repeat(32));
        let most = format!("{}{}", "2".repeat(40), "0".repeat(24));
        let all1 = "1".repeat(64);
        let all2 = "2".repeat(64);
        // 途中で落ちたプレイだけなら、一番先までたどり着いたもの
        r.save(&chart(1), &scored(100, 10_000, 30.0, &half, false))
            .unwrap();
        r.save(&chart(1), &scored(200, 20_000, 40.0, &most, false))
            .unwrap();
        let b = r.best_score(path, None).unwrap();
        assert_eq!(b.rate, None);
        assert_eq!(b.score, Some(20_000));
        assert_eq!(b.progress.as_deref(), Some(most.as_str()));
        // 最後まで遊んだプレイがあれば、達成率が一番高いもの（同じなら先のもの）
        let s3 = r
            .save(&chart(1), &scored(300, 500_000, 70.5, &all1, true))
            .unwrap();
        r.save(&chart(1), &scored(400, 400_000, 60.0, &all2, true))
            .unwrap();
        let s5 = r
            .save(&chart(1), &scored(500, 600_000, 70.5, &all2, true))
            .unwrap();
        let b = r.best_score(path, None).unwrap();
        assert_eq!(b.rate, Some(70.5));
        assert_eq!(b.score, Some(600_000));
        assert_eq!(b.progress.as_deref(), Some(all1.as_str()));
        // 前のプレイだけから
        let b = r.best_score(path, Some(s5)).unwrap();
        assert_eq!(b.score, Some(500_000));
        let b = r.best_score(path, Some(s3)).unwrap();
        assert_eq!(b.rate, None);
        // スコアの無い練習やほかの譜面は数えない
        assert_eq!(r.best_score("other.dtx", None).unwrap().score, None);
    }

    #[test]
    fn progress_length_counts_reached_sections() {
        assert_eq!(PlayScore::reached(&"0".repeat(64)), 0);
        assert_eq!(
            PlayScore::reached(&format!("{}{}", "13".repeat(5), "0".repeat(54))),
            10
        );
        assert_eq!(PlayScore::reached("2222"), 0);
    }

    #[test]
    fn version_1_files_are_upgraded() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute_batch(
            "INSERT INTO charts VALUES (1, 'a.dtx', 'A', '', '00', 0);
             INSERT INTO sessions VALUES (1, 1, 'play', 0, '00', '{}');
             INSERT INTO laps VALUES (1, 1, 0, 1.0, 1, NULL, 0, 0, 0, 0, 0, 0, 0, NULL, NULL);
             INSERT INTO hits VALUES (1, 0, 0, 1, 'SD', 'PERFECT', 1.0, 100, NULL, NULL);",
        )
        .unwrap();
        let r = Records::init(conn).unwrap();
        assert_eq!(r.schema_version().unwrap(), MIGRATIONS.len());
        // 前の版の音符は拍の位置が無いので、くせには入らない
        assert!(r.recent_timed_hits(10).unwrap().is_empty());
        assert_eq!(r.recent_results("a.dtx", 10, 10, None).unwrap().len(), 1);
    }
}
