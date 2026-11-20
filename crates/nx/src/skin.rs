//! スキンのフォルダを選ぶ（上流 `CSkin.ReloadSkinPaths` と同じ順）。
//!
//! 1. Config.ini の `SkinPath`（`System/` からの相対）がスキンならそれ
//! 2. `System/Default/`
//! 3. `System/` の中のスキンで、名前の順で最初のもの
//! 4. `System/` そのもの（昔の置き方）
//!
//! スキンかどうかは `Graphics/1_background.jpg` があるかで見る（上流 `CSkin.bIsValid`）。
//! NX は Windows のアプリで、ファイル名の大文字・小文字が書いてあるとおりとは限らないので、
//! 大文字・小文字を区別するファイルシステムでも見つかるように探す。

use std::path::{Path, PathBuf};

/// `dir` が NX のインストール先か（`System` フォルダがある）
pub fn is_install(dir: &Path) -> bool {
    find_file(dir, "System").is_some_and(|p| p.is_dir())
}

/// `dir` がスキンのフォルダか
pub fn is_skin(dir: &Path) -> bool {
    find_file(dir, "Graphics/1_background.jpg").is_some_and(|p| p.is_file())
}

/// 使うスキンのフォルダ
pub fn skin_dir(root: &Path, skin_path: Option<&str>) -> PathBuf {
    let system = find_file(root, "System").unwrap_or_else(|| root.join("System"));
    if let Some(sp) = skin_path {
        let dir = crate::resolve(&system, sp);
        if is_skin(&dir) {
            return dir;
        }
    }
    if let Some(dir) = find_file(&system, "Default")
        && is_skin(&dir)
    {
        return dir;
    }
    let mut subdirs: Vec<PathBuf> = std::fs::read_dir(&system)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir() && is_skin(p))
                .collect()
        })
        .unwrap_or_default();
    subdirs.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    subdirs.into_iter().next().unwrap_or(system)
}

/// `base` からの相対パス `rel`（`/` か `\` 区切り）のファイルを、大文字・小文字を区別せずに探す
pub fn find_file(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = base.to_owned();
    for part in rel.split(['/', '\\']).filter(|p| !p.is_empty()) {
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let lower = part.to_lowercase();
        let found = std::fs::read_dir(&cur)
            .ok()?
            .filter_map(Result::ok)
            .find(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.to_lowercase() == lower)
            })?;
        cur = found.path();
    }
    Some(cur)
}
