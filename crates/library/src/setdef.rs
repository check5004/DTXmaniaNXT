//! set.def（1 つのフォルダに複数の難易度の譜面をまとめる定義）。
//!
//! 上流 DTXManiaNX（`CSetDef.cs`）のふるまいに合わせている。
//! - `#TITLE` から次の `#TITLE` までが 1 曲（ブロック）
//! - `#L1FILE`〜`#L5FILE` が難易度 1〜5 の譜面、`#L1LABEL`〜`#L5LABEL` がその名前
//! - 譜面があって名前が無ければ NOVICE / REGULAR / EXPERT / MASTER / DTXMania
//! - 名前だけあって譜面が無い難易度は無かったことにする
//! - `;` から後は注釈

/// 難易度の数
pub const LEVELS: usize = 5;

/// 名前が書いていないときの難易度の名前（上流と同じ）
pub const DEFAULT_LABELS: [&str; LEVELS] = ["NOVICE", "REGULAR", "EXPERT", "MASTER", "DTXMania"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetDefBlock {
    pub title: String,
    pub genre: String,
    /// 難易度ごとの譜面のファイル（set.def に書いてあるまま）
    pub files: [Option<String>; LEVELS],
    /// 難易度ごとの名前（譜面のある難易度だけ）
    pub labels: [Option<String>; LEVELS],
}

impl SetDefBlock {
    fn in_use(&self) -> bool {
        !self.title.is_empty()
            || !self.genre.is_empty()
            || self.files.iter().any(Option::is_some)
            || self.labels.iter().any(Option::is_some)
    }

    fn finish(mut self) -> Self {
        for ((file, label), default) in self.files.iter().zip(&mut self.labels).zip(DEFAULT_LABELS)
        {
            if file.is_none() {
                *label = None;
            } else if label.is_none() {
                *label = Some(default.to_owned());
            }
        }
        self
    }
}

/// set.def の文字を読む
pub fn parse_set_def(text: &str) -> Vec<SetDefBlock> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    let mut block = SetDefBlock::default();
    for line in text.lines() {
        let line = line.trim_start_matches([' ', '\t']);
        let Some(body) = line.strip_prefix('#') else {
            continue;
        };
        let body = body.split(';').next().unwrap_or("");
        let Some((key, value)) = split_key(body) else {
            continue;
        };
        let key = key.to_ascii_uppercase();
        if key == "TITLE" {
            if block.in_use() {
                out.push(std::mem::take(&mut block).finish());
            }
            block.title = value.to_owned();
            continue;
        }
        if key == "GENRE" {
            block.genre = value.to_owned();
            continue;
        }
        if let Some((i, kind)) = level_key(&key) {
            let v = (!value.is_empty()).then(|| value.to_owned());
            match kind {
                LevelKey::File => block.files[i] = v,
                LevelKey::Label => block.labels[i] = v,
            }
        }
    }
    if block.in_use() {
        out.push(block.finish());
    }
    out
}

enum LevelKey {
    File,
    Label,
}

/// `L1FILE` → (0, File)
fn level_key(key: &str) -> Option<(usize, LevelKey)> {
    let rest = key.strip_prefix('L')?;
    let n = rest.chars().next()?.to_digit(10)? as usize;
    if !(1..=LEVELS).contains(&n) {
        return None;
    }
    match &rest[1..] {
        "FILE" => Some((n - 1, LevelKey::File)),
        "LABEL" => Some((n - 1, LevelKey::Label)),
        _ => None,
    }
}

/// `KEY: value` / `KEY value` を分ける（値の前後の空白と `:` は取る）
pub(crate) fn split_key(body: &str) -> Option<(&str, &str)> {
    let end = body
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(body.len());
    if end == 0 {
        return None;
    }
    let key = &body[..end];
    let value = body[end..].trim_matches([' ', '\t', ':', '\r']);
    Some((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_songs_with_defaults() {
        let text = "\
#TITLE: 曲A ; 注釈
#L1LABEL BASIC
#L1FILE a_bsc.dtx
#L3FILE: a_ext.dtx
#L4LABEL MASTER

#TITLE 曲B
#GENRE ROCK
#L2FILE b.dtx
";
        let b = parse_set_def(text);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].title, "曲A");
        assert_eq!(b[0].files[0].as_deref(), Some("a_bsc.dtx"));
        assert_eq!(b[0].labels[0].as_deref(), Some("BASIC"));
        assert_eq!(b[0].files[2].as_deref(), Some("a_ext.dtx"));
        // 名前が無いので上流の既定の名前
        assert_eq!(b[0].labels[2].as_deref(), Some("EXPERT"));
        // 名前だけで譜面が無い難易度は無し
        assert_eq!(b[0].labels[3], None);
        assert_eq!(b[1].title, "曲B");
        assert_eq!(b[1].genre, "ROCK");
        assert_eq!(b[1].labels[1].as_deref(), Some("REGULAR"));
    }

    #[test]
    fn files_before_title_make_a_block() {
        let b = parse_set_def("#L1FILE x.dtx\r\n");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].title, "");
        assert_eq!(b[0].files[0].as_deref(), Some("x.dtx"));
    }

    #[test]
    fn ignores_unknown_lines() {
        assert!(parse_set_def("; 注釈\nfoo\n#L6FILE x.dtx\n").is_empty());
    }
}
