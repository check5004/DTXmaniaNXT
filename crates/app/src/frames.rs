//! フレームの時間を測る。F9 で画面の右上に出し、出しているあいだは 1 秒ごとにログにも書く。
//!
//! 選曲画面のスクロールなどがコマ落ちしていないかを、Windows の実機でも確かめられるようにするためのもの。
//! 「間隔」はフレームとフレームの間の時間（画面の滑らかさ）、「画面」はアプリが画面の中身を作るのに
//! 使った時間（文字を画像にするなど）、「GPU へ」はそこから GPU に渡し終えるまで（画面の空きを待つ時間は除く）。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, pos2, vec2};

/// グラフに出すフレームの数
const KEEP: usize = 240;
/// ログに書く間隔
const LOG_EVERY: Duration = Duration::from_secs(1);
/// 間隔がふだん（中央値）のこの倍を超えたフレームを「落ちた」と数える
const HITCH: f32 = 1.5;

/// 1 フレームの時間（ミリ秒）
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    interval: f32,
    /// 画面の中身を作る
    build: f32,
    /// 作り始めから GPU に渡し終えるまで（画面の空きを待つ時間は除く）
    work: f32,
}

/// まとめ（直近の何フレームか）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub fps: f32,
    pub interval_avg: f32,
    pub interval_max: f32,
    pub build_avg: f32,
    pub build_max: f32,
    pub work_avg: f32,
    pub work_max: f32,
    /// 間隔がふだんの [`HITCH`] 倍を超えたフレームの数
    pub hitches: usize,
}

fn summarize(frames: &[Frame]) -> Option<Summary> {
    if frames.is_empty() {
        return None;
    }
    let n = frames.len() as f32;
    let total: f32 = frames.iter().map(|f| f.interval).sum();
    let mut sorted: Vec<f32> = frames.iter().map(|f| f.interval).collect();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    Some(Summary {
        frames: frames.len(),
        fps: if total > 0.0 { n * 1000.0 / total } else { 0.0 },
        interval_avg: total / n,
        interval_max: sorted[sorted.len() - 1],
        build_avg: frames.iter().map(|f| f.build).sum::<f32>() / n,
        build_max: frames.iter().map(|f| f.build).fold(0.0, f32::max),
        work_avg: frames.iter().map(|f| f.work).sum::<f32>() / n,
        work_max: frames.iter().map(|f| f.work).fold(0.0, f32::max),
        hitches: frames
            .iter()
            .filter(|f| f.interval > median * HITCH)
            .count(),
    })
}

/// フレームの時間の記録
pub struct FrameStats {
    shown: bool,
    /// 前のフレームの始まり
    last: Option<Instant>,
    /// 今のフレームの始まりと、そこまでの間隔、画面の中身を作り終えた時刻
    current: Option<(Instant, f32, Option<Instant>)>,
    recent: VecDeque<Frame>,
    /// ログに書く前のぶん
    pending: Vec<Frame>,
    logged: Instant,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            shown: false,
            last: None,
            current: None,
            recent: VecDeque::with_capacity(KEEP),
            pending: Vec::new(),
            logged: Instant::now(),
        }
    }
}

impl FrameStats {
    /// 出す・隠す
    pub fn toggle(&mut self) {
        self.shown = !self.shown;
        self.recent.clear();
        self.pending.clear();
        self.logged = Instant::now();
        log::info!(
            "frame times: {}",
            if self.shown { "shown" } else { "hidden" }
        );
    }

    /// フレームを作り始める
    pub fn begin(&mut self) {
        let now = Instant::now();
        let interval = self
            .last
            .map_or(0.0, |l| now.duration_since(l).as_secs_f32() * 1000.0);
        self.last = Some(now);
        self.current = Some((now, interval, None));
    }

    /// 画面の中身を作り終えた
    pub fn built(&mut self) {
        if let Some((_, _, built)) = &mut self.current {
            *built = Some(Instant::now());
        }
    }

    /// フレームを出し終えた。`waited` は GPU の画面の空きを待った時間
    pub fn end(&mut self, waited: Duration) {
        let Some((start, interval, built)) = self.current.take() else {
            return;
        };
        if !self.shown || interval == 0.0 {
            return;
        }
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        let work = ms(start.elapsed().saturating_sub(waited));
        let build = built.map_or(work, |b| ms(b.duration_since(start)));
        let f = Frame {
            interval,
            build,
            work,
        };
        if self.recent.len() == KEEP {
            self.recent.pop_front();
        }
        self.recent.push_back(f);
        self.pending.push(f);
        if self.logged.elapsed() >= LOG_EVERY {
            self.logged = Instant::now();
            if let Some(s) = summarize(&self.pending) {
                log::info!(
                    "frames: {:.0} fps, interval avg {:.1} max {:.1} ms, build avg {:.1} max {:.1} ms, work avg {:.1} max {:.1} ms, hitches {}",
                    s.fps,
                    s.interval_avg,
                    s.interval_max,
                    s.build_avg,
                    s.build_max,
                    s.work_avg,
                    s.work_max,
                    s.hitches
                );
            }
            self.pending.clear();
        }
    }

    /// 右上に、直近の数字とグラフを出す（画面の座標で描く）
    pub fn draw(&self, painter: &egui::Painter, screen: Rect) {
        if !self.shown {
            return;
        }
        let frames: Vec<Frame> = self.recent.iter().copied().collect();
        // 数字は直近 1 秒ほど
        let tail = &frames[frames.len().saturating_sub(60)..];
        let w = 300.0;
        let h = 126.0;
        let rect = Rect::from_min_size(
            pos2(screen.right() - w - 8.0, screen.top() + 8.0),
            vec2(w, h),
        );
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(200));
        let text = match summarize(tail) {
            Some(s) => format!(
                "{:.0} fps　落ちたフレーム {}\n間隔 平均 {:.1} / 最大 {:.1} ms\n画面 平均 {:.1} / 最大 {:.1} ms\nGPU へ 平均 {:.1} / 最大 {:.1} ms",
                s.fps,
                s.hitches,
                s.interval_avg,
                s.interval_max,
                s.build_avg,
                s.build_max,
                s.work_avg,
                s.work_max
            ),
            None => "測っています…".to_owned(),
        };
        painter.text(
            rect.min + vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            text,
            FontId::proportional(13.0),
            Color32::WHITE,
        );
        // グラフ：灰色が間隔、水色が GPU へ渡すまで、橙が画面の中身を作る時間。線は 60 fps（16.7 ms）
        let graph = Rect::from_min_max(
            pos2(rect.left() + 8.0, rect.top() + 78.0),
            rect.max - vec2(8.0, 6.0),
        );
        let scale = graph.height() / 40.0;
        let bar = graph.width() / KEEP as f32;
        let y_of = |ms: f32| graph.bottom() - (ms * scale).min(graph.height());
        for (i, f) in frames.iter().enumerate() {
            let x = graph.left() + i as f32 * bar;
            let r = |ms: f32| {
                Rect::from_min_max(
                    pos2(x, y_of(ms)),
                    Pos2::new(x + bar.max(1.0), graph.bottom()),
                )
            };
            painter.rect_filled(r(f.interval), 0.0, Color32::from_gray(110));
            painter.rect_filled(r(f.work), 0.0, Color32::from_rgb(90, 170, 230));
            painter.rect_filled(r(f.build), 0.0, Color32::from_rgb(255, 150, 40));
        }
        let line = y_of(1000.0 / 60.0);
        painter.line_segment(
            [pos2(graph.left(), line), pos2(graph.right(), line)],
            Stroke::new(1.0, Color32::from_rgb(255, 255, 255)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_slow_frames() {
        let mut frames = vec![
            Frame {
                interval: 16.7,
                build: 1.0,
                work: 2.0
            };
            59
        ];
        frames.push(Frame {
            interval: 40.0,
            build: 25.0,
            work: 30.0,
        });
        let s = summarize(&frames).unwrap();
        assert_eq!(s.frames, 60);
        assert_eq!(s.hitches, 1);
        assert!((s.interval_max - 40.0).abs() < 1e-3);
        assert!((s.work_max - 30.0).abs() < 1e-3);
        assert!((s.build_max - 25.0).abs() < 1e-3);
        assert!((s.fps - 60.0 * 1000.0 / (16.7 * 59.0 + 40.0)).abs() < 1e-2);
        assert_eq!(summarize(&[]), None);
    }
}
