//! 音声出力。音を読み込んで出力の形式（ステレオ、出力のサンプルレート）にそろえ、
//! 決めたフレームに鳴らす。出力のコールバックごとに「このバッファが鳴るホスト時刻」を時計に渡す。

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engine::clock::AudioClock;
use engine::practice::Placement;
use symphonia::core::audio::sample::Sample as _;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

use crate::time::{duration_us, host_us};
use crate::xa;

/// 出力の形式にそろえた音（ステレオで交互に並んだ f32）
#[derive(Clone)]
pub struct Sound(Arc<[f32]>);

impl Sound {
    /// ステレオで交互に並んだサンプルから作る
    pub fn from_samples(samples: Vec<f32>) -> Self {
        Self(Arc::from(samples))
    }

    pub fn samples(&self) -> &[f32] {
        &self.0
    }

    pub fn frames(&self) -> usize {
        self.0.len() / 2
    }
}

/// カウントのクリック。短いサイン波を減衰させたもの。`accent` なら高い音（1 拍目）
pub fn click(rate: u32, accent: bool) -> Sound {
    let freq = if accent { 1760.0 } else { 1175.0 };
    let rate = f64::from(rate);
    let frames = (rate * 0.04) as usize;
    let samples: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let t = i as f64 / rate;
            let v = ((std::f64::consts::TAU * freq * t).sin() * (-t / 0.008).exp() * 0.7) as f32;
            [v, v]
        })
        .collect();
    Sound(Arc::from(samples))
}

/// 音のファイルを読み、ステレオ・`rate` Hz にそろえる。
/// BandJAM XA（`.xa`）は名前ではなく中身の頭で見分ける（symphonia は読めない）
pub fn load_sound(path: &Path, rate: u32) -> Result<Sound> {
    let bytes = std::fs::read(path).with_context(|| format!("open {}", path.display()))?;
    let sound = if xa::is_xa(&bytes) {
        xa::decode(&bytes).map(|x| xa_sound(&x, rate))
    } else {
        let ext = path.extension().and_then(|e| e.to_str());
        decode_sound(Box::new(std::io::Cursor::new(bytes)), ext, rate)
    };
    sound.with_context(|| path.display().to_string())
}

/// 読んだ BandJAM XA の音をステレオ・`rate` Hz にそろえる
fn xa_sound(x: &xa::Xa, rate: u32) -> Sound {
    let samples: Vec<f32> = x.samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
    let stereo = to_stereo(&samples, x.channels);
    Sound(Arc::from(resample_linear(&stereo, x.rate, rate)))
}

/// メモリにある音のファイル（アプリに入れたスキンの音）を読み、ステレオ・`rate` Hz にそろえる
pub fn load_sound_bytes(
    bytes: impl AsRef<[u8]> + Send + Sync + 'static,
    ext: Option<&str>,
    rate: u32,
) -> Result<Sound> {
    decode_sound(Box::new(std::io::Cursor::new(bytes)), ext, rate)
}

fn decode_sound(source: Box<dyn MediaSource>, ext: Option<&str>, rate: u32) -> Result<Sound> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .context("probe")?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("no audio track"))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("not audio"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())?;
    let track_id = track.id;

    let mut src_rate = 0u32;
    let mut channels = 0usize;
    let mut interleaved: Vec<f32> = Vec::new();
    let mut chunk: Vec<f32> = Vec::new();
    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                src_rate = buf.spec().rate();
                channels = buf.spec().channels().count();
                chunk.resize(buf.samples_interleaved(), f32::MID);
                buf.copy_to_slice_interleaved(&mut chunk);
                interleaved.extend_from_slice(&chunk);
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    if channels == 0 || src_rate == 0 {
        return Ok(Sound(Arc::from([])));
    }
    let stereo = to_stereo(&interleaved, channels);
    Ok(Sound(Arc::from(resample_linear(&stereo, src_rate, rate))))
}

fn to_stereo(src: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        2 => src.to_vec(),
        1 => src.iter().flat_map(|&s| [s, s]).collect(),
        n => src.chunks_exact(n).flat_map(|f| [f[0], f[1]]).collect(),
    }
}

/// 線形補間のリサンプル（フェーズ 1 の仮。音質を詰めるときに差し替える）
fn resample_linear(stereo: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || stereo.len() < 4 {
        return stereo.to_vec();
    }
    let frames = stereo.len() / 2;
    let out_frames = (frames as u64 * u64::from(to) / u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(to);
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let pos = i as f64 * step;
        let i0 = (pos as usize).min(frames - 1);
        let i1 = (i0 + 1).min(frames - 1);
        let t = (pos - i0 as f64) as f32;
        for c in 0..2 {
            out.push(stereo[i0 * 2 + c] * (1.0 - t) + stereo[i1 * 2 + c] * t);
        }
    }
    out
}

/// 切れ目にかけるフェードの長さ（秒）。プツッという音を消す
pub const FADE_SECONDS: f64 = 0.005;

/// 全体の音量の既定（%）。設定が無いときもこの大きさで鳴らす（上流の既定の 100 は大きすぎる）
pub const DEFAULT_MASTER_VOLUME: u8 = 50;

/// 全体の音量（0〜100 %、上流 `MasterVolume`）。どの出力にも、混ぜたあとにかける
static MASTER_VOLUME: AtomicU8 = AtomicU8::new(DEFAULT_MASTER_VOLUME);

/// 全体の音量を変える（100 より大きければ 100）。鳴っている音にもすぐ効く
pub fn set_master_volume(percent: u8) {
    MASTER_VOLUME.store(percent.min(100), Ordering::Relaxed);
}

/// 全体の音量の倍率（0〜1。上流と同じく % をそのまま倍率にする）
fn master_gain() -> f32 {
    f32::from(MASTER_VOLUME.load(Ordering::Relaxed)) / 100.0
}

/// 音の系統（音量ミキサーのつまみ 1 本ずつ）。混ぜる前に音ごとに系統の音量をかけ、混ぜたあとに全体の音量をかける
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    /// 曲の音。BGM、譜面の効果音、ギターとベース、自動で叩くレーン、選曲のプレビュー（上流の AutoVolume）
    Song,
    /// 叩いた音（上流の ChipVolume）
    Hits,
    /// メトロノームと、練習の数え始めのクリック
    Click,
    /// 画面の効果音（カーソル・決定・結果の声など、スキンの `Sounds`）
    Se,
}

impl Bus {
    pub const ALL: [Bus; 4] = [Bus::Song, Bus::Hits, Bus::Click, Bus::Se];

    fn index(self) -> usize {
        self as usize
    }
}

/// 系統ごとの音量（0〜100 %）。設定を読むまでは設定の既定と同じ
static BUS_VOLUME: [AtomicU8; 4] = [
    AtomicU8::new(80),
    AtomicU8::new(100),
    AtomicU8::new(100),
    AtomicU8::new(100),
];

/// 系統 `bus` の音量を変える（100 より大きければ 100）。鳴っている音にもすぐ効く
pub fn set_bus_volume(bus: Bus, percent: u8) {
    BUS_VOLUME[bus.index()].store(percent.min(100), Ordering::Relaxed);
}

/// 系統ごとの音量の倍率（0〜1、[`Bus::ALL`] の並び）
fn bus_gains() -> [f32; 4] {
    std::array::from_fn(|i| f32::from(BUS_VOLUME[i].load(Ordering::Relaxed)) / 100.0)
}

/// バッファの `j` フレーム目（全部で `n`）の音量。`from` から `to` へ直線で移し、最後のフレームで `to` になる
fn ramp((from, to): (f32, f32), j: u64, n: u64) -> f32 {
    if from == to {
        to
    } else {
        from + (to - from) * (j + 1) as f32 / n.max(1) as f32
    }
}

/// 混ぜた音（`channels` 個ずつ交互）に全体の音量をかけ、-1〜1 に収める。
/// 音量が変わったときは、このバッファの中で `from` から `to` へ直線で移す（プツッという音を出さない）
fn apply_master(data: &mut [f32], channels: usize, from: f32, to: f32) {
    let channels = channels.max(1);
    let frames = (data.len() / channels) as u64;
    for (i, frame) in data.chunks_mut(channels).enumerate() {
        let g = ramp((from, to), i as u64, frames);
        for s in frame {
            *s = (*s * g).clamp(-1.0, 1.0);
        }
    }
}

/// 同じ音を同時に鳴らせる数（上流の PolyphonicSounds の既定）。これを超えると、同じ音の一番古いものを止める
pub const MAX_OVERLAP: usize = 4;

enum Command {
    /// 決めたフレームから鳴らす（曲の BGM など）
    At(Voice),
    /// できるだけ早く鳴らす（叩いた音）。`tag` で後から止められる
    Now {
        sound: Sound,
        gain: f32,
        pan: f32,
        tag: u8,
        bus: Bus,
        /// 同じ音の重なりを [`MAX_OVERLAP`] までにする（譜面の音）
        limit: bool,
    },
    /// `tag` と同じビットを持つ音を短いフェードで止める（選曲のプレビューの音、メトロノーム）。
    /// 鳴らす予定でまだ鳴り始めていない音は取りやめる
    Stop(u8),
    /// `tag` と同じビットを持つ、もう鳴り始めた音だけを短いフェードで止める（ハイハット、チョーク。
    /// 先に鳴らす予定の自動の音は残す）
    StopRinging(u8),
    /// 決めたフレームで、`tag` と同じビットを持ち、そのときまでに鳴り始めた音を止める（自動で叩く左ペダルなど）
    StopAt { frame: u64, tag: u8 },
    /// このフレームより後から鳴らす予定の音と止める予定を取りやめる（練習で並べ直す周の音）
    CancelFrom(u64),
    /// 鳴っている音と、鳴らす予定の音をすべて短いフェードで止める（早送り・巻き戻し）
    StopAll,
}

/// 同じ音の重なりの数え方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlap {
    /// 数えない（練習の周の BGM、クリック、画面の効果音）
    Free,
    /// 鳴り始めたら、同じ音の重なりを数える
    Pending,
    /// 数えた（後から鳴る同じ音の重なりに入る）
    Counted,
}

/// 鳴らす音 1 つ。音の `skip` フレーム目から `len` フレームを、出力の `start` フレーム目から鳴らす
struct Voice {
    sound: Sound,
    start: u64,
    skip: u64,
    len: u64,
    gain: f32,
    /// 左右（-1 が左、1 が右）。ステレオの音の反対側を下げる（上流の BASS の `BASS_ATTRIB_PAN` と同じ）
    pan: f32,
    /// 途中から始めたので頭にフェードをかける
    fade_in: bool,
    /// 途中で切るので終わりにフェードをかける
    fade_out: bool,
    /// 止めるときの目印（ビットの組。0 は止めない）
    tag: u8,
    /// 音量をかける系統
    bus: Bus,
    /// 同じ音の重なりの数え方
    overlap: Overlap,
}

impl Voice {
    fn whole(sound: Sound, start: u64, gain: f32, bus: Bus) -> Self {
        let len = sound.frames() as u64;
        Self {
            sound,
            start,
            skip: 0,
            len,
            gain,
            pan: 0.0,
            fade_in: false,
            fade_out: false,
            tag: 0,
            bus,
            overlap: Overlap::Free,
        }
    }

    fn end(&self) -> u64 {
        self.start + self.len
    }

    /// 出力の `at` フレーム目から `fade` フレームのフェードで止める（もっと前に終わるならそのまま）
    fn cut(&mut self, at: u64, fade: u64) {
        let len = at.saturating_sub(self.start) + fade;
        if len < self.len {
            self.len = len;
            self.fade_out = true;
        }
    }

    /// 出力の `frames_done` フレーム目から `n` フレームぶん（`data`）に足す。`fade` はフェードのフレーム数。
    /// `bus` は系統の音量（このバッファの頭と終わり。変わったときはバッファの中で直線で移す）
    fn mix(
        &self,
        data: &mut [f32],
        channels: usize,
        (frames_done, n): (u64, u64),
        fade: u64,
        bus: (f32, f32),
    ) {
        if self.start >= frames_done + n || self.end() <= frames_done {
            return;
        }
        let from = frames_done.max(self.start);
        let to = (frames_done + n).min(self.end());
        let (left, right) = pan_gains(self.pan);
        for f in from..to {
            let i = f - self.start;
            let g = self.gain
                * ramp(bus, f - frames_done, n)
                * envelope(i, self.len, fade, self.fade_in, self.fade_out);
            let src = ((self.skip + i) * 2) as usize;
            let dst = ((f - frames_done) as usize) * channels;
            data[dst] += self.sound.0[src] * g * left;
            if channels > 1 {
                data[dst + 1] += self.sound.0[src + 1] * g * right;
            }
        }
    }
}

/// 出力のコールバックの中で持つ、鳴らす音と止める予定
struct Mixer {
    voices: Vec<Voice>,
    /// 決めたフレームで止める予定（フレーム、目印）
    stops: Vec<(u64, u8)>,
    /// 同じ音の重なりを数えるときの作業用（コールバックの中で毎回確保しないように使い回す）
    ringing: Vec<usize>,
    /// 止めるときのフェードのフレーム数
    fade: u64,
}

impl Mixer {
    fn new(fade: u64) -> Self {
        Self {
            voices: Vec::with_capacity(1024),
            stops: Vec::with_capacity(256),
            ringing: Vec::with_capacity(16),
            fade,
        }
    }

    /// 命令を 1 つ受ける。`now` はこれから混ぜるバッファの頭のフレーム
    fn command(&mut self, cmd: Command, now: u64) {
        match cmd {
            Command::At(v) => self.voices.push(v),
            Command::Now {
                sound,
                gain,
                pan,
                tag,
                bus,
                limit,
            } => {
                let mut v = Voice::whole(sound, now, gain, bus);
                v.pan = pan;
                v.tag = tag;
                if limit {
                    v.overlap = Overlap::Pending;
                }
                self.voices.push(v);
            }
            Command::Stop(tag) => {
                // まだ鳴り始めていない音は捨て、鳴っている音はフェードで止める
                self.voices.retain(|v| v.tag & tag == 0 || v.start < now);
                for v in self.voices.iter_mut().filter(|v| v.tag & tag != 0) {
                    v.cut(now, self.fade);
                }
            }
            Command::StopRinging(tag) => {
                for v in self
                    .voices
                    .iter_mut()
                    .filter(|v| v.tag & tag != 0 && v.start <= now)
                {
                    v.cut(now, self.fade);
                }
            }
            Command::StopAt { frame, tag } => self.stops.push((frame, tag)),
            Command::CancelFrom(frame) => {
                self.voices.retain(|v| v.start < frame);
                self.stops.retain(|&(f, _)| f < frame);
            }
            Command::StopAll => {
                // まだ始まっていない音は捨て、鳴っている音はフェードで止める
                self.voices.retain(|v| v.start < now);
                for v in self.voices.iter_mut() {
                    v.cut(now, self.fade);
                }
                self.stops.clear();
            }
        }
    }

    /// `now` から `n` フレームのバッファを混ぜる前に、そこで来る止める予定を当て、
    /// 鳴り始める譜面の音ごとに同じ音の重なりを [`MAX_OVERLAP`] までにする
    fn prepare(&mut self, now: u64, n: u64) {
        let end = now + n;
        let mut i = 0;
        while i < self.stops.len() {
            let (frame, tag) = self.stops[i];
            if frame < end {
                self.stops.swap_remove(i);
                let at = frame.max(now);
                for v in self
                    .voices
                    .iter_mut()
                    .filter(|v| v.tag & tag != 0 && v.start < frame)
                {
                    v.cut(at, self.fade);
                }
            } else {
                i += 1;
            }
        }
        // 鳴り始める音を早いものから 1 つずつ数える
        while let Some(i) = (0..self.voices.len())
            .filter(|&i| self.voices[i].overlap == Overlap::Pending && self.voices[i].start < end)
            .min_by_key(|&i| self.voices[i].start)
        {
            let start = self.voices[i].start;
            let voices = &self.voices;
            self.ringing.clear();
            self.ringing.extend((0..voices.len()).filter(|&j| {
                let o = &voices[j];
                o.overlap == Overlap::Counted
                    && o.end() > start
                    && Arc::ptr_eq(&o.sound.0, &voices[i].sound.0)
            }));
            if self.ringing.len() >= MAX_OVERLAP {
                self.ringing.sort_by_key(|&j| voices[j].start);
                let over = self.ringing.len() - MAX_OVERLAP;
                for k in 0..=over {
                    let j = self.ringing[k];
                    self.voices[j].cut(start, self.fade);
                }
            }
            self.voices[i].overlap = Overlap::Counted;
        }
    }
}

/// バッファ全体に直線のフェードをかける（`rise` なら 0 から上げ、でなければ 0 まで下げる）
fn fade_buffer(data: &mut [f32], channels: usize, rise: bool) {
    let frames = data.len() / channels.max(1);
    if frames == 0 {
        return;
    }
    for (i, frame) in data.chunks_mut(channels.max(1)).enumerate() {
        let t = i as f32 / frames as f32;
        let g = if rise { t } else { 1.0 - t };
        for v in frame {
            *v *= g;
        }
    }
}

/// 左右 `pan`（-1〜1）のときの左と右の音量。真ん中は両方 1 で、右へ振るほど左を下げる
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    (1.0 - pan.max(0.0), 1.0 + pan.min(0.0))
}

/// 長さ `len` の音の `i` フレーム目の音量（0〜1）。切れ目に `fade` フレームの直線のフェード
pub fn envelope(i: u64, len: u64, fade: u64, fade_in: bool, fade_out: bool) -> f32 {
    if fade == 0 {
        return 1.0;
    }
    let mut g = 1.0f32;
    if fade_in && i < fade {
        g = g.min(i as f32 / fade as f32);
    }
    let left = len.saturating_sub(i + 1);
    if fade_out && left < fade {
        g = g.min(left as f32 / fade as f32);
    }
    g
}

/// 鳴らしている出力
pub struct AudioOut {
    stream: cpal::Stream,
    tx: rtrb::Producer<Command>,
    clock: Arc<Mutex<AudioClock>>,
    rate: u32,
    start_ms: f64,
    /// 一時停止（出力を無音にし、フレームを進めない）
    paused: Arc<AtomicBool>,
    pub device_name: String,
}

impl AudioOut {
    /// 既定の出力デバイスを開く。`start_ms` は 0 フレーム目が曲の何ミリ秒にあたるか。
    /// 音を読み終えてから `start` で鳴らし始める（読んでいるあいだに曲の頭が過ぎないように）
    pub fn open(start_ms: f64) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no audio output device"))?;
        let device_name = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "unknown".into());
        let supported = device.default_output_config()?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            log::warn!(
                "output format is {:?}; asking for f32",
                supported.sample_format()
            );
        }
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate;
        let channels = usize::from(config.channels);
        let clock = Arc::new(Mutex::new(AudioClock::new(rate, start_ms)));
        let (tx, mut rx) = rtrb::RingBuffer::<Command>::new(4096);

        let cb_clock = clock.clone();
        let paused = Arc::new(AtomicBool::new(false));
        let cb_paused = paused.clone();
        // 前のバッファを止めていたか（止める・動かす切れ目にフェードをかける）
        let mut was_paused = false;
        let fade = (f64::from(rate) * FADE_SECONDS) as u64;
        let mut mixer = Mixer::new(fade);
        let mut frames_done: u64 = 0;
        let mut master = master_gain();
        let mut buses = bus_gains();
        let stream = device.build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let now = host_us();
                let ts = info.timestamp();
                let play_us = now + duration_us(ts.playback.duration_since(ts.callback));
                while let Ok(cmd) = rx.pop() {
                    mixer.command(cmd, frames_done);
                }
                let n = (data.len() / channels) as u64;
                data.fill(0.0);
                let pause = cb_paused.load(Ordering::Relaxed);
                if pause && was_paused {
                    // 止めているあいだは無音で、フレームも曲の時刻も進めない
                    if let Ok(mut c) = cb_clock.try_lock() {
                        c.on_hold(play_us, frames_done);
                    }
                    return;
                }
                mixer.prepare(frames_done, n);
                let to_buses = bus_gains();
                for v in &mixer.voices {
                    let b = v.bus.index();
                    v.mix(
                        data,
                        channels,
                        (frames_done, n),
                        fade,
                        (buses[b], to_buses[b]),
                    );
                }
                buses = to_buses;
                mixer.voices.retain(|v| v.end() > frames_done + n);
                let to = master_gain();
                apply_master(data, channels, master, to);
                master = to;
                if pause != was_paused {
                    // 止める切れ目は下げきり、動かし直す切れ目は上げる
                    fade_buffer(data, channels, !pause);
                    was_paused = pause;
                }
                if let Ok(mut c) = cb_clock.try_lock() {
                    c.on_buffer(play_us, frames_done);
                }
                frames_done += n;
            },
            |err| log::warn!("audio stream: {err}"),
            None,
        )?;
        log::info!("audio: {device_name}, {rate} Hz, {channels} ch");
        Ok(Self {
            stream,
            tx,
            clock,
            rate,
            start_ms,
            paused,
            device_name,
        })
    }

    /// 鳴らし始める（0 フレーム目が `start_ms`）
    pub fn start(&self) -> Result<()> {
        self.stream.play()?;
        Ok(())
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// ホスト時刻 `host_us` の曲の時刻
    pub fn song_ms_at(&self, host_us: i64) -> Option<f64> {
        self.clock.lock().ok()?.song_ms_at(host_us)
    }

    /// 一時停止する（`true`）か、動かし直す（`false`）。止めているあいだは無音で、曲の時刻も進まない
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// 曲の時刻を `delta_ms` だけずらす（早送り・巻き戻し）。鳴っている音と鳴らす予定の音はすべて止めるので、
    /// ずらした先で鳴らす音は呼ぶ側が渡し直す
    pub fn seek(&mut self, delta_ms: f64) {
        let _ = self.tx.push(Command::StopAll);
        self.start_ms += delta_ms;
        if let Ok(mut c) = self.clock.lock() {
            c.shift(delta_ms);
        }
    }

    /// 時刻 `ms` が出力の何フレーム目か（出力を始める前ならマイナス）
    fn frame_at(&self, ms: f64) -> f64 {
        (ms - self.start_ms) / 1000.0 * f64::from(self.rate)
    }

    /// 曲の時刻 `song_ms` に鳴らす
    pub fn play_at(&mut self, sound: &Sound, song_ms: f64, gain: f32, bus: Bus) {
        self.play_at_tagged(sound, song_ms, gain, 0, bus);
    }

    /// 曲の時刻 `song_ms` に鳴らし、止めるときの目印 `tag`（ビットの組。0 は止めない）を付ける。
    /// `stop` で、まだ鳴り始めていなければ取りやめ、鳴っていれば止める（メトロノーム）
    pub fn play_at_tagged(&mut self, sound: &Sound, song_ms: f64, gain: f32, tag: u8, bus: Bus) {
        let frame = self.frame_at(song_ms).max(0.0) as u64;
        let mut v = Voice::whole(sound.clone(), frame, gain, bus);
        v.tag = tag;
        let _ = self.tx.push(Command::At(v));
    }

    /// 音の一部を、置いたとおりに鳴らす（練習の周の BGM）。途中から始めるときと途中で切るときは短いフェードをかける
    pub fn play_placed(&mut self, sound: &Sound, p: &Placement, gain: f32, bus: Bus) {
        let rate = f64::from(self.rate);
        let mut start = self.frame_at(p.at_ms);
        let mut skip = p.skip_ms / 1000.0 * rate;
        let mut len = p.len_ms / 1000.0 * rate;
        let mut fade_in = p.cut_start;
        // 出力を始める前の部分は飛ばす
        if start < 0.0 {
            skip -= start;
            len += start;
            start = 0.0;
            fade_in = true;
        }
        let skip = skip.round() as u64;
        let len = (len.round() as u64).min((sound.frames() as u64).saturating_sub(skip));
        if len == 0 {
            return;
        }
        let _ = self.tx.push(Command::At(Voice {
            sound: sound.clone(),
            start: start.round() as u64,
            skip,
            len,
            gain,
            pan: 0.0,
            fade_in,
            fade_out: p.cut_end,
            tag: 0,
            bus,
            overlap: Overlap::Free,
        }));
    }

    /// 曲の時刻 `song_ms` 以後に鳴り始める予定の音を取りやめる（もう鳴り始めた音はそのまま）
    pub fn cancel_from(&mut self, song_ms: f64) {
        let frame = self.frame_at(song_ms).max(0.0).round() as u64;
        let _ = self.tx.push(Command::CancelFrom(frame));
    }

    /// 譜面の音を曲の時刻 `song_ms` に、音量 `gain` と左右 `pan` で、系統 `bus` に鳴らす。
    /// `len_ms` があれば、そこで短いフェードで切る（次の音で前の音を止めるチャンネル）。
    /// `tag`（0 は付けない）を付けると、[`stop`](Self::stop) と [`stop_at`](Self::stop_at) で止められる。
    /// 同じ音の重なりは [`MAX_OVERLAP`] まで
    pub fn play_chip(
        &mut self,
        sound: &Sound,
        song_ms: f64,
        len_ms: Option<f64>,
        (gain, pan): (f32, f32),
        (bus, tag): (Bus, u8),
    ) {
        let frame = self.frame_at(song_ms);
        let rate = f64::from(self.rate);
        let mut v = Voice::whole(sound.clone(), frame.max(0.0) as u64, gain, bus);
        v.pan = pan;
        v.tag = tag;
        v.overlap = Overlap::Pending;
        // 出力を始める前の部分は飛ばす
        if frame < 0.0 {
            v.skip = (-frame).round() as u64;
            v.fade_in = true;
        }
        let mut len = (sound.frames() as u64).saturating_sub(v.skip);
        if let Some(ms) = len_ms {
            let cut = (ms / 1000.0 * rate + frame.min(0.0)).round().max(0.0) as u64;
            if cut < len {
                len = cut;
                v.fade_out = true;
            }
        }
        if len == 0 {
            return;
        }
        v.len = len;
        let _ = self.tx.push(Command::At(v));
    }

    pub fn play_now(&mut self, sound: &Sound, gain: f32, bus: Bus) {
        self.play_tagged(sound, gain, 0, bus);
    }

    /// 叩いた音をすぐ鳴らす。左右 `pan` と、止めるときの目印 `tag`（ビットの組。0 は止めない）を付ける。
    /// 同じ音の重なりは [`MAX_OVERLAP`] まで
    pub fn play_hit(&mut self, sound: &Sound, (gain, pan): (f32, f32), tag: u8) {
        let _ = self.tx.push(Command::Now {
            sound: sound.clone(),
            gain,
            pan,
            tag,
            bus: Bus::Hits,
            limit: true,
        });
    }

    /// すぐ鳴らし、`tag`（ビットの組。1 以上）を付けておく。`stop` で止められる
    pub fn play_tagged(&mut self, sound: &Sound, gain: f32, tag: u8, bus: Bus) {
        let _ = self.tx.push(Command::Now {
            sound: sound.clone(),
            gain,
            pan: 0.0,
            tag,
            bus,
            limit: false,
        });
    }

    /// `tag` と同じビットを持つ音を止める（鳴らす予定でまだ鳴り始めていない音は取りやめる）
    pub fn stop(&mut self, tag: u8) {
        let _ = self.tx.push(Command::Stop(tag));
    }

    /// `tag` と同じビットを持つ、もう鳴り始めた音だけを止める（鳴らす予定の先の音は残す）
    pub fn stop_ringing(&mut self, tag: u8) {
        let _ = self.tx.push(Command::StopRinging(tag));
    }

    /// 曲の時刻 `song_ms` に、`tag` と同じビットを持ち、そのときまでに鳴り始めた音を止める
    /// （同じ時刻に [`play_chip`](Self::play_chip) で鳴らす音は止めない。フレームの丸め方をそろえてある）
    pub fn stop_at(&mut self, tag: u8, song_ms: f64) {
        let frame = self.frame_at(song_ms).max(0.0) as u64;
        let _ = self.tx.push(Command::StopAt { frame, tag });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xa_files_are_read_by_their_header_whatever_the_name() {
        // 8 ビット・モノラル・24000 Hz の BandJAM XA を 1 ブロック（32 サンプル）。0x40 は 16384
        let mut bytes = b"KWD1".to_vec();
        bytes.extend(33u32.to_le_bytes());
        bytes.extend(32u32.to_le_bytes());
        bytes.extend(24_000u16.to_le_bytes());
        bytes.extend([8, 1]);
        bytes.resize(32, 0);
        bytes.push(0x00);
        bytes.extend([0x40; 32]);
        // 名前が .ogg でも中身で見分ける
        let path = std::env::temp_dir().join(format!("nxt-xa-test-{}.ogg", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let s = load_sound(&path, 48_000);
        std::fs::remove_file(&path).unwrap();
        let s = s.unwrap();
        // 2 倍のサンプルレートにそろえ、ステレオにする
        assert_eq!(s.frames(), 64);
        assert_eq!(&s.samples()[..2], &[0.5, 0.5]);
    }

    #[test]
    fn mono_becomes_stereo() {
        assert_eq!(to_stereo(&[0.1, 0.2], 1), vec![0.1, 0.1, 0.2, 0.2]);
        assert_eq!(
            to_stereo(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6], 3),
            vec![0.1, 0.2, 0.4, 0.5]
        );
    }

    #[test]
    fn fades_only_at_cut_edges() {
        // フェードなし
        assert_eq!(envelope(0, 100, 10, false, false), 1.0);
        assert_eq!(envelope(99, 100, 10, false, false), 1.0);
        // 頭のフェード: 0 から上がる
        assert_eq!(envelope(0, 100, 10, true, false), 0.0);
        assert_eq!(envelope(5, 100, 10, true, false), 0.5);
        assert_eq!(envelope(10, 100, 10, true, false), 1.0);
        // 終わりのフェード: 最後のフレームで 0
        assert_eq!(envelope(99, 100, 10, false, true), 0.0);
        assert_eq!(envelope(94, 100, 10, false, true), 0.5);
        assert_eq!(envelope(89, 100, 10, false, true), 1.0);
    }

    #[test]
    fn a_voice_plays_only_its_part() {
        // 音は 0, 1, 2, … の値のフレーム。2 フレーム目から 3 フレームを、出力の 1 フレーム目から
        let sound = Sound(Arc::from(
            (0..8)
                .flat_map(|i| [i as f32, i as f32])
                .collect::<Vec<_>>(),
        ));
        let v = Voice {
            sound,
            start: 1,
            skip: 2,
            len: 3,
            gain: 1.0,
            pan: 0.0,
            fade_in: false,
            fade_out: false,
            tag: 0,
            bus: Bus::Song,
            overlap: Overlap::Free,
        };
        let mut out = vec![0.0; 6 * 2];
        v.mix(&mut out, 2, (0, 6), 0, (1.0, 1.0));
        assert_eq!(out, vec![0., 0., 2., 2., 3., 3., 4., 4., 0., 0., 0., 0.]);
        // 出力のバッファの途中から（4 フレーム目〜）
        let mut out = vec![0.0; 2 * 2];
        v.mix(&mut out, 2, (3, 2), 0, (1.0, 1.0));
        assert_eq!(out, vec![4., 4., 0., 0.]);
    }

    /// `frames` フレームの無音の音
    fn silence(frames: usize) -> Sound {
        Sound(Arc::from(vec![0.0f32; frames * 2]))
    }

    fn voice(sound: &Sound, start: u64, tag: u8) -> Voice {
        let mut v = Voice::whole(sound.clone(), start, 1.0, Bus::Song);
        v.tag = tag;
        v
    }

    fn lens(m: &Mixer) -> Vec<u64> {
        m.voices.iter().map(|v| v.len).collect()
    }

    #[test]
    fn stopping_ringing_voices_keeps_the_ones_to_come() {
        let s = silence(1000);
        let mut m = Mixer::new(10);
        m.command(Command::At(voice(&s, 0, 1)), 0);
        // 先で鳴らす予定の音は止めない
        m.command(Command::At(voice(&s, 500, 1)), 0);
        m.command(Command::At(voice(&s, 0, 2)), 0);
        m.command(Command::At(voice(&s, 0, 1 | 4)), 0);
        m.command(Command::StopRinging(1), 100);
        assert_eq!(lens(&m), vec![110, 1000, 1000, 110]);
        assert!(m.voices[0].fade_out && !m.voices[1].fade_out);
    }

    #[test]
    fn stop_also_drops_the_voices_to_come() {
        let s = silence(1000);
        let mut m = Mixer::new(10);
        m.command(Command::At(voice(&s, 0, 2)), 0);
        m.command(Command::At(voice(&s, 500, 2)), 0);
        m.command(Command::At(voice(&s, 500, 1)), 0);
        m.command(Command::Stop(2), 100);
        assert_eq!(lens(&m), vec![110, 1000]);
        assert_eq!(m.voices[1].tag, 1);
    }

    #[test]
    fn a_scheduled_stop_cuts_what_started_before_it() {
        let s = silence(1000);
        let mut m = Mixer::new(10);
        m.command(Command::At(voice(&s, 0, 1)), 0);
        m.command(Command::StopAt { frame: 300, tag: 1 }, 0);
        // 止める予定の前に叩いた音も止まり、止める時刻ちょうどから後の音は止まらない
        m.command(
            Command::Now {
                sound: s.clone(),
                gain: 1.0,
                pan: 0.0,
                tag: 1,
                bus: Bus::Hits,
                limit: false,
            },
            200,
        );
        m.command(Command::At(voice(&s, 300, 1)), 200);
        m.command(Command::At(voice(&s, 400, 1)), 200);
        m.command(Command::At(voice(&s, 0, 2)), 200);
        m.prepare(0, 256);
        assert_eq!(lens(&m), vec![1000; 5]);
        m.prepare(256, 256);
        assert_eq!(lens(&m), vec![310, 110, 1000, 1000, 1000]);
        assert!(m.stops.is_empty());
    }

    #[test]
    fn cancelling_and_seeking_drop_scheduled_stops() {
        let mut m = Mixer::new(10);
        m.command(Command::StopAt { frame: 300, tag: 1 }, 0);
        m.command(Command::StopAt { frame: 600, tag: 1 }, 0);
        m.command(Command::CancelFrom(500), 0);
        assert_eq!(m.stops, vec![(300, 1)]);
        m.command(Command::StopAll, 0);
        assert!(m.stops.is_empty());
    }

    #[test]
    fn a_fifth_overlap_of_the_same_chip_cuts_the_oldest() {
        let s = silence(1000);
        let other = silence(1000);
        let mut m = Mixer::new(10);
        for start in [0, 10, 20, 30, 40] {
            let mut v = voice(&s, start, 0);
            v.overlap = Overlap::Pending;
            m.command(Command::At(v), 0);
        }
        // 別の音と、数えない音（クリックなど）は数に入れない
        let mut v = voice(&other, 5, 0);
        v.overlap = Overlap::Pending;
        m.command(Command::At(v), 0);
        m.command(Command::At(voice(&s, 15, 0)), 0);
        m.prepare(0, 35);
        // 4 つまでは止めない（40 フレーム目の音はまだ鳴り始めていない）
        assert_eq!(lens(&m), vec![1000; 7]);
        m.prepare(35, 35);
        assert_eq!(lens(&m), vec![50, 1000, 1000, 1000, 1000, 1000, 1000]);
        assert!(m.voices.iter().all(|v| v.overlap != Overlap::Pending));
    }

    #[test]
    fn the_bus_volume_scales_each_voice_and_glides_when_it_changes() {
        let v = Voice::whole(Sound(Arc::from(vec![1.0f32; 8])), 0, 0.5, Bus::Hits);
        // 系統の音量 80 % なら、音の音量 0.5 にさらにかける
        let mut out = vec![0.0; 4 * 2];
        v.mix(&mut out, 2, (0, 4), 0, (0.8, 0.8));
        assert!(out.iter().all(|&s| (s - 0.4).abs() < 1e-6));
        // 100 % から 0 % へ変えたバッファは、4 フレームで下げきる（プツッと切らない）
        let mut out = vec![0.0; 4 * 2];
        v.mix(&mut out, 2, (0, 4), 0, (1.0, 0.0));
        assert_eq!(out, vec![0.375, 0.375, 0.25, 0.25, 0.125, 0.125, 0.0, 0.0]);
    }

    #[test]
    fn bus_volumes_are_kept_per_bus() {
        set_bus_volume(Bus::Click, 40);
        set_bus_volume(Bus::Se, 250);
        let g = bus_gains();
        assert_eq!(g[Bus::Click.index()], 0.4);
        assert_eq!(g[Bus::Se.index()], 1.0);
        set_bus_volume(Bus::Click, 100);
    }

    #[test]
    fn pausing_fades_the_edge_of_the_buffer() {
        let mut down = vec![1.0f32; 8];
        fade_buffer(&mut down, 2, false);
        assert_eq!(down, vec![1.0, 1.0, 0.75, 0.75, 0.5, 0.5, 0.25, 0.25]);
        let mut up = vec![1.0f32; 4];
        fade_buffer(&mut up, 2, true);
        assert_eq!(up, vec![0.0, 0.0, 0.5, 0.5]);
    }

    #[test]
    fn pan_lowers_the_other_side() {
        assert_eq!(pan_gains(0.0), (1.0, 1.0));
        assert_eq!(pan_gains(-0.5), (1.0, 0.5));
        assert_eq!(pan_gains(1.0), (0.0, 1.0));
        assert_eq!(pan_gains(-3.0), (1.0, 0.0));
        let v = Voice {
            pan: 0.5,
            ..Voice::whole(Sound(Arc::from(vec![1.0f32; 4])), 0, 1.0, Bus::Song)
        };
        let mut out = vec![0.0; 4];
        v.mix(&mut out, 2, (0, 2), 0, (1.0, 1.0));
        assert_eq!(out, vec![0.5, 1.0, 0.5, 1.0]);
    }

    #[test]
    fn master_volume_scales_then_clips() {
        // 50 % なら半分。混ぜて 1 を超えた所も、半分にしてから収める
        let mut out = vec![0.4, -0.4, 1.6, -3.0];
        apply_master(&mut out, 2, 0.5, 0.5);
        assert_eq!(out, vec![0.2, -0.2, 0.8, -1.0]);
        // 変わったときはバッファの中で少しずつ。最後のフレームで新しい音量になる
        let mut out = vec![1.0; 4 * 2];
        apply_master(&mut out, 2, 1.0, 0.5);
        assert_eq!(out, vec![0.875, 0.875, 0.75, 0.75, 0.625, 0.625, 0.5, 0.5]);
        // モノラルの出力でも
        let mut out = vec![1.0, 1.0];
        apply_master(&mut out, 1, 0.0, 0.0);
        assert_eq!(out, vec![0.0, 0.0]);
    }

    #[test]
    fn clicks_are_short_and_start_silent() {
        let c = click(48_000, true);
        assert_eq!(c.frames(), 1920);
        assert_eq!(c.0[0], 0.0);
        assert!(c.0.iter().all(|s| s.abs() <= 0.7));
    }

    #[test]
    fn resample_doubles_length() {
        let src = [0.0, 0.0, 1.0, 1.0];
        let out = resample_linear(&src, 22_050, 44_100);
        assert_eq!(out.len(), 8);
        assert_eq!(&out[..4], &[0.0, 0.0, 0.5, 0.5]);
    }
}
