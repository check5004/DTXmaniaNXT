//! ウィンドウにドロップされた zip や曲のフォルダを、別のスレッドで曲のフォルダへ写す（写し方は [`library::import`]）。
//!
//! 頼まれた順に 1 つずつ写す。写しているあいだは進み具合を、終わったら結果を [`Adder::poll`] で受け取る。
//! 曲の多いフォルダ（曲の集まりそのもの）は写さず、曲のフォルダとして登録するよう返す。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use i18n::{count, t, tf};
use library::import::{self, Added, ImportError, Source};

/// これより曲の多いフォルダは写さずに、曲のフォルダとして登録する（曲のフォルダがまだ無いときも）
pub const COPY_SONG_LIMIT: usize = 50;

/// 頼みごと 1 つ
struct Request {
    path: PathBuf,
    /// 写す先（曲のフォルダ）
    dest: PathBuf,
    /// 今の曲のフォルダ
    song_dirs: Vec<PathBuf>,
}

/// 1 つ終わった結果
#[derive(Debug)]
pub enum Outcome {
    /// 写した（中身が同じフォルダがあったなら、写さずにそれ）
    Added {
        dest: PathBuf,
        added: Added,
        /// だいたいの曲の数（譜面のあるフォルダの数）
        songs: usize,
    },
    /// すでに曲のフォルダの中にあるフォルダ。写さずに探し直す
    InLibrary {
        dir: PathBuf,
    },
    /// 曲の多いフォルダ。写さずに曲のフォルダとして登録する
    Register {
        dir: PathBuf,
    },
    Failed {
        from: PathBuf,
        error: ImportError,
    },
}

impl Outcome {
    /// 終わったあとに選曲で移る譜面（あれば）
    pub fn first_chart(&self) -> Option<&Path> {
        match self {
            Outcome::Added { added, .. } => added.charts.first().map(PathBuf::as_path),
            _ => None,
        }
    }

    /// 知らせる文
    pub fn message(&self) -> String {
        match self {
            Outcome::Added { added, .. } if added.already => t(
                "This song has already been added",
                "この曲はもう追加してあります",
            )
            .to_owned(),
            Outcome::Added { dest, songs, .. } => {
                let name = folder_name(dest);
                let n = count(*songs, "song", "songs", "曲");
                tf!("Added {n} to {name}", "「{name}」に {n}を追加しました")
            }
            Outcome::InLibrary { .. } => t(
                "This folder is already in your song folder. Rescanned songs.",
                "このフォルダはもう曲のフォルダに入っています。曲を探し直しました",
            )
            .to_owned(),
            Outcome::Register { dir } => {
                let name = folder_name(dir);
                tf!(
                    "Added \"{name}\" to your song folders",
                    "「{name}」を曲のフォルダに足しました"
                )
            }
            Outcome::Failed { from, error } => error_message(from, error),
        }
    }
}

/// 続けてドロップされた分の結果を 1 つの知らせにする。1 つなら、その知らせ
pub fn summary(outcomes: &[Outcome]) -> Option<String> {
    let [first, ..] = outcomes else {
        return None;
    };
    if outcomes.len() == 1 {
        return Some(first.message());
    }
    let songs: usize = outcomes
        .iter()
        .map(|o| match o {
            Outcome::Added { added, songs, .. } if !added.already => *songs,
            _ => 0,
        })
        .sum();
    let failed = outcomes
        .iter()
        .filter(|o| matches!(o, Outcome::Failed { .. }))
        .count();
    let dest = outcomes.iter().find_map(|o| match o {
        Outcome::Added { dest, added, .. } if !added.already => Some(dest),
        _ => None,
    });
    let mut text = match dest {
        Some(dest) => {
            let name = folder_name(dest);
            let n = count(songs, "song", "songs", "曲");
            tf!("Added {n} to {name}", "「{name}」に {n}を追加しました")
        }
        // 写したものが無ければ、ほかの結果（登録した、もう入っていた）か、できなかったわけ
        None => outcomes
            .iter()
            .find(|o| !matches!(o, Outcome::Failed { .. }))
            .unwrap_or(first)
            .message(),
    };
    if failed > 0 && failed < outcomes.len() {
        text += &tf!(
            " ({failed} couldn't be added)",
            "（{failed} 件は追加できませんでした）"
        );
    }
    Some(text)
}

/// フォルダの名前（無ければパス）
pub fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

/// 追加できなかったわけの文
pub fn error_message(from: &Path, error: &ImportError) -> String {
    let name = folder_name(from);
    match error {
        ImportError::Unsupported(ext) if ext.is_empty() => tf!(
            "Can't add \"{name}\". Drop a zip file or a song folder.",
            "「{name}」は追加できません。zip ファイルか曲のフォルダをドロップしてください"
        ),
        ImportError::Unsupported(ext) => {
            let ext = match ext.as_str() {
                "7z" => "7z".to_owned(),
                "lha" => "LZH".to_owned(),
                "tgz" | "gz" | "bz2" | "xz" => "tar".to_owned(),
                e => e.to_ascii_uppercase(),
            };
            if matches!(ext.as_str(), "RAR" | "7z" | "LZH" | "tar" | "TAR") {
                tf!(
                    "{ext} files can't be read. Extract it, then drop the folder.",
                    "{ext} は読めません。展開してからフォルダをドロップしてください"
                )
            } else {
                tf!(
                    "Can't add \"{name}\". Drop a zip file or a song folder.",
                    "「{name}」は追加できません。zip ファイルか曲のフォルダをドロップしてください"
                )
            }
        }
        ImportError::NoCharts => tf!(
            "No DTX charts in \"{name}\"",
            "「{name}」には DTX の譜面がありません"
        ),
        ImportError::Encrypted => t(
            "Password-protected zip files can't be added",
            "パスワード付きの zip は追加できません",
        )
        .to_owned(),
        ImportError::Method(_) => tf!(
            "\"{name}\" uses a compression method that can't be read. Extract it, then drop the folder.",
            "「{name}」は読めない圧縮のしかたです。展開してからフォルダをドロップしてください"
        ),
        ImportError::Broken(_) => tf!(
            "\"{name}\" seems to be broken. Try downloading it again.",
            "「{name}」は壊れているようです。もう一度ダウンロードしてみてください"
        ),
        ImportError::TooLarge => tf!(
            "\"{name}\" is too large to add",
            "「{name}」は大きすぎて追加できません"
        ),
        ImportError::ContainsSongFolder => tf!(
            "\"{name}\" contains a song folder, so it can't be copied",
            "「{name}」には曲のフォルダが入っているので、写せません"
        ),
        ImportError::Io(e) => tf!(
            "Couldn't add \"{name}\": {e}",
            "「{name}」を追加できませんでした: {e}"
        ),
        ImportError::Stopped => tf!("Stopped adding \"{name}\"", "「{name}」の追加をやめました"),
    }
}

enum Msg {
    Start { name: String },
    Progress { done: u64, total: u64 },
    Done(Outcome),
}

/// 写している途中のようす
#[derive(Debug, Clone)]
pub struct Working {
    /// ドロップされたものの名前
    pub name: String,
    pub done: u64,
    pub total: u64,
    /// 始めた時刻（くるくるの向き）
    pub since: Instant,
    /// このあとに待っている数
    pub waiting: usize,
}

impl Working {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32
        }
    }
}

/// 写す係（スレッドは初めて頼まれたときに作る）
#[derive(Default)]
pub struct Adder {
    tx: Option<Sender<Request>>,
    rx: Option<Receiver<Msg>>,
    stop: Arc<AtomicBool>,
    /// 頼んでまだ終わっていない数
    pending: usize,
    working: Option<Working>,
}

impl Drop for Adder {
    fn drop(&mut self) {
        // 閉じるときは途中でやめる（作りかけのフォルダは消える）
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Adder {
    /// `path` を `dest` に写すよう頼む
    pub fn add(&mut self, path: PathBuf, dest: PathBuf, song_dirs: Vec<PathBuf>) {
        if self.tx.is_none() {
            let (tx, rx) = channel::<Request>();
            let (mtx, mrx) = channel();
            let stop = self.stop.clone();
            std::thread::Builder::new()
                .name("song adder".into())
                .spawn(move || {
                    for req in rx {
                        let _ = mtx.send(Msg::Start {
                            name: folder_name(&req.path),
                        });
                        let mut last = 0;
                        let outcome = handle(&req, &stop, &mut |done, total| {
                            // 1 % ごとに知らせる（細かすぎると列が詰まる）
                            let step = (total / 100).max(1);
                            if done == 0 || done == total || done / step != last {
                                last = done / step;
                                let _ = mtx.send(Msg::Progress { done, total });
                            }
                        });
                        if mtx.send(Msg::Done(outcome)).is_err() {
                            return;
                        }
                    }
                })
                .expect("cannot start the song adder thread");
            self.tx = Some(tx);
            self.rx = Some(mrx);
        }
        if let Some(tx) = &self.tx
            && tx
                .send(Request {
                    path,
                    dest,
                    song_dirs,
                })
                .is_ok()
        {
            self.pending += 1;
        }
    }

    /// 頼んだものがまだ残っているか
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// 写している途中なら、そのようす
    pub fn working(&self) -> Option<&Working> {
        self.working.as_ref()
    }

    /// 届いた知らせを読み、終わったものを返す
    pub fn poll(&mut self) -> Vec<Outcome> {
        let Some(rx) = &self.rx else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for msg in rx.try_iter() {
            match msg {
                Msg::Start { name } => {
                    self.working = Some(Working {
                        name,
                        done: 0,
                        total: 0,
                        since: Instant::now(),
                        waiting: self.pending.saturating_sub(1),
                    });
                }
                Msg::Progress { done, total } => {
                    if let Some(w) = &mut self.working {
                        w.done = done;
                        w.total = total;
                    }
                }
                Msg::Done(o) => {
                    self.pending = self.pending.saturating_sub(1);
                    self.working = None;
                    out.push(o);
                }
            }
        }
        if let Some(w) = &mut self.working {
            w.waiting = self.pending.saturating_sub(1);
        }
        out
    }
}

/// 1 つ写す
fn handle(req: &Request, stop: &AtomicBool, progress: &mut dyn FnMut(u64, u64)) -> Outcome {
    let failed = |error| Outcome::Failed {
        from: req.path.clone(),
        error,
    };
    let source = match import::source_of(&req.path) {
        Ok(s) => s,
        Err(e) => return failed(e),
    };
    if source == Source::Folder {
        // すでに曲のフォルダの中にあるなら写さない（探し直せば一覧に出る）
        if req
            .song_dirs
            .iter()
            .any(|d| import::is_within(&req.path, d))
        {
            return Outcome::InLibrary {
                dir: req.path.clone(),
            };
        }
        // 曲のフォルダを含むフォルダは、写しても登録しても同じ曲が 2 つ並ぶ
        if req
            .song_dirs
            .iter()
            .chain([&req.dest])
            .any(|d| import::is_within(d, &req.path))
        {
            return failed(ImportError::ContainsSongFolder);
        }
        // 曲のフォルダがまだ無ければ（初めて使うとき）、ドロップしたフォルダをそのまま曲のフォルダにする。
        // 曲の多いフォルダ（曲の集まりそのもの）も写さない
        if req.song_dirs.is_empty()
            || import::count_song_dirs(&req.path, COPY_SONG_LIMIT) > COPY_SONG_LIMIT
        {
            return Outcome::Register {
                dir: req.path.clone(),
            };
        }
    }
    let plan = match import::plan(&req.path) {
        Ok(p) => p,
        Err(e) => return failed(e),
    };
    log::info!(
        "adding songs: {} → {} ({} charts, {} files, {} bytes)",
        req.path.display(),
        req.dest.display(),
        plan.charts,
        plan.files(),
        plan.bytes
    );
    match import::import(&plan, &req.dest, stop, progress) {
        Ok(added) => {
            log::info!(
                "added songs: {} ({})",
                added.dir.display(),
                if added.already {
                    "already there"
                } else {
                    "copied"
                }
            );
            Outcome::Added {
                dest: req.dest.clone(),
                added,
                songs: plan.song_dirs,
            }
        }
        Err(e) => {
            log::warn!("could not add {}: {e}", req.path.display());
            failed(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("nxt-adder-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn wait(adder: &mut Adder) -> Vec<Outcome> {
        let start = Instant::now();
        let mut out = Vec::new();
        while adder.busy() && start.elapsed() < Duration::from_secs(10) {
            out.extend(adder.poll());
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }

    #[test]
    fn a_song_folder_is_copied_and_the_first_chart_is_returned() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        let root = temp("copy");
        let src = root.join("Song A");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("mstr.dtx"), b"#TITLE a").unwrap();
        let dest = root.join("DTXFiles");
        let mut adder = Adder::default();
        adder.add(src.clone(), dest.clone(), vec![dest.clone()]);
        let out = wait(&mut adder);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].first_chart(),
            Some(dest.join("Song A").join("mstr.dtx").as_path())
        );
        assert_eq!(out[0].message(), "「DTXFiles」に 1 曲を追加しました");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn folders_inside_or_around_song_folders_are_not_copied() {
        let root = temp("around");
        let lib = root.join("DTXFiles");
        let inner = lib.join("Song");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("a.dtx"), b"#TITLE a").unwrap();
        let mut adder = Adder::default();
        adder.add(inner.clone(), lib.clone(), vec![lib.clone()]);
        adder.add(root.clone(), lib.clone(), vec![lib.clone()]);
        let out = wait(&mut adder);
        assert!(matches!(&out[0], Outcome::InLibrary { dir } if *dir == inner));
        assert!(matches!(
            &out[1],
            Outcome::Failed {
                error: ImportError::ContainsSongFolder,
                ..
            }
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_first_folder_becomes_a_song_folder() {
        let root = temp("first");
        let songs = root.join("Songs");
        fs::create_dir_all(songs.join("A")).unwrap();
        fs::write(songs.join("A/a.dtx"), b"#TITLE a").unwrap();
        let mut adder = Adder::default();
        adder.add(songs.clone(), root.join("Documents/Songs"), Vec::new());
        let out = wait(&mut adder);
        assert!(matches!(&out[0], Outcome::Register { dir } if *dir == songs));
        assert!(!root.join("Documents").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_big_collection_is_registered_instead_of_copied() {
        let root = temp("big");
        let pack = root.join("Collection");
        for i in 0..=COPY_SONG_LIMIT {
            let d = pack.join(format!("song {i}"));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("a.dtx"), b"#TITLE a").unwrap();
        }
        let dest = root.join("DTXFiles");
        let mut adder = Adder::default();
        adder.add(pack.clone(), dest.clone(), vec![dest.clone()]);
        let out = wait(&mut adder);
        assert!(matches!(&out[0], Outcome::Register { dir } if *dir == pack));
        assert!(!dest.join("Collection").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn several_drops_make_one_message() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        let added = |dir: &str, songs, already| Outcome::Added {
            dest: PathBuf::from("/songs/DTXFiles"),
            added: Added {
                dir: PathBuf::from(dir),
                charts: Vec::new(),
                already,
            },
            songs,
        };
        let failed = || Outcome::Failed {
            from: PathBuf::from("a.rar"),
            error: ImportError::Unsupported("rar".into()),
        };
        assert_eq!(summary(&[]), None);
        assert_eq!(
            summary(&[added("a", 1, false), added("b", 2, false), failed()]).unwrap(),
            "「DTXFiles」に 3 曲を追加しました（1 件は追加できませんでした）"
        );
        assert_eq!(
            summary(&[added("a", 1, true), failed()]).unwrap(),
            "この曲はもう追加してあります（1 件は追加できませんでした）"
        );
        assert_eq!(
            summary(&[failed(), failed()]).unwrap(),
            "RAR は読めません。展開してからフォルダをドロップしてください"
        );
    }

    #[test]
    fn unsupported_archives_say_to_extract_them() {
        let _ja = i18n::scoped(i18n::Lang::Ja);
        let msg = error_message(
            Path::new("song.rar"),
            &ImportError::Unsupported("rar".into()),
        );
        assert_eq!(
            msg,
            "RAR は読めません。展開してからフォルダをドロップしてください"
        );
    }
}
