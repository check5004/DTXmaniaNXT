//! 選曲の状態。曲のフォルダを別のスレッドで探し、BOX の出入りと、選んでいる曲・難易度を持つ。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use library::{LEVELS, Library, Node, Progress, Song};

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
            self.cursor = self.cursor.min(self.nodes().len().saturating_sub(1));
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

    pub fn selected(&self) -> Option<&Node> {
        self.nodes().get(self.cursor)
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
        let n = self.nodes().len();
        if n == 0 {
            return;
        }
        let n = n as isize;
        self.cursor = (self.cursor as isize + delta).rem_euclid(n) as usize;
    }

    pub fn select(&mut self, index: usize) {
        if index < self.nodes().len() {
            self.cursor = index;
        }
    }

    /// BOX なら入り、曲なら遊ぶ譜面を返す
    /// 選んでいる曲の、今の難易度の譜面
    pub fn selected_chart(&self) -> Option<PathBuf> {
        let s = self.selected_song()?;
        let level = self.level_for(s)?;
        s.charts[level].as_ref().map(|c| c.path.clone())
    }

    pub fn enter(&mut self) -> Option<PathBuf> {
        match self.selected()? {
            Node::Box(_) => {
                self.path.push(self.cursor);
                self.cursor = 0;
                None
            }
            Node::Song(s) => {
                let level = self.level_for(s)?;
                s.charts[level].as_ref().map(|c| c.path.clone())
            }
        }
    }

    /// BOX から出る。一番上なら false
    pub fn back(&mut self) -> bool {
        match self.path.pop() {
            Some(i) => {
                self.cursor = i;
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
                genre: String::new(),
                dir: PathBuf::new(),
                nodes: vec![song("a", &[0, 3]), song("b", &[1])],
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
        assert_eq!(l.nodes().len(), 2);
        // 難易度 2 が無いので上の 3
        assert_eq!(l.enter(), Some(PathBuf::from("a3.dtx")));
        l.move_by(1);
        // 2 から上に無いので下の 1
        assert_eq!(l.enter(), Some(PathBuf::from("b1.dtx")));
        assert!(l.back());
        assert_eq!(l.cursor, 0);
        assert!(!l.back());
    }

    #[test]
    fn cycle_level_wraps_to_the_easiest() {
        let mut l = list();
        l.enter();
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
        assert_eq!(l.cursor, 1);
        l.move_by(1);
        assert_eq!(l.cursor, 0);
    }

    #[test]
    fn level_follows_the_song() {
        let mut l = list();
        l.enter();
        // a は 0 と 3。いまは 3 扱いなので、下げると 0
        l.change_level(-1);
        assert_eq!(l.level, 0);
        l.change_level(-1);
        assert_eq!(l.level, 0);
        l.change_level(1);
        assert_eq!(l.level, 3);
    }
}
