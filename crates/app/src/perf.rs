//! FPS と遅延の表示（このアプリで足したもの。絵は足さずにコードで描く）。
//! 設定「FPS と遅延の表示」か、どの画面でも F9 で「出さない → 小さく → 詳しく」と切り替える。
//! 画面の右上の隅に出し、プレイエリア（レーン）と左の列にはかぶせない。
//!
//! 出すもの:
//! - FPS と、落ちたフレーム（間隔がふだんの 1.5 倍を超えたもの。[`crate::frames`]）
//! - 叩いた音: 叩いてから、その音が鳴り始めるまで（直近の打撃の平均）。内訳は
//!   MIDI の揺れ（届いた時刻 − MIDI の時刻から見積もった叩いた時刻。届くのがいつもより遅れたぶん）、
//!   処理待ち（届いてから判定して音を出力へ渡すまで。画面のフレームを待つぶん）、
//!   出力（渡してから鳴るまで。次のバッファを待つぶんと、ドライバーとデバイスの遅れ）
//! - 音の出力: 出力へ渡した音が鳴るまで（ドライバーの知らせ）と、1 回に渡すバッファの長さ
//!
//! 判定は叩いた時刻（MIDI の時刻）と音の再生位置で行うので、ここに出る遅れは判定には入らない。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Rect, Stroke, pos2, vec2};

use crate::audio::{HitTiming, OutputLatency};
use crate::frames::{FrameStats, KEEP, Summary};
use crate::gfx::Canvas;

/// 表示の大きさ（設定の選択肢と同じ並び）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Compact,
    Detail,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Compact, Self::Detail];

    /// 設定の選択肢の何番目か から（範囲の外は出さない）
    pub fn from_index(i: usize) -> Self {
        Self::ALL.get(i).copied().unwrap_or(Self::Off)
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// F9 で次へ（出さない → 小さく → 詳しく → 出さない）
    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }
}

/// 数字を作り直す間隔（毎フレーム変えると読めない）
const REFRESH: Duration = Duration::from_millis(250);
/// FPS の数字に使うフレームの数（およそ 1 秒）
const FPS_OF: usize = 60;
/// 覚えておく打撃の数と、平均に使う数
const HIT_KEEP: usize = 64;
const HIT_MEAN_OF: usize = 16;
/// 最後に叩いてからこれだけ過ぎたら、叩いた音の数字を薄くする（前の曲の数字）
const HIT_STALE: Duration = Duration::from_secs(20);
/// 音の出力の遅れの平均に使う間
const OUTPUT_WINDOW: Duration = Duration::from_secs(1);
/// 遅れをログに書く間隔
const LOG_EVERY: Duration = Duration::from_secs(5);

/// 色。良い・気になる・遅い と、項目の名前
const GOOD: Color32 = Color32::from_rgb(140, 230, 150);
const WARN: Color32 = Color32::from_rgb(255, 205, 80);
const BAD: Color32 = Color32::from_rgb(255, 110, 100);
const LABEL: Color32 = Color32::from_rgb(200, 200, 205);
const DIM: Color32 = Color32::from_rgb(130, 130, 135);

/// 叩いた音の遅れのまとめ（ミリ秒）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitStats {
    /// 平均に使った打撃の数
    pub hits: usize,
    /// 叩いてから鳴るまで（音を鳴らした打撃だけ。鳴らさなければ `None`）
    pub total_avg: Option<f32>,
    pub total_max: Option<f32>,
    /// MIDI の揺れ（届いた − 叩いた）
    pub midi_avg: f32,
    /// 処理待ち（渡した − 届いた）
    pub wait_avg: f32,
    pub wait_max: f32,
    /// 出力（鳴った − 渡した）
    pub out_avg: Option<f32>,
}

/// 直近の打撃のまとめ
pub fn hit_stats<'a>(hits: impl IntoIterator<Item = &'a HitTiming>) -> Option<HitStats> {
    let ms = |us: i64| us as f32 / 1000.0;
    let mut n = 0usize;
    let (mut midi, mut wait, mut wait_max) = (0.0f32, 0.0f32, 0.0f32);
    let mut totals = Vec::new();
    let mut outs = Vec::new();
    for h in hits {
        n += 1;
        midi += ms(h.arrival_us - h.hit_us).max(0.0);
        let w = ms(h.handled_us - h.arrival_us).max(0.0);
        wait += w;
        wait_max = wait_max.max(w);
        if let Some(s) = h.sound_us {
            totals.push(ms(s - h.hit_us));
            outs.push(ms(s - h.handled_us));
        }
    }
    if n == 0 {
        return None;
    }
    let avg = |v: &[f32]| (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32);
    Some(HitStats {
        hits: n,
        total_avg: avg(&totals),
        total_max: totals.iter().copied().reduce(f32::max),
        midi_avg: midi / n as f32,
        wait_avg: wait / n as f32,
        wait_max,
        out_avg: avg(&outs),
    })
}

/// 画面に出している数字（[`REFRESH`] ごとに作り直す）
#[derive(Debug, Clone, Copy)]
struct Readout {
    frames: Option<Summary>,
    hits: Option<HitStats>,
    /// 最後に叩いてから [`HIT_STALE`] より過ぎた
    hits_stale: bool,
    output: Option<OutputLatency>,
}

/// FPS と遅延の表示
pub struct PerfView {
    pub frames: FrameStats,
    mode: Mode,
    /// 毎フレーム見た音の出力の遅れ
    outputs: VecDeque<(Instant, OutputLatency)>,
    /// 直近の打撃（古い順）
    hits: VecDeque<HitTiming>,
    last_hit: Option<Instant>,
    readout: Option<(Instant, Readout)>,
    logged: Instant,
    /// 前にログに書いてから来た打撃の数
    unlogged_hits: usize,
}

impl Default for PerfView {
    fn default() -> Self {
        Self {
            frames: FrameStats::default(),
            mode: Mode::Off,
            outputs: VecDeque::new(),
            hits: VecDeque::with_capacity(HIT_KEEP),
            last_hit: None,
            readout: None,
            logged: Instant::now(),
            unlogged_hits: 0,
        }
    }
}

/// 叩いた音の遅れの色
fn total_color(ms: f32) -> Color32 {
    if ms <= 20.0 {
        GOOD
    } else if ms <= 40.0 {
        WARN
    } else {
        BAD
    }
}

/// 音の出力の遅れの色
fn output_color(ms: f32) -> Color32 {
    if ms <= 15.0 {
        GOOD
    } else if ms <= 30.0 {
        WARN
    } else {
        BAD
    }
}

/// FPS の色（直近 1 秒の落ちたフレームの数で）
fn fps_color(hitches: usize) -> Color32 {
    match hitches {
        0 => GOOD,
        1..=2 => WARN,
        _ => BAD,
    }
}

impl PerfView {
    /// 表示の大きさを変える（出しているあいだだけフレームを測る）
    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.frames.set_shown(mode != Mode::Off);
        self.readout = None;
        self.outputs.clear();
        self.logged = Instant::now();
        self.unlogged_hits = 0;
    }

    /// フレームごとに、今の音の出力の遅れと、届いた打撃の時刻を渡す
    pub fn feed(&mut self, output: Option<OutputLatency>, hits: Vec<HitTiming>) {
        if self.mode == Mode::Off {
            return;
        }
        let now = Instant::now();
        if let Some(o) = output {
            self.outputs.push_back((now, o));
        }
        while self
            .outputs
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > OUTPUT_WINDOW)
        {
            self.outputs.pop_front();
        }
        if !hits.is_empty() {
            self.last_hit = Some(now);
            self.unlogged_hits += hits.len();
        }
        for h in hits {
            if self.hits.len() == HIT_KEEP {
                self.hits.pop_front();
            }
            self.hits.push_back(h);
        }
        if self.logged.elapsed() >= LOG_EVERY {
            self.logged = now;
            self.log();
        }
    }

    /// 遅れをログに書く（叩いたときだけ）
    fn log(&mut self) {
        if self.unlogged_hits == 0 {
            return;
        }
        let n = self.unlogged_hits.min(self.hits.len());
        self.unlogged_hits = 0;
        let Some(h) = hit_stats(self.hits.iter().skip(self.hits.len() - n)) else {
            return;
        };
        let o = self.output_avg();
        let opt = |v: Option<f32>| v.map_or("-".to_owned(), |v| format!("{v:.1}"));
        log::info!(
            "latency: {} hits, hit to sound avg {} max {} ms (midi jitter {:.1}, wait {:.1} max {:.1}, output {}), output {} ms buffer {} ms",
            h.hits,
            opt(h.total_avg),
            opt(h.total_max),
            h.midi_avg,
            h.wait_avg,
            h.wait_max,
            opt(h.out_avg),
            opt(o.map(|o| o.latency_ms)),
            opt(o.map(|o| o.buffer_ms)),
        );
    }

    /// 直近 [`OUTPUT_WINDOW`] の音の出力の遅れの平均
    fn output_avg(&self) -> Option<OutputLatency> {
        let (_, last) = *self.outputs.back()?;
        let n = self.outputs.len() as f32;
        Some(OutputLatency {
            latency_ms: self.outputs.iter().map(|(_, o)| o.latency_ms).sum::<f32>() / n,
            buffer_ms: self.outputs.iter().map(|(_, o)| o.buffer_ms).sum::<f32>() / n,
            rate: last.rate,
        })
    }

    fn readout(&mut self) -> Readout {
        if let Some((at, r)) = self.readout
            && at.elapsed() < REFRESH
        {
            return r;
        }
        let skip = self.hits.len().saturating_sub(HIT_MEAN_OF);
        let r = Readout {
            frames: self.frames.summary(FPS_OF),
            hits: hit_stats(self.hits.iter().skip(skip)),
            hits_stale: self.last_hit.is_none_or(|t| t.elapsed() > HIT_STALE),
            output: self.output_avg(),
        };
        self.readout = Some((Instant::now(), r));
        r
    }

    /// 右上の隅に出す（NX の 1280×720 の座標で描く）
    pub fn draw(&mut self, c: &Canvas) {
        match self.mode {
            Mode::Off => {}
            Mode::Compact => {
                let r = self.readout();
                draw_compact(c, &r);
            }
            Mode::Detail => {
                let r = self.readout();
                self.draw_detail(c, &r);
            }
        }
    }

    /// 詳しく: 数字とフレームのグラフと、叩いた音の内訳
    fn draw_detail(&self, c: &Canvas, r: &Readout) {
        let s = c.scale();
        let w = 340.0;
        let x = 1280.0 - 6.0 - w;
        let y = 6.0;
        let h = 216.0;
        let p = c.painter();
        p.rect_filled(c.rect(x, y, w, h), 6.0 * s, Color32::from_black_alpha(200));
        let left = x + 10.0;
        let mut line_y = y + 7.0;
        let px = 14.0;
        let row = 19.0;
        // FPS とフレームの時間
        let mut job = Job::new(px * s);
        match r.frames {
            Some(f) => {
                job.num(&format!("{:.0}", f.fps), fps_color(f.hitches));
                job.label(i18n::t(" fps   Dropped ", " fps　落ちたフレーム "));
                job.num(&f.hitches.to_string(), fps_color(f.hitches));
                job.label(i18n::t(" (last 1 s)", "（直近 1 秒）"));
            }
            None => job.label(i18n::t("Measuring…", "測っています…")),
        }
        job.paint(c, left, line_y, Align2::LEFT_TOP);
        line_y += row;
        if let Some(f) = r.frames {
            let small = 12.5 * s;
            let mut job = Job::new(small);
            job.label(i18n::t("Frame Interval ", "フレームの間隔 "));
            job.num(
                &format!("{:.1} / {:.1}", f.interval_avg, f.interval_max),
                LABEL,
            );
            job.label(i18n::t(" ms (avg / max)", " ms（平均 / 最大）"));
            job.paint(c, left, line_y, Align2::LEFT_TOP);
            line_y += row - 3.0;
            let mut job = Job::new(small);
            job.label(i18n::t("Build ", "画面を作る "));
            job.num(&format!("{:.1} / {:.1}", f.build_avg, f.build_max), LABEL);
            job.label(i18n::t("   GPU ", "　GPU へ "));
            job.num(&format!("{:.1} / {:.1}", f.work_avg, f.work_max), LABEL);
            job.label(" ms");
            job.paint(c, left, line_y, Align2::LEFT_TOP);
        } else {
            line_y += row - 3.0;
        }
        line_y += row - 2.0;
        // グラフ：灰色が間隔、水色が GPU へ渡すまで、橙が画面の中身を作る時間。線は 60 fps（16.7 ms）
        let graph = (left, line_y, w - 20.0, 44.0);
        self.draw_frame_graph(c, graph);
        line_y += graph.3 + 6.0;
        // 叩いた音
        let dim = r.hits_stale;
        let mut job = Job::new(px * s);
        job.label(i18n::t("Hit Latency ", "叩いた音 "));
        match r.hits {
            Some(HitStats {
                total_avg: Some(avg),
                total_max: Some(max),
                hits,
                ..
            }) => {
                job.num(
                    &format!("{avg:.0}"),
                    if dim { DIM } else { total_color(avg) },
                );
                job.label(i18n::t(" ms (max ", " ms（最大 "));
                job.num(&format!("{max:.0}"), if dim { DIM } else { LABEL });
                job.label(&i18n::tf!(
                    " ms, last {hits} hits)",
                    " ms、直近 {hits} 打）"
                ));
            }
            Some(_) => job.label(i18n::t(
                "— (hit sounds are off)",
                "—（叩いた音を鳴らしていません）",
            )),
            None => job.label(i18n::t(
                "— (hit a pad while playing)",
                "—（演奏で叩くと出ます）",
            )),
        }
        job.paint(c, left, line_y, Align2::LEFT_TOP);
        line_y += row;
        if let Some(hs) = r.hits {
            let mut job = Job::new(12.5 * s);
            job.label(i18n::t("  MIDI Jitter ", "　MIDI の揺れ "));
            job.num(&format!("{:.1}", hs.midi_avg), LABEL);
            job.label(i18n::t("   Queue ", "　処理待ち "));
            job.num(&format!("{:.1}", hs.wait_avg), LABEL);
            job.label(i18n::t("   Output ", "　出力 "));
            job.num(
                &hs.out_avg.map_or("—".to_owned(), |v| format!("{v:.1}")),
                LABEL,
            );
            job.label(" ms");
            job.paint(c, left, line_y, Align2::LEFT_TOP);
        }
        line_y += row;
        // 音の出力
        let mut job = Job::new(px * s);
        job.label(i18n::t("Audio Output ", "音の出力 "));
        match r.output {
            Some(o) => {
                job.num(&format!("{:.0}", o.latency_ms), output_color(o.latency_ms));
                job.label(i18n::t(" ms (buffer ", " ms（バッファ "));
                job.num(&format!("{:.1}", o.buffer_ms), LABEL);
                job.label(&i18n::tf!(" ms, {} Hz)", " ms、{} Hz）", o.rate));
            }
            None => job.label(i18n::t("— (no audio output)", "—（音が出ていません）")),
        }
        job.paint(c, left, line_y, Align2::LEFT_TOP);
        line_y += row;
        let mut job = Job::new(11.5 * s);
        job.label(i18n::t(
            "Judgement uses the hit time, so these delays don't affect it",
            "判定は叩いた時刻で行うので、これらの遅れは判定に入りません",
        ));
        job.paint(c, left, line_y + 1.0, Align2::LEFT_TOP);
    }

    fn draw_frame_graph(&self, c: &Canvas, (x, y, w, h): (f32, f32, f32, f32)) {
        let p = c.painter();
        let graph = c.rect(x, y, w, h);
        p.rect_filled(graph, 0.0, Color32::from_white_alpha(10));
        let scale = graph.height() / 40.0;
        let bar = graph.width() / KEEP as f32;
        let y_of = |ms: f32| graph.bottom() - (ms * scale).min(graph.height());
        for (i, f) in self.frames.recent().enumerate() {
            let x = graph.left() + i as f32 * bar;
            let r = |ms: f32| {
                Rect::from_min_max(pos2(x, y_of(ms)), pos2(x + bar.max(1.0), graph.bottom()))
            };
            p.rect_filled(r(f.interval), 0.0, Color32::from_gray(110));
            p.rect_filled(r(f.work), 0.0, Color32::from_rgb(90, 170, 230));
            p.rect_filled(r(f.build), 0.0, Color32::from_rgb(255, 150, 40));
        }
        let line = y_of(1000.0 / 60.0);
        p.line_segment(
            [pos2(graph.left(), line), pos2(graph.right(), line)],
            Stroke::new(1.0, Color32::from_white_alpha(160)),
        );
    }
}

/// 小さく: 右上の隅に 1 行
fn draw_compact(c: &Canvas, r: &Readout) {
    let s = c.scale();
    let mut job = Job::new(13.5 * s);
    match r.frames {
        Some(f) => {
            job.num(&format!("{:.0}", f.fps), fps_color(f.hitches));
            job.label(" fps");
        }
        None => job.label("— fps"),
    }
    job.label(i18n::t("   Hit Latency ", "　叩いた音 "));
    match r.hits.and_then(|h| h.total_avg) {
        Some(ms) => {
            job.num(
                &format!("{ms:.0}"),
                if r.hits_stale { DIM } else { total_color(ms) },
            );
            job.label(" ms");
        }
        None => job.label("—"),
    }
    job.label(i18n::t("   Output ", "　出力 "));
    match r.output {
        Some(o) => {
            job.num(&format!("{:.0}", o.latency_ms), output_color(o.latency_ms));
            job.label(" ms");
        }
        None => job.label("—"),
    }
    let galley = c.painter().layout_job(job.job);
    // 一時停止の板と簡易設定の板（上端 y 24）にかからない高さに収める
    let pad = vec2(8.0, 1.5) * s;
    let right_top = c.pos(1280.0 - 4.0, 2.0);
    let rect = Rect::from_min_size(
        right_top - vec2(galley.size().x + pad.x * 2.0, 0.0),
        galley.size() + pad * 2.0,
    );
    c.painter()
        .rect_filled(rect, rect.height() / 2.0, Color32::from_black_alpha(170));
    c.painter().galley(rect.min + pad, galley, LABEL);
}

/// 色の違う文字をつないだ 1 行
struct Job {
    job: LayoutJob,
    px: f32,
}

impl Job {
    fn new(px: f32) -> Self {
        Self {
            job: LayoutJob::default(),
            px,
        }
    }

    fn label(&mut self, text: &str) {
        self.job.append(
            text,
            0.0,
            TextFormat::simple(FontId::new(self.px, FontFamily::Proportional), LABEL),
        );
    }

    /// 数字（桁の幅がそろう字で）
    fn num(&mut self, text: &str, color: Color32) {
        self.job.append(
            text,
            0.0,
            TextFormat::simple(FontId::new(self.px, FontFamily::Monospace), color),
        );
    }

    fn paint(self, c: &Canvas, x: f32, y: f32, align: Align2) {
        let galley = c.painter().layout_job(self.job);
        let rect = align.anchor_size(c.pos(x, y), galley.size());
        c.painter().galley(rect.min, galley, LABEL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(hit: i64, arrival: i64, handled: i64, sound: Option<i64>) -> HitTiming {
        HitTiming {
            hit_us: hit,
            arrival_us: arrival,
            handled_us: handled,
            sound_us: sound,
        }
    }

    #[test]
    fn hit_stats_split_the_delay() {
        let hits = [
            // 叩いて 1 ms で届き、4 ms 待って渡し、15 ms で鳴った（計 20 ms）
            timing(0, 1_000, 5_000, Some(20_000)),
            // 届くのが 3 ms、待ち 12 ms、鳴るまで 15 ms（計 30 ms）
            timing(100_000, 103_000, 115_000, Some(130_000)),
            // 音を鳴らさなかった打撃は、内訳の MIDI と処理待ちにだけ入る
            timing(200_000, 202_000, 210_000, None),
        ];
        let s = hit_stats(&hits).unwrap();
        assert_eq!(s.hits, 3);
        assert_eq!(s.total_avg, Some(25.0));
        assert_eq!(s.total_max, Some(30.0));
        assert_eq!(s.midi_avg, 2.0);
        assert_eq!(s.wait_avg, 8.0);
        assert_eq!(s.wait_max, 12.0);
        assert_eq!(s.out_avg, Some(15.0));
        assert_eq!(hit_stats(&[]), None);
        let silent = hit_stats(&[timing(0, 0, 2_000, None)]).unwrap();
        assert_eq!(silent.total_avg, None);
        assert_eq!(silent.out_avg, None);
    }

    #[test]
    fn f9_cycles_the_modes() {
        assert_eq!(Mode::Off.next(), Mode::Compact);
        assert_eq!(Mode::Compact.next(), Mode::Detail);
        assert_eq!(Mode::Detail.next(), Mode::Off);
        assert_eq!(Mode::from_index(2), Mode::Detail);
        assert_eq!(Mode::from_index(9), Mode::Off);
    }
}
