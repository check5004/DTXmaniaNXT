//! 見つけた DTXMania の設定を読み、どの DTXMania か（派生と版）を見分ける。
//!
//! どの派生も同じ `CConfigIni` から分かれたので、キーの名前と値の書き方はほぼ同じ。ただし派生ごとに
//! 足したキーや、値の意味が違うキーがあるので、引き継ぐ側（アプリ）は [`Family`] を見て、
//! 確かめた派生の値だけを使う。

use std::path::Path;
use std::time::SystemTime;

use crate::locate::{Format, Found};
use crate::{Ini, xml};

/// DTXMania の派生
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// DTXManiaNX v2（BocuD 版。Version が `v2.` で始まる）
    NxV2,
    /// DTXManiaNX 1.x（limyz 版と、そこから分かれたもの。Version が `v1.` で始まる）
    Nx1,
    /// DTXManiaXG verK（Version が `Ver` で始まる）
    XgVerK,
    /// 本家 DTXMania の Config.ini の版（099 系まで。Version が `099g(191022)` の形）
    Classic,
    /// 本家 DTXMania の Config.xml の版（1xx 系）
    ClassicXml,
    /// 見分けられない（Config.ini の形は同じ）
    Unknown,
}

impl Family {
    /// 設定のファイルの版の文字列と exe の名前から見分ける
    pub fn detect(format: Format, version: Option<&str>, exe: Option<&str>) -> Self {
        if format == Format::Xml {
            return Self::ClassicXml;
        }
        let v = version.unwrap_or("").trim();
        let lower = v.to_ascii_lowercase();
        if lower.starts_with("v2.") {
            return Self::NxV2;
        }
        if lower.starts_with("v1.") {
            return Self::Nx1;
        }
        if lower.starts_with("ver") {
            return Self::XgVerK;
        }
        // 本家は「3 桁の数字（と英字 1 つ）(6 桁の日付)」
        let digits = v.chars().take_while(char::is_ascii_digit).count();
        if digits == 3
            && v[digits..]
                .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                .starts_with('(')
        {
            return Self::Classic;
        }
        // 版が無いか読めないときは exe の名前で（NX の exe の名前は NX にしか無い）
        match exe.map(str::to_ascii_lowercase).as_deref() {
            Some("dtxmanianx.exe") => Self::Nx1,
            _ => Self::Unknown,
        }
    }

    /// 画面に出す名前（製品の名前なので訳さない）
    pub fn product(self) -> &'static str {
        match self {
            Self::NxV2 | Self::Nx1 => "DTXManiaNX",
            Self::XgVerK => "DTXManiaXG verK",
            Self::Classic | Self::ClassicXml | Self::Unknown => "DTXMania",
        }
    }

    /// NX の系統（v2 と 1.x）か
    pub fn is_nx(self) -> bool {
        matches!(self, Self::NxV2 | Self::Nx1)
    }
}

/// 読んだ設定
#[derive(Debug, Clone)]
pub struct Source {
    pub found: Found,
    /// 設定の中身（Config.xml も同じ形に直してある）
    pub ini: Ini,
    pub family: Family,
    /// 設定に書いてある版（`[System] Version`）
    pub version: Option<String>,
    /// 設定のファイルを最後に保存した日時
    pub saved: Option<SystemTime>,
}

impl Source {
    /// 画面に出す名前（「DTXManiaNX v2.0.0 Beta (20260121)」など）
    pub fn title(&self) -> String {
        match &self.version {
            Some(v) => format!("{} {v}", self.family.product()),
            None => self.family.product().to_owned(),
        }
    }
}

/// 読めなかった理由
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// ファイルを開けない（ほかのアプリが使っている、権限が無いなど）
    Io(String),
    /// 中身が DTXMania の設定ではない
    NotDtxmania,
}

/// 大きすぎる設定のファイルは読まない（ふつうは 30 KB ほど）
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 見つけた DTXMania の設定を読む
pub fn read(found: &Found) -> Result<Source, ReadError> {
    let meta = std::fs::metadata(&found.config).map_err(|e| ReadError::Io(e.to_string()))?;
    if meta.len() > MAX_BYTES {
        return Err(ReadError::NotDtxmania);
    }
    let bytes = std::fs::read(&found.config).map_err(|e| ReadError::Io(e.to_string()))?;
    let text = dtx::decode(&bytes).0;
    let ini = match found.format {
        Format::Ini => Ini::parse(&text),
        Format::Xml => xml::to_ini(&text).ok_or(ReadError::NotDtxmania)?,
    };
    if !ini.looks_like_dtxmania() {
        return Err(ReadError::NotDtxmania);
    }
    let version = ini
        .get("System", "Version")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    let family = Family::detect(found.format, version.as_deref(), found.exe.as_deref());
    Ok(Source {
        found: found.clone(),
        ini,
        family,
        version,
        saved: meta.modified().ok(),
    })
}

/// 設定のファイルを直接読む（テストと、場所の分かっているとき）
pub fn read_path(config: &Path) -> Result<Source, ReadError> {
    let found = crate::locate::locate(config).map_err(|_| ReadError::NotDtxmania)?;
    read(&found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_families_apart() {
        let d = |v: &str| Family::detect(Format::Ini, Some(v), None);
        assert_eq!(d("v2.0.0 Beta (20260121)"), Family::NxV2);
        assert_eq!(d("v1.4.2 20240519"), Family::Nx1);
        assert_eq!(d("v1.5.3 2024031700 -sizuku Edition-"), Family::Nx1);
        assert_eq!(d("Ver4.12(231107)"), Family::XgVerK);
        assert_eq!(d("099g(191022)"), Family::Classic);
        assert_eq!(d("097(150118)"), Family::Classic);
        assert_eq!(d("Unknown"), Family::Unknown);
        assert_eq!(
            Family::detect(Format::Ini, None, Some("DTXManiaNX.exe")),
            Family::Nx1
        );
        assert_eq!(
            Family::detect(Format::Ini, None, Some("DTXManiaGR.exe")),
            Family::Unknown
        );
        assert_eq!(
            Family::detect(Format::Xml, Some("118(200510)"), None),
            Family::ClassicXml
        );
    }

    #[test]
    fn reads_a_utf16_config() {
        let dir = std::env::temp_dir().join(format!("nx-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = "[System]\r\nVersion=v2.0.0 Beta (20260121)\r\n[DrumsKeyAssign]\r\nHH=M042\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(dir.join("Config.ini"), bytes).unwrap();
        let s = read_path(&dir.join("Config.ini")).unwrap();
        assert_eq!(s.family, Family::NxV2);
        assert_eq!(s.title(), "DTXManiaNX v2.0.0 Beta (20260121)");
        assert_eq!(s.ini.get("DrumsKeyAssign", "HH"), Some("M042"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
