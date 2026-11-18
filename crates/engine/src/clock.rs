//! 時計。判定は「曲の時刻」で行うので、次の 2 つをそろえる。
//!
//! - 音: 音声出力が「このバッファの最初のサンプルを鳴らすホスト時刻」と「それまでに鳴らしたサンプル数」を
//!   知らせてくる。これを [`AudioClock`] に渡すと、任意のホスト時刻の曲の時刻がわかる（再生位置が時計）。
//! - MIDI: 受信のたびに、デバイスの時刻（あれば）とアプリに届いたホスト時刻がわかる。
//!   [`MidiTimestampMapper`] はデバイスの時刻をホスト時刻に直し、OS の配送の遅れのばらつきを取り除く。
//!
//! ホスト時刻はどちらもマイクロ秒の単調時計（`std::time::Instant` を起点からの経過に直したもの）を使う。

use std::collections::VecDeque;

/// 音の再生位置から曲の時刻を求める
#[derive(Debug, Clone)]
pub struct AudioClock {
    sample_rate: f64,
    /// 0 サンプル目が曲の何ミリ秒にあたるか（前奏の空白を入れるならマイナス）
    start_ms: f64,
    /// 再生の速さ（練習モードの速度変更。1.0 が等速）
    speed: f64,
    anchor: Option<Anchor>,
}

#[derive(Debug, Clone, Copy)]
struct Anchor {
    host_us: i64,
    song_ms: f64,
}

impl AudioClock {
    pub fn new(sample_rate: u32, start_ms: f64) -> Self {
        Self {
            sample_rate: f64::from(sample_rate),
            start_ms,
            speed: 1.0,
            anchor: None,
        }
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.max(0.01);
    }

    /// 音声出力のバッファごとに呼ぶ。`play_host_us` はそのバッファの最初のサンプルが鳴るホスト時刻、
    /// `frames_before` はそれまでに出したフレーム数
    pub fn on_buffer(&mut self, play_host_us: i64, frames_before: u64) {
        let song_ms = self.start_ms + frames_before as f64 / self.sample_rate * 1000.0 * self.speed;
        self.anchor = Some(Anchor {
            host_us: play_host_us,
            song_ms,
        });
    }

    /// ホスト時刻 `host_us` の曲の時刻。まだ音が出ていなければ `None`
    pub fn song_ms_at(&self, host_us: i64) -> Option<f64> {
        let a = self.anchor?;
        Some(a.song_ms + (host_us - a.host_us) as f64 / 1000.0 * self.speed)
    }
}

/// MIDI のデバイス時刻をホスト時刻に直す。
///
/// `届いた時刻 − デバイスの時刻` は「本当の時差 + 配送の遅れ」で、配送の遅れは 0 以上でばらつく。
/// そこで直近の窓の中の最小値を本当の時差とみなす。時計のずれ（ドリフト）は窓を区切ることで追う。
#[derive(Debug, Clone)]
pub struct MidiTimestampMapper {
    window_us: i64,
    /// (デバイス時刻, 時差) の単調な最小値キュー
    mins: VecDeque<(i64, i64)>,
    last_device_us: Option<i64>,
}

impl Default for MidiTimestampMapper {
    fn default() -> Self {
        Self::new(10_000_000)
    }
}

impl MidiTimestampMapper {
    /// `window_us`: 最小値を探す窓の長さ（デバイス時刻のマイクロ秒）
    pub fn new(window_us: i64) -> Self {
        Self {
            window_us,
            mins: VecDeque::new(),
            last_device_us: None,
        }
    }

    /// 1 つ受信するごとに呼び、叩いた瞬間のホスト時刻の見積もりを返す。
    /// デバイスが時刻を持たない（`None`）なら、届いた時刻をそのまま使う
    pub fn map(&mut self, device_us: Option<i64>, arrival_host_us: i64) -> i64 {
        let Some(dev) = device_us else {
            return arrival_host_us;
        };
        // デバイスの時計が戻った（ポートを開き直したなど）ら、覚えたことを捨てる
        if self.last_device_us.is_some_and(|last| dev < last) {
            self.mins.clear();
        }
        self.last_device_us = Some(dev);

        let offset = arrival_host_us - dev;
        while self.mins.back().is_some_and(|&(_, o)| o >= offset) {
            self.mins.pop_back();
        }
        self.mins.push_back((dev, offset));
        while self
            .mins
            .front()
            .is_some_and(|&(d, _)| d < dev - self.window_us)
        {
            self.mins.pop_front();
        }
        let min_offset = self.mins.front().map_or(offset, |&(_, o)| o);
        dev + min_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_clock_follows_the_play_position() {
        let mut c = AudioClock::new(48_000, -1000.0);
        assert_eq!(c.song_ms_at(0), None);
        // 0 フレーム目がホスト時刻 5 秒に鳴る
        c.on_buffer(5_000_000, 0);
        assert_eq!(c.song_ms_at(5_000_000), Some(-1000.0));
        assert_eq!(c.song_ms_at(6_500_000), Some(500.0));
        // 48000 フレーム（1 秒）後のバッファ。ホストの時計が 2ms 進んでいても、音の位置が正
        c.on_buffer(6_002_000, 48_000);
        assert_eq!(c.song_ms_at(6_002_000), Some(0.0));
    }

    #[test]
    fn audio_clock_speed() {
        let mut c = AudioClock::new(48_000, 0.0);
        c.set_speed(0.5);
        c.on_buffer(0, 48_000);
        // 半分の速さでは 1 秒ぶんのサンプルで曲は 0.5 秒進む
        assert_eq!(c.song_ms_at(0), Some(500.0));
        assert_eq!(c.song_ms_at(1_000_000), Some(1000.0));
    }

    #[test]
    fn midi_mapper_removes_delivery_jitter() {
        let mut m = MidiTimestampMapper::default();
        // 本当の時差は 1_000_000。配送の遅れが 3ms, 0.5ms, 2ms とばらつく
        assert_eq!(m.map(Some(100), 1_000_100 + 3000), 1_000_100 + 3000);
        assert_eq!(m.map(Some(200_000), 1_200_000 + 500), 1_200_000 + 500);
        // 最小の時差（0.5ms 遅れ）を覚えているので、遅れて届いても叩いた時刻は 0.5ms 遅れにそろう
        assert_eq!(m.map(Some(400_000), 1_400_000 + 2000), 1_400_000 + 500);
    }

    #[test]
    fn midi_mapper_without_device_time() {
        let mut m = MidiTimestampMapper::default();
        assert_eq!(m.map(None, 123), 123);
    }

    #[test]
    fn midi_mapper_forgets_old_minimums_and_resets() {
        let mut m = MidiTimestampMapper::new(1_000_000);
        m.map(Some(0), 1_000_000); // 時差 1_000_000
        // 2 秒後には古い最小値は窓の外。時差 1_000_300 が新しい基準
        assert_eq!(m.map(Some(2_000_000), 3_000_300), 3_000_300);
        // デバイスの時計が戻ったら、覚えたことを捨てる
        assert_eq!(m.map(Some(10), 5_000_000), 5_000_000);
    }
}
