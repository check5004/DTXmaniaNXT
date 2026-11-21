//! NX のスキンと Config.ini を、NXT のフォルダへ写す。
//!
//! 写した先は NX のフォルダと同じ形（`Config.ini` と `System/<スキン>/`）にするので、
//! [`Install::open`] でそのまま読める。NX のフォルダを消しても、写したもので同じ画面が出る。
//! 画像と音はユーザーの PC の中で写すだけで、NXT のリポジトリには入れない。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Install, find_file, is_skin};

/// `install` の使っているスキンと Config.ini を `dest` へ写し、写したものを開く。
/// 前に写したものがあれば置き換える（写し終えてから入れ替えるので、途中で失敗しても前のものは残る）
pub fn copy_install(install: &Install, dest: &Path) -> io::Result<Install> {
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("the destination has no parent folder"))?;
    fs::create_dir_all(parent)?;
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "nx".to_owned());
    let tmp = parent.join(format!("{name}.tmp"));
    let old = parent.join(format!("{name}.old"));
    remove_if_exists(&tmp)?;
    remove_if_exists(&old)?;
    fs::create_dir_all(&tmp)?;
    if let Err(e) = fill(install, &tmp) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if dest.exists() {
        fs::rename(dest, &old)?;
    }
    if let Err(e) = fs::rename(&tmp, dest) {
        // 入れ替えに失敗したら前のものを戻す
        if old.exists() {
            let _ = fs::rename(&old, dest);
        }
        return Err(e);
    }
    let _ = fs::remove_dir_all(&old);
    Install::open(dest).ok_or_else(|| io::Error::other("the copy is not an NX install"))
}

/// 写す中身：Config.ini と、スキンを `System/` から同じ場所に
fn fill(install: &Install, to: &Path) -> io::Result<()> {
    if let Some(ini) = find_file(&install.root, "Config.ini").filter(|p| p.is_file()) {
        fs::copy(ini, to.join("Config.ini"))?;
    }
    let system = find_file(&install.root, "System").unwrap_or_else(|| install.root.join("System"));
    let skin_to = to.join("System").join(skin_rel(&system, &install.skin));
    // System そのものがスキンのときは、その下のほかのスキンは写さない
    let skip_skins = install.skin == system;
    copy_dir(&install.skin, &skin_to, skip_skins)
}

/// スキンの `System/` からの場所。`System/` の外なら最後の名前だけ
fn skin_rel(system: &Path, skin: &Path) -> PathBuf {
    match skin.strip_prefix(system) {
        Ok(rel) => rel.to_owned(),
        Err(_) => skin
            .file_name()
            .map_or_else(|| PathBuf::from("Default"), PathBuf::from),
    }
}

fn copy_dir(from: &Path, to: &Path, skip_skins: bool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            if skip_skins && is_skin(&path) {
                continue;
            }
            copy_dir(&path, &target, false)?;
        } else if path.is_file() {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn remove_if_exists(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skins_keep_their_place_under_system() {
        let system = Path::new("/nx/System");
        assert_eq!(
            skin_rel(system, Path::new("/nx/System/Default")),
            PathBuf::from("Default")
        );
        assert_eq!(skin_rel(system, system), PathBuf::new());
        assert_eq!(
            skin_rel(system, Path::new("/elsewhere/MySkin")),
            PathBuf::from("MySkin")
        );
    }
}
