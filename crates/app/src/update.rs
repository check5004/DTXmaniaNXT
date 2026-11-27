//! 更新（Velopack）。Setup.exe で入れたときだけ、GitHub のリリースから新しい版を探す。
//!
//! - 設定の「更新」で 3 つから選ぶ（[`UpdateMode`]）。既定の「知らせる」は、起動したときに新しい版があるかを
//!   確かめるだけで、落として入れ替えるのは自分でタイトルの「更新」の行を選んだとき（ユーザーの指示、2026-11-22）
//! - 確かめるのも落とすのも別のスレッドで行う（画面と判定を止めない）。落としている間は進み具合（%）を受け取る
//! - 落としたものは、自分で「再起動して入れ替える」を選ぶか、アプリを終えたときに入れ替える
//! - Setup.exe で入れていない（`cargo run`、macOS と Linux の tar.gz）なら何もしない
//! - 通信できない、GitHub が答えないときは記録に残すだけで、そのまま遊べる
//! - 更新の材料（`releases.win.json` と `.nupkg`）は `.github/release/velopack.sh` がリリースに載せる

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use i18n::t;
use velopack::sources::GithubSource;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, VelopackApp};

/// 新しい版を探すリポジトリ（Cargo.toml の `repository`）
const REPO: &str = env!("CARGO_PKG_REPOSITORY");

/// 新しい版の扱い（設定の「更新」。config.toml の `update`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateMode {
    /// 起動したときに確かめてタイトルに知らせる。落として入れ替えるのは自分で選んだとき
    #[default]
    Notify,
    /// 起動したときに落とし、終えるときに入れ替える（0.5.0 までの自動更新）
    Auto,
    /// 確かめない
    Off,
}

impl UpdateMode {
    pub const ALL: [UpdateMode; 3] = [UpdateMode::Notify, UpdateMode::Auto, UpdateMode::Off];

    pub fn name(self) -> &'static str {
        match self {
            UpdateMode::Notify => "notify",
            UpdateMode::Auto => "auto",
            UpdateMode::Off => "off",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// 設定の画面の表示
    pub fn label(self) -> &'static str {
        match self {
            UpdateMode::Notify => t("Notify", "お知らせ"),
            UpdateMode::Auto => t("Automatic", "自動"),
            UpdateMode::Off => t("Off", "確認しない"),
        }
    }
}

/// 今の版（例: `0.5.1`）
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

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
            log::info!("no update (not installed with Setup.exe): {e}");
            None
        }
    }
}

/// 画面を確かめるための見せかけの新しい版（開発用。`DTXMANIANXT_FAKE_UPDATE=0.9.0`。
/// 落とすのは数秒で終わる見せかけで、何も入れ替えない。版に `fail` を含めると途中で落とせなくなる）
fn fake_version() -> Option<String> {
    std::env::var("DTXMANIANXT_FAKE_UPDATE")
        .ok()
        .filter(|v| !v.is_empty())
}

/// 見せかけに落とす（5 % ずつ）
fn fake_download(version: &str, progress: &mpsc::Sender<i16>) -> bool {
    for p in (0..=100).step_by(5) {
        if p == 50 && version.contains("fail") {
            return false;
        }
        let _ = progress.send(p);
        thread::sleep(std::time::Duration::from_millis(300));
    }
    true
}

/// 確かめた結果
enum Found {
    /// 新しい版がある（まだ落としていない）
    New(Box<UpdateInfo>),
    /// 新しい版を前に落としてあり、まだ入れ替えていない
    Downloaded(String),
}

/// 新しい版を確かめる（別のスレッドで動く）
fn check() -> Option<Found> {
    if let Some(v) = fake_version() {
        let mut info = UpdateInfo::default();
        info.TargetFullRelease.Version = v;
        return Some(Found::New(Box::new(info)));
    }
    let m = manager()?;
    let pending = m.get_update_pending_restart().map(|a| a.Version);
    match m.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(info)) => {
            let version = info.TargetFullRelease.Version.clone();
            log::info!("an update to {version} is available");
            if pending.as_deref() == Some(version.as_str()) {
                Some(Found::Downloaded(version))
            } else {
                Some(Found::New(info))
            }
        }
        Ok(_) => {
            log::info!("{} is the latest", m.get_current_version_as_string());
            pending.map(Found::Downloaded)
        }
        Err(e) => {
            log::warn!("cannot check for updates: {e}");
            pending.map(Found::Downloaded)
        }
    }
}

/// 名前を付けたスレッドで動かす。動かせなければ記録に残して false
fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> bool {
    match thread::Builder::new().name(name.to_owned()).spawn(f) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("cannot start the {name} thread: {e}");
            false
        }
    }
}

/// 画面に出す更新のようす
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// 出すものが無い（確かめない、確かめている途中、新しい版が無い、Setup.exe で入れていない）
    Idle,
    /// 新しい版がある（まだ落としていない）
    Available(String),
    /// 落としている（0〜100 %）
    Downloading { version: String, percent: u8 },
    /// 落とし終えた。再起動するか、終えたときに入れ替える
    Ready(String),
    /// 落とせなかった（もう一度落とせる）
    Failed(String),
}

impl Status {
    /// 新しい版の番号（出すものが無ければ None）
    pub fn version(&self) -> Option<&str> {
        match self {
            Status::Idle => None,
            Status::Available(v)
            | Status::Downloading { version: v, .. }
            | Status::Ready(v)
            | Status::Failed(v) => Some(v),
        }
    }
}

/// 落とし終えた、落とせなかったなど、画面に一度だけ知らせること
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Ready(String),
    Failed(String),
}

/// 別のスレッドでしていること
enum Job {
    Check(Receiver<Found>),
    Download {
        progress: Receiver<i16>,
        done: Receiver<bool>,
    },
}

/// 更新の係。起動したときに確かめ、頼まれたら落とし、終わるときか再起動で入れ替える
pub struct Updater {
    mode: UpdateMode,
    status: Status,
    /// 確かめて見つけた新しい版（落とすのに使う）
    info: Option<Box<UpdateInfo>>,
    job: Option<Job>,
    /// この起動で確かめ始めた
    checked: bool,
    /// 「再起動して入れ替える」を選んだ
    restart: bool,
}

impl Updater {
    /// `mode` が「確かめない」でなければ、新しい版を確かめ始める
    pub fn start(mode: UpdateMode) -> Self {
        let mut u = Self {
            mode,
            status: Status::Idle,
            info: None,
            job: None,
            checked: false,
            restart: false,
        };
        if mode == UpdateMode::Off {
            log::info!("update check is OFF");
        } else {
            u.check();
        }
        u
    }

    fn check(&mut self) {
        self.checked = true;
        let (tx, rx) = mpsc::channel();
        if spawn("update-check", move || {
            if let Some(f) = check() {
                let _ = tx.send(f);
            }
        }) {
            self.job = Some(Job::Check(rx));
        }
    }

    /// 設定の「更新」を変えた。確かめていなければ確かめ、「自動」なら見つけてある版を落とし始める。
    /// 「確かめない」にしたら、まだ落としていない新しい版の知らせを消す
    pub fn set_mode(&mut self, mode: UpdateMode) {
        self.mode = mode;
        match mode {
            UpdateMode::Off => {
                if let Status::Available(_) | Status::Failed(_) = self.status {
                    self.status = Status::Idle;
                }
                if let Some(Job::Check(_)) = self.job {
                    self.job = None;
                }
            }
            UpdateMode::Notify | UpdateMode::Auto => {
                if !self.checked {
                    self.check();
                } else if mode == UpdateMode::Auto {
                    self.download();
                }
            }
        }
    }

    /// 別のスレッドからの知らせを受け取る。毎フレーム呼ぶ
    pub fn poll(&mut self) -> Option<Event> {
        match self.job.as_ref()? {
            Job::Check(rx) => match rx.try_recv() {
                Ok(found) => {
                    self.job = None;
                    match found {
                        Found::New(info) => {
                            self.status = Status::Available(info.TargetFullRelease.Version.clone());
                            self.info = Some(info);
                            if self.mode == UpdateMode::Auto {
                                self.download();
                            }
                        }
                        Found::Downloaded(v) => self.status = Status::Ready(v),
                    }
                    None
                }
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    None
                }
                Err(TryRecvError::Empty) => None,
            },
            Job::Download { progress, done } => {
                let mut percent = None;
                while let Ok(p) = progress.try_recv() {
                    percent = Some(p);
                }
                if let (Some(p), Status::Downloading { percent: now, .. }) =
                    (percent, &mut self.status)
                {
                    *now = (*now).max(p.clamp(0, 100) as u8);
                }
                let ok = match done.try_recv() {
                    Ok(ok) => ok,
                    Err(TryRecvError::Disconnected) => false,
                    Err(TryRecvError::Empty) => return None,
                };
                self.job = None;
                let version = self.status.version().unwrap_or_default().to_owned();
                if ok {
                    self.status = Status::Ready(version.clone());
                    Some(Event::Ready(version))
                } else {
                    self.status = Status::Failed(version.clone());
                    Some(Event::Failed(version))
                }
            }
        }
    }

    /// 今のようす
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// 見つけてある新しい版を落とし始める（落とせなかったあとなら、もう一度）。始めたら true
    pub fn download(&mut self) -> bool {
        if !matches!(self.status, Status::Available(_) | Status::Failed(_)) {
            return false;
        }
        let Some(info) = self.info.clone() else {
            return false;
        };
        let version = info.TargetFullRelease.Version.clone();
        let (ptx, progress) = mpsc::channel();
        let (dtx, done) = mpsc::channel();
        let started = spawn("update-download", move || {
            if fake_version().is_some() {
                let _ = dtx.send(fake_download(&version, &ptx));
                return;
            }
            let ok = manager().is_some_and(|m| {
                log::info!("downloading the update to {version}");
                match m.download_updates(&info, Some(ptx)) {
                    Ok(()) => {
                        log::info!("downloaded the update to {version}");
                        true
                    }
                    Err(e) => {
                        log::warn!("cannot download the update to {version}: {e}");
                        false
                    }
                }
            });
            let _ = dtx.send(ok);
        });
        if !started {
            return false;
        }
        let version = self.status.version().unwrap_or_default().to_owned();
        self.status = Status::Downloading {
            version,
            percent: 0,
        };
        self.job = Some(Job::Download { progress, done });
        true
    }

    /// アプリを終えたときに入れ替える版（落とし終えていて、「確かめない」でないとき）
    pub fn pending_on_exit(&self) -> Option<&str> {
        match &self.status {
            Status::Ready(v) if self.mode != UpdateMode::Off || self.restart => Some(v),
            _ => None,
        }
    }

    /// 「再起動して入れ替える」を選んだ。落とし終えていれば、その版を返す（呼んだ側がアプリを終える）
    pub fn request_restart(&mut self) -> Option<String> {
        let Status::Ready(v) = &self.status else {
            return None;
        };
        self.restart = true;
        Some(v.clone())
    }

    /// アプリを終える直前に呼ぶ。落としてある新しい版があれば、このアプリが終わったあとに入れ替える
    /// （「再起動して入れ替える」を選んでいれば、入れ替えたあと起動し直す）。前に落としたまま入れ替えられなかった版
    /// （落ちた、強制終了した）もここで入れる。「確かめない」なら、再起動を選んだときのほかは何もしない
    pub fn apply_on_exit(&mut self) {
        if self.mode == UpdateMode::Off && !self.restart {
            return;
        }
        let Some(m) = manager() else {
            return;
        };
        let Some(asset) = m.get_update_pending_restart() else {
            return;
        };
        let restart_args: [&str; 0] = [];
        // 自分で選んだ再起動なら、入れ替えられなかったときに Velopack の知らせを出す
        let silent = !self.restart;
        match m.wait_exit_then_apply_updates(&asset, silent, self.restart, restart_args) {
            Ok(()) => log::info!(
                "{} will be installed after exit (restart: {})",
                asset.Version,
                self.restart
            ),
            Err(e) => log::warn!("cannot install the update to {}: {e}", asset.Version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn updater(status: Status) -> Updater {
        Updater {
            mode: UpdateMode::Notify,
            status,
            info: None,
            job: None,
            checked: true,
            restart: false,
        }
    }

    #[test]
    fn updates_come_from_this_repository() {
        assert_eq!(REPO, "https://github.com/check5004/DTXmaniaNXT");
    }

    #[test]
    fn mode_names_round_trip() {
        for m in UpdateMode::ALL {
            assert_eq!(UpdateMode::from_name(m.name()), Some(m));
        }
        assert_eq!(UpdateMode::from_name("?"), None);
        assert_eq!(UpdateMode::default(), UpdateMode::Notify);
    }

    #[test]
    fn nothing_is_checked_when_off() {
        let mut u = Updater::start(UpdateMode::Off);
        assert!(u.job.is_none());
        assert!(!u.checked);
        assert_eq!(u.poll(), None);
        assert_eq!(u.status(), &Status::Idle);
    }

    #[test]
    fn a_found_update_waits_for_the_player() {
        let (tx, rx) = mpsc::channel();
        let mut u = updater(Status::Idle);
        u.job = Some(Job::Check(rx));
        assert_eq!(u.poll(), None);
        tx.send(Found::Downloaded("0.6.0".to_owned())).unwrap();
        assert_eq!(u.poll(), None);
        assert_eq!(u.status(), &Status::Ready("0.6.0".to_owned()));
        assert!(u.job.is_none());
    }

    #[test]
    fn download_progress_and_finish_are_reported() {
        let (ptx, progress) = mpsc::channel();
        let (dtx, done) = mpsc::channel();
        let mut u = updater(Status::Downloading {
            version: "0.6.0".to_owned(),
            percent: 0,
        });
        u.job = Some(Job::Download { progress, done });
        ptx.send(5).unwrap();
        ptx.send(40).unwrap();
        assert_eq!(u.poll(), None);
        assert_eq!(
            u.status(),
            &Status::Downloading {
                version: "0.6.0".to_owned(),
                percent: 40
            }
        );
        // 差分から全体に切り替えて 0 に戻っても、出す % は戻さない
        ptx.send(0).unwrap();
        assert_eq!(u.poll(), None);
        assert!(matches!(
            u.status(),
            Status::Downloading { percent: 40, .. }
        ));
        dtx.send(true).unwrap();
        assert_eq!(u.poll(), Some(Event::Ready("0.6.0".to_owned())));
        assert_eq!(u.status(), &Status::Ready("0.6.0".to_owned()));
        assert_eq!(u.request_restart(), Some("0.6.0".to_owned()));
        assert!(u.restart);
    }

    #[test]
    fn a_failed_download_can_be_retried() {
        let (_ptx, progress) = mpsc::channel();
        let (dtx, done) = mpsc::channel();
        let mut u = updater(Status::Downloading {
            version: "0.6.0".to_owned(),
            percent: 10,
        });
        u.job = Some(Job::Download { progress, done });
        drop(dtx);
        assert_eq!(u.poll(), Some(Event::Failed("0.6.0".to_owned())));
        assert_eq!(u.status(), &Status::Failed("0.6.0".to_owned()));
        // 再起動は落とし終えたときだけ
        assert_eq!(u.request_restart(), None);
    }

    #[test]
    fn turning_off_hides_an_update_not_yet_downloaded() {
        let mut u = updater(Status::Available("0.6.0".to_owned()));
        u.set_mode(UpdateMode::Off);
        assert_eq!(u.status(), &Status::Idle);
        let mut u = updater(Status::Ready("0.6.0".to_owned()));
        u.set_mode(UpdateMode::Off);
        assert_eq!(u.status(), &Status::Ready("0.6.0".to_owned()));
    }
}
