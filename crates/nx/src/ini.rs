//! DTXMania の Config.ini をそのまま読む（どの項目を使うかは呼ぶ側で決める）。
//!
//! 読み方は上流の `CConfigIni.tReadFromString` と同じ: 行の前の空白とタブを除き、`;` で始まる行は注釈、
//! `[` で始まる行は `]` までがセクションの名前。`=` がちょうど 1 つの行だけを `キー=値` として読み、前後の空白を除く
//! （`=` が 2 つ以上ある行は上流も読まない）。キーとセクションの名前は大文字・小文字を区別する。
//! 同じキーが何度も書いてあれば、後のものが勝つ（上流は読んだ順に値を上書きする）。

/// セクション 1 つ（書いてある順のキーと値）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

/// 読んだ Config.ini
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ini {
    pub sections: Vec<Section>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        for raw in text.lines() {
            let line = raw.replace('\t', " ");
            let line = line.trim_start();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let name = rest.split(']').next().unwrap_or_default();
                out.sections.push(Section {
                    name: name.to_owned(),
                    entries: Vec::new(),
                });
                continue;
            }
            let mut parts = line.split('=');
            let (Some(key), Some(value), None) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            // セクションより前のキーは上流も読まない
            if let Some(section) = out.sections.last_mut() {
                section
                    .entries
                    .push((key.trim().to_owned(), value.trim().to_owned()));
            }
        }
        out
    }

    /// `[section]` の `key` の値（何度も書いてあれば最後のもの）
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .filter(|s| s.name == section)
            .flat_map(|s| s.entries.iter())
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// セクションがあるか
    pub fn has_section(&self, section: &str) -> bool {
        self.sections.iter().any(|s| s.name == section)
    }

    /// セクションのキーと値（同じセクションが何度もあればつなげる）
    pub fn entries<'a>(
        &'a self,
        section: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.sections
            .iter()
            .filter(move |s| s.name == section)
            .flat_map(|s| s.entries.iter())
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// DTXMania の Config.ini らしいか（`[System]` か `[DrumsKeyAssign]` がある）
    pub fn looks_like_dtxmania(&self) -> bool {
        self.has_section("DrumsKeyAssign")
            || (self.has_section("System") && self.get("System", "Version").is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_keys_and_the_last_value() {
        let ini = Ini::parse(
            ";-------------------\r\n[System]\r\n\r\nVersion=v2.0.0 Beta (20260121)\r\n\t DTXPath=.\\;D:\\DTX\\\r\nMasterVolume=80\r\nMasterVolume = 90\r\nCardNameDrums=a=b\r\n; 注釈=無視\r\n[DrumsKeyAssign]\r\nHH=K033,M042\r\nnot a pair\r\n",
        );
        assert_eq!(ini.get("System", "Version"), Some("v2.0.0 Beta (20260121)"));
        assert_eq!(ini.get("System", "DTXPath"), Some(".\\;D:\\DTX\\"));
        assert_eq!(ini.get("System", "MasterVolume"), Some("90"));
        assert_eq!(ini.get("System", "; 注釈"), None);
        // = が 2 つの行は読まない
        assert_eq!(ini.get("System", "CardNameDrums"), None);
        assert_eq!(ini.get("DrumsKeyAssign", "HH"), Some("K033,M042"));
        // キーは大文字・小文字を区別する（上流と同じ）
        assert_eq!(ini.get("System", "mastervolume"), None);
        assert!(ini.looks_like_dtxmania());
        assert_eq!(ini.entries("DrumsKeyAssign").count(), 1);
    }

    #[test]
    fn other_ini_files_are_not_dtxmania() {
        assert!(!Ini::parse("[Settings]\nfoo=1\n").looks_like_dtxmania());
        assert!(!Ini::parse("foo=1\n").looks_like_dtxmania());
        assert!(Ini::parse("foo=1\n").sections.is_empty());
    }
}
