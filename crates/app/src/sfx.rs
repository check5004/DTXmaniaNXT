//! 画面の効果音（スキンの `Sounds` の ogg）と、選曲のプレビューの音。
//! 演奏の音とは別の出力で鳴らす。効果音は効果音の系統、プレビューは曲の音の系統の音量で鳴る（[`Bus`]）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use library::SortKind;

use crate::audio::{
    AudioOut, Bus, OutputLatency, Sound, click, click_gain, load_sound, load_sound_bytes,
};

/// 効果音の種類（上流 `CSkin` のシステムサウンド）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Se {
    /// カーソルを動かした
    Move,
    /// 決めた
    Decide,
    /// 戻った
    Cancel,
    /// 難易度などを変えた
    Change,
    /// 曲を読み込んでいる（読み込みの画面はこの音の長さだけ出す）
    NowLoading,
    /// ゲージが尽きた
    StageFailed,
    /// 結果の画面（最後まで遊んだ）
    StageClear,
    /// 結果の画面（POOR と MISS が無い）
    FullCombo,
    /// 結果の画面（全部 PERFECT）
    Excellent,
    /// 結果の画面（達成率が自己ベストを超えた）
    NewRecord,
    /// タイトルの画面に入った
    Title,
    /// タイトルで GAME START を決めた
    GameStart,
    /// 終わるとき
    GameEnd,
    /// 難易度を BASIC・ADVANCED・EXTREME・MASTER に変えた（v2 の HH 2 回。DTX は `Change`）
    Basic,
    Advanced,
    Extreme,
    Master,
    /// 並べ替えを変えた（v2 の `Graphics/Sorting/*.wav`）
    Sort(SortKind),
}

impl Se {
    pub const ALL: [Se; 26] = [
        Se::Move,
        Se::Decide,
        Se::Cancel,
        Se::Change,
        Se::NowLoading,
        Se::StageFailed,
        Se::StageClear,
        Se::FullCombo,
        Se::Excellent,
        Se::NewRecord,
        Se::Title,
        Se::GameStart,
        Se::GameEnd,
        Se::Basic,
        Se::Advanced,
        Se::Extreme,
        Se::Master,
        Se::Sort(SortKind::ALL[0]),
        Se::Sort(SortKind::ALL[1]),
        Se::Sort(SortKind::ALL[2]),
        Se::Sort(SortKind::ALL[3]),
        Se::Sort(SortKind::ALL[4]),
        Se::Sort(SortKind::ALL[5]),
        Se::Sort(SortKind::ALL[6]),
        Se::Sort(SortKind::ALL[7]),
        Se::Sort(SortKind::ALL[8]),
    ];

    /// 難易度（0〜4）を変えたときの音
    pub fn level(level: usize) -> Se {
        match level {
            0 => Se::Basic,
            1 => Se::Advanced,
            2 => Se::Extreme,
            3 => Se::Master,
            _ => Se::Change,
        }
    }

    pub fn file(self) -> &'static str {
        match self {
            Se::Move => "Sounds/Move.ogg",
            Se::Decide => "Sounds/Decide.ogg",
            Se::Cancel => "Sounds/Cancel.ogg",
            Se::Change => "Sounds/Change.ogg",
            Se::NowLoading => "Sounds/Now loading.ogg",
            Se::StageFailed => "Sounds/Stage failed.ogg",
            Se::StageClear => "Sounds/Stage clear.ogg",
            Se::FullCombo => "Sounds/Full combo.ogg",
            Se::Excellent => "Sounds/Excellent.ogg",
            Se::NewRecord => "Sounds/New Record.ogg",
            Se::Title => "Sounds/Title.ogg",
            Se::GameStart => "Sounds/Game start.ogg",
            Se::GameEnd => "Sounds/Game end.ogg",
            Se::Basic => "Sounds/Basic.ogg",
            Se::Advanced => "Sounds/Advanced.ogg",
            Se::Extreme => "Sounds/Extreme.ogg",
            Se::Master => "Sounds/Master.ogg",
            Se::Sort(k) => match k.icon() {
                "allmusic" => "Graphics/Sorting/allmusic.wav",
                "artist" => "Graphics/Sorting/artist.wav",
                "difficulty" => "Graphics/Sorting/difficulty.wav",
                "level" => "Graphics/Sorting/level.wav",
                "player" => "Graphics/Sorting/player.wav",
                "skill" => "Graphics/Sorting/skill.wav",
                "title" => "Graphics/Sorting/title.wav",
                _ => "Graphics/Sorting/recommended.wav",
            },
        }
    }

    /// 鳴らす大きさ（上流 v2 の並べ替えの音は 80）
    fn volume(self) -> f32 {
        match self {
            Se::Sort(_) => 0.8,
            _ => 1.0,
        }
    }
}

/// プレビューの音に付ける目印（目印はビットの組なので、目印ごとに別のビットにする）
const PREVIEW_TAG: u8 = 1;
/// 読み込みの音に付ける目印（読み込みをやめたら止める）
const LOADING_TAG: u8 = 2;
/// タイトルの音に付ける目印（上流と同じく、ほかの画面へ移ったり終わる音を鳴らしたら止める）
const TITLE_TAG: u8 = 4;
/// 練習メニューで流している区間の音に付ける目印
const SECTION_TAG: u8 = 8;
/// 区間の音をくり返すときの、終わりから次の頭までの間（ミリ秒）
const SECTION_GAP_MS: f64 = 600.0;
/// 区間の音を鳴らし始めるまでの余裕（ミリ秒）。出力のバッファより先に置く
const SECTION_LEAD_MS: f64 = 80.0;

pub struct Sfx {
    out: Option<AudioOut>,
    sounds: HashMap<Se, Sound>,
    /// 鳴らしている（読んでいる）プレビューの音のファイル
    preview: Option<PathBuf>,
    jobs: Sender<(PathBuf, u32)>,
    done: Receiver<(PathBuf, Option<Sound>)>,
    /// くり返し流している区間の音
    section: Option<SectionLoop>,
}

/// くり返し流している区間の音
struct SectionLoop {
    sound: Sound,
    /// 1 回の長さ（ミリ秒）
    len_ms: f64,
    /// カウントのクリック（音の頭からのミリ秒と、鳴らす音と音量。練習のカウントと同じ）
    clicks: Vec<(f64, Sound, f32)>,
    /// 出力の時刻で、鳴らし始めた（鳴らす予定の）頭。古い順
    starts: Vec<f64>,
}

impl Sfx {
    pub fn new() -> Self {
        let out = match AudioOut::open(0.0).and_then(|o| o.start().map(|_| o)) {
            Ok(o) => Some(o),
            Err(e) => {
                log::warn!("no sound for menus: {e:#}");
                None
            }
        };
        let (jobs, rx) = channel::<(PathBuf, u32)>();
        let (tx, done) = channel();
        std::thread::spawn(move || {
            for (path, rate) in rx {
                let s = load_sound(&path, rate)
                    .map_err(|e| log::warn!("{}: {e:#}", path.display()))
                    .ok();
                if tx.send((path, s)).is_err() {
                    break;
                }
            }
        });
        Self {
            out,
            sounds: HashMap::new(),
            preview: None,
            jobs,
            done,
            section: None,
        }
    }

    /// スキンの効果音を読む
    pub fn load(&mut self) {
        self.sounds.clear();
        let Some(rate) = self.out.as_ref().map(AudioOut::rate) else {
            return;
        };
        for se in Se::ALL {
            let Some(bytes) = crate::skin::read(se.file()) else {
                continue;
            };
            let ext = se.file().rsplit('.').next();
            match load_sound_bytes(bytes, ext, rate) {
                Ok(s) => {
                    self.sounds.insert(se, s);
                }
                Err(e) => log::warn!("skin: {}: {e:#}", se.file()),
            }
        }
    }

    /// 画面の音の出力の遅れ（FPS と遅延の表示へ）
    pub fn latency(&self) -> Option<OutputLatency> {
        self.out.as_ref().and_then(AudioOut::latency)
    }

    /// スキンにその音がある
    pub fn has(&self, se: Se) -> bool {
        self.sounds.contains_key(&se)
    }

    pub fn play(&mut self, se: Se) {
        if let (Some(out), Some(s)) = (&mut self.out, self.sounds.get(&se)) {
            out.play_now(s, se.volume(), Bus::Se);
        }
    }

    /// 目印を付けて鳴らし、その長さを返す（音が無ければ 0）
    fn play_tagged(&mut self, se: Se, tag: u8) -> Duration {
        let (Some(out), Some(s)) = (&mut self.out, self.sounds.get(&se)) else {
            return Duration::ZERO;
        };
        out.stop(tag);
        out.play_tagged(s, 1.0, tag, Bus::Se);
        Duration::from_secs_f64(s.frames() as f64 / f64::from(out.rate()))
    }

    /// 読み込みの音を鳴らし、その長さを返す（音が無ければ 0）
    pub fn play_loading(&mut self) -> Duration {
        self.stop_title();
        self.play_tagged(Se::NowLoading, LOADING_TAG)
    }

    /// タイトルの音を鳴らす（鳴っていれば頭から）
    pub fn play_title(&mut self) {
        self.play_tagged(Se::Title, TITLE_TAG);
    }

    pub fn stop_title(&mut self) {
        if let Some(out) = &mut self.out {
            out.stop(TITLE_TAG);
        }
    }

    /// 終わる音を鳴らし、その長さを返す（タイトルの音は止める）
    pub fn play_end(&mut self) -> Duration {
        self.stop_title();
        self.play_tagged(Se::GameEnd, TITLE_TAG)
    }

    /// 読み込みの音を止める
    pub fn stop_loading(&mut self) {
        if let Some(out) = &mut self.out {
            out.stop(LOADING_TAG);
        }
    }

    /// プレビューの音を鳴らす（同じファイルを鳴らしているなら何もしない）。None なら止める
    pub fn preview(&mut self, path: Option<PathBuf>) {
        if self.preview == path {
            return;
        }
        if let Some(out) = &mut self.out {
            out.stop(PREVIEW_TAG);
        }
        self.preview = path.clone();
        if let (Some(p), Some(out)) = (path, &self.out) {
            let _ = self.jobs.send((p, out.rate()));
        }
    }

    /// 出力のサンプリング周波数（出力が無ければ None）
    pub fn rate(&self) -> Option<u32> {
        self.out.as_ref().map(AudioOut::rate)
    }

    /// 出力の今の時刻（ミリ秒）
    fn out_now_ms(&self) -> Option<f64> {
        self.out.as_ref()?.song_ms_at(crate::time::host_us())
    }

    /// 区間の音をくり返し流す（前の区間の音とプレビューは止める）。曲の音の系統の音量で鳴る。
    /// `clicks_ms` はカウントのクリックの時刻（音の頭からのミリ秒、最初が 1 拍目）。メトロノームの系統の音量で鳴る
    pub fn play_section(&mut self, sound: Sound, clicks_ms: &[f64]) {
        self.stop_section();
        self.preview(None);
        let (Some(rate), Some(now)) = (self.rate(), self.out_now_ms()) else {
            return;
        };
        let len_ms = sound.frames() as f64 / f64::from(rate) * 1000.0;
        if len_ms <= 0.0 {
            return;
        }
        let clicks = clicks_ms
            .iter()
            .enumerate()
            .map(|(i, &ms)| (ms, click(rate, i == 0), click_gain(i == 0)))
            .collect();
        self.section = Some(SectionLoop {
            sound,
            len_ms,
            clicks,
            starts: Vec::new(),
        });
        self.queue_section(now + SECTION_LEAD_MS);
    }

    /// 区間の音を止める
    pub fn stop_section(&mut self) {
        if self.section.take().is_some()
            && let Some(out) = &mut self.out
        {
            out.stop(SECTION_TAG);
        }
    }

    /// 区間の音の、今鳴っている所（頭から 0〜1）。鳴っていない（くり返しの間）なら None
    pub fn section_position(&self) -> Option<f32> {
        let s = self.section.as_ref()?;
        let now = self.out_now_ms()?;
        let start = s.starts.iter().rev().find(|&&t| t <= now)?;
        let f = (now - start) / s.len_ms;
        (f < 1.0).then_some(f as f32)
    }

    /// 区間の音を時刻 `at`（出力の時刻）に鳴らす予定にする
    fn queue_section(&mut self, at: f64) {
        let (Some(out), Some(s)) = (&mut self.out, &mut self.section) else {
            return;
        };
        out.play_at_tagged(&s.sound, at, 1.0, SECTION_TAG, Bus::Song);
        for (ms, c, gain) in &s.clicks {
            out.play_at_tagged(c, at + ms, *gain, SECTION_TAG, Bus::Click);
        }
        s.starts.push(at);
        // 古いものは要らない
        if s.starts.len() > 4 {
            s.starts.remove(0);
        }
    }

    /// 読み終えたプレビューの音を鳴らす。区間の音は、次の回の頭が近づいたら予定にする。毎フレーム呼ぶ
    pub fn poll(&mut self) {
        if let (Some(now), Some(s)) = (self.out_now_ms(), &self.section)
            && let Some(&last) = s.starts.last()
        {
            let next = last + s.len_ms + SECTION_GAP_MS;
            if next - now < 300.0 {
                self.queue_section(next.max(now + SECTION_LEAD_MS));
            }
        }
        for (path, sound) in self.done.try_iter() {
            if self.preview.as_ref() != Some(&path) {
                continue;
            }
            if let (Some(out), Some(s)) = (&mut self.out, sound) {
                // プレビューは曲の音（BGM の音量）。前は 0.8 で決め打ちしていた（BGM の音量の既定 80 % と同じ）
                out.play_tagged(&s, 1.0, PREVIEW_TAG, Bus::Song);
            }
        }
    }
}
