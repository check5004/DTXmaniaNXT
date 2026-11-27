//! 練習の区間の BGM を 1 本の音にまとめ、周の速度に合わせて伸ばす。
//!
//! 伸ばすのは重い（数秒の音に数百ミリ秒）ので別のスレッドで行い、できた速度の音から周に使う。
//! 音を出すコールバックの中では伸ばさない。叩いた音とクリックは伸ばさない。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;

use crate::audio::{FADE_SECONDS, MAX_OVERLAP, Sound, envelope, pan_gains};
use crate::chart_sound::AutoVoice;

/// 遅くするときの音程
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pitch {
    /// 音程を保つ
    #[default]
    Keep,
    /// テープのように音程ごと遅くする（音はにじまないが低くなる）
    Tape,
}

impl Pitch {
    pub const ALL: [Pitch; 2] = [Self::Keep, Self::Tape];

    /// 記録や設定に書く名前
    pub fn name(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Tape => "tape",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// 自動で鳴らす音（BGM・効果音など）を、曲の時刻 `[from_ms, to_ms)` の 1 本の音（ステレオ、`rate` Hz）にまとめる。
/// 音ごとの音量・左右・止める時刻も入れる。範囲より前から鳴っている音は途中から入れ、範囲の終わりで切る。
/// 出力と同じく、同じ音の重なりは [`MAX_OVERLAP`] まで（超えたら一番古いものを止める）
pub fn mix_bgm(
    voices: &[AutoVoice],
    sounds: &HashMap<u16, Sound>,
    rate: u32,
    from_ms: f64,
    to_ms: f64,
) -> Vec<f32> {
    let per_ms = f64::from(rate) / 1000.0;
    let frames = ((to_ms - from_ms) * per_ms).round().max(0.0) as usize;
    let fade = (f64::from(rate) * FADE_SECONDS) as u64;
    let mut out = vec![0.0f32; frames * 2];
    // 音ごとの鳴らし始め（フレーム）、長さ、終わりで切るか
    let mut plan: Vec<Option<(i64, u64, bool)>> = voices
        .iter()
        .map(|v| {
            let s = sounds.get(&v.wav)?;
            let start = ((v.ms - from_ms) * per_ms).round() as i64;
            let mut len = s.frames() as u64;
            let mut cut = false;
            if let Some(ms) = v.len_ms {
                let l = (ms * per_ms).round().max(0.0) as u64;
                if l < len {
                    len = l;
                    cut = true;
                }
            }
            Some((start, len, cut))
        })
        .collect();
    let mut order: Vec<usize> = (0..voices.len()).filter(|&i| plan[i].is_some()).collect();
    order.sort_by_key(|&i| plan[i].map(|p| p.0));
    // 音ごとの、まだ鳴っている音（`voices` の添え字、鳴らし始めの順）
    let mut ringing: HashMap<u16, Vec<usize>> = HashMap::new();
    for i in order {
        let Some((start, _, _)) = plan[i] else {
            continue;
        };
        let list = ringing.entry(voices[i].wav).or_default();
        list.retain(|&j| plan[j].is_some_and(|(s, l, _)| s + l as i64 > start));
        if list.len() >= MAX_OVERLAP {
            for j in list.drain(..=list.len() - MAX_OVERLAP) {
                if let Some((s, len, cut)) = plan[j].as_mut() {
                    let l = (start - *s).max(0) as u64 + fade;
                    if l < *len {
                        *len = l;
                        *cut = true;
                    }
                }
            }
        }
        list.push(i);
    }
    for (v, p) in voices.iter().zip(&plan) {
        let (Some(s), Some((start, len, cut))) = (sounds.get(&v.wav), *p) else {
            continue;
        };
        let from = start.max(0);
        let to = (start + len as i64).min(frames as i64);
        if from >= to {
            continue;
        }
        let (left, right) = pan_gains(v.pan);
        let src = s.samples();
        for f in from..to {
            let i = (f - start) as u64;
            let g = v.gain * envelope(i, len, fade, false, cut);
            let (si, di) = (i as usize * 2, f as usize * 2);
            out[di] += src[si] * g * left;
            out[di + 1] += src[si + 1] * g * right;
        }
    }
    out
}

/// 速度 `percent` の音の長さ（フレーム）。もとの長さ `frames` の 100 / percent 倍
pub fn stretched_frames(frames: usize, percent: u32) -> usize {
    (frames as f64 * 100.0 / f64::from(percent.max(1))).round() as usize
}

/// `src`（ステレオ、`rate` Hz）を速度 `percent` に伸ばす。長さはちょうど `stretched_frames` になる。
/// 音程を保つ伸ばしができないときは、音程ごと遅くする
pub fn stretch(src: &[f32], rate: u32, percent: u32, pitch: Pitch) -> Vec<f32> {
    if percent == 100 {
        return src.to_vec();
    }
    let out_frames = stretched_frames(src.len() / 2, percent);
    match pitch {
        Pitch::Keep => match keep_pitch(src, rate, percent) {
            // 伸ばした長さには 0.2% ほどの端数が出るので、ちょうどの長さにそろえる（音程は 3 セントほどしか動かない）
            Ok(v) => fit(&v, out_frames),
            Err(e) => {
                log::warn!("cannot keep the pitch at {percent}%: {e}");
                fit(src, out_frames)
            }
        },
        Pitch::Tape => fit(src, out_frames),
    }
}

fn keep_pitch(src: &[f32], rate: u32, percent: u32) -> Result<Vec<f32>, timestretch::SessionError> {
    let tempo = timestretch::TempoRate::new(f64::from(percent) / 100.0)?;
    let out = timestretch::render(
        src,
        timestretch::SessionConfig {
            source_sample_rate: rate,
            output_sample_rate: rate,
            channels: 2,
            parameters: timestretch::Parameters::keylock(tempo),
            ..Default::default()
        },
    )?;
    Ok(out.samples)
}

/// ちょうど `out_frames` フレームに伸び縮みさせる（3 次の補間）。音程も同じ割合で変わる
fn fit(src: &[f32], out_frames: usize) -> Vec<f32> {
    let frames = src.len() / 2;
    if frames == out_frames {
        return src.to_vec();
    }
    if frames == 0 {
        return vec![0.0; out_frames * 2];
    }
    let at = |i: i64, c: usize| src[(i.clamp(0, frames as i64 - 1) as usize) * 2 + c];
    let step = frames as f64 / out_frames.max(1) as f64;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let pos = i as f64 * step;
        let i1 = pos.floor() as i64;
        let t = (pos - i1 as f64) as f32;
        for c in 0..2 {
            // Catmull-Rom
            let (p0, p1, p2, p3) = (at(i1 - 1, c), at(i1, c), at(i1 + 1, c), at(i1 + 2, c));
            let v = p1
                + 0.5
                    * t
                    * (p2 - p0
                        + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3
                            + t * (3.0 * (p1 - p2) + p3 - p0)));
            out.push(v);
        }
    }
    out
}

/// 区間の BGM を 1 本にまとめた音と、速度ごとに伸ばした音
pub struct SectionAudio {
    /// 音の頭の曲の時刻
    pub from_ms: f64,
    /// 等速での長さ（ミリ秒）
    pub song_len_ms: f64,
    ready: HashMap<u32, Sound>,
    pending: HashSet<u32>,
    tx: mpsc::Sender<u32>,
    rx: mpsc::Receiver<(u32, Sound)>,
}

impl SectionAudio {
    /// まとめた音 `mix`（曲の時刻 `from_ms` から）を受け取る。`first`（%）の速度の音はここで作り、
    /// ほかの速度は `request` で別のスレッドに頼む
    pub fn new(mix: Vec<f32>, from_ms: f64, rate: u32, pitch: Pitch, first: u32) -> Self {
        Self::build(mix, from_ms, rate, pitch, Some(first))
    }

    /// [`SectionAudio::new`] と同じだが、`first`（%）の速度の音も別のスレッドで作る（すぐには使わない区間の音）
    pub fn later(mix: Vec<f32>, from_ms: f64, rate: u32, pitch: Pitch, first: u32) -> Self {
        let mut a = Self::build(mix, from_ms, rate, pitch, None);
        a.request(first);
        a
    }

    fn build(mix: Vec<f32>, from_ms: f64, rate: u32, pitch: Pitch, now: Option<u32>) -> Self {
        let song_len_ms = (mix.len() / 2) as f64 * 1000.0 / f64::from(rate);
        let mix: Arc<[f32]> = Arc::from(mix);
        let mut ready = HashMap::new();
        if let Some(first) = now {
            ready.insert(
                first,
                Sound::from_samples(stretch(&mix, rate, first, pitch)),
            );
        }
        let (tx, jobs) = mpsc::channel::<u32>();
        let (done, rx) = mpsc::channel();
        let src = mix.clone();
        let spawned = std::thread::Builder::new()
            .name("stretch".into())
            .spawn(move || {
                // 練習が終わって受け手がいなくなったら止まる
                while let Ok(percent) = jobs.recv() {
                    let t = std::time::Instant::now();
                    let s = Sound::from_samples(stretch(&src, rate, percent, pitch));
                    log::info!(
                        "stretched the section to {percent}% in {:.0}ms",
                        t.elapsed().as_secs_f64() * 1000.0
                    );
                    if done.send((percent, s)).is_err() {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("cannot start the stretch thread: {e}");
        }
        Self {
            from_ms,
            song_len_ms,
            ready,
            pending: HashSet::new(),
            tx,
            rx,
        }
    }

    /// できあがった音を受け取る
    pub fn poll(&mut self) {
        while let Ok((percent, s)) = self.rx.try_recv() {
            self.pending.remove(&percent);
            self.ready.insert(percent, s);
        }
    }

    /// 速度 `percent` の音を作っておくよう頼む（できているか頼んであれば何もしない）
    pub fn request(&mut self, percent: u32) {
        if self.ready.contains_key(&percent) || !self.pending.insert(percent) {
            return;
        }
        if self.tx.send(percent).is_err() {
            self.pending.remove(&percent);
        }
    }

    /// 速度 `percent` の音（まだできていなければ None）
    pub fn get(&self, percent: u32) -> Option<&Sound> {
        self.ready.get(&percent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(frames: &[f32]) -> Sound {
        Sound::from_samples(frames.iter().flat_map(|&v| [v, v]).collect())
    }

    #[test]
    fn bgm_chips_are_mixed_at_their_times() {
        // 1000 Hz なので 1 フレーム 1ms
        let mut sounds = HashMap::new();
        sounds.insert(1, sound(&[1.0, 1.0, 1.0]));
        sounds.insert(2, sound(&[0.5; 10]));
        let v = |ms: f64, wav: u16| AutoVoice {
            ms,
            wav,
            gain: 1.0,
            pan: 0.0,
            len_ms: None,
            hat: None,
        };
        // 音 1 は 2ms から、音 2 は区間より前の -5ms から鳴っている
        let out = mix_bgm(
            &[v(2.0, 1), v(-5.0, 2), v(9.0, 1)],
            &sounds,
            1000,
            0.0,
            10.0,
        );
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, vec![0.5, 0.5, 1.5, 1.5, 1.5, 0.0, 0.0, 0.0, 0.0, 1.0]);
        // 区間の外の音と無い音は入らない
        let out = mix_bgm(&[v(20.0, 1), v(0.0, 9)], &sounds, 1000, 0.0, 4.0);
        assert!(out.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn auto_voices_keep_volume_pan_and_cut() {
        // 1000 Hz で 1 フレーム 1ms。フェードは 5 フレーム
        let mut sounds = HashMap::new();
        sounds.insert(1, sound(&[1.0; 20]));
        let voice = AutoVoice {
            ms: 0.0,
            wav: 1,
            gain: 0.5,
            pan: 0.5,
            len_ms: Some(10.0),
            hat: None,
        };
        let out = mix_bgm(&[voice], &sounds, 1000, 0.0, 20.0);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
        // 右へ振ったので左は半分。10ms で止め、終わりの 5 フレームを下げる
        assert_eq!(left[0], 0.25);
        assert_eq!(right[0], 0.5);
        assert_eq!(right[4], 0.5);
        assert!(right[7] < 0.5 && right[7] > 0.0);
        assert_eq!(right[9], 0.0);
        assert!(right[10..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn the_same_chip_overlaps_at_most_four_times() {
        // 1000 Hz で 1 フレーム 1ms。フェードは 5 フレーム。同じ音 1 を 0〜4ms に 5 回
        let mut sounds = HashMap::new();
        sounds.insert(1, sound(&[1.0; 100]));
        sounds.insert(2, sound(&[1.0; 100]));
        let v = |ms: f64, wav: u16| AutoVoice {
            ms,
            wav,
            gain: 1.0,
            pan: 0.0,
            len_ms: None,
            hat: None,
        };
        let five: Vec<AutoVoice> = (0..5).map(|i| v(f64::from(i), 1)).collect();
        let out = mix_bgm(&five, &sounds, 1000, 0.0, 30.0);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        // 5 回目（4ms）で一番古い音をフェードで止めるので、そのあとは 4 つぶん
        assert_eq!(left[3], 4.0);
        assert!(left[6] > 4.0 && left[6] < 5.0);
        assert_eq!(left[20], 4.0);
        // 別の音は数に入れない
        let mut mixed = five[..4].to_vec();
        mixed.push(v(4.0, 2));
        let out = mix_bgm(&mixed, &sounds, 1000, 0.0, 30.0);
        assert_eq!(out[20 * 2], 5.0);
    }

    /// 440Hz のサイン波（ステレオ、48kHz）
    fn sine(seconds: f64) -> Vec<f32> {
        let n = (48_000.0 * seconds) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (std::f64::consts::TAU * 440.0 * i as f64 / 48_000.0).sin() as f32 * 0.5;
                [v, v]
            })
            .collect()
    }

    /// ゼロを下から上へ横切る回数から出した周波数
    fn frequency(samples: &[f32]) -> f64 {
        let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        crossings as f64 / (left.len() as f64 / 48_000.0)
    }

    #[test]
    fn stretched_sounds_have_the_exact_length() {
        let src = sine(1.0);
        for percent in [70, 75, 100, 125] {
            for pitch in Pitch::ALL {
                let out = stretch(&src, 48_000, percent, pitch);
                assert_eq!(out.len() / 2, stretched_frames(48_000, percent));
            }
        }
        assert_eq!(stretched_frames(48_000, 70), 68_571);
    }

    #[test]
    fn keeping_the_pitch_keeps_the_pitch() {
        let src = sine(1.0);
        let keep = stretch(&src, 48_000, 70, Pitch::Keep);
        let tape = stretch(&src, 48_000, 70, Pitch::Tape);
        // 音程を保てば 440Hz のまま、テープなら 70% の 308Hz
        assert!(
            (frequency(&keep) - 440.0).abs() < 10.0,
            "{}",
            frequency(&keep)
        );
        assert!(
            (frequency(&tape) - 308.0).abs() < 10.0,
            "{}",
            frequency(&tape)
        );
    }

    #[test]
    fn section_audio_makes_speeds_in_the_background() {
        let mut a = SectionAudio::new(sine(0.5), -1000.0, 48_000, Pitch::Tape, 70);
        assert!((a.song_len_ms - 500.0).abs() < 1e-6);
        assert!(a.get(70).is_some() && a.get(75).is_none());
        a.request(75);
        a.request(75);
        let t = std::time::Instant::now();
        while a.get(75).is_none() && t.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            a.poll();
        }
        assert_eq!(a.get(75).map(|s| s.frames()), Some(32_000));
    }
}
