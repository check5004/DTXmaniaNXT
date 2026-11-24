//! 練習メニューで、選んでいる所の音（BGM とドラム）を流すための、曲の音。
//!
//! 曲の音をすべて読むのは時間がかかるので、メニューを開いたら別のスレッドで読む（読み終えるまでは絵だけ）。
//! 区間の音は、自動で鳴らす音とドラムの音符の音を 1 本にまとめて作る（[`mix_bgm`]）。これも別のスレッドで作る。
//! 練習と同じく、区間の頭の 1 小節前（助走）から、4 拍のカウントのクリックを付けて流す
//! （区間だけを流すと、練習を始めたときに助走の分だけずれて聞こえる。2026-11-24 ユーザー指摘）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dtx::TempoMap;
use engine::practice::{Section, SectionTimes};

use crate::audio::Sound;
use crate::chart_sound::{AutoMix, AutoVoice, ChartSounds};
use crate::game::load_sounds;
use crate::loading::Job;
use crate::practice_menu;
use crate::sfx::Sfx;
use crate::stretch::mix_bgm;

/// 区間を選んでから音を流し始めるまでの待ち（HT・LT で続けて動かしている間は流さない）
const SETTLE: Duration = Duration::from_millis(300);

/// 1 曲の音
pub struct SongSounds {
    /// 鳴らす音（BGM・効果音と、ドラムの音符。時刻の順）
    voices: Vec<AutoVoice>,
    sounds: HashMap<u16, Sound>,
    tempo: TempoMap,
    rate: u32,
}

/// 区間を流す音
pub struct SectionSound {
    /// 助走の頭から区間の終わりまでの音
    pub sound: Sound,
    /// カウントのクリックの時刻（音の頭からのミリ秒。最初が 1 拍目）
    pub clicks_ms: Vec<f64>,
    /// 音の中で区間が始まる所（頭から 0〜1）
    pub lead: f32,
}

impl SongSounds {
    /// 譜面と、その音をすべて読む（時間がかかる）。`rate` は出力のサンプリング周波数
    pub fn load(path: &Path, rate: u32, auto: AutoMix) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let chart = dtx::load(&bytes);
        let tempo = TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        let mut chart_sounds = ChartSounds::mixed(&chart, &tempo, auto);
        chart_sounds.add_auto_notes(notes.iter().map(|n| (n.ms, n.wav, n.lane)));
        chart_sounds.mute_hats();
        let sounds = load_sounds(
            &chart,
            &chart_sounds,
            path.parent().unwrap_or(Path::new(".")),
            rate,
        );
        Ok(Self {
            voices: chart_sounds.auto,
            sounds,
            tempo,
            rate,
        })
    }

    /// 区間 `s` の音。練習の最初の周と同じく、助走（区間の 1 小節前か 4 拍前の早いほう）から区間の終わりまで
    pub fn section(&self, s: Section) -> Option<SectionSound> {
        if s.last as usize >= self.tempo.measure_count() {
            return None;
        }
        let t = SectionTimes::new(&self.tempo, s);
        let (from, to) = (t.run_up_ms, t.end_ms);
        if to <= t.start_ms || t.start_ms < from {
            return None;
        }
        let sound = Sound::from_samples(mix_bgm(&self.voices, &self.sounds, self.rate, from, to));
        Some(SectionSound {
            sound,
            clicks_ms: t.count_in_ms.iter().map(|c| c - from).collect(),
            lead: ((t.start_ms - from) / (to - from)) as f32,
        })
    }
}

/// 練習メニューで選んでいる所の音を、曲の音を読む所から流す所まで受け持つ
#[derive(Default)]
pub struct SectionAudio {
    /// 音を用意している曲
    path: Option<PathBuf>,
    loading: Option<Job<SongSounds>>,
    sounds: Option<Arc<SongSounds>>,
    /// 音が出せない（出力が無い、読めなかった）
    off: bool,
    /// 選んでいる区間と、それを選んだ時刻
    wanted: Option<(Section, Instant)>,
    /// 区間の音を作っている
    mixing: Option<(Section, Job<Option<SectionSound>>)>,
    /// 流している区間と、音の中で区間が始まる所
    playing: Option<(Section, f32)>,
}

impl SectionAudio {
    /// 曲 `path` の音を別のスレッドで読み始める（同じ曲をもう読んでいれば何もしない）
    pub fn prepare(&mut self, path: &Path, rate: Option<u32>, auto: AutoMix) {
        if self.path.as_deref() == Some(path) {
            return;
        }
        *self = Self {
            path: Some(path.to_owned()),
            ..Self::default()
        };
        let Some(rate) = rate else {
            self.off = true;
            return;
        };
        let path = path.to_owned();
        self.loading = Some(Job::spawn("section sounds", move || {
            SongSounds::load(&path, rate, auto)
        }));
    }

    /// 読んだ音を捨てる（選曲に戻ったとき）
    pub fn forget(&mut self, sfx: &mut Sfx) {
        self.stop(sfx);
        *self = Self::default();
    }

    /// 流すのをやめる（読んだ音は残す。練習から戻ったらすぐ流せるように）
    pub fn stop(&mut self, sfx: &mut Sfx) {
        if self.playing.take().is_some() {
            sfx.stop_section();
        }
        self.wanted = None;
        self.mixing = None;
    }

    /// 毎フレーム呼ぶ。`shown` はメニューの絵に出している区間。選んでから少し経ったらその区間の音を流す
    pub fn update(&mut self, sfx: &mut Sfx, shown: Option<Section>) -> practice_menu::Sound {
        if let Some(job) = &self.loading
            && let Some(r) = job.poll()
        {
            self.loading = None;
            match r {
                Ok(s) => self.sounds = Some(Arc::new(s)),
                Err(e) => {
                    log::warn!("練習メニューの音を読めませんでした: {e:#}");
                    self.off = true;
                }
            }
        }
        // 選び直したら、今の音はすぐ止め、少し待ってから新しい区間の音を作る
        if shown != self.wanted.map(|w| w.0) {
            if self.playing.take().is_some() {
                sfx.stop_section();
            }
            self.mixing = None;
            self.wanted = shown.map(|s| (s, Instant::now()));
        }
        if let (Some((s, at)), Some(sounds)) = (self.wanted, &self.sounds)
            && self.playing.map(|p| p.0) != Some(s)
            && self.mixing.is_none()
            && at.elapsed() >= SETTLE
        {
            let sounds = Arc::clone(sounds);
            self.mixing = Some((s, Job::spawn("section mix", move || Ok(sounds.section(s)))));
        }
        if let Some((s, job)) = &self.mixing
            && let Some(r) = job.poll()
        {
            let s = *s;
            self.mixing = None;
            if let Ok(Some(sound)) = r {
                sfx.play_section(sound.sound, &sound.clicks_ms);
                self.playing = Some((s, sound.lead));
            }
        }
        if self.off {
            practice_menu::Sound::Off
        } else if self.sounds.is_none() {
            practice_menu::Sound::Loading
        } else {
            // 白い線は区間の中だけ（助走のあいだは出さない）
            practice_menu::Sound::Playing(
                self.playing
                    .filter(|&(s, _)| Some(s) == shown)
                    .and_then(|(_, lead)| in_section(sfx.section_position()?, lead)),
            )
        }
    }
}

/// 助走から流す音の位置 `pos`（頭から 0〜1）を、区間の頭からの位置（0〜1）に直す。助走のあいだは None
fn in_section(pos: f32, lead: f32) -> Option<f32> {
    if pos < lead || lead >= 1.0 {
        return None;
    }
    Some((pos - lead) / (1.0 - lead))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BPM 120、4/4 で小節 0〜7 に 4 分の SD（音は鳴らさない）
    fn sounds() -> SongSounds {
        let mut text = String::from("#BPM: 120\n");
        for m in 0..8 {
            text.push_str(&format!("#{m:03}12: 01010101\n"));
        }
        let chart = dtx::parse(&text);
        SongSounds {
            voices: Vec::new(),
            sounds: HashMap::new(),
            tempo: TempoMap::build(&chart),
            rate: 1000,
        }
    }

    #[test]
    fn a_section_plays_from_the_bar_before_with_the_count_in() {
        // 小節 4〜5 は 8000〜12000ms。助走は小節 3 の頭（6000ms）から
        let s = sounds().section(Section::new(4, 5)).unwrap();
        assert_eq!(s.sound.frames(), 6000);
        assert_eq!(s.clicks_ms, vec![0.0, 500.0, 1000.0, 1500.0]);
        assert!((s.lead - 2000.0 / 6000.0).abs() < 1e-6);
    }

    #[test]
    fn the_first_bar_counts_in_before_the_song() {
        // 曲の頭の前は、テンポを延ばして 4 拍数える
        let s = sounds().section(Section::new(0, 0)).unwrap();
        assert_eq!(s.sound.frames(), 4000);
        assert_eq!(s.clicks_ms, vec![0.0, 500.0, 1000.0, 1500.0]);
        assert!((s.lead - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_section_past_the_end_has_no_sound() {
        assert!(sounds().section(Section::new(7, 9)).is_none());
    }

    #[test]
    fn the_white_line_shows_only_inside_the_section() {
        assert_eq!(in_section(0.1, 0.25), None);
        assert_eq!(in_section(0.25, 0.25), Some(0.0));
        assert_eq!(in_section(0.625, 0.25), Some(0.5));
        assert_eq!(in_section(0.5, 1.0), None);
    }
}
