//! 記録の分析。判定の結果を数えて、どの小節が苦手かを出す（AI は使わない）。
//!
//! 苦手度は、音符ごとの判定を点（PERFECT 0、GREAT 0.2、GOOD 0.5、POOR と MISS 1）にし、
//! 小節の中で新しい記録ほど重く平均したもの。0 なら全部 PERFECT、1 なら全部 MISS か POOR。

use crate::judge::Judgement;

/// 新しい記録ほど重くする割合。1 回前の記録は 0.8 倍、2 回前は 0.64 倍で数える
pub const RECENCY: f64 = 0.8;

/// 苦手度をこれ以上の小節を「苦手」とみなす
pub const WEAK_THRESHOLD: f64 = 0.25;

/// 苦手フレーズ 1 つの長さの上限（小節）
pub const MAX_PHRASE_MEASURES: usize = 4;

/// 苦手度に使う記録の数（直近のプレイ）
pub const HISTORY_PLAYS: usize = 10;

/// 判定 1 つの点。苦手なほど大きい
pub fn weakness_points(j: Judgement) -> f64 {
    match j {
        Judgement::Perfect => 0.0,
        Judgement::Great => 0.2,
        Judgement::Good => 0.5,
        Judgement::Poor | Judgement::Miss => 1.0,
    }
}

/// 1 回のプレイの中の、音符 1 つの結果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteResult {
    pub measure: u32,
    pub judgement: Judgement,
}

/// 小節ごとの苦手度（0〜1）。`plays` は新しい順に並べる。
/// 記録の無い小節（音符が無い、まだそこまで遊んでいない）は `None`
pub fn measure_weakness(plays: &[Vec<NoteResult>], measures: usize) -> Vec<Option<f64>> {
    let mut sum = vec![0.0; measures];
    let mut weight = vec![0.0; measures];
    let mut w = 1.0;
    for play in plays {
        for r in play {
            let m = r.measure as usize;
            if m < measures {
                sum[m] += w * weakness_points(r.judgement);
                weight[m] += w;
            }
        }
        w *= RECENCY;
    }
    sum.iter()
        .zip(&weight)
        .map(|(&s, &w)| (w > 0.0).then(|| s / w))
        .collect()
}

/// 苦手なフレーズ（続いた苦手な小節のまとまり）
#[derive(Debug, Clone, PartialEq)]
pub struct WeakPhrase {
    /// 最初と最後の小節（両端を含む）
    pub first: u32,
    pub last: u32,
    /// 音符の数で重みをつけた苦手度の平均
    pub weakness: f64,
    pub notes: usize,
}

impl WeakPhrase {
    /// 並べる順の基準。苦手で音符の多いフレーズほど大きい
    pub fn score(&self) -> f64 {
        self.weakness * self.notes as f64
    }
}

/// 苦手度が `WEAK_THRESHOLD` 以上の小節をつなげてフレーズにし、苦手で音符の多い順に `top` 個まで返す。
/// 記録の無い小節で途切れ、長いものは `MAX_PHRASE_MEASURES` 小節ずつに切る。
/// `notes_per_measure[m]` は小節 m の音符の数
pub fn weak_phrases(
    weakness: &[Option<f64>],
    notes_per_measure: &[usize],
    top: usize,
) -> Vec<WeakPhrase> {
    let notes = |m: usize| notes_per_measure.get(m).copied().unwrap_or(0);
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for m in 0..=weakness.len() {
        let weak = weakness
            .get(m)
            .copied()
            .flatten()
            .is_some_and(|w| w >= WEAK_THRESHOLD);
        match (weak, start) {
            (true, None) => start = Some(m),
            (false, Some(s)) => {
                runs.push((s, m));
                start = None;
            }
            _ => {}
        }
    }
    let mut phrases: Vec<WeakPhrase> = runs
        .into_iter()
        .flat_map(|(s, e)| {
            (s..e)
                .step_by(MAX_PHRASE_MEASURES)
                .map(move |a| (a, (a + MAX_PHRASE_MEASURES).min(e)))
        })
        .map(|(a, b)| {
            let n: usize = (a..b).map(notes).sum();
            let total: f64 = (a..b)
                .map(|m| weakness[m].unwrap_or(0.0) * notes(m) as f64)
                .sum();
            let weakness = if n > 0 {
                total / n as f64
            } else {
                (a..b).filter_map(|m| weakness[m]).sum::<f64>() / (b - a) as f64
            };
            WeakPhrase {
                first: a as u32,
                last: (b - 1) as u32,
                weakness,
                notes: n,
            }
        })
        .collect();
    phrases.sort_by(|x, y| y.score().total_cmp(&x.score()).then(x.first.cmp(&y.first)));
    phrases.truncate(top);
    phrases
}

#[cfg(test)]
mod tests {
    use super::*;
    use Judgement::*;

    fn r(measure: u32, judgement: Judgement) -> NoteResult {
        NoteResult { measure, judgement }
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn weakness_of_one_play() {
        let play = vec![
            r(0, Perfect),
            r(0, Perfect),
            r(1, Miss),
            r(1, Perfect),
            r(3, Great),
            r(3, Good),
        ];
        let w = measure_weakness(&[play], 4);
        assert!(close(w[0], 0.0));
        // (1 + 0) / 2
        assert!(close(w[1], 0.5));
        // 音符の無い小節
        assert_eq!(w[2], None);
        // (0.2 + 0.5) / 2
        assert!(close(w[3], 0.35));
    }

    #[test]
    fn newer_plays_weigh_more() {
        // 新しいプレイは PERFECT、1 回前は MISS: (0 × 1 + 1 × 0.8) / (1 + 0.8)
        let w = measure_weakness(&[vec![r(0, Perfect)], vec![r(0, Miss)]], 1);
        assert!(close(w[0], 0.8 / 1.8));
        // 逆なら (1 × 1 + 0 × 0.8) / 1.8
        let w = measure_weakness(&[vec![r(0, Miss)], vec![r(0, Perfect)]], 1);
        assert!(close(w[0], 1.0 / 1.8));
    }

    #[test]
    fn measures_beyond_the_chart_are_ignored() {
        let w = measure_weakness(&[vec![r(5, Miss)]], 2);
        assert_eq!(w, vec![None, None]);
    }

    #[test]
    fn phrases_join_split_and_rank() {
        let weakness = [
            Some(0.1),
            Some(0.5),
            Some(0.6),
            None,
            Some(0.3),
            Some(0.3),
            Some(0.3),
            Some(0.3),
            Some(0.3),
            Some(0.9),
        ];
        let notes = [4, 4, 4, 0, 2, 2, 2, 2, 2, 1];
        let p = weak_phrases(&weakness, &notes, 10);
        let got: Vec<(u32, u32, usize)> = p.iter().map(|p| (p.first, p.last, p.notes)).collect();
        // 1〜2: 苦手度 (0.5×4 + 0.6×4)/8 = 0.55、点 4.4
        // 4〜7: 5 小節続くので 4 小節で切る。0.3 × 8 = 2.4
        // 8〜9: 0.3×2 + 0.9×1 = 1.5 を 3 音符で割って 0.5、点 1.5
        assert_eq!(got, vec![(1, 2, 8), (4, 7, 8), (8, 9, 3)]);
        assert!((p[0].weakness - 0.55).abs() < 1e-9);
        assert!((p[2].weakness - 0.5).abs() < 1e-9);
        // 上位だけ
        assert_eq!(weak_phrases(&weakness, &notes, 1).len(), 1);
    }

    #[test]
    fn no_phrases_without_weak_measures() {
        assert!(weak_phrases(&[Some(0.1), None, Some(0.24)], &[1, 0, 1], 5).is_empty());
    }
}
