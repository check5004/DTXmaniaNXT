//! 譜面エディタ（DTX Web Editor、https://github.com/check5004/dtx-mania-web-editor）を既定のブラウザで開く。
//! エディタはブラウザで動く Web アプリで、原曲を聴きながら DTX の譜面を作れる。Google Drive で、何台かの PC の
//! 曲のフォルダを同期することもできる。設定の画面の「曲」から開く（2026-11-22 ユーザー依頼）。

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// エディタの公開先（GitHub Pages）
pub const URL: &str = "https://check5004.github.io/dtx-mania-web-editor/";

/// 続けて開かない間（パッドの 2 度打ちや、1 回の打撃でノートが 2 つ届く機器でタブが 2 つ開かないように）
const AGAIN_AFTER: Duration = Duration::from_secs(3);

static LAST_OPENED: Mutex<Option<Instant>> = Mutex::new(None);

/// 開いた結果
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    Opened,
    /// さっき開いたばかりなので開かなかった
    Recently,
}

/// 既定のブラウザでエディタを開く
pub fn open() -> std::io::Result<Opened> {
    open_url(URL)
}

/// 既定のブラウザでページを開く（クレジットの一覧も、ここを通して続けて開かないようにする）
pub fn open_url(url: &str) -> std::io::Result<Opened> {
    let now = Instant::now();
    let mut last = LAST_OPENED.lock().unwrap_or_else(|e| e.into_inner());
    if !should_open(*last, now) {
        return Ok(Opened::Recently);
    }
    webbrowser::open(url)?;
    *last = Some(now);
    Ok(Opened::Opened)
}

fn should_open(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= AGAIN_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_open_twice_in_a_row() {
        let t = Instant::now();
        assert!(should_open(None, t));
        assert!(!should_open(Some(t), t + Duration::from_millis(200)));
        assert!(should_open(Some(t), t + AGAIN_AFTER));
    }
}
