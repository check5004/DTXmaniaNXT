//! 選曲の状態。曲のフォルダを別のスレッドで探し、BOX の出入りと、選んでいる曲・難易度を持つ。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use library::{LEVELS, Library, Node, Progress, Song};

/// 選曲の一覧の 1 行
#[derive(Debug, Clone, Copy)]
pub enum Entry<'a> {
    /// BOX から出る（上流の `<< BACK`）
    Back,
    Node(&'a Node),
    /// この BOX の中から 1 曲を選ぶ（上流の `< RANDOM SELECT >`）
    Random,
}

fn collect_songs<'a>(nodes: &'a [Node], out: &mut Vec<&'a Song>) {
    for n in nodes {
        match n {
            Node::Song(s) => out.push(s),
            Node::Box(b) => collect_songs(&b.nodes, out),
        }
    }
}

/// `0..n` のどれか（xorshift。乱数のための依存を増やさない）
fn pick(seed: u64, n: usize) -> usize {
    let mut x = seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x % n as u64) as usize
}

enum ScanMsg {
    Progress(Progress),
    Done(Library),
}

struct Scanning {
    rx: Receiver<ScanMsg>,
    stop: Arc<AtomicBool>,
    progress: Progress,
}

impl Drop for Scanning {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct SongList {
    pub library: Library,
    /// 今いる BOX までの道（各階層で開いた BOX の位置）
    path: Vec<usize>,
    pub cursor: usize,
    /// 選んでいる難易度（0〜4）。曲にその難易度が無ければ近いものを使う
    pub level: usize,
    scanning: Option<Scanning>,
}

impl SongList {
    pub fn new(level: usize) -> Self {
        Self {
            level: level.min(LEVELS - 1),
            ..Default::default()
        }
    }

    /// 探し直す（探している途中ならやめて、始めから）
    pub fn rescan(&mut self, dirs: Vec<PathBuf>) {
        self.scanning = None;
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let lib = library::scan(&dirs, &stop2, &mut |p| {
                let _ = progress_tx.send(ScanMsg::Progress(p));
            });
            log::info!("songs: {} in {} folders", lib.songs, lib.folders);
            let _ = tx.send(ScanMsg::Done(lib));
        });
        self.scanning = Some(Scanning {
            rx,
            stop,
            progress: Progress::default(),
        });
    }

    /// 探している途中なら、その数
    pub fn scanning(&self) -> Option<Progress> {
        self.scanning.as_ref().map(|s| s.progress)
    }

    /// 探した結果を受け取る
    pub fn poll(&mut self) {
        let Some(s) = &mut self.scanning else {
            return;
        };
        let mut done = None;
        for msg in s.rx.try_iter() {
            match msg {
                ScanMsg::Progress(p) => s.progress = p,
                ScanMsg::Done(lib) => done = Some(lib),
            }
        }
        if let Some(lib) = done {
            self.scanning = None;
            if lib.stopped {
                return;
            }
            self.library = lib;
            // 前の場所がまだあればそこに、無ければ一番上に
            if self.try_nodes(&self.path).is_none() {
                self.path.clear();
                self.cursor = 0;
            }
            self.cursor = self.cursor.min(self.len().saturating_sub(1));
        }
    }

    fn try_nodes(&self, path: &[usize]) -> Option<&[Node]> {
        let mut nodes = self.library.nodes.as_slice();
        for &i in path {
            match nodes.get(i)? {
                Node::Box(b) => nodes = &b.nodes,
                Node::Song(_) => return None,
            }
        }
        Some(nodes)
    }

    /// 今いる BOX の中身
    pub fn nodes(&self) -> &[Node] {
        self.try_nodes(&self.path).unwrap_or(&[])
    }

    /// 今いる BOX の名前の並び（外から）
    pub fn breadcrumbs(&self) -> Vec<&str> {
        let mut out = Vec::new();
        let mut nodes = self.library.nodes.as_slice();
        for &i in &self.path {
            if let Some(Node::Box(b)) = nodes.get(i) {
                out.push(b.title.as_str());
                nodes = &b.nodes;
            }
        }
        out
    }

    /// 今いる BOX の行の数（`<< BACK` と `< RANDOM SELECT >` も数える）
    pub fn len(&self) -> usize {
        let n = self.nodes().len();
        self.back_rows() + n + usize::from(n > 0)
    }

    /// 先頭の `<< BACK` の行の数（BOX の中なら 1）
    fn back_rows(&self) -> usize {
        usize::from(!self.path.is_empty())
    }

    /// `i` 行目（上流と同じく、BOX の中は先頭に `<< BACK`、曲があれば最後に `< RANDOM SELECT >`）
    pub fn entry(&self, i: usize) -> Option<Entry<'_>> {
        let back = self.back_rows();
        let nodes = self.nodes();
        if i < back {
            return Some(Entry::Back);
        }
        match nodes.get(i - back) {
            Some(n) => Some(Entry::Node(n)),
            None if i == back + nodes.len() && !nodes.is_empty() => Some(Entry::Random),
            None => None,
        }
    }

    pub fn selected_entry(&self) -> Option<Entry<'_>> {
        self.entry(self.cursor)
    }

    pub fn selected(&self) -> Option<&Node> {
        match self.selected_entry()? {
            Entry::Node(n) => Some(n),
            _ => None,
        }
    }

    pub fn selected_song(&self) -> Option<&Song> {
        match self.selected()? {
            Node::Song(s) => Some(s),
            Node::Box(_) => None,
        }
    }

    /// 曲に合わせた難易度（選んでいる難易度が無ければ近いもの）
    pub fn level_for(&self, song: &Song) -> Option<usize> {
        song.nearest_level(self.level)
    }

    /// 上下に動く（端から端へ回る）
    pub fn move_by(&mut self, delta: isize) {
        let n = self.len();
        if n == 0 {
            return;
        }
        let n = n as isize;
        self.cursor = (self.cursor as isize + delta).rem_euclid(n) as usize;
    }

    /// 選んでいる曲の、今の難易度の譜面
    pub fn selected_chart(&self) -> Option<PathBuf> {
        let s = self.selected_song()?;
        let level = self.level_for(s)?;
        s.charts[level].as_ref().map(|c| c.path.clone())
    }

    /// 譜面の難易度の名前（set.def の `#L1LABEL` など）。一覧に無いか名前が無ければ空
    pub fn chart_label(&self, path: &Path) -> String {
        self.library
            .all_songs()
            .into_iter()
            .flat_map(|s| s.charts.iter().flatten())
            .find(|c| c.path == path)
            .map(|c| c.label.clone())
            .unwrap_or_default()
    }

    /// BOX なら入り、`<< BACK` なら出て、曲なら遊ぶ譜面を返す。`< RANDOM SELECT >` は
    /// この BOX の中（下の BOX も含む）から 1 曲選ぶ
    pub fn enter(&mut self) -> Option<PathBuf> {
        match self.selected_entry()? {
            Entry::Back => {
                self.back();
                None
            }
            Entry::Random => self.random_chart(),
            Entry::Node(Node::Box(_)) => {
                self.path.push(self.cursor - self.back_rows());
                // 上流と同じく、入ったら先頭（`<< BACK`）を選ぶ
                self.cursor = 0;
                None
            }
            Entry::Node(Node::Song(s)) => {
                let level = self.level_for(s)?;
                s.charts[level].as_ref().map(|c| c.path.clone())
            }
        }
    }

    /// 今いる BOX の中の曲から 1 つ選び、今の難易度に近い譜面を返す
    fn random_chart(&self) -> Option<PathBuf> {
        let mut songs = Vec::new();
        collect_songs(self.nodes(), &mut songs);
        if songs.is_empty() {
            return None;
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let song = songs[pick(seed, songs.len())];
        let level = self.level_for(song)?;
        song.charts[level].as_ref().map(|c| c.path.clone())
    }

    /// BOX から出る。一番上なら false
    pub fn back(&mut self) -> bool {
        match self.path.pop() {
            Some(i) => {
                self.cursor = i + self.back_rows();
                true
            }
            None => false,
        }
    }

    /// 難易度をひとつ上へ。いちばん上の次はいちばん下に戻る（上流の HH 2 回と同じ）
    pub fn cycle_level(&mut self) {
        let Some(song) = self.selected_song() else {
            return;
        };
        let Some(mut level) = self.level_for(song) else {
            return;
        };
        for _ in 0..LEVELS {
            level = (level + 1) % LEVELS;
            if song.charts[level].is_some() {
                break;
            }
        }
        self.level = level;
    }

    /// 難易度を変える（選んでいる曲にある難易度だけをたどる）
    pub fn change_level(&mut self, delta: isize) {
        let Some(song) = self.selected_song() else {
            return;
        };
        let levels: Vec<usize> = song.levels().collect();
        if levels.is_empty() {
            return;
        }
        let cur = self.level_for(song).unwrap_or(levels[0]);
        let pos = levels.iter().position(|&l| l == cur).unwrap_or(0) as isize;
        let next = (pos + delta).clamp(0, levels.len() as isize - 1) as usize;
        self.level = levels[next];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use library::{ChartHead, SongBox, SongChart};

    fn song(title: &str, levels: &[usize]) -> Node {
        let mut charts: [Option<SongChart>; LEVELS] = Default::default();
        for &l in levels {
            charts[l] = Some(SongChart {
                label: format!("L{l}"),
                path: PathBuf::from(format!("{title}{l}.dtx")),
                head: ChartHead::default(),
            });
        }
        Node::Song(Box::new(Song {
            title: title.into(),
            artist: String::new(),
            genre: String::new(),
            dir: PathBuf::new(),
            set_def_block: None,
            charts,
        }))
    }

    fn list() -> SongList {
        let mut l = SongList::new(2);
        l.library.nodes = vec![
            Node::Box(SongBox {
                title: "BOX".into(),
                nodes: vec![song("a", &[0, 3]), song("b", &[1])],
                ..Default::default()
            }),
            song("c", &[2]),
        ];
        l
    }

    #[test]
    fn enter_and_leave_a_box() {
        let mut l = list();
        assert_eq!(l.enter(), None);
        assert_eq!(l.breadcrumbs(), vec!["BOX"]);
        // `<< BACK`、a、b、`< RANDOM SELECT >`
        assert_eq!(l.len(), 4);
        assert!(matches!(l.selected_entry(), Some(Entry::Back)));
        l.move_by(1);
        // 難易度 2 が無いので上の 3
        assert_eq!(l.enter(), Some(PathBuf::from("a3.dtx")));
        l.move_by(1);
        // 2 から上に無いので下の 1
        assert_eq!(l.enter(), Some(PathBuf::from("b1.dtx")));
        // `<< BACK` で出ると、入った BOX を選んでいる
        l.move_by(-2);
        assert_eq!(l.enter(), None);
        assert!(l.breadcrumbs().is_empty());
        assert_eq!(l.cursor, 0);
        l.enter();
        assert!(l.back());
        assert_eq!(l.cursor, 0);
        assert!(!l.back());
    }

    #[test]
    fn random_picks_a_song_in_the_box() {
        let mut l = list();
        // 一番上は BOX、c、`< RANDOM SELECT >`（`<< BACK` は無い）
        assert_eq!(l.len(), 3);
        l.move_by(-1);
        assert!(matches!(l.selected_entry(), Some(Entry::Random)));
        let p = l.enter().unwrap();
        assert!(
            ["a3.dtx", "b1.dtx", "c2.dtx"].contains(&p.to_str().unwrap()),
            "{p:?}"
        );
        assert!(pick(12345, 3) < 3);
    }

    #[test]
    fn cycle_level_wraps_to_the_easiest() {
        let mut l = list();
        l.enter();
        l.move_by(1);
        // a は 0 と 3。いまは 3 扱い
        l.cycle_level();
        assert_eq!(l.level, 0);
        l.cycle_level();
        assert_eq!(l.level, 3);
    }

    #[test]
    fn move_wraps_around() {
        let mut l = list();
        l.move_by(-1);
        assert_eq!(l.cursor, 2);
        l.move_by(1);
        assert_eq!(l.cursor, 0);
    }

    #[test]
    fn level_follows_the_song() {
        let mut l = list();
        l.enter();
        l.move_by(1);
        // a は 0 と 3。いまは 3 扱いなので、下げると 0
        l.change_level(-1);
        assert_eq!(l.level, 0);
        l.change_level(-1);
        assert_eq!(l.level, 0);
        l.change_level(1);
        assert_eq!(l.level, 3);
    }
}
