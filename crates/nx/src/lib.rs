//! DTXManiaNX のインストール先を読む。
//!
//! - `Config.ini`（UTF-16LE）から曲のフォルダ（`DTXPath`）、スキン（`SkinPath`）、パッドの割り当て（`[DrumsKeyAssign]`）
//! - 使うスキンのフォルダ（`System/<スキン>/`）を上流 DTXManiaNX（`CSkin.cs`）と同じ順で選ぶ
//!
//! - 使うスキンと Config.ini を NXT のフォルダへ写す（NX のフォルダを消しても同じ画面で動くように）
//!
//! スキンの画像や音は NXT のリポジトリに同梱しない。ユーザーの PC の中で、NX のフォルダから写して使う。

mod config;
mod copy;
mod skin;

pub use config::{KeyAssign, NxConfig};
pub use copy::copy_install;
pub use skin::{find_file, is_install, is_skin, skin_dir};

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
