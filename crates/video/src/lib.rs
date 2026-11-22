//! 譜面の動画（`#AVI`）を読む。
//!
//! 読むのは動画ごとの別のスレッドで、演奏画面は毎フレーム「今の動画の時刻」を渡して、できた絵を受け取るだけ。
//! 絵は出す時刻の少し前に作っておき、時刻が来たら差し替える。出す時刻を過ぎてしまった絵は作らずに飛ばす。
//!
//! Windows は OS の Media Foundation で読む（MP4 の H.264、AVI の Xvid・DivX など）。
//! ほかの OS はまだ読めない（開いても絵が出ないだけ）。

use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use anyhow::Result;

#[cfg(windows)]
mod mf;

/// 読んだ 1 枚（RGBA、上の行から）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// この OS で動画を読めるか（読めない OS では [`Player`] が絵を出さない）
pub const SUPPORTED: bool = cfg!(windows);

/// 動画を読む。落とすと読むスレッドも止まる
pub struct Player<T> {
    shared: Arc<Shared<T>>,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    wake: Condvar,
}

struct State<T> {
    /// 見せたい動画の時刻（マイクロ秒）。None なら読まずに待つ
    target_us: Option<i64>,
    stop: bool,
    /// いちばん新しい絵と、その番号（変わったら貼り直す）
    shown: Option<(u64, Arc<T>)>,
}

impl<T: Send + Sync + 'static> Player<T> {
    /// `path` を読み始める。絵は `max`（幅, 高さ）に収まるように縮め、読むスレッドで `make` を通してから渡す
    pub fn open(path: &Path, max: (u32, u32), make: impl Fn(Frame) -> T + Send + 'static) -> Self {
        let path = path.to_owned();
        Self::spawn(
            move || open_source(&path, max).map_err(|e| e.context(path.display().to_string())),
            make,
        )
    }

    fn spawn(
        open: impl FnOnce() -> Result<Box<dyn Source>> + Send + 'static,
        make: impl Fn(Frame) -> T + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                target_us: None,
                stop: false,
                shown: None,
            }),
            wake: Condvar::new(),
        });
        let s = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("video".into())
            .spawn(move || {
                let done = open().and_then(|src| Worker::new(src).run(&s, &make));
                if let Err(e) = done {
                    log::warn!("動画を読めません: {e:#}");
                }
            });
        if let Err(e) = spawned {
            log::warn!("動画を読むスレッドを作れません: {e}");
        }
        Self { shared }
    }

    /// 見せたい動画の時刻（ミリ秒、動画の頭から）。None なら読むのを止めて待つ（出した絵はそのまま）
    pub fn show_at(&self, ms: Option<f64>) {
        let us = ms.map(|ms| (ms.max(0.0) * 1000.0) as i64);
        let mut st = lock(&self.shared.state);
        if st.target_us != us {
            st.target_us = us;
            self.shared.wake.notify_one();
        }
    }

    /// 今出す絵と、その番号（新しい絵になるたびに増える）
    pub fn frame(&self) -> Option<(u64, Arc<T>)> {
        lock(&self.shared.state).shown.clone()
    }
}

impl<T> Drop for Player<T> {
    fn drop(&mut self) {
        lock(&self.shared.state).stop = true;
        self.shared.wake.notify_one();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 読む元（Windows では Media Foundation）
trait Source {
    /// 次の絵へ進む。終わりなら None（そのときも、最後に読んだ絵は `pixels` で取れる）
    fn next(&mut self) -> Result<Option<Timing>>;
    /// 最後に読んだ絵
    fn pixels(&mut self) -> Result<Frame>;
    /// `us` の少し前（キーフレーム）から読み直す
    fn seek(&mut self, us: i64) -> Result<()>;
}

/// 絵を出す時刻と長さ（マイクロ秒）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Timing {
    start_us: i64,
    dur_us: i64,
}

#[cfg(windows)]
fn open_source(path: &Path, max: (u32, u32)) -> Result<Box<dyn Source>> {
    Ok(Box::new(mf::MfSource::open(path, max)?))
}

#[cfg(not(windows))]
fn open_source(_path: &Path, _max: (u32, u32)) -> Result<Box<dyn Source>> {
    anyhow::bail!("この OS ではまだ動画を読めません（今は Windows だけ）")
}

/// 後ろへ戻ったとき、ずれとして見逃す幅。これより戻ったら読み直す
const BACK_SLACK_US: i64 = 100_000;
/// これより先へ飛んだら、順に読まずに読み直す
const JUMP_US: i64 = 2_000_000;

/// 読むスレッドの中身
struct Worker<T> {
    src: Box<dyn Source>,
    /// 読んだがまだ出していない絵の時刻と、作った絵（出す時刻が先の絵は、先に作っておく）
    held: Option<(Timing, Option<Arc<T>>)>,
    /// 最後に出した絵の時刻（読み直したら None）
    shown_us: Option<i64>,
    /// 最後に読み直した時刻（読み始めは 0）
    base_us: i64,
    eof: bool,
    serial: u64,
}

impl<T> Worker<T> {
    fn new(src: Box<dyn Source>) -> Self {
        Self {
            src,
            held: None,
            shown_us: None,
            base_us: 0,
            eof: false,
            serial: 0,
        }
    }

    /// 見せたい時刻が変わるたびに、その時刻の絵を出す。止めるまで返らない
    fn run(mut self, shared: &Shared<T>, make: &dyn Fn(Frame) -> T) -> Result<()> {
        let mut done = None;
        loop {
            let target = {
                let mut st = lock(&shared.state);
                loop {
                    if st.stop {
                        return Ok(());
                    }
                    match st.target_us {
                        Some(t) if Some(t) != done => break t,
                        _ => st = shared.wake.wait(st).unwrap_or_else(|e| e.into_inner()),
                    }
                }
            };
            done = Some(target);
            self.step(target, make, &mut |serial, img| {
                lock(&shared.state).shown = Some((serial, img));
            })?;
        }
    }

    /// 時刻 `t` に出す絵を `publish` に渡し、次の絵を作っておく
    fn step(
        &mut self,
        t: i64,
        make: &dyn Fn(Frame) -> T,
        publish: &mut dyn FnMut(u64, Arc<T>),
    ) -> Result<()> {
        if self.needs_seek(t) {
            self.src.seek(t)?;
            self.held = None;
            self.shown_us = None;
            self.base_us = t;
            self.eof = false;
        }
        loop {
            let (timing, ready) = match self.held.take() {
                Some(h) => h,
                None if self.eof => return Ok(()),
                None => match self.src.next()? {
                    Some(timing) => (timing, None),
                    None => {
                        self.eof = true;
                        return Ok(());
                    }
                },
            };
            if timing.start_us > t {
                // まだ先の絵。今のうちに作っておく
                let img = match ready {
                    Some(img) => img,
                    None => Arc::new(make(self.src.pixels()?)),
                };
                self.held = Some((timing, Some(img)));
                return Ok(());
            }
            if timing.start_us + timing.dur_us <= t {
                // 出す時刻を過ぎた絵は作らずに次へ。ただし最後の絵なら出す
                match self.src.next()? {
                    Some(next) => {
                        self.held = Some((next, None));
                        continue;
                    }
                    None => self.eof = true,
                }
            }
            let img = match ready {
                Some(img) => img,
                None => Arc::new(make(self.src.pixels()?)),
            };
            self.serial += 1;
            publish(self.serial, img);
            self.shown_us = Some(timing.start_us);
        }
    }

    /// 順に読むより読み直したほうがよいか（後ろへ戻った、遠くへ飛んだ）
    fn needs_seek(&self, t: i64) -> bool {
        let held = self.held.as_ref().map(|h| h.0.start_us);
        let at = self.shown_us.or(held).unwrap_or(self.base_us);
        if t < at - BACK_SLACK_US {
            return true;
        }
        let ahead = held.map_or(at, |h| h.max(at));
        !self.eof && t > ahead + JUMP_US
    }
}

/// 絵の中の範囲（ピクセル）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
struct Area {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Area {
    fn full(w: u32, h: u32) -> Self {
        Self { x: 0, y: 0, w, h }
    }

    /// Media Foundation の `MFVideoArea`（16 バイト）を読む。絵からはみ出すなら絵全体
    fn from_video_area(blob: &[u8; 16], width: u32, height: u32) -> Self {
        let i16_at = |i: usize| i16::from_le_bytes([blob[i], blob[i + 1]]);
        let i32_at =
            |i: usize| i32::from_le_bytes([blob[i], blob[i + 1], blob[i + 2], blob[i + 3]]);
        // MFOffset は { fract: u16, value: i16 }。端数は捨てる
        let (x, y) = (i16_at(2), i16_at(6));
        let (w, h) = (i32_at(8), i32_at(12));
        let fits =
            |o: i16, len: i32, all: u32| o >= 0 && len > 0 && o as i64 + len as i64 <= all as i64;
        if fits(x, w, width) && fits(y, h, height) {
            Self {
                x: x as u32,
                y: y as u32,
                w: w as u32,
                h: h as u32,
            }
        } else {
            Self::full(width, height)
        }
    }
}

/// `max` に収まる大きさ（縦横比はそのまま、偶数に丸める）。収まっていればそのまま
#[cfg_attr(not(windows), allow(dead_code))]
fn fit_size(w: u32, h: u32, max: (u32, u32)) -> (u32, u32) {
    if w <= max.0 && h <= max.1 || w == 0 || h == 0 {
        return (w, h);
    }
    let s = (max.0 as f64 / w as f64).min(max.1 as f64 / h as f64);
    let even = |v: u32| ((v as f64 * s / 2.0).round() as u32 * 2).max(2);
    (even(w), even(h))
}

/// BGRX の画素（1 行 `pitch` バイト。負なら下の行から並ぶ）の `area` を RGBA の絵にする。
/// `data` は並びのいちばん低い番地から。`height` は並びの行の数。足りなければ None
#[cfg_attr(not(windows), allow(dead_code))]
fn bgrx_to_frame(data: &[u8], height: u32, pitch: i64, area: Area) -> Option<Frame> {
    if area.y + area.h > height || area.w == 0 || area.h == 0 {
        return None;
    }
    let step = pitch.unsigned_abs() as usize;
    let (x0, x1) = (area.x as usize * 4, (area.x + area.w) as usize * 4);
    let mut rgba = Vec::with_capacity(area.w as usize * area.h as usize * 4);
    for y in area.y..area.y + area.h {
        let row = if pitch >= 0 { y } else { height - 1 - y } as usize * step;
        let line = data.get(row + x0..row + x1)?;
        for p in line.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    Some(Frame {
        width: area.w,
        height: area.h,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// 100 ミリ秒ごとの絵が `count` 枚、500 ミリ秒ごとにキーフレームの動画のふり。絵の中身は何枚目か
    struct Fake {
        count: i64,
        next: i64,
        current: Option<i64>,
        made: Arc<AtomicUsize>,
        seeks: Arc<Mutex<Vec<i64>>>,
    }

    const STEP: i64 = 100_000;

    impl Source for Fake {
        fn next(&mut self) -> Result<Option<Timing>> {
            if self.next >= self.count {
                return Ok(None);
            }
            self.current = Some(self.next);
            self.next += 1;
            Ok(Some(Timing {
                start_us: (self.next - 1) * STEP,
                dur_us: STEP,
            }))
        }

        fn pixels(&mut self) -> Result<Frame> {
            self.made.fetch_add(1, Ordering::SeqCst);
            let i = self.current.expect("読んでいない");
            Ok(Frame {
                width: 1,
                height: 1,
                rgba: vec![i as u8, 0, 0, 255],
            })
        }

        fn seek(&mut self, us: i64) -> Result<()> {
            self.seeks.lock().unwrap().push(us);
            self.next = (us / (5 * STEP) * 5).clamp(0, self.count);
            self.current = None;
            Ok(())
        }
    }

    struct Probe {
        worker: Worker<u8>,
        made: Arc<AtomicUsize>,
        seeks: Arc<Mutex<Vec<i64>>>,
        shown: Vec<u8>,
    }

    impl Probe {
        fn new(count: i64) -> Self {
            let made = Arc::new(AtomicUsize::new(0));
            let seeks = Arc::new(Mutex::new(Vec::new()));
            let fake = Fake {
                count,
                next: 0,
                current: None,
                made: Arc::clone(&made),
                seeks: Arc::clone(&seeks),
            };
            Self {
                worker: Worker::new(Box::new(fake)),
                made,
                seeks,
                shown: Vec::new(),
            }
        }

        fn at(&mut self, ms: i64) -> &mut Self {
            let shown = &mut self.shown;
            self.worker
                .step(ms * 1000, &|f: Frame| f.rgba[0], &mut |_, img| {
                    shown.push(*img)
                })
                .unwrap();
            self
        }

        fn made(&self) -> usize {
            self.made.load(Ordering::SeqCst)
        }

        fn seeks(&self) -> Vec<i64> {
            self.seeks.lock().unwrap().clone()
        }
    }

    #[test]
    fn shows_frames_in_order_and_makes_the_next_one_early() {
        let mut p = Probe::new(10);
        p.at(0).at(50).at(100).at(250);
        assert_eq!(p.shown, [0, 1, 2]);
        // 出した 3 枚と、先に作った 4 枚目
        assert_eq!(p.made(), 4);
        assert!(p.seeks().is_empty());
    }

    #[test]
    fn skips_late_frames_without_making_them() {
        let mut p = Probe::new(10);
        p.at(0).at(450);
        assert_eq!(p.shown, [0, 4]);
        // 0、先に作った 1、4、先に作った 5（2 と 3 は作らない）
        assert_eq!(p.made(), 4);
        assert!(p.seeks().is_empty());
    }

    #[test]
    fn seeks_when_jumping_far_or_going_back() {
        let mut p = Probe::new(100);
        p.at(0).at(5_050);
        assert_eq!(p.seeks(), [5_050_000]);
        assert_eq!(p.shown, [0, 50]);
        // 少しのずれでは読み直さない
        p.at(5_000);
        assert_eq!(p.seeks().len(), 1);
        p.at(1_230);
        assert_eq!(p.seeks(), [5_050_000, 1_230_000]);
        assert_eq!(p.shown, [0, 50, 12]);
        // 途中から始める（練習の区間など）
        let mut p = Probe::new(100);
        p.at(7_000);
        assert_eq!(p.seeks(), [7_000_000]);
        assert_eq!(p.shown, [70]);
    }

    #[test]
    fn keeps_the_last_frame_after_the_end() {
        let mut p = Probe::new(10);
        p.at(0).at(1_500);
        // 9 枚目は時刻を過ぎていても、最後の絵なので出す
        assert_eq!(p.shown, [0, 9]);
        p.at(1_800).at(1_400);
        assert_eq!(p.shown, [0, 9]);
        assert!(p.seeks().is_empty());
        // 戻れば読み直す
        p.at(300);
        assert_eq!(p.seeks(), [300_000]);
        assert_eq!(p.shown, [0, 9, 3]);
    }

    #[test]
    fn player_hands_frames_over_from_its_thread() {
        let made = Arc::new(AtomicUsize::new(0));
        let seeks = Arc::new(Mutex::new(Vec::new()));
        let fake = Fake {
            count: 10,
            next: 0,
            current: None,
            made,
            seeks,
        };
        let player = Player::spawn(move || Ok(Box::new(fake)), |f: Frame| f.rgba[0]);
        assert!(player.frame().is_none());
        let wait_for = |want: u8| {
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some((_, img)) = player.frame()
                    && *img == want
                {
                    break;
                }
                assert!(Instant::now() < until, "{want} 枚目が来ない");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        player.show_at(Some(0.0));
        wait_for(0);
        player.show_at(Some(320.0));
        wait_for(3);
        let (serial, _) = player.frame().unwrap();
        // 止めても出した絵は残る
        player.show_at(None);
        assert_eq!(player.frame().unwrap().0, serial);
    }

    #[test]
    fn opening_a_missing_file_shows_nothing() {
        let player = Player::open(Path::new("no/such/movie.mp4"), (1280, 720), |f| f);
        player.show_at(Some(0.0));
        std::thread::sleep(Duration::from_millis(50));
        assert!(player.frame().is_none());
    }

    #[test]
    fn converts_bgrx_rows_top_down_and_bottom_up() {
        // 3×2、1 行 16 バイト（12 バイトの絵 + 4 バイトの詰め物）
        let px = |b: u8| [b, b + 1, b + 2, 0];
        let mut top_down = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                top_down.extend_from_slice(&px(y * 30 + x * 10));
            }
            top_down.extend_from_slice(&[9; 4]);
        }
        let area = Area {
            x: 1,
            y: 0,
            w: 2,
            h: 2,
        };
        let f = bgrx_to_frame(&top_down, 2, 16, area).unwrap();
        assert_eq!((f.width, f.height), (2, 2));
        assert_eq!(&f.rgba[..4], &[12, 11, 10, 255]);
        assert_eq!(&f.rgba[12..], &[52, 51, 50, 255]);
        // 下の行から並ぶ（pitch が負）なら、並びの後ろの行が上
        let f = bgrx_to_frame(&top_down, 2, -16, area).unwrap();
        assert_eq!(&f.rgba[..4], &[42, 41, 40, 255]);
        assert_eq!(&f.rgba[12..], &[22, 21, 20, 255]);
        // 足りなければ作らない
        assert!(bgrx_to_frame(&top_down[..20], 2, 16, area).is_none());
        assert!(
            bgrx_to_frame(
                &top_down,
                2,
                16,
                Area {
                    x: 0,
                    y: 1,
                    w: 3,
                    h: 2
                }
            )
            .is_none()
        );
    }

    #[test]
    fn reads_the_display_area() {
        let mut blob = [0u8; 16];
        blob[2..4].copy_from_slice(&0i16.to_le_bytes());
        blob[6..8].copy_from_slice(&4i16.to_le_bytes());
        blob[8..12].copy_from_slice(&1920i32.to_le_bytes());
        blob[12..16].copy_from_slice(&1080i32.to_le_bytes());
        assert_eq!(
            Area::from_video_area(&blob, 1920, 1088),
            Area {
                x: 0,
                y: 4,
                w: 1920,
                h: 1080
            }
        );
        // はみ出すなら全体
        assert_eq!(
            Area::from_video_area(&blob, 1920, 1080),
            Area::full(1920, 1080)
        );
    }

    #[test]
    fn fits_big_movies_into_720p() {
        assert_eq!(fit_size(1280, 720, (1280, 720)), (1280, 720));
        assert_eq!(fit_size(640, 480, (1280, 720)), (640, 480));
        assert_eq!(fit_size(1920, 1080, (1280, 720)), (1280, 720));
        assert_eq!(fit_size(1440, 1080, (1280, 720)), (960, 720));
        assert_eq!(fit_size(2560, 1080, (1280, 720)), (1280, 540));
    }
}
