//! 曲のフォルダを作って探す。上流 DTXManiaNX の探し方と同じ木になるか。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use library::{Node, scan};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p =
            std::env::temp_dir().join(format!("dtxmanianxt-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

fn titles(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| match n {
            Node::Box(b) => format!("[{}]", b.title),
            Node::Song(s) => s.title.clone(),
        })
        .collect()
}

#[test]
fn builds_the_same_tree_as_upstream() {
    let t = TempDir::new("scan");
    let r = &t.0;
    // 素通りのフォルダ。set.def が無いので譜面 1 つずつが別の曲
    write(
        r,
        "plain/song1/a.dtx",
        b"#TITLE: A\n#ARTIST: X\n#DLEVEL: 50\n",
    );
    write(r, "plain/song1/b.dtx", b"#TITLE: B\n#DLEVEL: 825\n");
    write(r, "plain/song1/readme.txt", b"not a chart");
    // DTXFiles. で始まるフォルダは BOX
    write(
        r,
        "DTXFiles.Rock/s2/set.def",
        b"#TITLE S2\r\n#L1LABEL BASIC\r\n#L1FILE Easy.DTX\r\n#L4FILE hard.dtx\r\n",
    );
    // set.def は Easy.DTX と書いてあるが、ファイルは easy.dtx（大文字・小文字は見ない）
    write(
        r,
        "DTXFiles.Rock/s2/easy.dtx",
        b"#TITLE: S2 easy\n#BPM: 140\n",
    );
    // box.def のあるフォルダは BOX（名前は box.def の #TITLE。Shift_JIS）
    let mut box_def = b"#TITLE ".to_vec();
    box_def.extend_from_slice(&[0x83, 0x7b, 0x83, 0x62, 0x83, 0x4e, 0x83, 0x58]); // ボックス
    box_def.extend_from_slice(b"\r\n");
    write(r, "boxed/box.def", &box_def);
    write(r, "boxed/s3/s3.dtx", b"#ARTIST: Y\n");
    // DTXFiles. の BOX でも box.def の #TITLE が勝つ
    write(r, "DTXFiles.Old/box.def", b"#TITLE: New\n");
    // 譜面が 1 つも無い set.def のブロックは出さない
    write(
        r,
        "plain/song4/set.def",
        b"#TITLE Ghost\n#L1FILE none.dtx\n",
    );

    let stop = AtomicBool::new(false);
    let mut last = None;
    let lib = scan(std::slice::from_ref(r), &stop, &mut |p| last = Some(p));

    // フォルダの名前の順（DTXFiles.Old、DTXFiles.Rock、boxed、plain）
    assert_eq!(
        titles(&lib.nodes),
        vec!["[New]", "[Rock]", "[ボックス]", "A", "B"]
    );
    assert_eq!(lib.songs, 4);
    assert!(!lib.stopped);
    assert_eq!(last.map(|p| p.songs), Some(4));

    let Node::Box(rock) = &lib.nodes[1] else {
        panic!("not a box")
    };
    let Node::Song(s2) = &rock.nodes[0] else {
        panic!("not a song")
    };
    assert_eq!(s2.title, "S2");
    assert_eq!(s2.set_def_block, Some(0));
    assert_eq!(s2.levels().collect::<Vec<_>>(), vec![0]);
    let easy = s2.charts[0].as_ref().unwrap();
    assert_eq!(easy.label, "BASIC");
    assert_eq!(easy.head.bpm, Some(140.0));
    // 大文字・小文字を見ないファイルシステム（Windows・macOS）では set.def の綴りのまま見つかる
    let name = easy
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();
    assert_eq!(name, "easy.dtx");
    assert!(easy.path.is_file());

    let Node::Box(boxed) = &lib.nodes[2] else {
        panic!("not a box")
    };
    // 曲名が無ければファイルの名前
    assert_eq!(titles(&boxed.nodes), vec!["s3"]);

    let Node::Song(b) = &lib.nodes[4] else {
        panic!("not a song")
    };
    assert_eq!(b.charts[0].as_ref().unwrap().head.level, Some(8.25));
    assert_eq!(b.charts[0].as_ref().unwrap().label, "");
}

#[test]
fn stops_when_asked() {
    let t = TempDir::new("stop");
    write(&t.0, "a/a.dtx", b"#TITLE A\n");
    let stop = AtomicBool::new(true);
    let lib = scan(std::slice::from_ref(&t.0), &stop, &mut |_| {});
    assert!(lib.stopped);
    assert!(lib.nodes.is_empty());
}

#[test]
fn reads_the_whole_chart_when_the_head_has_no_title() {
    let t = TempDir::new("long");
    let mut text = String::new();
    for i in 0..5000 {
        text.push_str(&format!("#WAV{:02}: sound{i}.wav\n", i % 100));
    }
    text.push_str("#TITLE: Late\n#DLEVEL: 30\n");
    assert!(text.len() > 64 * 1024);
    write(&t.0, "late.dtx", text.as_bytes());
    let stop = AtomicBool::new(false);
    let lib = scan(std::slice::from_ref(&t.0), &stop, &mut |_| {});
    assert_eq!(titles(&lib.nodes), vec!["Late"]);
}
