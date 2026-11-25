//! 曲のフォルダを探して、選曲の一覧（BOX と曲の木）を作る。
//!
//! 上流 DTXManiaNX（`CSongManager.cs`）の探し方に合わせている。
//! - set.def のあるフォルダ: set.def のブロックごとに 1 曲。L1〜L5 の譜面のうち、ファイルがあるものだけ。
//!   譜面が 1 つも無いブロックは出さない
//! - set.def の無いフォルダ: 譜面 1 つずつが別の曲
//! - 名前が `DTXFiles.` で始まるフォルダは BOX（名前は後ろの部分。box.def の `#TITLE` があればそちら）
//! - box.def のあるフォルダは BOX（名前は box.def の `#TITLE`）
//! - そのほかのフォルダは素通り（中の曲や BOX は、今の階層に並ぶ）
//! - 探し始めのフォルダ自身は BOX にしない
//!
//! 譜面は頭だけ読む（曲名・アーティスト・レベルなど）。何も書かない。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::setdef::{LEVELS, parse_set_def, split_key};

/// 曲の一覧に出す譜面の拡張子。上流は .gda / .g2d / .bms / .bme も出すが、
/// ギターの譜面と BMS はまだ遊べないので出さない
const CHART_EXTENSIONS: &[&str] = &["dtx"];

/// 譜面の頭として読む大きさ。足りなければ全部読む
const HEAD_BYTES: usize = 64 * 1024;

/// フォルダの深さの上限（リンクの輪で止まらないように）
const MAX_DEPTH: usize = 16;

/// 譜面の頭（一覧に出す情報）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartHead {
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub comment: String,
    pub bpm: Option<f64>,
    /// ドラムのレベル（8.25 など）
    pub level: Option<f64>,
    pub preview: String,
    pub preimage: String,
}

/// 曲の難易度 1 つ
#[derive(Debug, Clone, PartialEq)]
pub struct SongChart {
    /// 難易度の名前（set.def が無ければ空）
    pub label: String,
    pub path: PathBuf,
    pub head: ChartHead,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Song {
    pub title: String,
    pub artist: String,
    pub genre: String,
    /// 曲のフォルダ
    pub dir: PathBuf,
    /// set.def の何番目のブロックか（set.def が無ければ None）
    pub set_def_block: Option<usize>,
    /// 難易度 1〜5（set.def が無い曲は 1 つ目だけ）
    pub charts: [Option<SongChart>; LEVELS],
}

impl Song {
    /// 譜面のある難易度の番号（0 から）
    pub fn levels(&self) -> impl Iterator<Item = usize> + '_ {
        (0..LEVELS).filter(|&i| self.charts[i].is_some())
    }

    /// `want` に近い、譜面のある難易度（上流と同じく、無ければ上に、それも無ければ下に探す）
    pub fn nearest_level(&self, want: usize) -> Option<usize> {
        let want = want.min(LEVELS - 1);
        (want..LEVELS)
            .chain((0..want).rev())
            .find(|&i| self.charts[i].is_some())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SongBox {
    pub title: String,
    pub genre: String,
    /// box.def の `#ARTIST`・`#COMMENT`・`#PREIMAGE`・`#PREVIEW`（無ければ空。画像と音は BOX のフォルダからの相対）
    pub artist: String,
    pub comment: String,
    pub preimage: String,
    pub preview: String,
    pub dir: PathBuf,
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Box(SongBox),
    Song(Box<Song>),
}

impl Node {
    pub fn title(&self) -> &str {
        match self {
            Node::Box(b) => &b.title,
            Node::Song(s) => &s.title,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    pub nodes: Vec<Node>,
    pub folders: usize,
    pub songs: usize,
    /// 途中でやめた
    pub stopped: bool,
}

impl Library {
    /// 木をたどって、すべての曲
    pub fn all_songs(&self) -> Vec<&Song> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Song>) {
            for n in nodes {
                match n {
                    Node::Box(b) => walk(&b.nodes, out),
                    Node::Song(s) => out.push(s.as_ref()),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes, &mut out);
        out
    }
}

/// 探している途中の数
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub folders: usize,
    pub songs: usize,
}

struct State<'a> {
    folders: usize,
    songs: usize,
    stop: &'a AtomicBool,
    progress: &'a mut dyn FnMut(Progress),
}

impl State<'_> {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn report(&mut self) {
        (self.progress)(Progress {
            folders: self.folders,
            songs: self.songs,
        });
    }
}

/// `roots` の下を探す。`stop` が立ったら途中でやめる
pub fn scan(roots: &[PathBuf], stop: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Library {
    let mut state = State {
        folders: 0,
        songs: 0,
        stop,
        progress,
    };
    let mut nodes = Vec::new();
    for root in roots {
        scan_dir(root, 0, &mut nodes, &mut state);
    }
    Library {
        nodes,
        folders: state.folders,
        songs: state.songs,
        stopped: state.stopped(),
    }
}

struct Entries {
    /// (名前, パス)。名前の順
    files: Vec<(String, PathBuf)>,
    dirs: Vec<(String, PathBuf)>,
}

impl Entries {
    fn read(dir: &Path) -> Option<Self> {
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for e in fs::read_dir(dir).ok()?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            // fs::metadata はリンクの先を見る
            match fs::metadata(&path) {
                // 曲を追加している途中の作業フォルダは、まだ一覧に出さない
                Ok(m) if m.is_dir() && crate::import::is_work_dir(&name) => {}
                Ok(m) if m.is_dir() => dirs.push((name, path)),
                Ok(m) if m.is_file() => files.push((name, path)),
                _ => {}
            }
        }
        files.sort();
        dirs.sort();
        Some(Self { files, dirs })
    }

    /// 名前の大文字・小文字を見ずにファイルを探す
    fn file(&self, name: &str) -> Option<&Path> {
        self.files
            .iter()
            .find(|(n, _)| same_name(n, name))
            .map(|(_, p)| p.as_path())
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

fn scan_dir(dir: &Path, depth: usize, out: &mut Vec<Node>, st: &mut State) {
    if depth > MAX_DEPTH || st.stopped() {
        return;
    }
    let Some(entries) = Entries::read(dir) else {
        return;
    };
    st.folders += 1;

    // このフォルダの曲
    if let Some(set_def) = entries.file("set.def") {
        let text = fs::read(set_def)
            .map(|b| dtx::decode(&b).0)
            .unwrap_or_default();
        for (bi, block) in parse_set_def(&text).into_iter().enumerate() {
            let mut charts: [Option<SongChart>; LEVELS] = Default::default();
            for (i, file) in block.files.iter().enumerate() {
                let Some(file) = file else {
                    continue;
                };
                let Some(path) = resolve(dir, file) else {
                    continue;
                };
                let Some(head) = read_head(&path) else {
                    continue;
                };
                charts[i] = Some(SongChart {
                    label: block.labels[i].clone().unwrap_or_default(),
                    path,
                    head,
                });
            }
            if let Some(song) = make_song(dir, Some(bi), &block.title, &block.genre, charts) {
                out.push(Node::Song(Box::new(song)));
                st.songs += 1;
            }
        }
    } else {
        for (name, path) in &entries.files {
            if !is_chart(name) {
                continue;
            }
            let Some(head) = read_head(path) else {
                continue;
            };
            let mut charts: [Option<SongChart>; LEVELS] = Default::default();
            charts[0] = Some(SongChart {
                label: String::new(),
                path: path.clone(),
                head,
            });
            if let Some(song) = make_song(dir, None, "", "", charts) {
                out.push(Node::Song(Box::new(song)));
                st.songs += 1;
            }
        }
    }
    st.report();

    // 下のフォルダ
    for (name, path) in &entries.dirs {
        if st.stopped() {
            return;
        }
        let dot = dot_box_name(name);
        let box_def = Entries::read(path)
            .and_then(|e| e.file("box.def").map(Path::to_owned))
            .and_then(|p| fs::read(p).ok())
            .map(|b| read_box_def(&dtx::decode(&b).0));
        if dot.is_none() && box_def.is_none() {
            scan_dir(path, depth + 1, out, st);
            continue;
        }
        let def = box_def.unwrap_or_default();
        let title = if !def.title.is_empty() {
            def.title
        } else if let Some(d) = dot {
            d.to_owned()
        } else {
            name.clone()
        };
        let mut nodes = Vec::new();
        scan_dir(path, depth + 1, &mut nodes, st);
        out.push(Node::Box(SongBox {
            title,
            genre: def.genre,
            artist: def.artist,
            comment: def.comment,
            preimage: def.preimage,
            preview: def.preview,
            dir: path.clone(),
            nodes,
        }));
    }
}

fn is_chart(name: &str) -> bool {
    // macOS が作る「._」で始まる影のファイルは譜面ではない
    if name.starts_with("._") {
        return false;
    }
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| CHART_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn make_song(
    dir: &Path,
    set_def_block: Option<usize>,
    title: &str,
    genre: &str,
    charts: [Option<SongChart>; LEVELS],
) -> Option<Song> {
    // 上の難易度から、書いてある値を探す
    let top: Vec<&SongChart> = charts.iter().rev().flatten().collect();
    let first = top.last()?;
    let pick = |f: fn(&ChartHead) -> &str| -> String {
        top.iter()
            .map(|c| f(&c.head))
            .find(|v| !v.trim().is_empty())
            .unwrap_or("")
            .to_owned()
    };
    let title = if !title.trim().is_empty() {
        title.trim().to_owned()
    } else {
        let t = pick(|h| &h.title);
        if t.is_empty() {
            first
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            t
        }
    };
    let genre = if genre.trim().is_empty() {
        pick(|h| &h.genre)
    } else {
        genre.trim().to_owned()
    };
    Some(Song {
        title,
        artist: pick(|h| &h.artist),
        genre,
        dir: dir.to_owned(),
        set_def_block,
        charts,
    })
}

/// 名前が `DTXFiles.` で始まるフォルダは BOX。BOX の名前（後ろの部分）を返す
pub fn dot_box_name(dir_name: &str) -> Option<&str> {
    const PREFIX: &str = "dtxfiles.";
    (dir_name.len() >= PREFIX.len()
        && dir_name.is_char_boundary(PREFIX.len())
        && dir_name[..PREFIX.len()].eq_ignore_ascii_case(PREFIX))
    .then(|| &dir_name[PREFIX.len()..])
}

/// box.def のうち、いま使うもの
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxDef {
    pub title: String,
    pub genre: String,
    pub artist: String,
    pub comment: String,
    pub preimage: String,
    pub preview: String,
}

pub fn read_box_def(text: &str) -> BoxDef {
    let mut def = BoxDef::default();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for line in text.lines() {
        let Some(body) = line.trim_start_matches([' ', '\t']).strip_prefix('#') else {
            continue;
        };
        let body = body.split(';').next().unwrap_or("");
        let Some((key, value)) = split_key(body) else {
            continue;
        };
        let field = match key.to_ascii_uppercase().as_str() {
            "TITLE" => &mut def.title,
            "GENRE" => &mut def.genre,
            "ARTIST" => &mut def.artist,
            "COMMENT" => &mut def.comment,
            "PREIMAGE" => &mut def.preimage,
            "PREVIEW" => &mut def.preview,
            _ => continue,
        };
        *field = value.to_owned();
    }
    def
}

/// 曲のフォルダからの相対パスのファイルを探す（`\` も `/` も区切り。大文字・小文字は見ない）
pub fn resolve(dir: &Path, rel: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = rel
        .split(['\\', '/'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.is_empty() {
        return None;
    }
    let direct: PathBuf = parts.iter().fold(dir.to_owned(), |p, x| p.join(x));
    if direct.is_file() {
        return Some(direct);
    }
    let mut cur = dir.to_owned();
    for (i, part) in parts.iter().enumerate() {
        if *part == ".." {
            cur.pop();
            continue;
        }
        let last = i + 1 == parts.len();
        let found = fs::read_dir(&cur).ok()?.flatten().find(|e| {
            same_name(&e.file_name().to_string_lossy(), part)
                && fs::metadata(e.path()).is_ok_and(|m| if last { m.is_file() } else { m.is_dir() })
        })?;
        cur = found.path();
    }
    Some(cur)
}

/// 譜面の頭を読む。頭に曲名かレベルが無ければ全部読む
pub fn read_head(path: &Path) -> Option<ChartHead> {
    use std::io::Read;
    let mut f = fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(HEAD_BYTES);
    f.by_ref()
        .take(HEAD_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .ok()?;
    let whole = buf.len() <= HEAD_BYTES;
    let mut head = head_of(&buf, whole);
    if !whole && (head.title.is_empty() || head.level.is_none()) {
        f.read_to_end(&mut buf).ok()?;
        head = head_of(&buf, true);
    }
    Some(head)
}

fn head_of(bytes: &[u8], whole: bool) -> ChartHead {
    // 途中で切ったときは最後の行を捨てる（文字の途中で切れて文字コードを見誤らないように）
    let bytes = if whole {
        bytes
    } else {
        match bytes.iter().rposition(|&b| b == b'\n') {
            Some(i) => &bytes[..=i],
            None => bytes,
        }
    };
    let chart = dtx::parse(&dtx::decode(bytes).0);
    let field = |k: &str| chart.field(k).unwrap_or("").trim().to_owned();
    ChartHead {
        title: field("TITLE"),
        artist: field("ARTIST"),
        genre: field("GENRE"),
        comment: field("COMMENT"),
        bpm: chart.field("BPM").and_then(dtx::parse_nx_number),
        level: chart.drum_level(),
        preview: field("PREVIEW"),
        preimage: field("PREIMAGE"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_box() {
        assert_eq!(dot_box_name("DTXFiles.Rock"), Some("Rock"));
        assert_eq!(dot_box_name("dtxfiles.ロック"), Some("ロック"));
        assert_eq!(dot_box_name("DTXFiles"), None);
        assert_eq!(dot_box_name("曲"), None);
    }

    #[test]
    fn box_def_title() {
        let d = read_box_def(
            "#TITLE: ボックス ; 注釈\n#GENRE J-POP\n#artist 誰か\n#COMMENT 説明\n#PREIMAGE box.png\n#PREVIEW pre.ogg\n",
        );
        assert_eq!(d.title, "ボックス");
        assert_eq!(d.genre, "J-POP");
        assert_eq!(d.artist, "誰か");
        assert_eq!(d.comment, "説明");
        assert_eq!(d.preimage, "box.png");
        assert_eq!(d.preview, "pre.ogg");
    }

    #[test]
    fn chart_extensions() {
        assert!(is_chart("a.DTX"));
        assert!(!is_chart("._a.dtx"));
        assert!(!is_chart("a.gda"));
        assert!(!is_chart("a.wav"));
    }

    #[test]
    fn nearest_level_goes_up_then_down() {
        let chart = |l: &str| {
            Some(SongChart {
                label: l.into(),
                path: PathBuf::new(),
                head: ChartHead::default(),
            })
        };
        let song = Song {
            title: String::new(),
            artist: String::new(),
            genre: String::new(),
            dir: PathBuf::new(),
            set_def_block: None,
            charts: [None, chart("B"), None, chart("D"), None],
        };
        assert_eq!(song.nearest_level(0), Some(1));
        assert_eq!(song.nearest_level(2), Some(3));
        assert_eq!(song.nearest_level(4), Some(3));
        assert_eq!(song.levels().collect::<Vec<_>>(), vec![1, 3]);
    }
}
