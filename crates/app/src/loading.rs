//! 曲を読み込むあいだの画面（上流 `CStageSongLoading`）。
//! スキンの 6_ の画像で、ジャケット、曲名、アーティスト、パートと難易度のパネル、レベルを出す。
//! 上流と同じく、読み込みが終わっても「Now loading」の音が鳴り終わるまでは出しておく。
//!
//! 曲は別のスレッドで読む。画面のスレッドで読むと、読み終わるまで描画もキーも止まる
//! （Windows では、読み込み中に Esc で戻ると画面が読み込みのまま固まることがあった）。

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use egui::Color32;
use library::ChartHead;

use crate::game::Play;
use crate::gfx::{Canvas, Gfx, Glyphs, Opt, fit_scale, src};
use crate::play_view::{difficulty_cell, rotate_about_center};
use crate::text::Style;

/// 読み込みの画面
pub struct Loading {
    head: ChartHead,
    preimage: Option<PathBuf>,
    /// 6_Difficulty.png の切り出しの左上
    diff_panel: (f32, f32),
    started: Instant,
    /// 「Now loading」の音の長さ
    wait: Duration,
    /// 別のスレッドで読んでいる曲（結果を受け取ったら None）
    job: Option<Job<Play>>,
    /// 読み終えた曲
    pub play: Option<Box<Play>>,
}

impl Loading {
    /// `path` は譜面、`label` は難易度の名前（選曲画面で選んだもの）。`job` は曲を読んでいるスレッド
    pub fn new(path: &Path, label: &str, wait: Duration, job: Job<Play>) -> Self {
        let head = library::scan::read_head(path).unwrap_or_default();
        let preimage = path
            .parent()
            .zip(Some(head.preimage.trim()))
            .filter(|(_, name)| !name.is_empty())
            .and_then(|(dir, name)| library::scan::resolve(dir, name));
        let diff_panel = difficulty_cell("6", label).unwrap_or((0.0, 0.0));
        Self {
            head,
            preimage,
            diff_panel,
            started: Instant::now(),
            wait,
            job: Some(job),
            play: None,
        }
    }

    /// 読み終えていれば、その結果（1 回だけ返す）
    pub fn take_result(&mut self) -> Option<Result<Play>> {
        let r = self.job.as_ref()?.poll()?;
        self.job = None;
        Some(r)
    }

    /// 読み込みが終わり、音も鳴り終えた
    pub fn done(&self) -> bool {
        self.play.is_some() && self.started.elapsed() >= self.wait
    }

    pub fn draw(&self, c: &Canvas, g: &mut Gfx) {
        match g.skin.tex("Graphics/6_background.jpg") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::BLACK),
        }
        self.draw_jacket(c, g);
        let edge = |px: f32| Style {
            px,
            color: Color32::WHITE,
            bold: false,
            shadow: None,
            edge: Some((Color32::BLACK, px / 12.0)),
            gradient: None,
        };
        if let Some(t) = g.texts.get(&self.head.title, &edge(40.0)) {
            t.draw_fit(c, 190.0, 285.0, 625.0);
        }
        if let Some(t) = g.texts.get(&self.head.artist, &edge(30.0)) {
            t.draw_fit(c, 190.0, 360.0, 625.0);
        }
        // ドラムのパートのパネル（6_Part.png の 1 段目）と難易度のパネル
        if let Some(t) = g.skin.tex("Graphics/6_Part.png") {
            c.part(&t, 191.0, 52.0, src(0.0, 0.0, 262.0, 50.0));
        }
        if let Some(t) = g.skin.tex("Graphics/6_Difficulty.png") {
            let (x, y) = self.diff_panel;
            c.part(&t, 191.0, 102.0, src(x, y, 262.0, 50.0));
        }
        // レベル（上流の XG 表示：整数の部分、点、小数の 2 桁）
        if let Some(level) = self.head.level.filter(|&l| l > 0.0) {
            let t = g.skin.tex("Graphics/6_LevelNumber.png");
            let (whole, frac) = level_parts(level);
            if let Some(t) = &t {
                c.part(t, 282.0, 243.0, src(1000.0, 92.0, 30.0, 38.0));
            }
            BIG.draw(c, t.as_ref(), 187.0, 152.0, &whole.to_string());
            BIG.draw(c, t.as_ref(), 307.0, 152.0, &format!("{frac:02}"));
        }
    }

    /// ジャケット（上流は 384 に縮めて (206, 66) へ動かし、画面の中心のまわりに 0.28 ラジアン回す）
    fn draw_jacket(&self, c: &Canvas, g: &mut Gfx) {
        let default = "Graphics/5_preimage default.png";
        let jacket = match &self.preimage {
            Some(p) => g
                .images
                .get(p)
                .or_else(|| g.images.failed(p).then(|| g.skin.tex(default)).flatten()),
            None => g.skin.tex(default),
        };
        let Some(t) = jacket else {
            return;
        };
        let f = fit_scale(&t, 384.0);
        let (cx, cy) = rotate_about_center(206.0, 66.0, 0.28);
        c.draw_rot(
            &t,
            cx,
            cy,
            0.28,
            Opt {
                sx: f,
                sy: f,
                ..Default::default()
            },
        );
    }
}

/// 別のスレッドでしている仕事 1 つ。結果を受け取らずに捨てたら、結果はそのスレッドで捨てる
/// （読み込み中に Esc で戻ったとき、読み終えた曲は画面のスレッドを止めずに捨てられる）
pub struct Job<T>(Receiver<Result<T>>);

impl<T: Send + 'static> Job<T> {
    /// `work` を別のスレッドで始める
    pub fn spawn(name: &str, work: impl FnOnce() -> Result<T> + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                // 受け取る側がもう無ければ、結果はここで捨てる
                let _ = tx.send(work());
            });
        if let Err(e) = spawned {
            // スレッドを作れなかった。`tx` も捨てられたので、`poll` はエラーを返す
            log::error!("cannot start {name}: {e}");
        }
        Self(rx)
    }

    /// 終わっていれば結果。まだなら None（待たない）
    pub fn poll(&self) -> Option<Result<T>> {
        match self.0.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            // 結果を送らずにスレッドが終わった（パニックか、スレッドを作れなかった）
            Err(TryRecvError::Disconnected) => {
                Some(Err(anyhow!("曲を読むスレッドが途中で止まりました")))
            }
        }
    }
}

/// レベル 8.25 を (8, 25) に分ける
fn level_parts(level: f64) -> (u32, u32) {
    let hundredths = (level * 100.0).round().max(0.0) as u32;
    (hundredths / 100, hundredths % 100)
}

/// 6_LevelNumber.png の大きい数字（上流 `st大文字位置`：100×130 が横に並び、送りは 90）
static BIG: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(
        (0..10u8)
            .map(|d| {
                let x = f32::from(d) * 100.0;
                (char::from(b'0' + d), src(x, 0.0, 100.0, 130.0), 90.0)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    ),
    missing: 90.0,
});

#[cfg(test)]
mod tests {
    use super::*;

    /// `poll` が結果を返すまで待つ（テスト用）
    fn wait<T: Send + 'static>(job: &Job<T>) -> Result<T> {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(r) = job.poll() {
                return r;
            }
            assert!(Instant::now() < until, "the job did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn job_runs_on_another_thread_without_blocking() {
        let (go, gate) = std::sync::mpsc::channel::<()>();
        let job = Job::spawn("test job", move || {
            gate.recv()?;
            Ok(42)
        });
        // 仕事が終わるまでは待たずに None
        assert!(job.poll().is_none());
        go.send(()).unwrap();
        assert_eq!(wait(&job).unwrap(), 42);
    }

    #[test]
    fn job_reports_errors_and_panics() {
        let failed = Job::<u32>::spawn("failing job", || Err(anyhow!("no such file")));
        assert_eq!(wait(&failed).unwrap_err().to_string(), "no such file");
        let panicked = Job::<u32>::spawn("panicking job", || panic!("boom"));
        assert!(wait(&panicked).is_err());
    }

    #[test]
    fn dropping_a_job_does_not_wait_for_it() {
        // Esc で戻ったときと同じく、終わっていない仕事を捨てても待たない
        let (go, gate) = std::sync::mpsc::channel::<()>();
        let (done_tx, done) = std::sync::mpsc::channel::<()>();
        let job = Job::spawn("dropped job", move || {
            gate.recv()?;
            done_tx.send(())?;
            Ok(1)
        });
        drop(job);
        // 仕事のほうは続き、終われば結果を捨てる
        go.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(10)).unwrap();
    }

    #[test]
    fn level_is_split_like_the_xg_display() {
        assert_eq!(level_parts(8.25), (8, 25));
        assert_eq!(level_parts(6.5), (6, 50));
        assert_eq!(level_parts(9.99), (9, 99));
        // 浮動小数の誤差で 7.0 が 6.99 にならない
        assert_eq!(level_parts(6.999_999_9), (7, 0));
    }
}
