//! 補正のおすすめ。記録のずれに、そのときの補正を足し戻した「生のずれ」から、
//! 入力の補正（全体）とこの曲の補正の値を出す。
//!
//! - 曲ごとに直近 [`SONG_SESSIONS`] 回のセッションの生のずれを、叩けた数で重みをつけて平均する
//! - 全体は、直近の [`GLOBAL_SONGS`] 曲の平均の真ん中の値（1 曲だけずれている曲に引っぱられない）
//! - この曲は「この曲の平均 − 全体」。全体を変えないなら今の全体の値から
//! - [`MIN_CHANGE_MS`] より小さい変化はすすめない。叩けた数が [`MIN_HITS`] に足りない曲は使わない

use std::collections::HashMap;

/// 曲ごとに使う直近のセッションの数
pub const SONG_SESSIONS: usize = 5;
/// 全体に使う直近の曲の数
pub const GLOBAL_SONGS: usize = 15;
/// 曲の平均を使うのに要る、叩けた音符の数
pub const MIN_HITS: u32 = 50;
/// これより小さい変化はすすめない（ミリ秒）
pub const MIN_CHANGE_MS: i64 = 3;

/// セッション 1 つの生のずれ
#[derive(Debug, Clone, PartialEq)]
pub struct LagSample {
    /// 曲を見分ける名前（同じ曲の難易度は同じ名前）
    pub song: String,
    /// 叩けた音符の数
    pub hits: u32,
    /// 生のずれの平均（ミリ秒、プラスは遅い）。補正を足し戻したもの
    pub raw_ms: f64,
}

/// 曲 1 つの生のずれ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SongLag {
    pub raw_ms: f64,
    /// 何回のセッションから出したか
    pub sessions: usize,
    pub hits: u32,
}

impl SongLag {
    /// おすすめに使えるだけ叩いた
    pub fn enough(&self) -> bool {
        self.hits >= MIN_HITS
    }
}

/// おすすめ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Advice {
    /// 全体の生のずれ（直近の曲の真ん中の値）と、何曲から出したか
    pub global: Option<(f64, usize)>,
    /// この曲の生のずれ（足りなくても入れる。使えるかは [`SongLag::enough`]）
    pub song: Option<SongLag>,
    /// すすめる入力の補正（全体）と、この曲の補正（ミリ秒）。すすめないなら今の値
    pub global_ms: i64,
    pub song_ms: i64,
    /// 今の値
    pub global_now: i64,
    pub song_now: i64,
}

impl Advice {
    /// すすめる値が今と違う
    pub fn changes(&self) -> bool {
        self.global_ms != self.global_now || self.song_ms != self.song_now
    }

    /// この曲を叩いたときに見える、今の補正でのずれの平均（ミリ秒）
    pub fn song_residual_ms(&self) -> Option<f64> {
        self.song
            .map(|s| s.raw_ms - (self.global_now + self.song_now) as f64)
    }

    /// 全体（ほかの曲）の、今の全体の補正でのずれ（ミリ秒）
    pub fn global_residual_ms(&self) -> Option<f64> {
        self.global.map(|(g, _)| g - self.global_now as f64)
    }
}

/// 曲ごとの生のずれ。`samples` は新しい順で、返すのは最後に遊んだのが新しい順
pub fn song_lags(samples: &[LagSample]) -> Vec<(String, SongLag)> {
    let mut order: Vec<String> = Vec::new();
    let mut sums: HashMap<&str, (f64, u32, usize)> = HashMap::new();
    for s in samples {
        if s.hits == 0 || !s.raw_ms.is_finite() {
            continue;
        }
        let e = sums.entry(s.song.as_str()).or_insert_with(|| {
            order.push(s.song.clone());
            (0.0, 0, 0)
        });
        if e.2 >= SONG_SESSIONS {
            continue;
        }
        e.0 += s.raw_ms * f64::from(s.hits);
        e.1 += s.hits;
        e.2 += 1;
    }
    order
        .into_iter()
        .map(|song| {
            let (sum, hits, sessions) = sums[song.as_str()];
            (
                song,
                SongLag {
                    raw_ms: sum / f64::from(hits),
                    sessions,
                    hits,
                },
            )
        })
        .collect()
}

/// 真ん中の値（偶数なら真ん中 2 つの平均）
fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// おすすめを出す。`samples` は新しい順、`song` はこの曲の名前、`global_now` と `song_now` は今の補正
pub fn advise(samples: &[LagSample], song: &str, global_now: i64, song_now: i64) -> Advice {
    let lags = song_lags(samples);
    let used: Vec<f64> = lags
        .iter()
        .filter(|(_, l)| l.enough())
        .take(GLOBAL_SONGS)
        .map(|(_, l)| l.raw_ms)
        .collect();
    let count = used.len();
    let global = median(used).map(|g| (g, count));
    let this = lags.iter().find(|(s, _)| s == song).map(|(_, l)| *l);
    let global_ms = match global {
        Some((g, _)) if (g.round() as i64 - global_now).abs() >= MIN_CHANGE_MS => g.round() as i64,
        _ => global_now,
    };
    let song_ms = match this {
        Some(l) if l.enough() => {
            let want = (l.raw_ms - global_ms as f64).round() as i64;
            if (want - song_now).abs() >= MIN_CHANGE_MS {
                want
            } else {
                song_now
            }
        }
        _ => song_now,
    };
    Advice {
        global,
        song: this,
        global_ms,
        song_ms,
        global_now,
        song_now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(song: &str, hits: u32, raw_ms: f64) -> LagSample {
        LagSample {
            song: song.into(),
            hits,
            raw_ms,
        }
    }

    #[test]
    fn songs_average_their_recent_sessions() {
        let samples = vec![
            s("a", 100, 10.0),
            s("b", 50, 2.0),
            s("a", 300, 14.0),
            s("a", 10, 100.0),
            s("a", 10, 100.0),
            s("a", 10, 100.0),
            // 6 回目より古いものは使わない
            s("a", 1000, -50.0),
        ];
        let lags = song_lags(&samples);
        assert_eq!(lags.len(), 2);
        assert_eq!(lags[0].0, "a");
        let a = lags[0].1;
        assert_eq!(a.sessions, 5);
        assert_eq!(a.hits, 430);
        assert!((a.raw_ms - (1000.0 + 4200.0 + 3000.0) / 430.0).abs() < 1e-9);
        assert_eq!(lags[1].1.raw_ms, 2.0);
    }

    #[test]
    fn one_late_song_gets_its_own_offset() {
        // 全体の補正 +3、ほかの曲はほぼ +3〜+5、この曲だけ +13（今の補正で +10 遅い）
        let mut samples = vec![s("this", 200, 13.0), s("this", 200, 13.4)];
        for (i, raw) in [4.0, 3.0, 5.0, 4.0, 2.0].into_iter().enumerate() {
            samples.push(s(&format!("other{i}"), 100, raw));
        }
        let a = advise(&samples, "this", 3, 0);
        assert_eq!(a.global.map(|g| g.1), Some(6));
        // 真ん中は 4（6 曲: 2, 3, 4, 4, 5, 13.2）。3 との差は 1 なので全体はそのまま
        assert_eq!(a.global_ms, 3);
        assert_eq!(a.song_ms, 10);
        assert!(a.changes());
        assert!((a.song_residual_ms().unwrap() - 10.2).abs() < 1e-9);
    }

    #[test]
    fn a_shared_lag_moves_the_global_offset() {
        // どの曲も 20 ms 遅い（機材の遅れ）。全体を上げ、この曲は 0 のまま
        let samples: Vec<_> = (0..4)
            .map(|i| s(&format!("s{i}"), 80, 20.0 + i as f64))
            .collect();
        let a = advise(&samples, "s0", 0, 0);
        assert_eq!(a.global_ms, 22);
        // この曲（20）は全体（22）との差が 2 なので、そのまま
        assert_eq!(a.song_ms, 0);
        // この曲に前に決めた補正が残っていれば、外すようにすすめる
        let a = advise(&samples, "s0", 0, 15);
        assert_eq!(a.global_ms, 22);
        assert_eq!(a.song_ms, -2);
    }

    #[test]
    fn nothing_changes_without_enough_hits() {
        let samples = vec![s("a", 30, 25.0), s("b", 49, -30.0)];
        let a = advise(&samples, "a", 0, 0);
        assert_eq!(a.global, None);
        assert!(!a.changes());
        // この曲のずれは見せる（使えるだけ無いことも分かる）
        assert!(!a.song.unwrap().enough());
        // 1 曲だけでも足りていれば、全体として出す（この曲のずれは全体の分）
        let a = advise(&[s("a", 60, 12.4)], "a", 0, 0);
        assert_eq!((a.global_ms, a.song_ms), (12, 0));
    }

    #[test]
    fn small_changes_are_not_advised() {
        let a = advise(&[s("a", 100, 2.4)], "a", 0, 0);
        assert!(!a.changes());
        let a = advise(&[s("a", 100, -3.0)], "a", 0, 0);
        assert_eq!(a.global_ms, -3);
    }
}
