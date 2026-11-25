//! 音源の音を録ったものを調べる（測る道具が使う）。
//!
//! - [`FrameClock`] は録った音のフレームとホスト時刻（µs）の対応。録音のコールバックごとの時刻から作る
//! - [`peak_db`]・[`rms_db`] は音の大きさ、[`onset`] は鳴り始め
//! - [`Shape`] は音の色（帯ごとの強さ）。2 つの音が同じ音色かを [`Shape::distance`] で比べる

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// これより小さい音は無音として扱う（dBFS）
pub const SILENCE_DB: f32 = -120.0;

/// 録った音のフレームとホスト時刻の対応（`host_us = offset_us + frame * 1e6 / rate`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameClock {
    /// 1 秒あたりのフレーム（音の機器の時計で、名目の値から少しずれることがある）
    pub rate: f64,
    /// 0 フレーム目を録ったホスト時刻
    pub offset_us: f64,
}

impl FrameClock {
    /// コールバックごとの（そのバッファの頭のフレーム, 頭を録ったホスト時刻の見積もり）から作る。
    ///
    /// 見積もりの時刻はコールバックが遅れた分だけ後ろにずれる（前にはずれない）ので、
    /// 傾きは最小二乗で求め、切片は下の方（10 パーセンタイル）に合わせる。
    /// 傾きが名目から 0.5% より外れたら名目の値を使う
    pub fn fit(points: &[(u64, i64)], nominal_rate: u32) -> Option<Self> {
        if points.is_empty() || nominal_rate == 0 {
            return None;
        }
        let nominal = f64::from(nominal_rate);
        let mut us_per_frame = 1e6 / nominal;
        if points.len() >= 10 {
            let n = points.len() as f64;
            let mx = points.iter().map(|p| p.0 as f64).sum::<f64>() / n;
            let my = points.iter().map(|p| p.1 as f64).sum::<f64>() / n;
            let mut sxy = 0.0;
            let mut sxx = 0.0;
            for &(x, y) in points {
                let dx = x as f64 - mx;
                sxy += dx * (y as f64 - my);
                sxx += dx * dx;
            }
            if sxx > 0.0 {
                let slope = sxy / sxx;
                if (slope * nominal / 1e6 - 1.0).abs() < 0.005 {
                    us_per_frame = slope;
                }
            }
        }
        let mut offsets: Vec<f64> = points
            .iter()
            .map(|&(x, y)| y as f64 - x as f64 * us_per_frame)
            .collect();
        offsets.sort_by(f64::total_cmp);
        let offset_us = offsets[(offsets.len() - 1) / 10];
        Some(Self {
            rate: 1e6 / us_per_frame,
            offset_us,
        })
    }

    /// ホスト時刻にあたるフレーム（範囲の外もそのまま返す）
    pub fn frame_at(&self, host_us: f64) -> f64 {
        (host_us - self.offset_us) * self.rate / 1e6
    }

    /// フレームにあたるホスト時刻
    pub fn host_at(&self, frame: f64) -> f64 {
        self.offset_us + frame * 1e6 / self.rate
    }
}

/// 振幅を dBFS にする
pub fn to_db(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        SILENCE_DB
    } else {
        (20.0 * amplitude.log10()).max(SILENCE_DB)
    }
}

/// 一番大きい振幅（dBFS）
pub fn peak_db(samples: &[f32]) -> f32 {
    to_db(samples.iter().fold(0.0f32, |m, &x| m.max(x.abs())))
}

/// 実効値（dBFS）
pub fn rms_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return SILENCE_DB;
    }
    let sum: f64 = samples.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
    to_db((sum / samples.len() as f64).sqrt() as f32)
}

/// となりのサンプルとの差の実効値（dBFS）。高い音ほど大きく出るので、切り替えの「プツッ」を見つけるのに使う
pub fn click_db(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return SILENCE_DB;
    }
    let sum: f64 = samples
        .windows(2)
        .map(|w| {
            let d = f64::from(w[1] - w[0]);
            d * d
        })
        .sum();
    to_db((sum / (samples.len() - 1) as f64).sqrt() as f32)
}

/// 振幅が初めて `threshold_db` を超えた所
pub fn onset(samples: &[f32], threshold_db: f32) -> Option<usize> {
    let threshold = 10f32.powf(threshold_db / 20.0);
    samples.iter().position(|&x| x.abs() >= threshold)
}

/// 中央値（空なら `None`）
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// 帯の数（50 Hz〜16 kHz を対数で等しく分ける）
pub const BANDS: usize = 24;
const LOW_HZ: f32 = 50.0;
const HIGH_HZ: f32 = 16_000.0;

/// 音の色。帯ごとの強さ（dB）から全体の平均を引いたもの（音の大きさに左右されない）
#[derive(Debug, Clone, PartialEq)]
pub struct Shape(pub [f32; BANDS]);

impl Shape {
    /// 録った音（叩いた所から数十ミリ秒）の色を求める。短すぎるときは `None`
    pub fn of(samples: &[f32], rate: u32) -> Option<Self> {
        if samples.len() < 256 || rate == 0 {
            return None;
        }
        let n = samples.len().next_power_of_two().min(16_384);
        let len = samples.len().min(n);
        let mut buf: Vec<Complex<f32>> = (0..n)
            .map(|i| {
                if i < len {
                    // ハン窓
                    let w = 0.5
                        - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (len - 1) as f32).cos();
                    Complex::new(samples[i] * w, 0.0)
                } else {
                    Complex::new(0.0, 0.0)
                }
            })
            .collect();
        FftPlanner::<f32>::new()
            .plan_fft_forward(n)
            .process(&mut buf);
        let hz_per_bin = rate as f32 / n as f32;
        let mut energy = [0f64; BANDS];
        let ratio = (HIGH_HZ / LOW_HZ).ln();
        for (k, c) in buf.iter().enumerate().take(n / 2).skip(1) {
            let hz = k as f32 * hz_per_bin;
            if !(LOW_HZ..HIGH_HZ).contains(&hz) {
                continue;
            }
            let band = (((hz / LOW_HZ).ln() / ratio) * BANDS as f32) as usize;
            energy[band.min(BANDS - 1)] += f64::from(c.norm_sqr());
        }
        let mut db = [0f32; BANDS];
        for (d, e) in db.iter_mut().zip(energy) {
            *d = (10.0 * (e + 1e-12).log10()) as f32;
        }
        let mean = db.iter().sum::<f32>() / BANDS as f32;
        for d in &mut db {
            *d -= mean;
        }
        Some(Self(db))
    }

    /// 2 つの色の違い（帯ごとの差の平均、dB）。同じ音色を叩き直したものはおおむね 2〜3 dB より小さい
    pub fn distance(&self, other: &Shape) -> f32 {
        self.0
            .iter()
            .zip(other.0.iter())
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / BANDS as f32
    }

    /// いくつかの色の平均（空なら `None`）
    pub fn mean(shapes: &[Shape]) -> Option<Shape> {
        if shapes.is_empty() {
            return None;
        }
        let mut out = [0f32; BANDS];
        for s in shapes {
            for (o, v) in out.iter_mut().zip(s.0.iter()) {
                *o += v;
            }
        }
        for o in &mut out {
            *o /= shapes.len() as f32;
        }
        Some(Shape(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// 叩いた音のような、指数で消えていく正弦波
    fn ping(hz: f32, len: usize, amp: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                amp * (-t * 20.0).exp() * (2.0 * std::f32::consts::PI * hz * t).sin()
            })
            .collect()
    }

    #[test]
    fn clock_follows_the_lower_edge_of_the_callback_times() {
        // 480 フレームずつ、10 ms ごと。コールバックはときどき 0〜3 ms 遅れる
        let points: Vec<(u64, i64)> = (0..200u64)
            .map(|i| {
                let late = [0, 0, 1000, 0, 3000, 0, 0, 2000, 0, 0][(i % 10) as usize];
                (i * 480, 5_000_000 + (i * 10_000) as i64 + late)
            })
            .collect();
        let c = FrameClock::fit(&points, RATE).unwrap();
        assert!((c.rate - 48_000.0).abs() < 50.0, "{c:?}");
        assert!((c.offset_us - 5_000_000.0).abs() < 1500.0, "{c:?}");
        assert!((c.frame_at(5_010_000.0) - 480.0).abs() < 80.0);
        assert!((c.host_at(c.frame_at(6_000_000.0)) - 6_000_000.0).abs() < 1e-3);
        assert_eq!(FrameClock::fit(&[], RATE), None);
        // 点が少ないときは名目の速さ
        let c = FrameClock::fit(&[(0, 100), (480, 10_100)], RATE).unwrap();
        assert_eq!(c.rate, 48_000.0);
        assert_eq!(c.offset_us, 100.0);
    }

    #[test]
    fn levels_and_onset() {
        let mut s = vec![0.0f32; 1000];
        s.extend(ping(200.0, 4800, 0.5));
        assert!((peak_db(&s) - to_db(0.5)).abs() < 0.5);
        assert!(rms_db(&s) < peak_db(&s));
        assert_eq!(rms_db(&[]), SILENCE_DB);
        assert_eq!(peak_db(&[0.0; 10]), SILENCE_DB);
        let at = onset(&s, -40.0).unwrap();
        assert!((1000..1010).contains(&at), "{at}");
        assert_eq!(onset(&[0.0; 100], -40.0), None);
        // 高い音ほど「プツッ」の値が大きい
        assert!(click_db(&ping(6000.0, 4800, 0.5)) > click_db(&ping(100.0, 4800, 0.5)) + 20.0);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn shapes_tell_different_sounds_apart() {
        let low = Shape::of(&ping(120.0, 4096, 0.5), RATE).unwrap();
        let low_quiet = Shape::of(&ping(120.0, 4096, 0.05), RATE).unwrap();
        let high = Shape::of(&ping(900.0, 4096, 0.5), RATE).unwrap();
        // 大きさが違っても同じ音色なら近い
        assert!(
            low.distance(&low_quiet) < 0.5,
            "{}",
            low.distance(&low_quiet)
        );
        assert!(low.distance(&high) > 3.0, "{}", low.distance(&high));
        let mean = Shape::mean(&[low.clone(), low_quiet]).unwrap();
        assert!(mean.distance(&low) < 0.5);
        assert_eq!(Shape::mean(&[]), None);
        assert_eq!(Shape::of(&[0.0; 10], RATE), None);
    }
}
