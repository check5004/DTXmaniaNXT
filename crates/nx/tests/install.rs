//! NX のフォルダを作って、上流と同じスキンを選ぶか。

use std::fs;
use std::path::{Path, PathBuf};

use nx::{Install, find_file, skin_dir};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("nxt-nx-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn touch(root: &Path, rel: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, b"x").unwrap();
}

/// Config.ini を NX と同じ UTF-16LE（BOM 付き）で書く
fn write_config(root: &Path, text: &str) {
    let mut bytes = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    fs::write(root.join("Config.ini"), bytes).unwrap();
}

#[test]
fn picks_the_skin_in_upstream_order() {
    let t = TempDir::new("order");
    let r = &t.0;
    // System/ そのものもスキン（NX の標準の置き方）
    touch(r, "System/Graphics/1_background.jpg");
    assert_eq!(skin_dir(r, None), r.join("System"));
    // System の下にスキンがあれば、名前の順で最初のもの
    touch(r, "System/b_skin/Graphics/1_background.jpg");
    touch(r, "System/A_skin/Graphics/1_background.jpg");
    touch(r, "System/empty/readme.txt");
    assert_eq!(skin_dir(r, None), r.join("System").join("A_skin"));
    // Default が先
    touch(r, "System/Default/Graphics/1_background.jpg");
    assert_eq!(skin_dir(r, None), r.join("System").join("Default"));
    // SkinPath が最優先。スキンでなければ使わない
    assert_eq!(
        skin_dir(r, Some(".\\b_skin\\")),
        r.join("System").join("b_skin")
    );
    assert_eq!(
        skin_dir(r, Some(".\\empty\\")),
        r.join("System").join("Default")
    );
    // SkinPath が空（`.\`）なら System そのもの
    assert_eq!(skin_dir(r, Some(".\\")), r.join("System"));
}

#[test]
fn reads_config_and_song_folders() {
    let t = TempDir::new("config");
    let r = &t.0;
    touch(r, "System/Graphics/1_background.jpg");
    write_config(
        r,
        "[System]\r\nDTXPath=.\\;.\\Songs\\\r\nSkinPath=\r\n[DrumsKeyAssign]\r\nBD=M036\r\n",
    );
    let nx = Install::open(r).expect("an install");
    assert_eq!(nx.skin, r.join("System"));
    assert_eq!(nx.song_dirs(), vec![r.clone(), r.join("Songs")]);
    assert_eq!(nx.config.midi_notes(), vec![(36, dtx::DrumLane::BD)]);
    assert!(Install::open(&r.join("System")).is_none());
}

#[test]
fn finds_files_without_case() {
    let t = TempDir::new("case");
    let r = &t.0;
    touch(r, "System/Graphics/5_Bar Box.PNG");
    let p = find_file(r, "system\\graphics\\5_bar box.png").unwrap();
    assert!(p.is_file());
    assert!(find_file(r, "System/Graphics/missing.png").is_none());
}

#[test]
fn copies_the_skin_and_config_so_nx_can_be_deleted() {
    let t = TempDir::new("copy");
    let nx_root = t.0.join("DTXManiaNX");
    touch(&nx_root, "System/Default/Graphics/1_background.jpg");
    touch(&nx_root, "System/Default/Sounds/Decide.ogg");
    touch(&nx_root, "System/Default/Script/difficult.dtxs");
    touch(&nx_root, "System/Other/Graphics/1_background.jpg");
    touch(&nx_root, "Songs/a/a.dtx");
    write_config(
        &nx_root,
        "[System]\r\nDTXPath=.\\Songs\\\r\nSkinPath=.\\Default\\\r\n[DrumsKeyAssign]\r\nBD=M036\r\n",
    );
    let original = Install::open(&nx_root).unwrap();
    let dest = t.0.join("config").join("nx");
    let copy = nx::copy_install(&original, &dest).unwrap();
    assert_eq!(copy.skin, dest.join("System").join("Default"));
    assert_eq!(copy.config.drums, original.config.drums);
    assert!(copy.skin.join("Sounds/Decide.ogg").is_file());
    assert!(copy.skin.join("Script/difficult.dtxs").is_file());
    // 使っていないスキンと曲は写さない
    assert!(!dest.join("System/Other").exists());
    assert!(!dest.join("Songs").exists());
    // 写し直すと置き換わる（作りかけのフォルダは残らない）
    touch(&nx_root, "System/Default/Graphics/new.png");
    let copy = nx::copy_install(&original, &dest).unwrap();
    assert!(copy.skin.join("Graphics/new.png").is_file());
    assert!(!t.0.join("config").join("nx.tmp").exists());
    assert!(!t.0.join("config").join("nx.old").exists());
    // NX を消しても写したもので開ける
    fs::remove_dir_all(&nx_root).unwrap();
    let reopened = Install::open(&dest).unwrap();
    assert_eq!(reopened.skin, dest.join("System").join("Default"));
}

#[test]
fn copying_the_old_layout_skips_other_skins() {
    let t = TempDir::new("copy-old");
    let nx_root = t.0.join("nx");
    // System そのものがスキン（SkinPath が `.\`）で、下に別のスキンもある
    touch(&nx_root, "System/Graphics/1_background.jpg");
    touch(&nx_root, "System/Sounds/Move.ogg");
    touch(&nx_root, "System/Default/Graphics/1_background.jpg");
    write_config(&nx_root, "[System]\r\nSkinPath=.\\\r\n");
    let original = Install::open(&nx_root).unwrap();
    assert_eq!(original.skin, nx_root.join("System"));
    let dest = t.0.join("copy");
    let copy = nx::copy_install(&original, &dest).unwrap();
    assert_eq!(copy.skin, dest.join("System"));
    assert!(dest.join("System/Sounds/Move.ogg").is_file());
    assert!(!dest.join("System/Default").exists());
}
