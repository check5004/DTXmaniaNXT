//! DTXManiaNX のインストール先を読む。
//!
//! - `Config.ini`（UTF-16LE）から曲のフォルダ（`DTXPath`）、スキン（`SkinPath`）、パッドの割り当て（`[DrumsKeyAssign]`）、表示の設定
//! - 使うスキンのフォルダ（`System/<スキン>/`）を上流 DTXManiaNX（`CSkin.cs`）と同じ順で選ぶ
//! - 大文字・小文字を区別せずにファイルを探す（曲のフォルダの中のファイルにも使う）
//! - 譜面の隣の `score.ini` のドラムの記録（NXT の記録に引き継ぐ）
//! - ほかの DTXMania（NX・XG・本家）の場所をショートカット・exe・フォルダから探し、Config.ini をそのまま読む
//!   （設定の画面の「他の DTXMania から引き継ぐ」）
//!
//! NXT の画面の画像と音はアプリに入れてあるので（`assets/skin`）、NX は要らない。
//! ここは 0.1 で NX のフォルダを選んでいた人の設定を引き継ぐのに使う。

mod config;
mod ini;
pub mod lnk;
pub mod locate;
mod score_ini;
mod skin;
pub mod source;
mod xml;

pub use config::{KeyAssign, NxConfig};
pub use ini::{Ini, Section};
pub use locate::{Format, Found, LocateError, discover, locate};
pub use score_ini::{ScoreIni, ScoreRecord};
pub use skin::{find_file, is_install, is_skin, skin_dir};
pub use source::{Family, ReadError, Source};

use std::path::{Path, PathBuf};

/// NX のインストール先 1 つ
#[derive(Debug, Clone)]
pub struct Install {
    /// DTXManiaNX.exe のあるフォルダ
    pub root: PathBuf,
    /// Config.ini の中身（無ければ既定）
    pub config: NxConfig,
    /// 使うスキンのフォルダ（`System/` かその下の 1 つ）
    pub skin: PathBuf,
}

impl Install {
    /// `root` が NX のフォルダなら読む
    pub fn open(root: &Path) -> Option<Self> {
        if !is_install(root) {
            return None;
        }
        let config = find_file(root, "Config.ini")
            .and_then(|p| std::fs::read(p).ok())
            .map(|b| NxConfig::parse(&dtx::decode(&b).0))
            .unwrap_or_default();
        let skin = skin_dir(root, config.skin_path.as_deref());
        Some(Self {
            root: root.to_owned(),
            config,
            skin,
        })
    }

    /// Config.ini の `DTXPath` の曲のフォルダ（相対パスは NX のフォルダから）
    pub fn song_dirs(&self) -> Vec<PathBuf> {
        self.config
            .dtx_paths
            .iter()
            .map(|p| resolve(&self.root, p))
            .collect()
    }
}

/// NX の書き方のパス（`.\DTXFiles\` など）を `base` からの絶対パスにする
pub fn resolve(base: &Path, path: &str) -> PathBuf {
    let path = path.trim();
    let normalized: String = path
        .chars()
        .map(|c| {
            if c == '\\' || c == '/' {
                std::path::MAIN_SEPARATOR
            } else {
                c
            }
        })
        .collect();
    let p = PathBuf::from(&normalized);
    if p.is_absolute() || looks_like_drive(path) {
        return p;
    }
    let mut out = base.to_owned();
    for part in normalized.split(std::path::MAIN_SEPARATOR) {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(part),
        }
    }
    out
}

/// `D:\…` のような Windows のドライブ付きのパス（Windows 以外でも絶対パスとして扱う）
fn looks_like_drive(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_nx_style_paths() {
        let base = Path::new("/nx");
        assert_eq!(resolve(base, ".\\"), PathBuf::from("/nx"));
        assert_eq!(
            resolve(base, ".\\DTXFiles\\"),
            PathBuf::from("/nx").join("DTXFiles")
        );
        assert_eq!(
            resolve(base, "songs/a"),
            PathBuf::from("/nx").join("songs").join("a")
        );
        assert_eq!(resolve(base, "..\\other"), PathBuf::from("/").join("other"));
        assert!(looks_like_drive("D:\\DTX"));
    }
}
