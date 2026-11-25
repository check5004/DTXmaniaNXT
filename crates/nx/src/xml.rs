//! 本家 DTXMania の 1xx 系の Config.xml（.NET の DataContractSerializer で書いたもの）から、
//! パッドの割り当てと曲のフォルダと版だけを読み、Config.ini と同じ形（[`Ini`]）に直す。
//!
//! 要素の名前の前の名前空間（`d2p1:` など）は見ない。パッドの割り当ては
//! `KeyAssign` の下のパッドの名前（HH、SD … HP）の下の `val` ごとに、`InputDevice`・`ID`・`Code` を読む。
//! ほかの設定（判定や見た目）は読まない（値の意味を Config.ini の版と照らして確かめていないため）。

use crate::ini::{Ini, Section};

/// Config.xml を [`Ini`] に直す（DTXMania の Config.xml でなければ `None`）
pub fn to_ini(text: &str) -> Option<Ini> {
    let mut stack: Vec<String> = Vec::new();
    let mut version = None;
    let mut song_path = None;
    // パッドごとの割り当て（Config.ini の書き方）
    let mut pads: Vec<(String, Vec<String>)> = Vec::new();
    let mut current = Assign::default();
    let mut saw_root = false;
    let mut rest = text;
    while let Some(lt) = rest.find('<') {
        let (body, tail) = rest.split_at(lt);
        let content = body.trim();
        if !content.is_empty() {
            let value = unescape(content);
            on_text(&stack, &value, &mut version, &mut song_path, &mut current);
        }
        let Some(gt) = tail.find('>') else {
            break;
        };
        let tag = &tail[1..gt];
        rest = &tail[gt + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            // 宣言と注釈（注釈の中の > で切れても、名前が合わないだけで害はない）
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = local(name);
            if name == "val" && in_key_assign(&stack) {
                if let Some(pad) = stack.get(stack.len().saturating_sub(3)).cloned()
                    && let Some(code) = current.to_ini()
                {
                    match pads.iter_mut().find(|(p, _)| *p == pad) {
                        Some((_, list)) => list.push(code),
                        None => pads.push((pad, vec![code])),
                    }
                }
                current = Assign::default();
            }
            if stack.last().is_some_and(|l| *l == name) {
                stack.pop();
            }
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name = local(tag.trim_end_matches('/'));
        if stack.is_empty() && name == "CConfigXml" {
            saw_root = true;
        }
        if !self_closing {
            stack.push(name);
        }
    }
    if !saw_root {
        return None;
    }
    let mut system = Section {
        name: "System".to_owned(),
        entries: Vec::new(),
    };
    system
        .entries
        .push(("Version".to_owned(), version.unwrap_or_default()));
    if let Some(p) = song_path {
        system.entries.push(("DTXPath".to_owned(), p));
    }
    let mut keys = Section {
        name: "DrumsKeyAssign".to_owned(),
        entries: Vec::new(),
    };
    for (pad, list) in pads {
        let key = match pad.as_str() {
            "HH" | "SD" | "BD" | "HT" | "LT" | "CY" | "FT" | "RD" | "LC" => pad.clone(),
            "HHO" => "HO".to_owned(),
            // HP（ハイハットのペダル）は Config.ini の版の LP
            "HP" => "LP".to_owned(),
            _ => continue,
        };
        keys.entries.push((key, list.join(",")));
    }
    Some(Ini {
        sections: vec![system, keys],
    })
}

/// 割り当て 1 つ（`val` の中身）
#[derive(Default)]
struct Assign {
    device: Option<String>,
    id: Option<u32>,
    code: Option<u32>,
}

impl Assign {
    /// Config.ini の書き方（K0126、M042 など）
    fn to_ini(&self) -> Option<String> {
        let id = self.id.unwrap_or(0);
        let code = self.code?;
        let digit = char::from_digit(id, 36)?.to_ascii_uppercase();
        let kind = match self.device.as_deref()? {
            "Keyboard" => 'K',
            "MIDIIn" => 'M',
            "JoyPad" => 'J',
            "Mouse" => 'N',
            _ => return None,
        };
        // キーボードとマウスの機器の番号は上流も 0 と書く
        let digit = if matches!(kind, 'K' | 'N') {
            '0'
        } else {
            digit
        };
        Some(format!("{kind}{digit}{code}"))
    }
}

fn on_text(
    stack: &[String],
    value: &str,
    version: &mut Option<String>,
    song_path: &mut Option<String>,
    current: &mut Assign,
) {
    let n = stack.len();
    let last = stack.last().map(String::as_str);
    let parent = n
        .checked_sub(2)
        .and_then(|i| stack.get(i))
        .map(String::as_str);
    match (parent, last) {
        (Some("strDTXManiaVersion"), Some("val")) => *version = Some(value.to_owned()),
        (Some("strSongDataPath"), Some("val")) => *song_path = Some(value.to_owned()),
        (Some("val"), Some(field)) if in_key_assign(&stack[..n - 1]) => match field {
            "InputDevice" => current.device = Some(value.to_owned()),
            "ID" => current.id = value.parse().ok(),
            "Code" => current.code = value.parse().ok(),
            _ => {}
        },
        _ => {}
    }
}

/// `…/KeyAssign/<パッド>/<要素>/val` の中か（`stack` の最後が val）
fn in_key_assign(stack: &[String]) -> bool {
    let n = stack.len();
    n >= 4 && stack[n - 1] == "val" && stack[n - 4] == "KeyAssign"
}

/// 名前空間を除いた名前（属性も除く）
fn local(tag: &str) -> String {
    let name = tag.split_whitespace().next().unwrap_or("");
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=semi]),
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DataContractSerializer の書き方に合わせた見本（実物は手元に無いので、型の定義から組み立てた）
    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<CConfigXml xmlns:i="http://www.w3.org/2001/XMLSchema-instance" xmlns="http://schemas.datacontract.org/2004/07/DTXMania">
  <bAVI><val>true</val></bAVI>
  <KeyAssign xmlns:d2p1="http://schemas.datacontract.org/2004/07/DTXMania">
    <d2p1:BD>
      <d2p1:COptionKeyAssign><d2p1:val><d2p1:Code>126</d2p1:Code><d2p1:ID>0</d2p1:ID><d2p1:InputDevice>Keyboard</d2p1:InputDevice></d2p1:val></d2p1:COptionKeyAssign>
      <d2p1:COptionKeyAssign><d2p1:val><d2p1:Code>36</d2p1:Code><d2p1:ID>1</d2p1:ID><d2p1:InputDevice>MIDIIn</d2p1:InputDevice></d2p1:val></d2p1:COptionKeyAssign>
      <d2p1:COptionKeyAssign><d2p1:val><d2p1:Code>0</d2p1:Code><d2p1:ID>0</d2p1:ID><d2p1:InputDevice>Unknown</d2p1:InputDevice></d2p1:val></d2p1:COptionKeyAssign>
    </d2p1:BD>
    <d2p1:HHO>
      <d2p1:COptionKeyAssign><d2p1:val><d2p1:Code>46</d2p1:Code><d2p1:ID>0</d2p1:ID><d2p1:InputDevice>MIDIIn</d2p1:InputDevice></d2p1:val></d2p1:COptionKeyAssign>
    </d2p1:HHO>
    <d2p1:HP>
      <d2p1:COptionKeyAssign><d2p1:val><d2p1:Code>44</d2p1:Code><d2p1:ID>0</d2p1:ID><d2p1:InputDevice>MIDIIn</d2p1:InputDevice></d2p1:val></d2p1:COptionKeyAssign>
    </d2p1:HP>
    <d2p1:GtR>
      <d2p1:COptionKeyAssign><d2p1:val><d2p1:Code>54</d2p1:Code><d2p1:ID>0</d2p1:ID><d2p1:InputDevice>Keyboard</d2p1:InputDevice></d2p1:val></d2p1:COptionKeyAssign>
    </d2p1:GtR>
  </KeyAssign>
  <strDTXManiaVersion><val>118(200510)</val></strDTXManiaVersion>
  <strSongDataPath><val>.\;D:\DTX &amp; More\</val></strSongDataPath>
</CConfigXml>"#;

    #[test]
    fn reads_pads_paths_and_version() {
        let ini = to_ini(XML).unwrap();
        assert_eq!(ini.get("System", "Version"), Some("118(200510)"));
        assert_eq!(ini.get("System", "DTXPath"), Some(".\\;D:\\DTX & More\\"));
        assert_eq!(ini.get("DrumsKeyAssign", "BD"), Some("K0126,M136"));
        assert_eq!(ini.get("DrumsKeyAssign", "HO"), Some("M046"));
        assert_eq!(ini.get("DrumsKeyAssign", "LP"), Some("M044"));
        // ギターは読まない
        assert_eq!(ini.entries("DrumsKeyAssign").count(), 3);
        assert!(ini.looks_like_dtxmania());
    }

    #[test]
    fn other_xml_is_not_read() {
        assert!(to_ini("<?xml version=\"1.0\"?><configuration><a>1</a></configuration>").is_none());
        assert!(to_ini("not xml").is_none());
        assert_eq!(unescape("a&#x41;&#66;&bogus;"), "aAB&bogus;");
    }
}
