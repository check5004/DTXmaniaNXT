//! 叩く強さ（MIDI のベロシティ）でアクセントを見る。
//!
//! 強いかどうかは決まった数（100 以上など）では決めず、同じパッドのまわりの「アクセントでない音」と比べる
//! （2026-11-28 ユーザー「ベロシティは相対的に判別しよう」）。前後 [`WINDOW_BARS`] 小節にある
//! アクセントでない音の強さの中央値を、その人のそのときの「ふつうの強さ」とし、アクセントの音がそれより
//! はっきり強ければ（[`RATIO`] 倍以上、かつ [`MIN_GAP`] 以上の差）アクセントが出たとする。
//! 機器の感度や叩く位置、疲れて全体が弱くなっても、そのときのまわりと比べるので同じように見られる。
//!
//! 強さの届かない入力（キーボード）と、どの打も同じ強さで届く機器では見ない（[`has_strength`]）。

use dtx::DrumLane;

/// アクセントとみなす、ふつうの強さに対する倍率
pub const RATIO: f64 = 1.15;
/// アクセントとみなす、ふつうの強さとの差（ベロシティ）。弱い所では倍率だけだと差が小さすぎるので
pub const MIN_GAP: f64 = 6.0;
/// ふつうの強さを見る範囲（前後の小節の数）
pub const WINDOW_BARS: f64 = 1.0;
/// ふつうの強さを出すのに要る、まわりのアクセントでない音の数
pub const MIN_NEIGHBORS: usize = 2;
/// 強さを見るのに要る、強さの届いた打の数
pub const MIN_STROKES: usize = 8;

/// 判定した音符 1 つ（叩く強さを見る分）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    /// 譜面の位置（小節の番号に小節の中の位置 0〜1 を足したもの）
    pub at: f64,
    pub lane: DrumLane,
    /// 型でアクセントの音か
    pub accent: bool,
    /// 叩いた強さ。見逃し・自動・強さの届かない入力は None
    pub velocity: Option<u8>,
}

/// 強さを見られるか。強さの届いた打が [`MIN_STROKES`] 以上あり、みな同じ強さではない
pub fn has_strength(strokes: &[Stroke]) -> bool {
    let mut v = strokes.iter().filter_map(|s| s.velocity);
    let Some(first) = v.next() else {
        return false;
    };
    let mut n = 1;
    let mut varied = false;
    for x in v {
        n += 1;
        varied |= x != first;
    }
    n >= MIN_STROKES && varied
}

/// 中央値
pub fn median(mut v: Vec<f64>) -> Option<f64> {
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

/// 強さ `velocity` が、ふつうの強さ `normal` に比べてアクセントになっているか
pub fn stands_out(velocity: u8, normal: f64) -> bool {
    let v = f64::from(velocity);
    v >= normal * RATIO && v - normal >= MIN_GAP
}

/// `strokes[i]` のまわりのふつうの強さ（同じレーンで前後 [`WINDOW_BARS`] 小節の、アクセントでない音の中央値）。
/// `strokes` は位置の順。まわりの音が [`MIN_NEIGHBORS`] より少なければ None
pub fn normal_around(strokes: &[Stroke], i: usize) -> Option<f64> {
    let me = strokes.get(i)?;
    let from = strokes.partition_point(|s| s.at < me.at - WINDOW_BARS - 1e-9);
    let to = strokes.partition_point(|s| s.at <= me.at + WINDOW_BARS + 1e-9);
    let near: Vec<f64> = strokes[from..to]
        .iter()
        .filter(|s| s.lane == me.lane && !s.accent)
        .filter_map(|s| s.velocity.map(f64::from))
        .collect();
    if near.len() < MIN_NEIGHBORS {
        return None;
    }
    median(near)
}

/// 音符ごとに、アクセントが出たか（`strokes` は位置の順）。アクセントでない音・強さの無い音・
/// まわりと比べられない音は None
pub fn judge(strokes: &[Stroke]) -> Vec<Option<bool>> {
    strokes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let v = s.velocity.filter(|_| s.accent)?;
            normal_around(strokes, i).map(|n| stands_out(v, n))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at: f64, accent: bool, velocity: Option<u8>) -> Stroke {
        Stroke {
            at,
            lane: DrumLane::SD,
            accent,
            velocity,
        }
    }

    /// パラディドルを 1 小節（16 分）。アクセントの強さだけ変えて
    fn bar(accents: [u8; 4], normal: u8) -> Vec<Stroke> {
        (0..16)
            .map(|i| {
                let acc = i % 4 == 0;
                let v = if acc { accents[i / 4] } else { normal };
                s(1.0 + i as f64 / 16.0, acc, Some(v))
            })
            .collect()
    }

    #[test]
    fn an_accent_counts_when_it_is_clearly_above_the_normal_notes_around_it() {
        // ふつう 60: 1.15 倍の 69 以上、かつ +6 以上
        let st = bar([90, 69, 68, 60], 60);
        let j = judge(&st);
        assert_eq!(j[0], Some(true));
        assert_eq!(j[4], Some(true));
        assert_eq!(j[8], Some(false));
        assert_eq!(j[12], Some(false));
        // アクセントでない音は見ない
        assert!(
            j.iter()
                .enumerate()
                .all(|(i, x)| (i % 4 == 0) || x.is_none())
        );
    }

    #[test]
    fn the_same_gap_is_judged_against_how_hard_you_play_now() {
        // 弱く叩いていても（ふつう 30）、まわりよりはっきり強ければアクセント
        let soft = judge(&bar([42, 42, 42, 42], 30));
        assert!(soft[0] == Some(true));
        // 強く叩いていると（ふつう 100）、同じ +12 では足りない（1.15 倍の 115 が要る）
        let loud = judge(&bar([112, 112, 112, 112], 100));
        assert!(loud[0] == Some(false));
        // 弱い所でも差が 6 より小さければアクセントにしない（10 → 15 は 1.5 倍でも差 5）
        let tiny = judge(&bar([15, 15, 15, 15], 10));
        assert!(tiny[0] == Some(false));
    }

    #[test]
    fn the_normal_level_follows_the_neighbors_only() {
        // 前半はふつう 40、後半はふつう 80。アクセント 70 は前半では出ていて、後半では出ていない
        let mut st: Vec<Stroke> = Vec::new();
        for bar in 0..6 {
            let normal = if bar < 3 { 40 } else { 80 };
            for i in 0..8 {
                let acc = i % 4 == 0;
                st.push(s(
                    bar as f64 + i as f64 / 8.0,
                    acc,
                    Some(if acc { 70 } else { normal }),
                ));
            }
        }
        let j = judge(&st);
        assert_eq!(j[0], Some(true));
        assert_eq!(j[5 * 8], Some(false));
        // 違うレーンの音とは比べない
        let mut other = bar([90, 90, 90, 90], 60);
        for x in other.iter_mut().filter(|x| !x.accent) {
            x.lane = DrumLane::HH;
        }
        assert!(judge(&other).iter().all(Option::is_none));
    }

    #[test]
    fn missing_or_flat_strength_is_not_judged() {
        // まわりが見逃しばかりなら比べない
        let mut st = bar([90, 90, 90, 90], 60);
        for x in st.iter_mut().filter(|x| !x.accent) {
            x.velocity = None;
        }
        assert!(judge(&st).iter().all(Option::is_none));
        // キーボード（強さ無し）と、どれも同じ強さの機器は見ない
        assert!(!has_strength(&bar([100, 100, 100, 100], 100)));
        let keys: Vec<Stroke> = (0..16).map(|i| s(i as f64, i % 2 == 0, None)).collect();
        assert!(!has_strength(&keys));
        assert!(has_strength(&bar([90, 90, 90, 90], 60)));
        // 打が少なすぎれば見ない
        assert!(!has_strength(&bar([90, 90, 90, 90], 60)[..MIN_STROKES - 1]));
    }

    #[test]
    fn medians_of_odd_and_even_counts() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(vec![4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }
}
