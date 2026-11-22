//! 設定。OS の設定のフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の config.toml に置く。
//! 読めないときは既定の値で始め、ファイルは壊さない（書くのは値を変えたときだけ）。

use std::path::PathBuf;

use dtx::DrumLane;
use engine::practice::{LoopStyle, PassRule, SpeedPlan, snap_speed};
use serde::{Deserialize, Serialize};

use crate::practice::PracticePrefs;
use crate::stretch::Pitch;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// DTXManiaNX のフォルダ。選んだときに、使っているスキン（画面の画像と音）と Config.ini を
    /// 設定のフォルダの `nx` へ写し（[`Config::nx_copy_dir`]）、曲のフォルダをここから足す
    pub nx_dir: Option<PathBuf>,
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
    /// 練習の速度を段階アップで上げる（false なら固定）
    pub practice_ladder: bool,
    /// 練習の速度（%）。段階アップなら始めの速度
    pub practice_speed: u32,
    /// 段階アップの目標の速度（%）
    pub practice_target: u32,
    /// 遅くするときの音程（"keep" は保つ、"tape" は音程ごと遅く）
    pub practice_pitch: String,
    /// ゲージが尽きたら途中で終わる（STAGE FAILED）。書いていなければ NX の Config.ini の `StageFailed` に従う
    pub stage_failed: Option<bool>,
    /// 設定の画面で変えたパッドの割り当て（Roland の既定と NX の割り当てより優先する）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pad_notes: Vec<PadNote>,
    /// 起動したときに GitHub のリリースから新しい版を落とし、終えるときに入れ替える（Setup.exe で入れたときだけ）
    pub auto_update: bool,
}

/// パッドの割り当て 1 つ（ノート番号 → レーン）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadNote {
    pub note: u8,
    /// レーンの名前（HH、SD など）
    pub lane: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nx_dir: None,
            song_dirs: Vec::new(),
            input_offset_ms: 0.0,
            visual_offset_ms: 0.0,
            scroll_px_per_ms: 0.6,
            drum_sounds: true,
            level: 2,
            practice_style: LoopStyle::default().name().to_owned(),
            practice_rule: PassRule::default().name().to_owned(),
            practice_ladder: true,
            practice_speed: SpeedPlan::DEFAULT_START,
            practice_target: SpeedPlan::DEFAULT_TARGET,
            practice_pitch: Pitch::default().name().to_owned(),
            stage_failed: None,
            pad_notes: Vec::new(),
            auto_update: true,
        }
    }
}

impl Config {
    /// 設定のフォルダ（Windows は %APPDATA%\DTXmaniaNXT）
    pub fn dir() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("DTXmaniaNXT"))
    }

    pub fn path() -> Option<PathBuf> {
        Some(Self::dir()?.join("config.toml"))
    }

    /// NX から写したスキンと Config.ini を置くフォルダ（NX のフォルダと同じ形）
    pub fn nx_copy_dir() -> Option<PathBuf> {
        Some(Self::dir()?.join("nx"))
    }

    /// 曲のフォルダのどれかが NX のフォルダの中にある（NX を消すと曲も消える）
    pub fn songs_in_nx(&self) -> bool {
        self.nx_dir
            .as_deref()
            .is_some_and(|nx| self.song_dirs.iter().any(|d| d.starts_with(nx)))
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

    /// 練習の速度の決め方
    pub fn speed_plan(&self) -> SpeedPlan {
        let speed = snap_speed(i64::from(self.practice_speed));
        if self.practice_ladder {
            SpeedPlan::Ladder {
                start: speed,
                target: snap_speed(i64::from(self.practice_target)),
            }
        } else {
            SpeedPlan::Fixed(speed)
        }
    }

    /// 遅くするときの音程（読めない値なら既定）
    pub fn pitch(&self) -> Pitch {
        Pitch::from_name(&self.practice_pitch).unwrap_or_default()
    }

    /// 練習の決めごと（区間のほか）
    pub fn practice_prefs(&self) -> PracticePrefs {
        PracticePrefs {
            style: self.loop_style(),
            rule: self.pass_rule(),
            speed: self.speed_plan(),
            pitch: self.pitch(),
        }
    }

    /// 練習の決めごとを残す（次の練習でも使う）
    pub fn set_practice_prefs(&mut self, p: PracticePrefs) {
        self.practice_style = p.style.name().to_owned();
        self.practice_rule = p.rule.name().to_owned();
        match p.speed {
            SpeedPlan::Fixed(s) => {
                self.practice_ladder = false;
                self.practice_speed = s;
            }
            SpeedPlan::Ladder { start, target } => {
                self.practice_ladder = true;
                self.practice_speed = start;
                self.practice_target = target;
            }
        }
        self.practice_pitch = p.pitch.name().to_owned();
    }

    /// ノート番号をレーンに割り当てる（前の割り当ては消す）
    pub fn assign_pad(&mut self, note: u8, lane: DrumLane) {
        self.pad_notes.retain(|p| p.note != note);
        self.pad_notes.push(PadNote {
            note,
            lane: lane.name().to_owned(),
        });
    }

    /// 変えたパッドの割り当て（読めないレーンの名前は飛ばす）
    pub fn pad_overrides(&self) -> Vec<(u8, DrumLane)> {
        self.pad_notes
            .iter()
            .filter_map(|p| Some((p.note, DrumLane::from_name(&p.lane)?)))
            .collect()
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
        assert_eq!(c.speed_plan(), SpeedPlan::default());
        assert_eq!(c.pitch(), Pitch::Keep);
    }

    #[test]
    fn practice_prefs_round_trip() {
        let mut c = Config::default();
        let p = PracticePrefs {
            style: LoopStyle::Restart,
            rule: PassRule::Strict,
            speed: SpeedPlan::Fixed(85),
            pitch: Pitch::Tape,
        };
        c.set_practice_prefs(p);
        assert_eq!(c.practice_prefs(), p);
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.practice_prefs(), p);
        // 段階アップに戻しても目標は前の値のまま
        c.practice_ladder = true;
        assert_eq!(
            c.speed_plan(),
            SpeedPlan::Ladder {
                start: 85,
                target: 100
            }
        );
        // 刻みに合わない速度は直す
        c.practice_speed = 7;
        assert_eq!(c.speed_plan().first_speed(), 25);
    }

    #[test]
    fn unknown_practice_names_fall_back() {
        let c: Config =
            toml::from_str("practice_style = \"loop\"\npractice_rule = \"strict\"\n").unwrap();
        assert_eq!(c.loop_style(), LoopStyle::Connect);
        assert_eq!(c.pass_rule(), PassRule::Strict);
    }

    #[test]
    fn pad_notes_replace_the_same_note() {
        let mut c = Config::default();
        c.assign_pad(60, DrumLane::HT);
        c.assign_pad(61, DrumLane::LT);
        c.assign_pad(60, DrumLane::FT);
        assert_eq!(
            c.pad_overrides(),
            vec![(61, DrumLane::LT), (60, DrumLane::FT)]
        );
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.pad_overrides(), c.pad_overrides());
        let c: Config = toml::from_str("[[pad_notes]]\nnote = 1\nlane = \"XX\"\n").unwrap();
        assert!(c.pad_overrides().is_empty());
    }

    #[test]
    fn round_trip() {
        let mut c = Config::default();
        c.song_dirs.push(PathBuf::from("曲"));
        c.level = 4;
        c.stage_failed = Some(false);
        c.assign_pad(60, DrumLane::HT);
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
