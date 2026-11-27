//! 開発者メニューの画面（設定 ＞ アプリ の「バージョン」を 7 回続けて決定すると出る）。
//!
//! 左に調べること（環境の情報、MIDI を記録、ログ、[`dev_report::TOOLS`] の道具）を並べ、決定で始める。
//! 終わったら右に issue の下書き（伏せたあとの本文そのもの）を出し、下のボタンでブラウザで issue を作る・
//! コピーする・保存したファイルを見せる。ドラムでは HT・LT で選ぶか本文を送り、SD・FT でボタン、CY で決定、
//! LC で戻る。MIDI を記録しているあいだは叩いたものを全部記録するので、パッドでは操作しない（キーボードの
//! Enter で早めに終え、Esc で取りやめる）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::Color32;
use i18n::{t, tf};

use crate::dev_report::{
    self, Environment, Finding, MidiEntry, Names, REPORTS_DIR, Report, Send, TOOLS,
};
use crate::gfx::{Canvas, Gfx};
use crate::midi::RawMidi;
use crate::settings_ui::{self as ui, Hint, Pad};
use crate::sfx::Se;
use crate::text::Style;
use crate::time::host_us;

/// 左の一覧と右の板
const LIST: Rect = (40.0, 64.0, 380.0, 612.0);
const RIGHT: Rect = (444.0, 64.0, 796.0, 612.0);
/// 一覧の行の高さと間
const ROW_H: f32 = 62.0;
const ROW_GAP: f32 = 8.0;
/// 下書きの本文を出す所
const PREVIEW: Rect = (460.0, 160.0, 764.0, 398.0);
const PREVIEW_PX: f32 = 13.0;
const PREVIEW_LINE: f32 = 17.0;
/// 1 回で送る本文の行
const SCROLL: usize = 10;
/// ボタンの場所
const BUTTON_Y: f32 = 576.0;
const BUTTON_W: f32 = 244.0;
const BUTTON_H: f32 = 44.0;
const BUTTON_GAP: f32 = 16.0;
/// MIDI を記録する長さ
const MIDI_SECONDS: u64 = 10;
/// ログから出す行
const LOG_LINES: usize = 300;
/// 進み具合を覚えておく行
const PROGRESS_KEEP: usize = 200;
const TOAST_FOR: Duration = Duration::from_millis(3500);

type Rect = (f32, f32, f32, f32);

/// 動かした道具（取りやめの旗とスレッド）。画面を出たあとも、終わるときに後片付けを待てるように持っておく
static TOOL_THREADS: Mutex<Vec<(Arc<AtomicBool>, JoinHandle<()>)>> = Mutex::new(Vec::new());

/// まだ終わっていない道具がある
fn tools_running() -> bool {
    let mut threads = TOOL_THREADS.lock().unwrap_or_else(|e| e.into_inner());
    threads.retain(|(_, h)| !h.is_finished());
    !threads.is_empty()
}

/// アプリを終える前に呼ぶ。動いている道具に取りやめを頼み、後片付け（音源の設定を戻すなど）が終わるまで
/// `wait` だけ待つ
pub fn shutdown(wait: Duration) {
    let threads = std::mem::take(&mut *TOOL_THREADS.lock().unwrap_or_else(|e| e.into_inner()));
    if threads.iter().all(|(_, h)| h.is_finished()) {
        return;
    }
    for (cancel, _) in &threads {
        cancel.store(true, Ordering::SeqCst);
    }
    log::info!("waiting for the dev tools to finish");
    let until = Instant::now() + wait;
    while threads.iter().any(|(_, h)| !h.is_finished()) {
        if Instant::now() >= until {
            log::warn!("a dev tool did not finish in {} s", wait.as_secs());
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 画面への操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevInput {
    Up,
    Down,
    Left,
    Right,
    Decide,
    Cancel,
}

/// 操作の結果
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DevResponse {
    pub se: Option<Se>,
    /// 設定の画面へ戻る
    pub back: bool,
}

impl DevResponse {
    fn se(se: Se) -> Self {
        Self {
            se: Some(se),
            back: false,
        }
    }
}

/// 調べること
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Env,
    Midi,
    Log,
    PrevLog,
    Tool(usize),
}

impl Item {
    fn all() -> Vec<Item> {
        let mut v = vec![Item::Env, Item::Midi, Item::Log, Item::PrevLog];
        v.extend((0..TOOLS.len()).map(Item::Tool));
        v
    }

    fn key(self) -> &'static str {
        match self {
            Item::Env => "env",
            Item::Midi => "midi",
            Item::Log => "log",
            Item::PrevLog => "prev-log",
            Item::Tool(i) => TOOLS[i].key,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Item::Env => t("Environment Info", "環境の情報"),
            Item::Midi => t("Record MIDI (10 s)", "MIDI を記録（10 秒）"),
            Item::Log => t("Log", "ログ"),
            Item::PrevLog => t("Previous Log", "前回のログ"),
            Item::Tool(i) => TOOLS[i].name.get(),
        }
    }

    fn desc(self) -> &'static str {
        match self {
            Item::Env => t(
                "Gathers the version, OS, sound output, and MIDI devices. Every report ends with this too.",
                "バージョン・OS・音の出力・MIDI 機器をまとめます。どの報告にも最後に付きます。",
            ),
            Item::Midi => t(
                "Records every MIDI message received for 10 seconds: which device, the bytes, and the timing.\nWhile recording, pad hits are recorded instead of moving the menu. Press Enter to finish early or Esc to cancel.",
                "10 秒のあいだに届いた MIDI を、機器・中身・時刻つきですべて記録します。\n記録しているあいだは、叩いても画面は動きません。早めに終えるなら Enter、取りやめるなら Esc キーです。",
            ),
            Item::Log => t(
                "The last 300 lines of the log since the game started.",
                "起動してからのログの、最後の 300 行です。",
            ),
            Item::PrevLog => t(
                "The log from the last time the game ran. Use this after a crash.",
                "前回起動したときのログです。クラッシュしたあとに送るときに使います。",
            ),
            Item::Tool(i) => TOOLS[i].desc.get(),
        }
    }

    /// 一覧に出す短い説明
    fn summary(self) -> &'static str {
        match self {
            Item::Env => t(
                "Version, OS, sound, and MIDI devices",
                "バージョン・OS・音・MIDI 機器",
            ),
            Item::Midi => t(
                "Every message received for 10 seconds",
                "10 秒のあいだに届いた MIDI をすべて",
            ),
            Item::Log => t(
                "The last 300 lines since startup",
                "起動してからの最後の 300 行",
            ),
            Item::PrevLog => t(
                "The last run's log, for crashes",
                "前回のログ。クラッシュしたあとに",
            ),
            Item::Tool(i) => TOOLS[i].desc.get().lines().next().unwrap_or(""),
        }
    }

    fn confirm(self) -> Option<&'static str> {
        match self {
            Item::Tool(i) => TOOLS[i].confirm.map(|c| c.get()),
            _ => None,
        }
    }

    fn labels(self) -> &'static [&'static str] {
        match self {
            Item::Tool(i) => TOOLS[i].labels,
            _ => &[],
        }
    }
}

/// 道具のスレッドから届くもの
enum Msg {
    Progress(String),
    Done(Finding),
}

/// 道具を動かしているところ
struct Run {
    item: Item,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    lines: Vec<String>,
    started: Instant,
}

/// MIDI を記録しているところ
struct Rec {
    started: Instant,
    start_us: i64,
    entries: Vec<MidiEntry>,
    noise: usize,
}

enum Mode {
    Idle,
    /// 始める前の確かめ
    Confirm,
    Running(Run),
    Recording(Rec),
}

/// できた下書き
struct Ready {
    item: Item,
    report: Report,
    /// 折り返した本文の行（描くときに作る）
    lines: Option<Vec<String>>,
    scroll: usize,
}

/// 下書きのボタン
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Button {
    Browser,
    Copy,
    File,
}

const BUTTONS: [Button; 3] = [Button::Browser, Button::Copy, Button::File];

impl Button {
    fn label(self) -> &'static str {
        match self {
            Button::Browser => t("Open Issue in Browser", "ブラウザで issue を作る"),
            Button::Copy => t("Copy", "コピー"),
            Button::File => t("Show File", "ファイルを見る"),
        }
    }

    fn desc(self) -> &'static str {
        match self {
            Button::Browser => t(
                "Opens a new GitHub issue with this report filled in. Check it and submit it. If GitHub asks you to sign in first, sign in and press this again.",
                "この下書きを入れた GitHub の新しい issue の画面を開きます。中身を確かめて送ってください。先にサインインを求められたら、サインインしてからもう一度押してください。",
            ),
            Button::Copy => t(
                "Copies the title and report to the clipboard.",
                "タイトルと本文をクリップボードにコピーします。",
            ),
            Button::File => t(
                "Shows the saved file. Drag it onto an issue to attach it.",
                "保存したファイルを表示します。issue にドラッグすると添付できます。",
            ),
        }
    }
}

pub struct DevView {
    items: Vec<Item>,
    cursor: usize,
    mode: Mode,
    env: Environment,
    names: Names,
    /// MIDI のポートの名前
    ports: Vec<String>,
    /// 設定のフォルダ（ログを読み、報告を保存する）
    dir: Option<PathBuf>,
    ready: Option<Ready>,
    /// 下書きのボタンを操作している（でなければ左の一覧）
    on_report: bool,
    button: usize,
    toast: Option<(String, Instant)>,
}

impl DevView {
    pub fn new(env: Environment, ports: Vec<String>, dir: Option<PathBuf>) -> Self {
        Self {
            items: Item::all(),
            cursor: 0,
            mode: Mode::Idle,
            env,
            names: Names::current(),
            ports,
            dir,
            ready: None,
            on_report: false,
            button: 0,
            toast: None,
        }
    }

    fn item(&self) -> Item {
        self.items[self.cursor.min(self.items.len() - 1)]
    }

    /// 選んでいる項目の下書き
    fn ready_here(&self) -> Option<&Ready> {
        self.ready.as_ref().filter(|r| r.item == self.item())
    }

    /// 届いた MIDI をそのまま受け取りたい（記録している）。そのあいだはパッドで操作しない
    pub fn wants_midi(&self) -> bool {
        matches!(self.mode, Mode::Recording(_))
    }

    /// 記録している MIDI を足す
    pub fn feed_midi(&mut self, raw: &RawMidi) {
        let Mode::Recording(rec) = &mut self.mode else {
            return;
        };
        if dev_report::is_noise(&raw.bytes) {
            rec.noise += 1;
        } else {
            rec.entries.push(MidiEntry {
                at_us: raw.arrival_us - rec.start_us,
                device_us: raw.device_us,
                port: raw.port,
                bytes: raw.bytes.clone(),
            });
        }
    }

    pub fn toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now()));
    }

    /// 毎フレーム呼ぶ（道具の進み具合を受け取り、記録の時間が来たら終える）
    pub fn tick(&mut self) -> Option<Se> {
        match &mut self.mode {
            Mode::Running(run) => {
                let mut done = None;
                loop {
                    match run.rx.try_recv() {
                        Ok(Msg::Progress(line)) => {
                            run.lines.push(line);
                            if run.lines.len() > PROGRESS_KEEP {
                                run.lines.remove(0);
                            }
                        }
                        Ok(Msg::Done(f)) => {
                            done = Some(f);
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            // 道具が panic した
                            done = Some(Finding {
                                title: tf!(
                                    "{} stopped unexpectedly",
                                    "{}が途中で止まりました",
                                    run.item.name()
                                ),
                                body: format!(
                                    "{}\n\n```text\n{}\n```",
                                    t(
                                        "The tool stopped with an error. The log has the details. Progress so far:",
                                        "ツールがエラーで止まりました。詳しくはログにあります。止まるまでの進み具合:",
                                    ),
                                    run.lines.join("\n")
                                ),
                            });
                            break;
                        }
                    }
                }
                let item = run.item;
                done.map(|f| self.finish(item, f))
            }
            Mode::Recording(rec) if rec.started.elapsed() >= Duration::from_secs(MIDI_SECONDS) => {
                Some(self.finish_recording())
            }
            _ => None,
        }
    }

    /// 記録を終えて下書きにする
    fn finish_recording(&mut self) -> Se {
        let Mode::Recording(rec) = std::mem::replace(&mut self.mode, Mode::Idle) else {
            return Se::Decide;
        };
        let seconds = rec.started.elapsed().as_secs_f64().min(MIDI_SECONDS as f64);
        let f = dev_report::midi_finding(&rec.entries, &self.ports, seconds, rec.noise);
        self.finish(Item::Midi, f)
    }

    /// 結果を下書きにして保存する
    fn finish(&mut self, item: Item, finding: Finding) -> Se {
        self.mode = Mode::Idle;
        let mut report = Report::new(item.key(), finding, item.labels(), &self.env, &self.names);
        match self.dir.as_ref().map(|d| report.save(&d.join(REPORTS_DIR))) {
            Some(Ok(path)) => {
                log::info!("saved a report: {}", path.display());
            }
            Some(Err(e)) => {
                log::warn!("could not save the report: {e}");
                self.toast(tf!(
                    "Couldn't save the report: {e}",
                    "下書きを保存できませんでした: {e}"
                ));
            }
            None => {}
        }
        self.ready = Some(Ready {
            item,
            report,
            lines: None,
            scroll: 0,
        });
        if let Some(i) = self.items.iter().position(|&x| x == item) {
            self.cursor = i;
        }
        self.on_report = true;
        self.button = 0;
        Se::Decide
    }

    /// 選んでいる項目を始める
    fn start(&mut self) -> Se {
        let item = self.item();
        match item {
            Item::Env => self.finish(
                item,
                Finding {
                    title: t("Environment info", "環境の情報").to_owned(),
                    body: t(
                        "The environment of the PC running the game.",
                        "ゲームを動かしている PC の環境です。",
                    )
                    .to_owned(),
                },
            ),
            Item::Log | Item::PrevLog => {
                let f =
                    dev_report::log_finding(self.dir.as_deref(), item == Item::PrevLog, LOG_LINES);
                self.finish(item, f)
            }
            Item::Midi => {
                self.mode = Mode::Recording(Rec {
                    started: Instant::now(),
                    start_us: host_us(),
                    entries: Vec::new(),
                    noise: 0,
                });
                Se::Decide
            }
            Item::Tool(i) => {
                let tool = &TOOLS[i];
                if tools_running() {
                    self.toast(
                        t(
                            "The previous tool is still finishing. Try again in a moment.",
                            "前のツールがまだ終わっていません。少し待ってからもう一度どうぞ",
                        )
                        .to_owned(),
                    );
                    return Se::Cancel;
                }
                let run = tool.run;
                let cancel = Arc::new(AtomicBool::new(false));
                let (tx, rx) = channel();
                let flag = Arc::clone(&cancel);
                let spawned = std::thread::Builder::new()
                    .name(format!("dev-{}", tool.key))
                    .spawn(move || {
                        let progress_tx = tx.clone();
                        let mut progress = move |line: String| {
                            let _ = progress_tx.send(Msg::Progress(line));
                        };
                        let f = run(&flag, &mut progress);
                        let _ = tx.send(Msg::Done(f));
                    });
                match spawned {
                    Ok(handle) => {
                        log::info!("dev tool started: {}", tool.key);
                        TOOL_THREADS
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((Arc::clone(&cancel), handle));
                        self.mode = Mode::Running(Run {
                            item,
                            cancel,
                            rx,
                            lines: Vec::new(),
                            started: Instant::now(),
                        });
                        Se::Decide
                    }
                    Err(e) => {
                        log::warn!("could not start {}: {e}", tool.key);
                        self.toast(tf!("Couldn't start: {e}", "始められませんでした: {e}"));
                        Se::Cancel
                    }
                }
            }
        }
    }

    pub fn input(&mut self, input: DevInput) -> DevResponse {
        let report_focus = self.on_report && self.ready_here().is_some();
        match &mut self.mode {
            Mode::Confirm => match input {
                DevInput::Decide => {
                    self.mode = Mode::Idle;
                    DevResponse::se(self.start())
                }
                DevInput::Cancel => {
                    self.mode = Mode::Idle;
                    DevResponse::se(Se::Cancel)
                }
                _ => DevResponse::default(),
            },
            Mode::Running(run) => {
                if input != DevInput::Cancel {
                    return DevResponse::default();
                }
                if run.cancel.swap(true, Ordering::SeqCst) {
                    // 2 回目は止まるのを待たずに一覧へ（道具は後ろで後片付けをして終わる）
                    log::info!("left the dev tool without waiting: {}", run.item.key());
                    self.mode = Mode::Idle;
                    self.toast(
                        t(
                            "Went back without waiting. The tool finishes in the background.",
                            "終わるのを待たずに戻りました。ツールはバックグラウンドで終了します。",
                        )
                        .to_owned(),
                    );
                } else {
                    log::info!("cancelling the dev tool: {}", run.item.key());
                }
                DevResponse::se(Se::Cancel)
            }
            Mode::Recording(_) => match input {
                DevInput::Decide => DevResponse::se(self.finish_recording()),
                DevInput::Cancel => {
                    self.mode = Mode::Idle;
                    self.toast(t("Stopped recording", "記録をキャンセルしました").to_owned());
                    DevResponse::se(Se::Cancel)
                }
                _ => DevResponse::default(),
            },
            Mode::Idle if report_focus => self.report_input(input),
            Mode::Idle => self.list_input(input),
        }
    }

    fn list_input(&mut self, input: DevInput) -> DevResponse {
        self.on_report = false;
        match input {
            DevInput::Up | DevInput::Down => {
                let d: isize = if input == DevInput::Up { -1 } else { 1 };
                let next = self
                    .cursor
                    .saturating_add_signed(d)
                    .min(self.items.len() - 1);
                if next == self.cursor {
                    return DevResponse::default();
                }
                self.cursor = next;
                DevResponse::se(Se::Move)
            }
            DevInput::Right if self.ready_here().is_some() => {
                self.on_report = true;
                DevResponse::se(Se::Move)
            }
            DevInput::Decide => {
                if self.item().confirm().is_some() {
                    self.mode = Mode::Confirm;
                    DevResponse::se(Se::Decide)
                } else {
                    DevResponse::se(self.start())
                }
            }
            DevInput::Cancel => DevResponse {
                se: Some(Se::Cancel),
                back: true,
            },
            _ => DevResponse::default(),
        }
    }

    fn report_input(&mut self, input: DevInput) -> DevResponse {
        match input {
            DevInput::Left if self.button == 0 => {
                self.on_report = false;
                DevResponse::se(Se::Move)
            }
            DevInput::Left => {
                self.button -= 1;
                DevResponse::se(Se::Move)
            }
            DevInput::Right if self.button + 1 < BUTTONS.len() => {
                self.button += 1;
                DevResponse::se(Se::Move)
            }
            DevInput::Up | DevInput::Down => {
                let Some(r) = self.ready.as_mut() else {
                    return DevResponse::default();
                };
                let max = r
                    .lines
                    .as_ref()
                    .map_or(0, |l| l.len().saturating_sub(visible_lines()));
                let next = if input == DevInput::Up {
                    r.scroll.saturating_sub(SCROLL)
                } else {
                    (r.scroll + SCROLL).min(max)
                };
                if next == r.scroll {
                    return DevResponse::default();
                }
                r.scroll = next;
                DevResponse::se(Se::Move)
            }
            DevInput::Decide => {
                let ok = self.press(BUTTONS[self.button]);
                DevResponse::se(if ok { Se::Decide } else { Se::Cancel })
            }
            DevInput::Cancel => {
                self.on_report = false;
                DevResponse::se(Se::Cancel)
            }
            _ => DevResponse::default(),
        }
    }

    /// ボタンを押す。できたら true
    fn press(&mut self, button: Button) -> bool {
        let Some(report) = self.ready.as_ref().map(|r| &r.report) else {
            return false;
        };
        let (ok, text) = match button {
            Button::Browser => {
                let (url, pasted) = match report.send() {
                    Send::Url(url) => (url, None),
                    Send::Paste { url, body } => (url, Some(dev_report::copy(&body))),
                };
                match (crate::editor::open_url(&url), pasted) {
                    (Ok(crate::editor::Opened::Recently), _) => return false,
                    (Ok(_), None) => (
                        true,
                        t(
                            "Opened the issue page in your browser. Check it and submit it.",
                            "ブラウザで issue の画面を開きました。中身を確かめて送ってください",
                        )
                        .to_owned(),
                    ),
                    (Ok(_), Some(Ok(()))) => (
                        true,
                        t(
                            "Too long for the link, so it's on your clipboard. Paste it into the issue.",
                            "本文が長いのでクリップボードにコピーしました。issue の本文の欄に貼り付けてください",
                        )
                        .to_owned(),
                    ),
                    (Ok(_), Some(Err(e))) => {
                        log::warn!("could not copy the report: {e}");
                        (
                            true,
                            t(
                                "Too long for the link and couldn't copy it. Attach the saved file instead.",
                                "本文が長いうえ、コピーできませんでした。保存したファイルを添えてください",
                            )
                            .to_owned(),
                        )
                    }
                    (Err(e), _) => {
                        log::warn!("could not open a browser: {e}");
                        (
                            false,
                            t(
                                "Couldn't open a browser. Use Copy or the saved file instead.",
                                "ブラウザを開けませんでした。コピーか保存したファイルを使ってください",
                            )
                            .to_owned(),
                        )
                    }
                }
            }
            Button::Copy => match dev_report::copy(&report.markdown()) {
                Ok(()) => (
                    true,
                    t("Copied to the clipboard", "クリップボードにコピーしました").to_owned(),
                ),
                Err(e) => {
                    log::warn!("could not copy the report: {e}");
                    (
                        false,
                        tf!("Couldn't copy: {e}", "コピーできませんでした: {e}"),
                    )
                }
            },
            Button::File => match &report.saved {
                Some(path) => match dev_report::reveal(path) {
                    Ok(()) => (
                        true,
                        tf!("Saved as {}", "{} に保存してあります", file_name(path)),
                    ),
                    Err(e) => {
                        log::warn!("could not show {}: {e}", path.display());
                        (
                            false,
                            tf!(
                                "Couldn't open the folder. The file is {}",
                                "フォルダを開けませんでした。ファイルは {} です",
                                path.display()
                            ),
                        )
                    }
                },
                None => (
                    false,
                    t("The report wasn't saved", "下書きは保存できていません").to_owned(),
                ),
            },
        };
        self.toast(text);
        ok
    }

    fn hints(&self) -> Vec<Hint<'static>> {
        match &self.mode {
            Mode::Confirm => vec![
                (&[Pad::CY], t("Start (Enter)", "始める（Enter）")),
                (&[Pad::LC], t("Cancel (Esc)", "キャンセル（Esc）")),
            ],
            Mode::Running(run) => vec![(
                &[Pad::LC],
                if run.cancel.load(Ordering::SeqCst) {
                    t("Back Without Waiting (Esc)", "待たずに戻る（Esc）")
                } else {
                    t("Cancel (Esc)", "キャンセル（Esc）")
                },
            )],
            Mode::Recording(_) => vec![
                (&[], t("Enter: Finish Now", "Enter: ここで終了")),
                (&[], t("Esc: Cancel", "Esc: キャンセル")),
            ],
            Mode::Idle if self.on_report && self.ready_here().is_some() => vec![
                (&[Pad::SD, Pad::FT], t("Buttons (←→)", "ボタン（←→）")),
                (&[Pad::HT, Pad::LT], t("Scroll (↑↓)", "スクロール（↑↓）")),
                (&[Pad::CY], t("Select (Enter)", "決定（Enter）")),
                (&[Pad::LC], t("To the List (Esc)", "一覧へ（Esc）")),
            ],
            Mode::Idle => {
                let mut v: Vec<Hint<'static>> = vec![
                    (&[Pad::HT], t("Up", "上")),
                    (&[Pad::LT], t("Down", "下")),
                    (&[Pad::CY], t("Start (Enter)", "始める（Enter）")),
                ];
                if self.ready_here().is_some() {
                    v.push((&[Pad::FT], t("To the Report (→)", "下書きへ（→）")));
                }
                v.push((&[Pad::LC], t("Back (Esc)", "戻る（Esc）")));
                v
            }
        }
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx) {
        match g.skin.tex("Graphics/4_background.png") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(20, 18, 30)),
        }
        c.cover(Color32::from_black_alpha(150));
        g.heading(c, 40.0, 14.0, t("Developer Menu", "開発者メニュー"));
        ui::text_right(
            c,
            g,
            &tf!(
                "Reports go to github.com/{}",
                "送り先: github.com/{}",
                "check5004/DTXmaniaNXT"
            ),
            1240.0,
            22.0,
            &Style::edged(15.0, ui::DIM, Color32::BLACK, 1.0),
            600.0,
        );
        self.draw_list(c, g);
        self.draw_right(c, g);
        if matches!(self.mode, Mode::Confirm) {
            self.draw_confirm(c, g);
        }
        if let Some((text, at)) = &self.toast {
            if at.elapsed() < TOAST_FOR {
                draw_toast(c, g, text);
            } else {
                self.toast = None;
            }
        }
        ui::legend(c, g, &self.hints());
    }

    fn draw_list(&self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, _) = LIST;
        ui::panel(c, LIST.0, LIST.1, LIST.2, LIST.3);
        let name = ui::bold(18.0, ui::TEXT);
        let sel_name = ui::glow(18.0);
        let desc = ui::plain(13.0, ui::MUTED);
        for (i, &item) in self.items.iter().enumerate() {
            let ry = y + 16.0 + i as f32 * (ROW_H + ROW_GAP);
            let sel = i == self.cursor;
            if sel {
                let alpha = if self.on_report { 0.45 } else { 1.0 };
                ui::selection(c, x + 12.0, ry, w - 24.0, ROW_H, alpha);
            }
            let style = if sel { &sel_name } else { &name };
            ui::text(c, g, item.name(), x + 26.0, ry + 9.0, style, w - 60.0);
            let summary = fit(g, item.summary(), &desc, w - 52.0);
            ui::text(c, g, &summary, x + 26.0, ry + 36.0, &desc, w - 52.0);
            if self.ready.as_ref().is_some_and(|r| r.item == item) {
                ui::dot(c, x + w - 26.0, ry + 20.0, 4.0, ui::STRONG);
            }
        }
    }

    fn draw_right(&mut self, c: &Canvas, g: &mut Gfx) {
        let (x, y, w, h) = RIGHT;
        ui::panel(c, x, y, w, h);
        let item = self.item();
        let head = ui::bold(22.0, Color32::WHITE);
        let body = ui::plain(16.0, ui::TEXT);
        let small = ui::plain(13.0, ui::DIM);
        match &self.mode {
            Mode::Running(run) if run.item == item => {
                ui::text(c, g, item.name(), x + 16.0, y + 14.0, &head, w - 32.0);
                let secs = run.started.elapsed().as_secs();
                let state = if run.cancel.load(Ordering::SeqCst) {
                    tf!("Cancelling… {secs} s", "キャンセルしています… {secs} 秒")
                } else {
                    tf!("Running… {secs} s", "実行中… {secs} 秒")
                };
                ui::text(
                    c,
                    g,
                    &state,
                    x + 16.0,
                    y + 52.0,
                    &ui::bold(16.0, ui::YELLOW),
                    w,
                );
                let lines: Vec<&str> = run.lines.iter().map(String::as_str).collect();
                draw_lines(c, g, &lines, true);
                return;
            }
            Mode::Recording(rec) => {
                ui::text(c, g, item.name(), x + 16.0, y + 14.0, &head, w - 32.0);
                let left = MIDI_SECONDS.saturating_sub(rec.started.elapsed().as_secs());
                let state = tf!(
                    "Recording… {left} s left · {} messages",
                    "記録しています… 残り {left} 秒 ・ {} 件",
                    rec.entries.len() + rec.noise
                );
                ui::text(
                    c,
                    g,
                    &state,
                    x + 16.0,
                    y + 52.0,
                    &ui::bold(16.0, ui::YELLOW),
                    w,
                );
                ui::text(
                    c,
                    g,
                    if self.ports.is_empty() {
                        t(
                            "No MIDI input device is connected. Esc cancels.",
                            "MIDI の入力機器がつながっていません。Esc でキャンセルします。",
                        )
                    } else {
                        t(
                            "Hit the pads. Enter finishes now, Esc cancels.",
                            "パッドを叩いてください。Enter で終了、Esc でキャンセルします。",
                        )
                    },
                    x + 16.0,
                    y + 78.0,
                    &small,
                    w - 32.0,
                );
                let lines: Vec<String> = rec
                    .entries
                    .iter()
                    .map(|e| {
                        format!(
                            "{:>8.1} ms  #{}  {}",
                            e.at_us as f64 / 1000.0,
                            e.port + 1,
                            dev_report::describe(&e.bytes)
                        )
                    })
                    .collect();
                let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
                draw_lines(c, g, &lines, true);
                return;
            }
            _ => {}
        }
        let Some(ready) = self.ready.as_mut().filter(|r| r.item == item) else {
            ui::text(c, g, item.name(), x + 16.0, y + 14.0, &head, w - 32.0);
            let bottom = ui::paragraph(
                c,
                g,
                item.desc(),
                x + 16.0,
                y + 56.0,
                w - 32.0,
                26.0,
                y + h - 80.0,
                &body,
            );
            ui::text(
                c,
                g,
                t("Hit CY (Enter) to start.", "CY（Enter）で始めます。"),
                x + 16.0,
                bottom + 14.0,
                &ui::bold(16.0, ui::YELLOW),
                w - 32.0,
            );
            draw_steps(c, g);
            return;
        };
        let r = &ready.report;
        ui::text(
            c,
            g,
            &r.title,
            x + 16.0,
            y + 14.0,
            &ui::bold(19.0, Color32::WHITE),
            w - 32.0,
        );
        let saved = r.saved.as_deref().map_or_else(
            || t("Not saved", "保存できていません").to_owned(),
            |p| tf!("Saved: {}", "保存: {}", file_name(p)),
        );
        let info = format!(
            "{saved} ・ {}",
            tf!("{} characters", "{} 文字", r.body.chars().count())
        );
        ui::text(c, g, &info, x + 16.0, y + 46.0, &small, w - 300.0);
        ui::text(
            c,
            g,
            t(
                "Check it before sending. Folder names with your user name and the PC name are hidden.",
                "送る前に中身を確かめてください。ユーザー名の入ったフォルダの場所と PC の名前は伏せてあります。",
            ),
            x + 16.0,
            y + 70.0,
            &ui::plain(13.0, ui::YELLOW),
            w - 32.0,
        );
        let lines = ready
            .lines
            .get_or_insert_with(|| g.texts.wrap(&r.body, PREVIEW_PX, PREVIEW.2 - 24.0));
        let total = lines.len();
        let from = ready.scroll.min(total);
        let shown: Vec<&str> = lines[from..].iter().map(String::as_str).collect();
        draw_lines(c, g, &shown, false);
        let to = (from + visible_lines()).min(total);
        ui::text_right(
            c,
            g,
            &tf!(
                "Lines {}–{to} of {total}",
                "{}–{to} 行目 / {total} 行",
                from + 1
            ),
            PREVIEW.0 + PREVIEW.2,
            y + 47.0,
            &ui::plain(13.0, ui::MUTED),
            260.0,
        );
        for (i, b) in BUTTONS.iter().enumerate() {
            let bx = PREVIEW.0 + i as f32 * (BUTTON_W + BUTTON_GAP);
            let sel = self.on_report && i == self.button;
            if sel {
                ui::selection(c, bx, BUTTON_Y, BUTTON_W, BUTTON_H, 1.0);
            } else {
                ui::round_frame(
                    c,
                    bx,
                    BUTTON_Y,
                    BUTTON_W,
                    BUTTON_H,
                    3.0,
                    1.0,
                    Color32::from_white_alpha(77),
                );
            }
            let style = if sel {
                ui::bold(17.0, ui::YELLOW)
            } else {
                ui::plain(17.0, ui::TEXT)
            };
            ui::text_center(
                c,
                g,
                b.label(),
                bx + BUTTON_W / 2.0,
                BUTTON_Y + 11.0,
                &style,
                BUTTON_W - 16.0,
            );
        }
        let note = if self.on_report {
            BUTTONS[self.button].desc()
        } else {
            t(
                "Hit FT (→) to use the buttons.",
                "FT（→）でボタンを選べます。",
            )
        };
        ui::paragraph(
            c,
            g,
            note,
            PREVIEW.0,
            BUTTON_Y + BUTTON_H + 10.0,
            PREVIEW.2,
            18.0,
            y + h - 2.0,
            &small,
        );
    }

    fn draw_confirm(&self, c: &Canvas, g: &mut Gfx) {
        c.cover(Color32::from_black_alpha(140));
        let (x, y, w, h) = (330.0, 190.0, 620.0, 320.0);
        ui::pick_panel(c, x, y, w, h);
        ui::text(
            c,
            g,
            t("Before You Start", "始める前に"),
            x + 24.0,
            y + 20.0,
            &ui::bold(21.0, Color32::WHITE),
            w - 48.0,
        );
        ui::paragraph(
            c,
            g,
            self.item().confirm().unwrap_or(""),
            x + 24.0,
            y + 62.0,
            w - 48.0,
            26.0,
            y + h - 64.0,
            &ui::plain(17.0, ui::TEXT),
        );
        ui::text(
            c,
            g,
            t(
                "CY (Enter) to start · LC (Esc) to cancel",
                "CY（Enter）で始める ・ LC（Esc）でキャンセル",
            ),
            x + 24.0,
            y + h - 44.0,
            &ui::bold(16.0, ui::YELLOW),
            w - 48.0,
        );
    }
}

/// 右の板の下に、送るまでの流れを出す（まだ下書きが無いとき）
fn draw_steps(c: &Canvas, g: &mut Gfx) {
    let (x, y, w, h) = RIGHT;
    let top = y + h - 188.0;
    c.fill(
        x + 16.0,
        top - 14.0,
        w - 32.0,
        1.0,
        Color32::from_white_alpha(40),
    );
    ui::text(
        c,
        g,
        t("How Sending Works", "送るまでの流れ"),
        x + 16.0,
        top,
        &ui::bold(16.0, ui::DIM),
        w - 32.0,
    );
    let steps = [
        t(
            "Start an item. When it's done, a draft GitHub issue appears here and is saved as a file.",
            "項目を始めます。終わると、ここに GitHub の issue の下書きが出て、ファイルにも保存されます。",
        ),
        t(
            "Check the draft. Folder names with your user name and the PC name are hidden.",
            "下書きの中身を確かめます。ユーザー名の入ったフォルダの場所と PC の名前は伏せてあります。",
        ),
        t(
            "\"Open Issue in Browser\" opens GitHub with the draft filled in. Submit it there.",
            "「ブラウザで issue を作る」で、下書きを入れた GitHub の画面が開きます。そこで送ります。",
        ),
    ];
    let style = ui::plain(14.0, ui::DIM);
    for (i, step) in steps.iter().enumerate() {
        let sy = top + 34.0 + i as f32 * 44.0;
        ui::round_fill(c, x + 16.0, sy, 24.0, 24.0, 12.0, ui::MAGENTA);
        ui::text_center(
            c,
            g,
            &(i + 1).to_string(),
            x + 28.0,
            sy + 3.0,
            &ui::bold(14.0, Color32::WHITE),
            20.0,
        );
        ui::paragraph(
            c,
            g,
            step,
            x + 52.0,
            sy + 2.0,
            w - 72.0,
            20.0,
            sy + 44.0,
            &style,
        );
    }
}

/// 本文の欄に見える行の数
fn visible_lines() -> usize {
    ((PREVIEW.3 - 8.0) / PREVIEW_LINE).floor() as usize
}

/// 本文の欄に行を描く（`tail` なら最後の行が見えるように）
fn draw_lines(c: &Canvas, g: &mut Gfx, lines: &[&str], tail: bool) {
    let (x, y, w, h) = PREVIEW;
    c.fill(x, y, w, h, Color32::from_black_alpha(140));
    c.frame(x, y, w, h, Color32::from_white_alpha(40));
    let n = visible_lines();
    let from = if tail {
        lines.len().saturating_sub(n)
    } else {
        0
    };
    let style = ui::plain(PREVIEW_PX, ui::TEXT);
    for (i, line) in lines.iter().skip(from).take(n).enumerate() {
        if line.is_empty() {
            continue;
        }
        ui::text(
            c,
            g,
            line,
            x + 12.0,
            y + 4.0 + i as f32 * PREVIEW_LINE,
            &style,
            w - 24.0,
        );
    }
}

/// 幅に収まらなければ、後ろを「…」にして縮める
fn fit(g: &mut Gfx, s: &str, style: &Style, max_w: f32) -> String {
    if ui::text_width(g, s, style) <= max_w {
        return s.to_owned();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cut: String = chars.iter().collect::<String>().trim_end().to_owned() + "…";
        if ui::text_width(g, &cut, style) <= max_w {
            return cut;
        }
    }
    String::new()
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn draw_toast(c: &Canvas, g: &mut Gfx, text: &str) {
    let style = ui::plain(17.0, Color32::WHITE);
    let w = ui::text_width(g, text, &style).min(1000.0) + 40.0;
    let x = 640.0 - w / 2.0;
    c.fill(
        x,
        636.0,
        w,
        40.0,
        Color32::from_rgba_premultiplied(10, 8, 18, 235),
    );
    ui::round_frame(c, x, 636.0, w, 40.0, 2.0, 1.0, ui::ORANGE);
    ui::text_center(c, g, text, 640.0, 645.0, &style, 1000.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> DevView {
        DevView::new(Environment::default(), vec!["Pad".to_owned()], None)
    }

    #[test]
    fn the_environment_report_opens_at_the_buttons() {
        let mut v = view();
        let r = v.input(DevInput::Decide);
        assert_eq!(r.se, Some(Se::Decide));
        let ready = v.ready.as_ref().expect("a report");
        assert_eq!(ready.item, Item::Env);
        assert!(ready.report.title.starts_with("[dev] "));
        assert!(v.on_report);
        // ボタンの間を動き、左の端で一覧へ戻る
        assert_eq!(v.input(DevInput::Right).se, Some(Se::Move));
        assert_eq!(v.button, 1);
        v.input(DevInput::Left);
        v.input(DevInput::Left);
        assert!(!v.on_report);
        // 一覧から FT で下書きへ戻れる
        v.input(DevInput::Right);
        assert!(v.on_report);
        // LC は一覧へ、もう一度で設定へ戻る
        v.input(DevInput::Cancel);
        assert!(!v.on_report);
        assert!(v.input(DevInput::Cancel).back);
    }

    #[test]
    fn midi_is_recorded_until_finished() {
        let mut v = view();
        v.input(DevInput::Down);
        assert_eq!(v.item(), Item::Midi);
        v.input(DevInput::Decide);
        assert!(v.wants_midi());
        // 上下は効かない（記録中はパッドの操作を受けない）
        v.input(DevInput::Down);
        assert_eq!(v.item(), Item::Midi);
        let raw = |bytes: &[u8]| RawMidi {
            port: 0,
            device_us: 0,
            arrival_us: host_us(),
            bytes: bytes.to_vec(),
        };
        v.feed_midi(&raw(&[0x99, 38, 100]));
        v.feed_midi(&raw(&[0xF8]));
        v.input(DevInput::Decide);
        assert!(!v.wants_midi());
        let ready = v.ready.as_ref().expect("a report");
        assert!(ready.report.body.contains("ch10 note on 38 vel 100"));
    }

    #[test]
    fn cancelling_a_recording_makes_no_report() {
        let mut v = view();
        v.input(DevInput::Down);
        v.input(DevInput::Decide);
        v.input(DevInput::Cancel);
        assert!(!v.wants_midi());
        assert!(v.ready.is_none());
    }

    #[test]
    fn the_list_stops_at_the_ends() {
        let mut v = view();
        assert_eq!(v.input(DevInput::Up).se, None);
        for _ in 0..10 {
            v.input(DevInput::Down);
        }
        assert_eq!(v.cursor, v.items.len() - 1);
    }
}
