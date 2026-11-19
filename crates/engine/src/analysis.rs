//! 記録の分析。判定の結果を数えて、どの小節が苦手かを出す（AI は使わない）。
//!
//! 苦手度は、音符ごとの判定を点（PERFECT 0、GREAT 0.2、GOOD 0.5、POOR と MISS 1）にし、
//! 小節の中で新しい記録ほど重く平均したもの。0 なら全部 PERFECT、1 なら全部 MISS か POOR。
//!
//! ほかに、練習でクリアした区間を普通のプレイでも叩けたか（克服）と、
//! レーンと「拍の中の位置」ごとのずれのくせ（傾向）を出す。

use dtx::DrumLane;

use crate::judge::Judgement;

/// 新しい記録ほど重くする割合。1 回前の記録は 0.8 倍、2 回前は 0.64 倍で数える
pub const RECENCY: f64 = 0.8;

/// 苦手度をこれ以上の小節を「苦手」とみなす
pub const WEAK_THRESHOLD: f64 = 0.25;

/// 苦手フレーズ 1 つの長さの上限（小節）
pub const MAX_PHRASE_MEASURES: usize = 4;

/// 苦手度に使う記録の数。小節ごとに、その小節を叩いた直近の周をこの数まで使う
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

/// 小節ごとの苦手度（0〜1）。`plays` は新しい順に並べた周（普通のプレイ 1 回、または練習の 1 周）。
///
/// 新しさは小節ごとに数える。練習の周は区間の小節しか叩かないので、ほかの小節の記録を古くしない。
/// 小節ごとに、その小節を叩いた直近 [`HISTORY_PLAYS`] 周までを使う。
/// 記録の無い小節（音符が無い、まだそこまで遊んでいない）は `None`
pub fn measure_weakness(plays: &[Vec<NoteResult>], measures: usize) -> Vec<Option<f64>> {
    let mut sum = vec![0.0; measures];
    let mut weight = vec![0.0; measures];
    // 小節ごとの「その小節を叩いた、より新しい周の数」
    let mut age = vec![0usize; measures];
    let mut covered = vec![false; measures];
    for play in plays {
        covered.fill(false);
        for r in play {
            let m = r.measure as usize;
            if m < measures && age[m] < HISTORY_PLAYS {
                let w = RECENCY.powi(age[m] as i32);
                sum[m] += w * weakness_points(r.judgement);
                weight[m] += w;
                covered[m] = true;
            }
        }
        for (a, &c) in age.iter_mut().zip(&covered) {
            if c {
                *a += 1;
            }
        }
    }
    sum.iter()
        .zip(&weight)
        .map(|(&s, &w)| (w > 0.0).then(|| s / w))
        .collect()
}

/// 平均と標準偏差（標本、n − 1 で割る）。空なら両方 `None`、1 つなら標準偏差だけ `None`
pub fn mean_and_sd(xs: &[f64]) -> (Option<f64>, Option<f64>) {
    let n = xs.len();
    let mean = (n > 0).then(|| xs.iter().sum::<f64>() / n as f64);
    let sd = mean
        .filter(|_| n > 1)
        .map(|m| (xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt());
    (mean, sd)
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

/// 区間（小節 `first`〜`last`）の苦手度。音符の数で重みをつけた小節の苦手度の平均。記録が無ければ `None`
pub fn section_weakness(
    weakness: &[Option<f64>],
    notes_per_measure: &[usize],
    first: u32,
    last: u32,
) -> Option<f64> {
    let (mut sum, mut notes) = (0.0, 0usize);
    for m in first as usize..=last as usize {
        if let (Some(Some(w)), Some(&n)) = (weakness.get(m), notes_per_measure.get(m)) {
            sum += w * n as f64;
            notes += n;
        }
    }
    (notes > 0).then(|| sum / notes as f64)
}

/// 1 回のプレイの、区間（小節 `first`〜`last`）の点の平均。区間を叩いていなければ `None`
pub fn play_section_score(results: &[NoteResult], first: u32, last: u32) -> Option<f64> {
    let points: Vec<f64> = results
        .iter()
        .filter(|r| (first..=last).contains(&r.measure))
        .map(|r| weakness_points(r.judgement))
        .collect();
    (!points.is_empty()).then(|| points.iter().sum::<f64>() / points.len() as f64)
}

/// 練習でクリアした区間を「克服」したか。
/// `before` は練習する前の区間の苦手度、`now` は今回のプレイを入れた苦手度、`this_play` は今回のプレイだけの点。
/// 今回のプレイで苦手でなく（[`WEAK_THRESHOLD`] 未満）、苦手度が練習の前より下がっていれば克服
pub fn overcome(before: Option<f64>, now: Option<f64>, this_play: Option<f64>) -> bool {
    let Some(this_play) = this_play else {
        return false;
    };
    let lower = match (before, now) {
        (Some(b), Some(n)) => n < b,
        // 練習の前に普通のプレイの記録が無ければ、今回のプレイだけで決める
        (None, _) => true,
        (Some(_), None) => false,
    };
    this_play < WEAK_THRESHOLD && lower
}

/// 拍の中の位置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Subdivision {
    /// 拍の頭（表拍）
    OnBeat,
    /// 8 分の裏
    Eighth,
    /// 16 分の裏（拍の 1/4 と 3/4）
    Sixteenth,
    /// 3 連符（拍の 1/3 と 2/3）
    Triplet,
    /// それより細かい位置
    Other,
}

impl Subdivision {
    pub const ALL: [Subdivision; 5] = [
        Self::OnBeat,
        Self::Eighth,
        Self::Sixteenth,
        Self::Triplet,
        Self::Other,
    ];

    /// 拍の中の位置 `beat_pos`（0 以上 1 未満、拍の頭が 0）から
    pub fn of(beat_pos: f64) -> Self {
        const EPS: f64 = 1e-3;
        let at = |x: f64| (beat_pos - x).abs() < EPS;
        if at(0.0) || at(1.0) {
            Self::OnBeat
        } else if at(0.5) {
            Self::Eighth
        } else if at(0.25) || at(0.75) {
            Self::Sixteenth
        } else if at(1.0 / 3.0) || at(2.0 / 3.0) {
            Self::Triplet
        } else {
            Self::Other
        }
    }
}

/// 叩けた 1 打（傾向の材料）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedHit {
    pub lane: DrumLane,
    /// 拍の中の位置（0 以上 1 未満）
    pub beat_pos: f64,
    /// ずれ（ミリ秒、マイナスは早い）
    pub offset_ms: f64,
}

/// レーンと拍の中の位置ごとのずれ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tendency {
    pub lane: DrumLane,
    pub sub: Subdivision,
    pub hits: usize,
    pub mean_ms: f64,
    pub sd_ms: f64,
}

/// くせとして挙げるずれの平均の大きさ（ミリ秒）
pub const NOTABLE_MEAN_MS: f64 = 8.0;

/// レーンと拍の中の位置ごとに、ずれの平均と標準偏差を出す。`min_hits` 打に満たない組は出さない。
/// 並びはレーンの順、同じレーンの中は拍の中の位置の順
pub fn tendencies(hits: &[TimedHit], min_hits: usize) -> Vec<Tendency> {
    let mut groups: std::collections::BTreeMap<(usize, Subdivision), Vec<f64>> =
        std::collections::BTreeMap::new();
    for h in hits {
        let lane = DrumLane::ALL.iter().position(|&l| l == h.lane).unwrap_or(0);
        groups
            .entry((lane, Subdivision::of(h.beat_pos)))
            .or_default()
            .push(h.offset_ms);
    }
    groups
        .into_iter()
        .filter(|(_, xs)| xs.len() >= min_hits.max(2))
        .filter_map(|((lane, sub), xs)| {
            let (mean, sd) = mean_and_sd(&xs);
            Some(Tendency {
                lane: DrumLane::ALL[lane],
                sub,
                hits: xs.len(),
                mean_ms: mean?,
                sd_ms: sd?,
            })
        })
        .collect()
}

/// 目立つくせ。ずれの平均が [`NOTABLE_MEAN_MS`] 以上の組を、ずれの大きい順に `top` 個まで
pub fn notable_tendencies(t: &[Tendency], top: usize) -> Vec<Tendency> {
    let mut v: Vec<Tendency> = t
        .iter()
        .copied()
        .filter(|t| t.mean_ms.abs() >= NOTABLE_MEAN_MS)
        .collect();
    v.sort_by(|a, b| b.mean_ms.abs().total_cmp(&a.mean_ms.abs()));
    v.truncate(top);
    v
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
    fn recency_is_counted_per_measure() {
        // 最新は小節 1 だけの練習の周。小節 0 にとっては 1 回前のプレイが最新なので重み 1
        let plays = vec![
            vec![r(1, Perfect)],
            vec![r(0, Miss), r(1, Miss)],
            vec![r(0, Perfect), r(1, Perfect)],
        ];
        let w = measure_weakness(&plays, 2);
        assert!(close(w[0], 1.0 / 1.8));
        // 小節 1 は 3 周とも叩いている: (0 + 0.8 + 0) / (1 + 0.8 + 0.64)
        assert!(close(w[1], 0.8 / 2.44));
    }

    #[test]
    fn only_the_latest_plays_of_each_measure_count() {
        // 小節 0 を PERFECT で 10 周叩いたら、それより古い MISS は数えない
        let mut plays = vec![vec![r(0, Perfect)]; HISTORY_PLAYS];
        plays.push(vec![r(0, Miss), r(1, Miss)]);
        let w = measure_weakness(&plays, 2);
        assert!(close(w[0], 0.0));
        // 小節 1 にとっては唯一の記録
        assert!(close(w[1], 1.0));
    }

    #[test]
    fn mean_and_sd_of_offsets() {
        assert_eq!(mean_and_sd(&[]), (None, None));
        assert_eq!(mean_and_sd(&[4.0]), (Some(4.0), None));
        let (m, sd) = mean_and_sd(&[1.0, 3.0]);
        assert_eq!(m, Some(2.0));
        assert!((sd.unwrap() - 2f64.sqrt()).abs() < 1e-12);
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
    #[test]
    fn section_weakness_weighs_by_notes() {
        let w = vec![Some(0.5), Some(0.0), None, Some(1.0)];
        let n = vec![2, 6, 0, 2];
        // (0.5 × 2 + 0 × 6) / 8
        assert!(close(section_weakness(&w, &n, 0, 1), 0.125));
        // 記録の無い小節は入れない: (0 × 6 + 1 × 2) / 8
        assert!(close(section_weakness(&w, &n, 1, 3), 0.25));
        assert_eq!(section_weakness(&w, &n, 2, 2), None);
        assert_eq!(section_weakness(&w, &n, 5, 6), None);
    }

    #[test]
    fn play_section_scores() {
        let play = vec![r(0, Miss), r(1, Perfect), r(1, Good), r(2, Miss)];
        assert!(close(play_section_score(&play, 1, 1), 0.25));
        assert!(close(play_section_score(&play, 0, 1), 0.5));
        assert_eq!(play_section_score(&play, 3, 4), None);
    }

    #[test]
    fn overcoming_needs_a_good_play_and_lower_weakness() {
        assert!(overcome(Some(0.5), Some(0.4), Some(0.1)));
        // 今回のプレイが苦手なまま
        assert!(!overcome(Some(0.5), Some(0.4), Some(0.3)));
        // 苦手度が下がっていない
        assert!(!overcome(Some(0.2), Some(0.2), Some(0.0)));
        // 区間を叩いていない
        assert!(!overcome(Some(0.5), Some(0.4), None));
        // 練習の前に記録が無ければ今回のプレイだけで
        assert!(overcome(None, Some(0.1), Some(0.1)));
    }

    #[test]
    fn subdivisions_of_beat_positions() {
        assert_eq!(Subdivision::of(0.0), Subdivision::OnBeat);
        assert_eq!(Subdivision::of(0.9999), Subdivision::OnBeat);
        assert_eq!(Subdivision::of(0.5), Subdivision::Eighth);
        assert_eq!(Subdivision::of(0.25), Subdivision::Sixteenth);
        assert_eq!(Subdivision::of(0.75), Subdivision::Sixteenth);
        assert_eq!(Subdivision::of(1.0 / 3.0), Subdivision::Triplet);
        assert_eq!(Subdivision::of(2.0 / 3.0), Subdivision::Triplet);
        assert_eq!(Subdivision::of(0.125), Subdivision::Other);
    }

    fn th(lane: DrumLane, beat_pos: f64, offset_ms: f64) -> TimedHit {
        TimedHit {
            lane,
            beat_pos,
            offset_ms,
        }
    }

    #[test]
    fn tendencies_per_lane_and_position() {
        let hits = vec![
            th(DrumLane::BD, 0.25, 20.0),
            th(DrumLane::BD, 0.75, 16.0),
            th(DrumLane::BD, 0.25, 18.0),
            th(DrumLane::BD, 0.0, -2.0),
            th(DrumLane::BD, 0.0, 2.0),
            th(DrumLane::SD, 0.5, -10.0),
            th(DrumLane::SD, 0.5, -10.0),
            // 1 打だけの組は出さない
            th(DrumLane::HH, 0.0, 30.0),
        ];
        let t = tendencies(&hits, 2);
        let got: Vec<(DrumLane, Subdivision, usize)> =
            t.iter().map(|t| (t.lane, t.sub, t.hits)).collect();
        // レーンの順（DrumLane::ALL: HH、SD、BD…）
        let want = vec![
            (DrumLane::SD, Subdivision::Eighth, 2),
            (DrumLane::BD, Subdivision::OnBeat, 2),
            (DrumLane::BD, Subdivision::Sixteenth, 3),
        ];
        assert_eq!(got, want);
        let bd16 = t
            .iter()
            .find(|t| t.lane == DrumLane::BD && t.sub == Subdivision::Sixteenth)
            .unwrap();
        // 平均 18、標準偏差 2
        assert!((bd16.mean_ms - 18.0).abs() < 1e-9 && (bd16.sd_ms - 2.0).abs() < 1e-9);
        // 目立つのは BD の 16 分裏（+18）と SD の 8 分裏（−10）。表拍の BD（0）は入らない
        let n = notable_tendencies(&t, 5);
        assert_eq!(
            n.iter().map(|t| (t.lane, t.sub)).collect::<Vec<_>>(),
            vec![
                (DrumLane::BD, Subdivision::Sixteenth),
                (DrumLane::SD, Subdivision::Eighth)
            ]
        );
        assert_eq!(notable_tendencies(&t, 1).len(), 1);
    }
}
