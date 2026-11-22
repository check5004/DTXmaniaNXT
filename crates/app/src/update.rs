//! 自動更新（Velopack）。Setup.exe で入れたときだけ、GitHub のリリースから新しい版を探して落とし、
//! アプリを終えたときに入れ替える（次に起動すると新しい版）。
//!
//! - 探して落とすのは起動したときの 1 回だけで、別のスレッドで行う（画面と判定を止めない）
//! - 落とし終えたらタイトルの画面に知らせる
//! - Setup.exe で入れていない（`cargo run`、macOS と Linux の tar.gz）なら何もしない
//! - 通信できない、GitHub が答えないときは記録に残すだけで、そのまま遊べる
//! - 更新の材料（`releases.win.json` と `.nupkg`）は `.github/release/velopack.sh` がリリースに載せる

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use velopack::sources::GithubSource;
use velopack::{UpdateCheck, UpdateManager, VelopackApp};

/// 新しい版を探すリポジトリ（Cargo.toml の `repository`）
const REPO: &str = env!("CARGO_PKG_REPOSITORY");

/// インストール、アンインストール、更新の途中に Velopack から呼ばれたときは、その仕事をして終わる。
/// ふつうに起動したときは何もしない。main の初めに呼ぶ
pub fn run_hooks() {
    VelopackApp::build().run();
}

/// 更新の係。Setup.exe で入れていなければ作れない
fn manager() -> Option<UpdateManager> {
    match UpdateManager::new(GithubSource::new(REPO, None, false), None, None) {
        Ok(m) => Some(m),
        Err(e) => {
            log::info!("no auto update (not installed with Setup.exe): {e}");
            None
        }
    }
}

/// 新しい版を探して落とす（別のスレッドで動く）。落とし終えた版を返す
fn fetch() -> Option<String> {
    let m = manager()?;
    let info = match m.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(info)) => info,
        Ok(_) => {
            log::info!("{} is the latest", m.get_current_version_as_string());
            return None;
        }
        Err(e) => {
            log::warn!("cannot check for updates: {e}");
            return None;
        }
    };
    let version = info.TargetFullRelease.Version.clone();
    log::info!("downloading the update to {version}");
    match m.download_updates(&info, None) {
        Ok(()) => {
            log::info!("downloaded the update to {version}");
            Some(version)
        }
        Err(e) => {
            log::warn!("cannot download the update to {version}: {e}");
            None
        }
    }
}

/// 起動したときに新しい版を探して落とし、終わるときに入れ替える
pub struct Updater {
    /// 探しているスレッドからの知らせ（落とし終えた版）
    rx: Option<Receiver<String>>,
    /// 落とし終えた版
    ready: Option<String>,
}

impl Updater {
    /// `enabled` なら、新しい版を探して落とし始める
    pub fn start(enabled: bool) -> Self {
        let rx = if enabled {
            let (tx, rx) = mpsc::channel();
            let spawned = thread::Builder::new()
                .name("update".to_owned())
                .spawn(move || {
                    if let Some(v) = fetch() {
                        let _ = tx.send(v);
                    }
                });
            match spawned {
                Ok(_) => Some(rx),
                Err(e) => {
                    log::warn!("cannot start the update check: {e}");
                    None
                }
            }
        } else {
            log::info!("auto update is OFF");
            None
        };
        Self { rx, ready: None }
    }

    /// 落とし終えた新しい版（例: `0.3.0`）。まだなら None
    pub fn ready(&mut self) -> Option<&str> {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(v) => {
                    self.ready = Some(v);
                    self.rx = None;
                }
                Err(TryRecvError::Disconnected) => self.rx = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        self.ready.as_deref()
    }

    /// アプリを終える直前に呼ぶ。落としてある新しい版があれば、このアプリが終わったあとに入れ替える。
    /// 前に落としたまま入れ替えられなかった版（落ちた、強制終了した）もここで入れる。
    /// `enabled` が false（設定で OFF にした）なら何もしない
    pub fn apply_on_exit(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        let Some(m) = manager() else {
            return;
        };
        let Some(asset) = m.get_update_pending_restart() else {
            return;
        };
        let restart_args: [&str; 0] = [];
        match m.wait_exit_then_apply_updates(&asset, true, false, restart_args) {
            Ok(()) => log::info!("{} will be installed after exit", asset.Version),
            Err(e) => log::warn!("cannot install the update to {}: {e}", asset.Version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_come_from_this_repository() {
        assert_eq!(REPO, "https://github.com/check5004/DTXmaniaNXT");
    }

    #[test]
    fn nothing_is_checked_when_off() {
        let mut u = Updater::start(false);
        assert!(u.rx.is_none());
        assert_eq!(u.ready(), None);
    }

    #[test]
    fn a_finished_download_is_kept() {
        let (tx, rx) = mpsc::channel();
        let mut u = Updater {
            rx: Some(rx),
            ready: None,
        };
        assert_eq!(u.ready(), None);
        tx.send("0.3.0".to_owned()).unwrap();
        assert_eq!(u.ready(), Some("0.3.0"));
        drop(tx);
        assert_eq!(u.ready(), Some("0.3.0"));
    }
}
