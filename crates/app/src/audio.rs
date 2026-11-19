//! 音声出力。音を読み込んで出力の形式（ステレオ、出力のサンプルレート）にそろえ、
//! 決めたフレームに鳴らす。出力のコールバックごとに「このバッファが鳴るホスト時刻」を時計に渡す。

use std::path::Path;
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
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::time::{duration_us, host_us};

/// 出力の形式にそろえた音（ステレオで交互に並んだ f32）
#[derive(Clone)]
pub struct Sound(Arc<[f32]>);

impl Sound {
    pub fn frames(&self) -> usize {
        self.0.len() / 2
    }

    /// 長さ（ミリ秒）。`rate` は出力のサンプルレート
    pub fn ms(&self, rate: u32) -> f64 {
        self.frames() as f64 * 1000.0 / f64::from(rate)
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

/// 音のファイルを読み、ステレオ・`rate` Hz にそろえる
pub fn load_sound(path: &Path, rate: u32) -> Result<Sound> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .with_context(|| format!("probe {}", path.display()))?;
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
const FADE_SECONDS: f64 = 0.005;

enum Command {
    /// 決めたフレームから鳴らす（曲の BGM など）
    At(Voice),
    /// できるだけ早く鳴らす（叩いた音）
    Now { sound: Sound, gain: f32 },
}

/// 鳴らす音 1 つ。音の `skip` フレーム目から `len` フレームを、出力の `start` フレーム目から鳴らす
struct Voice {
    sound: Sound,
    start: u64,
    skip: u64,
    len: u64,
    gain: f32,
    /// 途中から始めたので頭にフェードをかける
    fade_in: bool,
    /// 途中で切るので終わりにフェードをかける
    fade_out: bool,
}

impl Voice {
    fn whole(sound: Sound, start: u64, gain: f32) -> Self {
        let len = sound.frames() as u64;
        Self {
            sound,
            start,
            skip: 0,
            len,
            gain,
            fade_in: false,
            fade_out: false,
        }
    }

    fn end(&self) -> u64 {
        self.start + self.len
    }

    /// 出力の `frames_done` フレーム目から `n` フレームぶん（`data`）に足す。`fade` はフェードのフレーム数
    fn mix(&self, data: &mut [f32], channels: usize, frames_done: u64, n: u64, fade: u64) {
        if self.start >= frames_done + n || self.end() <= frames_done {
            return;
        }
        let from = frames_done.max(self.start);
        let to = (frames_done + n).min(self.end());
        for f in from..to {
            let i = f - self.start;
            let g = self.gain * envelope(i, self.len, fade, self.fade_in, self.fade_out);
            let src = ((self.skip + i) * 2) as usize;
            let dst = ((f - frames_done) as usize) * channels;
            data[dst] += self.sound.0[src] * g;
            if channels > 1 {
                data[dst + 1] += self.sound.0[src + 1] * g;
            }
        }
    }
}

/// 長さ `len` の音の `i` フレーム目の音量（0〜1）。切れ目に `fade` フレームの直線のフェード
fn envelope(i: u64, len: u64, fade: u64, fade_in: bool, fade_out: bool) -> f32 {
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
    _stream: cpal::Stream,
    tx: rtrb::Producer<Command>,
    clock: Arc<Mutex<AudioClock>>,
    rate: u32,
    start_ms: f64,
    pub device_name: String,
}

impl AudioOut {
    /// 既定の出力デバイスを開く。`start_ms` は 0 フレーム目が曲の何ミリ秒にあたるか
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
        let mut voices: Vec<Voice> = Vec::with_capacity(1024);
        let mut frames_done: u64 = 0;
        let fade = (f64::from(rate) * FADE_SECONDS) as u64;
        let stream = device.build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let now = host_us();
                let ts = info.timestamp();
                let play_us = now + duration_us(ts.playback.duration_since(ts.callback));
                while let Ok(cmd) = rx.pop() {
                    match cmd {
                        Command::At(v) => voices.push(v),
                        Command::Now { sound, gain } => {
                            voices.push(Voice::whole(sound, frames_done, gain))
                        }
                    }
                }
                let n = (data.len() / channels) as u64;
                data.fill(0.0);
                for v in &voices {
                    v.mix(data, channels, frames_done, n, fade);
                }
                voices.retain(|v| v.end() > frames_done + n);
                for s in data.iter_mut() {
                    *s = s.clamp(-1.0, 1.0);
                }
                if let Ok(mut c) = cb_clock.try_lock() {
                    c.on_buffer(play_us, frames_done);
                }
                frames_done += n;
            },
            |err| log::warn!("audio stream: {err}"),
            None,
        )?;
        stream.play()?;
        log::info!("audio: {device_name}, {rate} Hz, {channels} ch");
        Ok(Self {
            _stream: stream,
            tx,
            clock,
            rate,
            start_ms,
            device_name,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// ホスト時刻 `host_us` の曲の時刻
    pub fn song_ms_at(&self, host_us: i64) -> Option<f64> {
        self.clock.lock().ok()?.song_ms_at(host_us)
    }

    /// 時刻 `ms` が出力の何フレーム目か（出力を始める前ならマイナス）
    fn frame_at(&self, ms: f64) -> f64 {
        (ms - self.start_ms) / 1000.0 * f64::from(self.rate)
    }

    /// 曲の時刻 `song_ms` に鳴らす
    pub fn play_at(&mut self, sound: &Sound, song_ms: f64, gain: f32) {
        let frame = self.frame_at(song_ms).max(0.0) as u64;
        let _ = self
            .tx
            .push(Command::At(Voice::whole(sound.clone(), frame, gain)));
    }

    /// 音の一部を、置いたとおりに鳴らす（練習の周の BGM）。途中から始めるときと途中で切るときは短いフェードをかける
    pub fn play_placed(&mut self, sound: &Sound, p: &Placement, gain: f32) {
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
            fade_in,
            fade_out: p.cut_end,
        }));
    }

    pub fn play_now(&mut self, sound: &Sound, gain: f32) {
        let _ = self.tx.push(Command::Now {
            sound: sound.clone(),
            gain,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            fade_in: false,
            fade_out: false,
        };
        let mut out = vec![0.0; 6 * 2];
        v.mix(&mut out, 2, 0, 6, 0);
        assert_eq!(out, vec![0., 0., 2., 2., 3., 3., 4., 4., 0., 0., 0., 0.]);
        // 出力のバッファの途中から（4 フレーム目〜）
        let mut out = vec![0.0; 2 * 2];
        v.mix(&mut out, 2, 3, 2, 0);
        assert_eq!(out, vec![4., 4., 0., 0.]);
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
