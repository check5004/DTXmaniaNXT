//! 音声出力。音を読み込んで出力の形式（ステレオ、出力のサンプルレート）にそろえ、
//! 決めたフレームに鳴らす。出力のコールバックごとに「このバッファが鳴るホスト時刻」を時計に渡す。

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engine::clock::AudioClock;
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

enum Command {
    /// 決めたフレームから鳴らす（曲の BGM など）
    At { sound: Sound, frame: u64, gain: f32 },
    /// できるだけ早く鳴らす（叩いた音）
    Now { sound: Sound, gain: f32 },
}

struct Voice {
    sound: Sound,
    start: u64,
    gain: f32,
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
        let stream = device.build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let now = host_us();
                let ts = info.timestamp();
                let play_us = now + duration_us(ts.playback.duration_since(ts.callback));
                while let Ok(cmd) = rx.pop() {
                    match cmd {
                        Command::At { sound, frame, gain } => voices.push(Voice {
                            sound,
                            start: frame,
                            gain,
                        }),
                        Command::Now { sound, gain } => voices.push(Voice {
                            sound,
                            start: frames_done,
                            gain,
                        }),
                    }
                }
                let n = (data.len() / channels) as u64;
                data.fill(0.0);
                for v in &voices {
                    let len = v.sound.frames() as u64;
                    if v.start >= frames_done + n || v.start + len <= frames_done {
                        continue;
                    }
                    let from = frames_done.max(v.start);
                    let to = (frames_done + n).min(v.start + len);
                    for f in from..to {
                        let src = ((f - v.start) * 2) as usize;
                        let dst = ((f - frames_done) as usize) * channels;
                        data[dst] += v.sound.0[src] * v.gain;
                        if channels > 1 {
                            data[dst + 1] += v.sound.0[src + 1] * v.gain;
                        }
                    }
                }
                voices.retain(|v| v.start + v.sound.frames() as u64 > frames_done + n);
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

    /// 曲の時刻 `song_ms` に鳴らす
    pub fn play_at(&mut self, sound: &Sound, song_ms: f64, gain: f32) {
        let frame = ((song_ms - self.start_ms) / 1000.0 * f64::from(self.rate)).max(0.0) as u64;
        let _ = self.tx.push(Command::At {
            sound: sound.clone(),
            frame,
            gain,
        });
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
    fn resample_doubles_length() {
        let src = [0.0, 0.0, 1.0, 1.0];
        let out = resample_linear(&src, 22_050, 44_100);
        assert_eq!(out.len(), 8);
        assert_eq!(&out[..4], &[0.0, 0.0, 0.5, 0.5]);
    }
}
