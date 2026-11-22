//! DTX テキストの解釈。

use std::collections::{BTreeMap, HashMap};

use crate::ch;
use crate::number::{parse_base36x2, parse_hex2, parse_measure, parse_nx_number};

/// 譜面上のチップ 1 つ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub channel: u8,
    pub measure: u32,
    /// 小節内の位置 = `pos / div`（0 以上 1 未満）
    pub pos: u32,
    pub div: u32,
    /// 値（`#WAVzz` の番号など。BPM チャンネル 0x03 だけは 16 進数の値）
    pub value: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct Chart {
    /// `#TITLE` などのヘッダ（キーは大文字。同じキーは後のものが有効）
    pub fields: HashMap<String, String>,
    /// ヘッダの行の `;` から後ろの注釈（キーは大文字。空でないものだけ）。
    /// DTXCreator は `#WAVzz` の行に、音につけた名前（「Snare Rim」など）を注釈として書く
    pub comments: HashMap<String, String>,
    pub chips: Vec<Chip>,
    /// 小節長の変更（小節番号 → 倍率）。変更は次の変更まで続く
    pub bar_lengths: BTreeMap<u32, f64>,
    pub warnings: Vec<Warning>,
    /// `#RANDOM` / `#IF` がある。分岐の中身はまだ読まない（フェーズ 1 以降で対応）
    pub has_random: bool,
}

impl Chart {
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .get(&key.to_ascii_uppercase())
            .map(String::as_str)
    }

    pub fn title(&self) -> &str {
        self.field("TITLE").unwrap_or("")
    }

    pub fn artist(&self) -> &str {
        self.field("ARTIST").unwrap_or("")
    }

    /// ドラムのレベル（8.25 など）。`#DLEVEL` が 100 以上なら 3 桁（825 → 8.25）、
    /// 2 桁以下なら `#DLVDEC` を小数の 2 桁目に足す（82 と 5 → 8.25）
    pub fn drum_level(&self) -> Option<f64> {
        let raw = parse_nx_number(self.field("DLEVEL")?)?;
        if raw >= 100.0 {
            return Some(raw / 100.0);
        }
        let dec = self
            .field("DLVDEC")
            .and_then(parse_nx_number)
            .unwrap_or(0.0);
        Some(raw / 10.0 + dec / 100.0)
    }

    /// `#WAVzz` のファイル名
    pub fn wav(&self, id: u16) -> Option<&str> {
        self.field(&format!("WAV{}", to_base36x2(id)))
    }

    /// `#WAVzz` の行の注釈（音の名前）
    pub fn wav_comment(&self, id: u16) -> Option<&str> {
        self.comments
            .get(&format!("WAV{}", to_base36x2(id)))
            .map(String::as_str)
    }
}

pub(crate) fn to_base36x2(n: u16) -> String {
    const D: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let n = usize::from(n.min(36 * 36 - 1));
    format!("{}{}", D[n / 36] as char, D[n % 36] as char)
}

struct CommandLine<'a> {
    command: &'a str,
    param: &'a str,
    /// `;` から後ろ（`;` は含まない。前後の空白は除く）
    comment: &'a str,
}

/// '#' で始まる 1 行を分解する。コマンドの後ろは ':' か空白で区切り、';' から後は注釈
fn split_command_line(line: &str) -> Option<CommandLine<'_>> {
    let s = line.trim_start_matches([' ', '\t']);
    let s = s.strip_prefix('#')?;
    let s = s.trim_start_matches([' ', '\t']);
    let cmd_end = s.find([':', ' ', '\t', ';']).unwrap_or(s.len());
    let command = &s[..cmd_end];
    let mut rest = &s[cmd_end..];
    rest = rest.strip_prefix(':').unwrap_or(rest);
    let param_end = rest.find(';').unwrap_or(rest.len());
    let param = rest[..param_end].trim_matches([' ', '\t']);
    let comment = rest
        .get(param_end + 1..)
        .unwrap_or("")
        .trim_matches([' ', '\t', '\r']);
    Some(CommandLine {
        command,
        param,
        comment,
    })
}

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

pub fn parse(text: &str) -> Chart {
    let mut chart = Chart::default();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut if_depth = 0u32;

    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let Some(p) = split_command_line(line) else {
            continue;
        };
        let cmd = p.command;
        if cmd.is_empty() || !cmd.is_char_boundary(cmd.len().min(6)) {
            continue;
        }
        // 上流は #IF / #ENDIF を他のどのコマンドより先に（前方一致で）判定する
        if starts_with_ci(cmd, "IF") {
            if_depth += 1;
            chart.has_random = true;
            continue;
        }
        if starts_with_ci(cmd, "ENDIF") {
            if_depth = if_depth.saturating_sub(1);
            continue;
        }
        if if_depth > 0 || starts_with_ci(cmd, "RANDOM") {
            chart.has_random |= starts_with_ci(cmd, "RANDOM");
            continue;
        }

        if let Some((measure, channel)) = chip_line_target(cmd) {
            if channel == ch::BAR_LENGTH {
                match parse_nx_number(p.param) {
                    Some(v) if v > 0.0 => {
                        chart.bar_lengths.insert(measure, v);
                    }
                    _ => chart.warnings.push(Warning {
                        line: line_no,
                        message: format!("小節長の値を読めません: {}", p.param),
                    }),
                }
                continue;
            }
            match split_chip_data(channel, p.param) {
                Some(values) => {
                    let div = values.len() as u32;
                    for (i, v) in values.into_iter().enumerate() {
                        if v != 0 {
                            chart.chips.push(Chip {
                                channel,
                                measure,
                                pos: i as u32,
                                div,
                                value: v,
                            });
                        }
                    }
                }
                None => chart.warnings.push(Warning {
                    line: line_no,
                    message: format!(
                        "チップの並びを読めません（上流でも無視される行です）: {}",
                        line.trim()
                    ),
                }),
            }
            continue;
        }

        let key = cmd.to_ascii_uppercase();
        if p.comment.is_empty() {
            chart.comments.remove(&key);
        } else {
            chart.comments.insert(key.clone(), p.comment.to_owned());
        }
        chart.fields.insert(key, p.param.to_owned());
    }

    if if_depth > 0 {
        chart.warnings.push(Warning {
            line: text.lines().count(),
            message: "#IF に対応する #ENDIF がありません".to_owned(),
        });
    }
    chart
}

/// コマンドがチップ配置行（`#mmmcc`）なら小節とチャンネル
fn chip_line_target(cmd: &str) -> Option<(u32, u8)> {
    if cmd.len() != 5 || !cmd.is_ascii() {
        return None;
    }
    Some((parse_measure(&cmd[..3])?, parse_hex2(&cmd[3..5])?))
}

/// チップの並び。'_' は無視し、36 進数以外の文字があれば行ごと無効
fn split_chip_data(channel: u8, param: &str) -> Option<Vec<u16>> {
    let s: String = param.chars().filter(|&c| c != '_').collect();
    if !s.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| {
            let pair = &s[i * 2..i * 2 + 2];
            if channel == ch::BPM {
                parse_hex2(pair).map(u16::from)
            } else {
                parse_base36x2(pair)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_chips() {
        let c = parse(
            "#TITLE: Test ;comment\n#BPM 120\n#WAV0A kick.wav\n#00113: 0A000A00\n#00102: 0.75\n",
        );
        assert_eq!(c.title(), "Test");
        assert_eq!(c.field("bpm"), Some("120"));
        assert_eq!(c.wav(10), Some("kick.wav"));
        assert_eq!(c.chips.len(), 2);
        assert_eq!(
            c.chips[1],
            Chip {
                channel: 0x13,
                measure: 1,
                pos: 2,
                div: 4,
                value: 10
            }
        );
        assert_eq!(c.bar_lengths.get(&1), Some(&0.75));
    }

    #[test]
    fn wav_comments() {
        let c = parse(
            "#WAV01: 037_Rimshot.ogg\t;Snare Rim\r\n#WAV02 snare.ogg\n#WAV03 a.ogg ;x\n#WAV03 b.ogg\n",
        );
        assert_eq!(c.wav(1), Some("037_Rimshot.ogg"));
        assert_eq!(c.wav_comment(1), Some("Snare Rim"));
        assert_eq!(c.wav_comment(2), None);
        // 後の定義が勝つ（注釈の無い行で上書きしたら注釈も消える）
        assert_eq!(c.wav(3), Some("b.ogg"));
        assert_eq!(c.wav_comment(3), None);
    }

    #[test]
    fn later_definition_wins() {
        let c = parse("#TITLE A\n#TITLE B\n");
        assert_eq!(c.title(), "B");
    }

    #[test]
    fn underscores_and_bad_lines() {
        let c = parse("#00112: 01_00_01_00\n#00113: 01!0\n");
        assert_eq!(c.chips.len(), 2);
        assert_eq!(c.warnings.len(), 1);
    }

    #[test]
    fn bpm_channel_is_hex() {
        let c = parse("#00103: 3C\n");
        assert_eq!(c.chips[0].value, 0x3C);
    }

    #[test]
    fn random_blocks_are_skipped() {
        let c = parse("#RANDOM 2\n#IF 1\n#00113: 01\n#ENDIF\n#00112: 01\n");
        assert!(c.has_random);
        assert_eq!(c.chips.len(), 1);
        assert_eq!(c.chips[0].channel, 0x12);
    }

    #[test]
    fn level() {
        assert_eq!(parse("#DLEVEL 825\n").drum_level(), Some(8.25));
        let l = parse("#DLEVEL 82\n#DLVDEC 5\n").drum_level().unwrap();
        assert!((l - 8.25).abs() < 1e-9);
    }
}
