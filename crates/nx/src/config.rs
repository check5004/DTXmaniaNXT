//! NX の Config.ini（`[セクション]` と `キー=値`、`;` で始まる行は注釈。`DTXPath` は `;` で区切るので、行の途中の `;` は注釈ではない）。

use std::collections::BTreeMap;

use dtx::DrumLane;

/// 読んだ Config.ini のうち NXT が使うもの
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NxConfig {
    /// `[System] DTXPath`（`;` 区切り。書いてあるまま）
    pub dtx_paths: Vec<String>,
    /// `[System] SkinPath`（`System/` からの相対。空なら None）
    pub skin_path: Option<String>,
    /// `[DrumsKeyAssign]` のレーンごとの割り当て
    pub drums: BTreeMap<DrumLane, Vec<KeyAssign>>,
    /// `[System] StageFailed`（ゲージが尽きたら途中で終わる。書いていなければ None。上流の既定は on）
    pub stage_failed: Option<bool>,
    /// `[System] ShowLagTime`（判定のずれの数字。0 は出さない、1 は出す、2 は PERFECT 以外だけ。
    /// 書いていないか範囲の外なら None）
    pub show_lag_time: Option<u8>,
    /// `[System] ShowLagHitCount`（結果の画面に早かった数と遅かった数を出す。上流の既定は off）
    pub show_lag_hit_count: bool,
    /// `[System] ShowLagTimeColor`（1 なら早いを赤、遅いを青にする。上流の既定の 0 は逆）
    pub lag_color_swapped: bool,
}

/// 入力の割り当て 1 つ（上流 `CConfigIni.tReadAndSetSkey` と同じ書き方）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAssign {
    /// `K` + `0` + キーコード
    Keyboard(u8),
    /// `M` + デバイス番号 1 桁（0〜9, A〜Z）+ ノート番号
    Midi { device: u8, note: u8 },
    /// `J` + デバイス番号 1 桁 + ボタン
    Joypad { device: u8, button: u8 },
    /// `N` + `0` + ボタン番号
    Mouse(u8),
}

impl KeyAssign {
    /// 1 つ読む（`M042` など。読めなければ `None`）
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_uppercase();
        if s.len() < 3 || !s.is_ascii() {
            return None;
        }
        let kind = s.as_bytes()[0];
        let device =
            "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".find(char::from(s.as_bytes()[1]))? as u8;
        let code: u8 = s[2..].parse().ok()?;
        Some(match kind {
            b'K' => Self::Keyboard(code),
            b'M' => Self::Midi { device, note: code },
            b'J' => Self::Joypad {
                device,
                button: code,
            },
            b'N' => Self::Mouse(code),
            _ => return None,
        })
    }
}

/// `[DrumsKeyAssign]` の値（`K033,M042` など。上流と同じく 16 個まで。読めないものは飛ばす）
pub fn parse_assigns(value: &str) -> Vec<KeyAssign> {
    value
        .split(',')
        .take(16)
        .filter_map(KeyAssign::parse)
        .collect()
}

/// `[DrumsKeyAssign]` のキーの名前のレーン
pub fn lane_of(key: &str) -> Option<DrumLane> {
    Some(match key {
        "HH" => DrumLane::HH,
        "SD" => DrumLane::SD,
        "BD" => DrumLane::BD,
        "HT" => DrumLane::HT,
        "LT" => DrumLane::LT,
        "FT" => DrumLane::FT,
        "CY" => DrumLane::CY,
        "HO" => DrumLane::HHO,
        "RD" => DrumLane::RD,
        "LC" => DrumLane::LC,
        "LP" => DrumLane::LP,
        "LBD" => DrumLane::LBD,
        _ => return None,
    })
}

impl NxConfig {
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_owned();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match section.as_str() {
                "System" => match key {
                    "DTXPath" => {
                        out.dtx_paths = value
                            .split(';')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(str::to_owned)
                            .collect();
                    }
                    "SkinPath" => {
                        out.skin_path = (!value.is_empty()).then(|| value.to_owned());
                    }
                    // 上流 `CConversion.bONorOFF`：先頭の文字が 0 でなければ on
                    "StageFailed" => {
                        out.stage_failed = value.chars().next().map(|c| c != '0');
                    }
                    "ShowLagTime" => {
                        out.show_lag_time = value.parse().ok().filter(|v| *v <= 2);
                    }
                    "ShowLagHitCount" => {
                        out.show_lag_hit_count = value.chars().next().is_some_and(|c| c != '0');
                    }
                    // 上流 `nGetNumberIfInRange(0, 1)`：範囲の外は既定のまま
                    "ShowLagTimeColor" => out.lag_color_swapped = value == "1",
                    _ => {}
                },
                "DrumsKeyAssign" => {
                    if let Some(lane) = lane_of(key) {
                        out.drums.insert(lane, parse_assigns(value));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// MIDI のノート番号ごとのレーン（デバイスは区別しない）
    pub fn midi_notes(&self) -> Vec<(u8, DrumLane)> {
        let mut out = Vec::new();
        for (&lane, assigns) in &self.drums {
            for a in assigns {
                if let KeyAssign::Midi { note, .. } = *a {
                    out.push((note, lane));
                }
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "\
;-------------------
[System]
; 注釈
DTXPath=.\\;D:\\DTXFiles\\;
SkinPath=
[DrumsKeyAssign]
HH=K033,M042
SD=K012,M038,M040
HO=M046
LBD=K077
XX=M001
[GuitarKeyAssign]
HH=M099
";

    #[test]
    fn reads_paths_skin_and_pads() {
        let c = NxConfig::parse(TEXT);
        assert_eq!(c.dtx_paths, vec![".\\", "D:\\DTXFiles\\"]);
        assert_eq!(c.skin_path, None);
        assert_eq!(
            c.drums[&DrumLane::HH],
            vec![
                KeyAssign::Keyboard(33),
                KeyAssign::Midi {
                    device: 0,
                    note: 42
                }
            ]
        );
        assert_eq!(c.drums[&DrumLane::LBD], vec![KeyAssign::Keyboard(77)]);
        assert_eq!(
            c.midi_notes(),
            vec![
                (38, DrumLane::SD),
                (40, DrumLane::SD),
                (42, DrumLane::HH),
                (46, DrumLane::HHO)
            ]
        );
    }

    #[test]
    fn reads_device_numbers_in_base36() {
        assert_eq!(
            KeyAssign::parse("ma51"),
            Some(KeyAssign::Midi {
                device: 10,
                note: 51
            })
        );
        assert_eq!(KeyAssign::parse("M0300"), None);
        assert_eq!(KeyAssign::parse("L001"), None);
    }

    #[test]
    fn stage_failed_follows_the_first_character() {
        assert_eq!(
            NxConfig::parse("[System]\nStageFailed=0\n").stage_failed,
            Some(false)
        );
        assert_eq!(
            NxConfig::parse("[System]\nStageFailed=1\n").stage_failed,
            Some(true)
        );
        assert_eq!(NxConfig::parse("[System]\n").stage_failed, None);
    }

    #[test]
    fn lag_hit_count_settings() {
        let c = NxConfig::parse("[System]\nShowLagHitCount=1\nShowLagTimeColor=1\nShowLagTime=2\n");
        assert_eq!(c.show_lag_time, Some(2));
        assert!(c.show_lag_hit_count);
        assert!(c.lag_color_swapped);
        let c = NxConfig::parse("[System]\nShowLagHitCount=0\nShowLagTimeColor=2\n");
        assert!(!c.show_lag_hit_count);
        assert!(!c.lag_color_swapped);
        assert_eq!(
            NxConfig::parse("[System]\nShowLagTime=3\n").show_lag_time,
            None
        );
        assert!(!NxConfig::parse("").show_lag_hit_count);
    }

    #[test]
    fn skin_path_is_kept() {
        let c = NxConfig::parse("[System]\nSkinPath=.\\Default\\\n");
        assert_eq!(c.skin_path.as_deref(), Some(".\\Default\\"));
    }
}
