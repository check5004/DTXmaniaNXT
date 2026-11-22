//! アプリに入れた DTXManiaNX v2.0 のスキン（画像と音）。
//!
//! 中身はリポジトリの `assets/skin`（上流 BocuD/DTXmaniaNX v2.0 の `Runtime/System`。出どころと扱いは `assets/README.md`）。
//! リリースのビルドでは exe に埋め込むので、exe 1 つで NX と同じ画面になる。
//! 開発中のビルド（debug）は `assets/skin` のフォルダから読む。
//!
//! NX の書き方（`Graphics\5_bar box.png`）でも、大文字・小文字が実際のファイル名と違っても見つける
//! （NX は Windows のアプリで、コードに書いた名前とファイル名の大文字・小文字が揃っていないことがある）。

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(rust_embed::Embed)]
#[folder = "../../assets/skin/"]
struct Bundled;

/// 探すときの名前（`/` 区切りの小文字）→ 実際の名前
static INDEX: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    Bundled::iter()
        .map(|name| (key(&name), name.into_owned()))
        .collect()
});

fn key(rel: &str) -> String {
    rel.split(['/', '\\'])
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/")
        .to_lowercase()
}

/// スキンにそのファイルがある
pub fn has(rel: &str) -> bool {
    INDEX.contains_key(&key(rel))
}

/// スキンのファイルの中身
pub fn read(rel: &str) -> Option<Cow<'static, [u8]>> {
    let name = INDEX.get(&key(rel))?;
    Bundled::get(name).map(|f| f.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_files_in_nx_spelling() {
        assert!(has("Graphics/1_background.jpg"));
        assert!(has("Graphics\\1_background.jpg"));
        assert!(has("graphics/1_BACKGROUND.JPG"));
        assert!(!has("Graphics/no such file.png"));
        let bytes = read("Graphics/1_background.jpg").unwrap();
        // JPEG の頭
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn has_every_sound_the_app_plays() {
        for se in crate::sfx::Se::ALL {
            assert!(has(se.file()), "{} がない", se.file());
        }
    }
}
