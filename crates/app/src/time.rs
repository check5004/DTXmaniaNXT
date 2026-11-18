//! アプリ全体で使うホスト時刻（起動からのマイクロ秒、単調）。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

static EPOCH: OnceLock<Instant> = OnceLock::new();

fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

/// 今のホスト時刻
pub fn host_us() -> i64 {
    instant_to_us(Instant::now())
}

pub fn instant_to_us(t: Instant) -> i64 {
    let e = epoch();
    if t >= e {
        t.duration_since(e).as_micros() as i64
    } else {
        -(e.duration_since(t).as_micros() as i64)
    }
}

pub fn duration_us(d: Duration) -> i64 {
    d.as_micros() as i64
}
