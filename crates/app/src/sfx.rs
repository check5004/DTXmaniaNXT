//! 画面の効果音（スキンの `Sounds` の ogg）と、選曲のプレビューの音。
//! 演奏の音とは別の出力で鳴らす。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::audio::{AudioOut, Sound, load_sound};
use crate::gfx::Skin;

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
}

impl Se {
    const ALL: [Se; 4] = [Se::Move, Se::Decide, Se::Cancel, Se::Change];

    fn file(self) -> &'static str {
        match self {
            Se::Move => "Sounds/Move.ogg",
            Se::Decide => "Sounds/Decide.ogg",
            Se::Cancel => "Sounds/Cancel.ogg",
            Se::Change => "Sounds/Change.ogg",
        }
    }
}

/// プレビューの音に付ける目印
const PREVIEW_TAG: u8 = 1;

pub struct Sfx {
    out: Option<AudioOut>,
    sounds: HashMap<Se, Sound>,
    /// 鳴らしている（読んでいる）プレビューの音のファイル
    preview: Option<PathBuf>,
    jobs: Sender<(PathBuf, u32)>,
    done: Receiver<(PathBuf, Option<Sound>)>,
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
        }
    }

    /// スキンの効果音を読む
    pub fn load(&mut self, skin: &Skin) {
        self.sounds.clear();
        let Some(rate) = self.out.as_ref().map(AudioOut::rate) else {
            return;
        };
        for se in Se::ALL {
            if let Some(p) = skin.path(se.file())
                && let Ok(s) = load_sound(&p, rate)
            {
                self.sounds.insert(se, s);
            }
        }
    }

    pub fn play(&mut self, se: Se) {
        if let (Some(out), Some(s)) = (&mut self.out, self.sounds.get(&se)) {
            out.play_now(s, 1.0);
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

    /// 読み終えたプレビューの音を鳴らす。毎フレーム呼ぶ
    pub fn poll(&mut self) {
        for (path, sound) in self.done.try_iter() {
            if self.preview.as_ref() != Some(&path) {
                continue;
            }
            if let (Some(out), Some(s)) = (&mut self.out, sound) {
                out.play_tagged(&s, 0.8, PREVIEW_TAG);
            }
        }
    }
}
