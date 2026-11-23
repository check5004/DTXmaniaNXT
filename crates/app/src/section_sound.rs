//! 練習メニューで、選んでいる所の音（BGM とドラム）を流すための、曲の音。
//!
//! 曲の音をすべて読むのは時間がかかるので、メニューを開いたら別のスレッドで読む（読み終えるまでは絵だけ）。
//! 区間の音は、自動で鳴らす音とドラムの音符の音を 1 本にまとめて作る（[`mix_bgm`]）。これも別のスレッドで作る。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dtx::TempoMap;
use engine::practice::Section;

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
    /// 小節の頭の時刻（ミリ秒）。最後は曲の終わり
    measure_ms: Vec<f64>,
    rate: u32,
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
        let measure_ms = (0..=tempo.measure_count())
            .map(|m| tempo.measure_start_ms(m))
            .collect();
        Ok(Self {
            voices: chart_sounds.auto,
            sounds,
            measure_ms,
            rate,
        })
    }

    /// 区間 `s` の音（頭の小節の頭から、終わりの小節の終わりまで）
    pub fn section(&self, s: Section) -> Option<Sound> {
        let from = *self.measure_ms.get(s.first as usize)?;
        let to = *self.measure_ms.get(s.last as usize + 1)?;
        if to <= from {
            return None;
        }
        Some(Sound::from_samples(mix_bgm(
            &self.voices,
            &self.sounds,
            self.rate,
            from,
            to,
        )))
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
    mixing: Option<(Section, Job<Option<Sound>>)>,
    /// 流している区間
    playing: Option<Section>,
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
            && self.playing != Some(s)
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
                sfx.play_section(sound);
                self.playing = Some(s);
            }
        }
        if self.off {
            practice_menu::Sound::Off
        } else if self.sounds.is_none() {
            practice_menu::Sound::Loading
        } else {
            practice_menu::Sound::Playing(
                self.playing
                    .filter(|&s| Some(s) == shown)
                    .and_then(|_| sfx.section_position()),
            )
        }
    }
}
