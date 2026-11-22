//! ログの行き先と、エラーで終わるときの知らせ方。
//!
//! Windows のリリース版はコンソールの窓を出さない（main.rs の `windows_subsystem`）ので、ログと panic は
//! 設定のフォルダ（Windows は %APPDATA%\DTXmaniaNXT）の `dtxmanianxt.log` にも書く。起動するたびに作り直し、
//! 前回のぶんは `dtxmanianxt.prev.log` に 1 つだけ残す（落ちたあと起動し直しても、落ちたときのログが消えない）。
//! コンソールがあるとき（開発中や macOS・Linux の端末から）は、今までどおりそこにも出す。
//! 起動できなかったときや落ちたときは、コンソールが無いと何も見えないので、ログの場所をダイアログで知らせる。

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::config::Config;

/// ログのファイルの名前
pub const LOG_FILE: &str = "dtxmanianxt.log";
/// 前回のログのファイルの名前
pub const PREV_LOG_FILE: &str = "dtxmanianxt.prev.log";

/// 主のスレッドで起きた panic の中身（落ちたことを知らせるダイアログに出す）
static MAIN_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// ログを始め、panic もログに書くようにする。ログのファイルの場所を返す（書けないときは None）
pub fn init() -> Option<PathBuf> {
    let path = Config::dir().map(|d| d.join(LOG_FILE));
    let opened = path.as_deref().map(open);
    let (file, error) = match opened {
        Some(Ok(f)) => (Some(f), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let written = file.is_some();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Pipe(Box::new(Tee { file })))
        .init();
    install_panic_hook();
    log::info!(
        "DTXmaniaNXT {} ({} {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    match (&path, error) {
        (Some(p), Some(e)) => log::warn!("cannot write the log file {}: {e}", p.display()),
        (None, _) => log::warn!("no settings folder; the log is not saved to a file"),
        _ => {}
    }
    path.filter(|_| written)
}

/// ログのファイルを作る。前からあるものは前回のぶんとして残す
fn open(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if path.exists() {
        // Windows でも、前々回のぶんがあれば置き換わる。移せなければ（ほかのアプリが開いているなど）上書きする
        let _ = fs::rename(path, path.with_file_name(PREV_LOG_FILE));
    }
    File::create(path)
}

/// ログをコンソールとファイルの両方に書く
struct Tee {
    file: Option<File>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // コンソールが無いとき（Windows のリリース版）は書けないが、std が黙って捨てる
        let _ = io::stderr().write_all(buf);
        // ログが書けなくても遊ぶのは止めない。1 行ずつ書いて溜めない（落ちても直前まで残る）
        if let Some(f) = &mut self.file {
            let _ = f.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(f) = &mut self.file {
            let _ = f.flush();
        }
        Ok(())
    }
}

/// panic をログに書く（std の既定はコンソールに出すだけ）。主のスレッドのものは、知らせるために取っておく
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!("thread '{name}' {info}\n{backtrace}");
        if name == "main"
            && let Ok(mut p) = MAIN_PANIC.lock()
        {
            *p = Some(info.to_string());
        }
    }));
}

/// 主のスレッドで起きた panic の中身（ログに書いたもの）
pub fn main_panic() -> Option<String> {
    MAIN_PANIC.lock().ok().and_then(|mut p| p.take())
}

/// エラーで終わることを知らせる。コンソールの無い Windows のリリース版だけダイアログを出す
/// （ほかはコンソールにログが出ている）。ウィンドウの処理の中から呼ぶと描画が割り込むので、ループを抜けてから呼ぶ
pub fn tell_user(error: &str, log: Option<&Path>) {
    if !cfg!(all(windows, not(debug_assertions))) {
        return;
    }
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("DTXmaniaNXT")
        .set_description(message(error, log))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// ダイアログの文
fn message(error: &str, log: Option<&Path>) -> String {
    let mut text = format!("エラーで終了しました。\n\n{error}");
    if let Some(p) = log {
        text.push_str(&format!(
            "\n\nくわしくはログをご覧ください。不具合を知らせるときは、このファイルを添えてください。\n{}",
            p.display()
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "dtxmanianxt-log-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn open_keeps_the_previous_log() {
        let dir = temp_dir("rotate");
        let path = dir.join("sub").join(LOG_FILE);
        let prev = dir.join("sub").join(PREV_LOG_FILE);

        // 初めて：フォルダごと作る
        let mut f = open(&path).unwrap();
        f.write_all(b"first\n").unwrap();
        drop(f);
        assert!(!prev.exists());

        // 2 回目：前回のぶんが prev に移り、新しいファイルは空
        let mut f = open(&path).unwrap();
        assert_eq!(fs::read_to_string(&prev).unwrap(), "first\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        f.write_all(b"second\n").unwrap();
        drop(f);

        // 3 回目：前々回のぶんは置き換わる
        open(&path).unwrap();
        assert_eq!(fs::read_to_string(&prev).unwrap(), "second\n");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tee_writes_the_file() {
        let dir = temp_dir("tee");
        let path = dir.join(LOG_FILE);
        let mut tee = Tee {
            file: Some(File::create(&path).unwrap()),
        };
        tee.write_all(b"[INFO] hello\n").unwrap();
        tee.flush().unwrap();
        drop(tee);
        assert_eq!(fs::read_to_string(&path).unwrap(), "[INFO] hello\n");

        // ファイルが無くても失敗しない
        let mut tee = Tee { file: None };
        assert_eq!(tee.write(b"x").unwrap(), 1);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn message_names_the_log() {
        let log = Path::new("C:/Users/a/AppData/Roaming/DTXmaniaNXT/dtxmanianxt.log");
        let text = message("cannot start the GPU", Some(log));
        assert!(text.contains("cannot start the GPU"));
        assert!(text.contains(&log.display().to_string()));
        assert!(!message("x", None).contains("ログ"));
    }
}
