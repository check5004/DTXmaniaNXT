//! 設定。OS の設定のフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の config.toml に置く。
//! 読めないときは既定の値で始め、ファイルは壊さない（書くのは値を変えたときだけ）。

use std::path::PathBuf;

use engine::practice::{LoopStyle, PassRule};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 曲のフォルダ
    pub song_dirs: Vec<PathBuf>,
    /// 入力の遅れの補正（ミリ秒）。叩いた時刻からこの分を引く
    pub input_offset_ms: f64,
    /// 表示の遅れの補正（ミリ秒）
    pub visual_offset_ms: f64,
    /// 1 ミリ秒あたりに落ちるピクセル
    pub scroll_px_per_ms: f32,
    /// 叩いたときに譜面の音を鳴らす（モジュールの音を使うなら切る）
    pub drum_sounds: bool,
    /// 最後に選んだ難易度（0〜4）
    pub level: usize,
    /// 練習の周のつなぎ方（"connect" か "restart"）
    pub practice_style: String,
    /// 練習の合格の基準（"easy"、"normal"、"strict"）
    pub practice_rule: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            song_dirs: Vec::new(),
            input_offset_ms: 0.0,
            visual_offset_ms: 0.0,
            scroll_px_per_ms: 0.6,
            drum_sounds: true,
            level: 2,
            practice_style: LoopStyle::default().name().to_owned(),
            practice_rule: PassRule::default().name().to_owned(),
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("DTXmaniaNXT").join("config.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("{}: {e}", path.display());
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // 途中で落ちても前のファイルが残るように、別の名前に書いてから置き換える
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("cannot save {}: {e:#}", path.display());
        }
    }

    /// 練習の周のつなぎ方（読めない値なら既定）
    pub fn loop_style(&self) -> LoopStyle {
        LoopStyle::from_name(&self.practice_style).unwrap_or_default()
    }

    /// 練習の合格の基準（読めない値なら既定）
    pub fn pass_rule(&self) -> PassRule {
        PassRule::from_name(&self.practice_rule).unwrap_or_default()
    }

    /// 曲のフォルダを足す（同じものは足さない）。足したら true
    pub fn add_song_dir(&mut self, dir: PathBuf) -> bool {
        if self.song_dirs.contains(&dir) {
            return false;
        }
        self.song_dirs.push(dir);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_use_defaults() {
        let c: Config = toml::from_str("input_offset_ms = 12.0\n").unwrap();
        assert_eq!(c.input_offset_ms, 12.0);
        assert_eq!(c.scroll_px_per_ms, 0.6);
        assert!(c.drum_sounds);
        assert_eq!(c.loop_style(), LoopStyle::Connect);
        assert_eq!(c.pass_rule(), PassRule::Normal);
    }

    #[test]
    fn unknown_practice_names_fall_back() {
        let c: Config =
            toml::from_str("practice_style = \"loop\"\npractice_rule = \"strict\"\n").unwrap();
        assert_eq!(c.loop_style(), LoopStyle::Connect);
        assert_eq!(c.pass_rule(), PassRule::Strict);
    }

    #[test]
    fn round_trip() {
        let mut c = Config::default();
        c.song_dirs.push(PathBuf::from("曲"));
        c.level = 4;
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
