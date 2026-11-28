//! 曲ごとの音源のキット（Roland V71・V51・V31。番地表は 3 機種で同じ）。
//!
//! - 曲を始めるとき（読み込みの画面）に、その曲に決めたキットへ音源を切り替える
//! - 選曲に戻るとき・アプリを終えるときに、切り替える前のキットに戻す。書き戻しの記録
//!   （[`crate::module_link`]）を使うので、落ちても次に起動したときに戻る
//! - キットを選んでいるあいだは、選んでいるキットに音源を切り替え、鳴る音が変わったころに
//!   試しのフレーズ（[`crate::kit_phrase`]）を音源で鳴らす。次を選ぶと途中でやめる
//! - キットの名前は音源から読み、設定のフォルダの `v51_kits.toml` など（機種ごと）に覚えておく
//! - 曲ごとのキットは機種ごとに残す（V71 のキット 12 と V51 のキット 12 は別の音なので）
//! - 機種の名前の MIDI ポートがあり、Identity Request に返事をした機種を「つないでいる」とする。
//!   つないでいなければ、画面には曲ごとのキットを出さない
//!
//! 音源とのやり取りはこのスレッド 1 本で順に行い、画面のスレッドは待たない。音源とのつながりは
//! 用があるときだけ開き、しばらく用が無ければ閉じる（測る道具と取り合わないように）。
//! 測定（V51、2026-11-28、issue #113）: キットを書いてから鳴る音が変わるまで 0.2〜0.5 秒より長い、RQ1 の往復は 72 ms。

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use drum_module::vseries::{self, Model};
use midir::MidiOutput;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::kit_phrase;
use crate::midi;
use crate::module_link::{ModuleLink, Restored, port_model};
use crate::time::host_us;

/// キットを書いてから、鳴る音が変わりきるまで待つ時間（測定では 0.5 秒でもまだ小さく鳴った）
pub const SETTLE: Duration = Duration::from_millis(1000);
/// 読み込みの画面で、キットの切り替えを待つ長さの上限（音源の返事が遅いときなど）
pub const MAX_WAIT: Duration = Duration::from_millis(3000);
/// 選んでいるあいだ、選ぶのが止まってから切り替えるまで（送るたびに切り替えないように）
const AUDITION_DELAY: Duration = Duration::from_millis(350);
/// 用が無くなってから音源とのつながりを閉じるまで
const LINK_IDLE: Duration = Duration::from_secs(2);
/// 起動してからキットの名前を読み始めるまで（起動したときの書き戻しのあと）
const NAMES_DELAY: Duration = Duration::from_secs(5);
/// 終えるとき、前のキットに戻すのを待つ長さ
const QUIT_WAIT: Duration = Duration::from_millis(2500);
/// 書き戻しの記録に残すわけ
const REASON: &str = "kit for a song";
/// フレーズの音がこれより近づいたら、受け取りを待たずに眠って時刻どおりに送る
/// （`recv_timeout` は Windows では 15 ms ほど遅れることがある）
const EXACT: Duration = Duration::from_millis(20);
/// 送ったノートが音源から返ってくる（Soft Thru が ON のとき）とみなす長さ
const ECHO_WINDOW_US: i64 = 80_000;

/// 今の様子（画面のスレッドが読む）
#[derive(Debug, Clone, Default)]
pub struct State {
    /// つないでいる音源の機種（Identity で確かめたもの。つないでいなければ None）
    pub model: Option<Model>,
    /// その機種のキットの名前（キット 1〜200 の順。読めていなければ空の文字）
    pub names: Vec<String>,
    /// 名前を読んでいるところなら、読み終えた数
    pub reading: Option<u16>,
    /// 音源が今鳴らしているキット（このアプリが知っている範囲で）
    pub now: Option<u16>,
    /// 切り替えを送った時刻（鳴る音が変わるまで待つのに使う）
    pub switched_at: Option<Instant>,
    /// 切り替えを待っている・送っているところ
    pub busy: bool,
    /// このアプリが切り替えて、まだ戻していない
    pub switched: bool,
}

impl State {
    /// キット `kit` の名前（読めていなければ空）
    pub fn name(&self, kit: u16) -> &str {
        self.names
            .get(usize::from(kit).wrapping_sub(1))
            .map_or("", String::as_str)
    }

    /// 切り替えが終わり、鳴る音も変わったころ
    pub fn settled(&self) -> bool {
        !self.busy && self.switched_at.is_none_or(|t| t.elapsed() >= SETTLE)
    }

    /// 名前を全部読んだか
    fn complete(&self) -> bool {
        self.names.len() == usize::from(vseries::KIT_COUNT)
    }
}

enum Cmd {
    /// 曲のためにキットを切り替える（None なら前のキットに戻す）。すぐに
    Apply(Option<u16>),
    /// 選んでいるキットにする（None なら前のキットに戻す）。選ぶのが止まってから。
    /// `play` なら、鳴る音が変わったころに試しのフレーズを鳴らす
    Audition { kit: Option<u16>, play: bool },
    /// 機種の名前のポートがある。音源を確かめ（ほかの機種とつないでいたら、つなぎ直す）、
    /// 読めていないキットの名前を読む
    Detect(Model),
    /// 機種の名前のポートが無くなった
    Gone,
    /// 演奏しているあいだは名前を読まない
    Quiet(bool),
}

/// 曲ごとのキットを受け持つスレッドとのつながり
pub struct Kits {
    tx: Option<Sender<Cmd>>,
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
    /// 終えたことを知らせる
    done: Option<Receiver<()>>,
    quiet: bool,
}

impl Kits {
    /// スレッドを始める。機種の名前の MIDI のポートがあれば、少しあとで音源を確かめてキットの名前を読む
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let (tx, rx) = channel();
        let (done_tx, done) = channel();
        let names_at = supported_port().map(|_| Instant::now() + NAMES_DELAY);
        let shared = state.clone();
        let worker = std::thread::Builder::new()
            .name("module-kits".into())
            .spawn(move || {
                Worker::new(shared, names_at).run(&rx);
                let _ = done_tx.send(());
            })
            .map_err(|e| log::warn!("drum module kits thread: {e}"))
            .ok();
        Self {
            tx: worker.is_some().then_some(tx),
            state,
            worker,
            done: Some(done),
            quiet: false,
        }
    }

    /// 今の様子
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// つないでいる音源の機種（つないでいなければ None。曲ごとのキットを出さない）
    pub fn model(&self) -> Option<Model> {
        self.state().model
    }

    fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        }
    }

    /// 曲のためにキット `kit` に切り替える（None なら前のキットに戻す）
    pub fn apply(&self, kit: Option<u16>) {
        {
            let mut st = self.state();
            // 切り替えていなければ、戻すものは無い
            if kit.is_none() && !st.switched && !st.busy {
                return;
            }
            st.busy = true;
        }
        self.send(Cmd::Apply(kit));
    }

    /// 選んだキット `kit` にして、試しのフレーズを鳴らす（None なら前のキットで鳴らす）
    pub fn audition(&self, kit: Option<u16>) {
        self.state().busy = true;
        self.send(Cmd::Audition { kit, play: true });
    }

    /// 選び終えた。前のキットに戻し、鳴らしているフレーズもやめる
    pub fn end_audition(&self) {
        self.state().busy = true;
        self.send(Cmd::Audition {
            kit: None,
            play: false,
        });
    }

    /// つないでいる音源を確かめ直す（簡易設定を開いたとき・MIDI の機器が変わったときに呼ぶ）。
    /// 機種の名前のポートが無ければ、すぐにつないでいないことにする。あれば、まだ確かめていないか、
    /// 機種が変わったか、名前を読み終えていなければ確かめに行く
    pub fn check(&self) {
        let port = supported_port();
        {
            let mut st = self.state();
            match port {
                None if st.model.is_none() => return,
                None => {
                    st.model = None;
                    st.names.clear();
                }
                Some(m) => {
                    if (st.model == Some(m) && st.complete()) || st.reading.is_some() {
                        return;
                    }
                }
            }
        }
        self.send(port.map_or(Cmd::Gone, Cmd::Detect));
    }

    /// 演奏しているあいだは音源に名前を聞かない（変わったときだけ送る）
    pub fn set_quiet(&mut self, on: bool) {
        if self.quiet != on {
            self.quiet = on;
            self.send(Cmd::Quiet(on));
        }
    }
}

impl Kits {
    /// 前のキットに戻してスレッドを終える（戻せなければ次に起動したときに戻る）。2 回目からは何もしない
    pub fn finish(&mut self) {
        // 送る口を閉じると、スレッドは戻してから終わる
        self.tx = None;
        if let Some(done) = self.done.take()
            && done.recv_timeout(QUIT_WAIT).is_ok()
            && let Some(w) = self.worker.take()
        {
            let _ = w.join();
        }
    }
}

impl Drop for Kits {
    fn drop(&mut self) {
        self.finish();
    }
}

/// キットの番号と名前（「12 Rock」。名前を読めていなければ「キット 12」）
pub fn label(kit: u16, name: &str) -> String {
    if name.is_empty() {
        i18n::tf!("Kit {kit}", "キット {kit}")
    } else {
        format!("{kit} {name}")
    }
}

/// 切り替えの頼み
#[derive(Debug, Clone, Copy)]
struct Want {
    kit: Option<u16>,
    /// この時刻になったら行う
    at: Instant,
    /// 切り替えたら試しのフレーズを鳴らす
    play: bool,
}

/// 鳴らしている試しのフレーズ
struct Playing {
    start: Instant,
    events: Vec<kit_phrase::Event>,
    /// 次に送るもの
    next: usize,
}

impl Playing {
    /// 次に送る時刻（送り終えたら None）
    fn due(&self) -> Option<Instant> {
        self.events.get(self.next).map(|e| self.start + e.at)
    }
}

struct Worker {
    state: Arc<Mutex<State>>,
    link: Option<ModuleLink>,
    /// 最後に音源とやり取りした時刻
    used: Instant,
    want: Option<Want>,
    /// これから名前を読むキット
    names: VecDeque<u16>,
    /// 名前を読み始める時刻（起動したとき）
    names_at: Option<Instant>,
    quiet: bool,
    playing: Option<Playing>,
    /// 見つからなかったことをログに書いた（続けて書かない）
    missing_logged: bool,
}

impl Worker {
    fn new(state: Arc<Mutex<State>>, names_at: Option<Instant>) -> Self {
        Self {
            state,
            link: None,
            used: Instant::now(),
            want: None,
            names: VecDeque::new(),
            names_at,
            quiet: false,
            playing: None,
            missing_logged: false,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn run(mut self, rx: &Receiver<Cmd>) {
        loop {
            let wait = self.wait();
            let got = if self.playing.is_none() {
                rx.recv_timeout(wait)
            } else if wait > EXACT {
                rx.recv_timeout(wait - EXACT)
            } else {
                // フレーズの次の音まで眠る（Windows でも 1 ms より細かく起きる）
                std::thread::sleep(wait);
                rx.try_recv().map_err(|e| match e {
                    TryRecvError::Empty => RecvTimeoutError::Timeout,
                    TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
                })
            };
            match got {
                Ok(cmd) => self.take(cmd),
                Err(RecvTimeoutError::Timeout) => self.step(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.stop_phrase();
        // 終えるときは、選んでいたキットも曲のキットも前のキットに戻す
        if self.state().switched {
            self.switch(None, false);
        }
    }

    fn take(&mut self, cmd: Cmd) {
        let now = Instant::now();
        match cmd {
            Cmd::Apply(kit) => {
                self.stop_phrase();
                self.want = Some(Want {
                    kit,
                    at: now,
                    play: false,
                });
            }
            Cmd::Audition { kit, play } => {
                self.stop_phrase();
                self.want = Some(Want {
                    kit,
                    at: now + AUDITION_DELAY,
                    play,
                });
            }
            Cmd::Detect(port) => self.detect(port),
            Cmd::Gone => {
                self.stop_phrase();
                self.link = None;
                self.names.clear();
                let mut st = self.state();
                st.model = None;
                st.names.clear();
                st.reading = None;
            }
            Cmd::Quiet(on) => {
                self.quiet = on;
                if on {
                    self.stop_phrase();
                }
            }
        }
    }

    /// 次に何かするまで待つ長さ
    fn wait(&self) -> Duration {
        let now = Instant::now();
        let until = |t: Instant| t.saturating_duration_since(now);
        if let Some(w) = self.want {
            return until(w.at);
        }
        if let Some(t) = self.playing.as_ref().and_then(Playing::due) {
            return until(t);
        }
        if !self.quiet {
            if !self.names.is_empty() {
                return Duration::ZERO;
            }
            if let Some(t) = self.names_at {
                return until(t);
            }
        }
        if self.link.is_some() {
            return until(self.used + LINK_IDLE);
        }
        Duration::from_secs(3600)
    }

    /// 時刻が来たことを 1 つ行う
    fn step(&mut self) {
        let now = Instant::now();
        if let Some(w) = self.want.filter(|w| w.at <= now) {
            self.want = None;
            self.switch(w.kit, w.play);
            return;
        }
        if self.want.is_some() {
            return;
        }
        // フレーズを鳴らしているあいだ（鳴る音が変わるのを待つあいだも）は名前を読まない
        if self.playing.is_some() {
            self.play_due(now);
            return;
        }
        if !self.quiet {
            if self.names_at.is_some_and(|t| t <= now) {
                self.names_at = None;
                self.queue_names();
                return;
            }
            if let Some(kit) = self.names.pop_front() {
                self.read_name(kit);
                return;
            }
        }
        if self.link.is_some() && self.used.elapsed() >= LINK_IDLE {
            self.link = None;
        }
    }

    fn queue_names(&mut self) {
        self.names = (1..=vseries::KIT_COUNT).collect();
        self.state().reading = Some(0);
    }

    /// 機種 `port` の名前のポートがある。つないでいる機種が違えばつなぎ直し、名前を読めていなければ読む
    fn detect(&mut self, port: Model) {
        if self.link.as_ref().is_some_and(|l| l.model != port) {
            self.link = None;
        }
        if self.link().is_some() && !self.state().complete() {
            self.queue_names();
        }
    }

    /// 音源とのつながり（無ければ探す）。見つからなければ None。
    /// 見つけた機種が前と違えば、その機種の覚えている名前にする
    fn link(&mut self) -> Option<&mut ModuleLink> {
        if self.link.is_none() {
            let mut notes = Vec::new();
            match ModuleLink::find(&mut |s| notes.push(s)) {
                Ok(l) => {
                    let mut st = self.state();
                    if st.model != Some(l.model) {
                        st.names = load_names(l.model);
                        st.model = Some(l.model);
                    }
                    drop(st);
                    self.missing_logged = false;
                    self.link = Some(l);
                }
                Err(e) => {
                    if !self.missing_logged {
                        log::info!("drum module kits: {e:#} {}", notes.join("; "));
                        self.missing_logged = true;
                    }
                    let mut st = self.state();
                    st.model = None;
                    st.names.clear();
                    return None;
                }
            }
        }
        self.used = Instant::now();
        self.link.as_mut()
    }

    /// やり取りに失敗した。つながりを閉じ、名前を読むのもフレーズもやめる
    fn lost(&mut self, e: &anyhow::Error) {
        log::warn!("drum module kits: {e:#}");
        self.link = None;
        self.names.clear();
        self.playing = None;
        let mut st = self.state();
        st.reading = None;
        st.busy = false;
    }

    /// キット `kit` にする（None なら前のキットに戻す）。`play` なら、そのあと試しのフレーズを鳴らす
    fn switch(&mut self, kit: Option<u16>, play: bool) {
        let Some(link) = self.link() else {
            self.state().busy = false;
            return;
        };
        let result = match kit {
            Some(kit) => switch_to(link, kit).map(|sent| (Some(kit), sent, true)),
            None => restore(link).map(|(now, sent)| (now, sent, false)),
        };
        let pending = link.pending() > 0;
        match result {
            Ok((now, sent, switched)) => {
                let start = {
                    let mut st = self.state();
                    st.now = now;
                    if sent {
                        st.switched_at = Some(Instant::now());
                    }
                    st.switched = pending && switched;
                    // 次の頼みが来ていれば、それが終わるまで busy のまま
                    st.busy = self.want.is_some();
                    // キットを書いたなら、鳴る音が変わるのを待ってから
                    match st.switched_at {
                        Some(t) if sent => t + SETTLE,
                        _ => Instant::now(),
                    }
                };
                if play && let Some(kit) = now {
                    self.prepare_phrase(kit, start);
                }
            }
            Err(e) => self.lost(&e),
        }
    }

    /// キット `kit` のノート番号を読み、試しのフレーズを `start` から鳴らす用意をする
    fn prepare_phrase(&mut self, kit: u16, start: Instant) {
        let Some(link) = self.link() else {
            return;
        };
        match link.read(vseries::kit_midi(kit)) {
            Ok(midi) => {
                self.playing = Some(Playing {
                    start,
                    events: kit_phrase::events(&midi),
                    next: 0,
                });
            }
            Err(e) => self.lost(&e),
        }
    }

    /// フレーズの、時刻が来た音を送る
    fn play_due(&mut self, now: Instant) {
        let Some(mut p) = self.playing.take() else {
            return;
        };
        if p.due().is_none_or(|t| t > now) {
            self.playing = Some(p);
            return;
        }
        let Some(link) = self.link() else {
            return;
        };
        let mut failed = None;
        while let Some(t) = p.due()
            && t <= now
        {
            let e = p.events[p.next];
            if let Err(err) = link.send(&e.bytes) {
                failed = Some(err);
                break;
            }
            if let Some((ch, note)) = e.note() {
                remember_sent(ch, note, host_us());
            }
            p.next += 1;
        }
        if let Some(e) = failed {
            self.lost(&e);
        } else if p.due().is_some() {
            self.playing = Some(p);
        }
    }

    /// 鳴らしているフレーズをやめる（途中なら、受けたハイハットのペダルの位置を戻す）
    fn stop_phrase(&mut self) {
        let Some(p) = self.playing.take() else {
            return;
        };
        if p.next == 0 || p.due().is_none() {
            return;
        }
        if let (Some(bytes), Some(link)) = (kit_phrase::stop(&p.events), self.link.as_mut())
            && let Err(e) = link.send(&bytes)
        {
            self.lost(&e);
        }
    }

    fn read_name(&mut self, kit: u16) {
        let Some(link) = self.link() else {
            self.names.clear();
            self.state().reading = None;
            return;
        };
        let model = link.model;
        match link.read(vseries::kit_common(kit)) {
            Ok(data) => {
                let name = vseries::kit_name(&data).unwrap_or_default();
                let done = self.names.is_empty();
                let mut st = self.state();
                let i = usize::from(kit - 1);
                if st.names.len() < usize::from(vseries::KIT_COUNT) {
                    st.names
                        .resize(usize::from(vseries::KIT_COUNT), String::new());
                }
                st.names[i] = name;
                st.reading = (!done).then_some(kit);
                if done {
                    let names = st.names.clone();
                    drop(st);
                    if let Err(e) = save_names(model, &names) {
                        log::warn!("{} kit names: {e:#}", model.name);
                    }
                    log::info!("{}: read {} kit names", model.name, names.len());
                }
            }
            Err(e) => self.lost(&e),
        }
    }
}

/// キットを切り替えるのに使う、音源とのやり取り（テストでは作りものの音源にする）
trait Link {
    fn read(&mut self, block: vseries::Block) -> Result<Vec<u8>>;
    fn remember(
        &mut self,
        block: vseries::Block,
        data: &[u8],
        offset: u32,
        len: usize,
        reason: &str,
    ) -> Result<()>;
    fn write(&mut self, addr: drum_module::roland::Address, data: &[u8]) -> Result<()>;
    fn pending(&self) -> usize;
    fn restore(&mut self) -> Result<Restored>;
}

impl Link for ModuleLink {
    fn read(&mut self, block: vseries::Block) -> Result<Vec<u8>> {
        ModuleLink::read(self, block)
    }
    fn remember(
        &mut self,
        block: vseries::Block,
        data: &[u8],
        offset: u32,
        len: usize,
        reason: &str,
    ) -> Result<()> {
        ModuleLink::remember(self, block, data, offset, len, reason)
    }
    fn write(&mut self, addr: drum_module::roland::Address, data: &[u8]) -> Result<()> {
        ModuleLink::write(self, addr, data)
    }
    fn pending(&self) -> usize {
        ModuleLink::pending(self)
    }
    fn restore(&mut self) -> Result<Restored> {
        ModuleLink::restore(self)
    }
}

/// キットを `kit` にする。前の値を書き戻しの記録に残してから書く。書いたら true（もうそのキットなら false）
fn switch_to(link: &mut impl Link, kit: u16) -> Result<bool> {
    let kit = kit.clamp(1, vseries::KIT_COUNT);
    let block = vseries::current();
    let data = link.read(block)?;
    let now = vseries::KIT_NUM
        .read(&data)
        .ok_or_else(|| anyhow!("no kit number in {:?}", data))?;
    if now + 1 == i32::from(kit) {
        return Ok(false);
    }
    link.remember(
        block,
        &data,
        vseries::KIT_NUM.offset,
        vseries::KIT_NUM.len,
        REASON,
    )?;
    let value = vseries::KIT_NUM.encode(i32::from(kit) - 1);
    link.write(block.at(vseries::KIT_NUM.offset), &value)?;
    log::info!("drum module: kit {} -> {kit}", now + 1);
    Ok(true)
}

/// 書き戻しの記録から前のキットに戻し、今のキットを読み直す。書き戻したら true も返す
fn restore(link: &mut impl Link) -> Result<(Option<u16>, bool)> {
    let mut wrote = false;
    if link.pending() > 0 {
        let r = link.restore()?;
        if !r.failed.is_empty() {
            log::warn!("drum module: could not restore {}", r.failed.join("; "));
        }
        wrote = r.written > 0;
    }
    let data = link.read(vseries::current())?;
    let now = vseries::KIT_NUM
        .read(&data)
        .and_then(|n| u16::try_from(n + 1).ok());
    Ok((now, wrote))
}

/// 最近音源に送ったノート（チャンネル、ノート番号、送ったホスト時刻）
static SENT: Mutex<VecDeque<(u8, u8, i64)>> = Mutex::new(VecDeque::new());

fn sent_notes() -> MutexGuard<'static, VecDeque<(u8, u8, i64)>> {
    SENT.lock().unwrap_or_else(|e| e.into_inner())
}

fn remember_sent(channel: u8, note: u8, at_us: i64) {
    let mut sent = sent_notes();
    sent.retain(|&(_, _, t)| at_us - t <= ECHO_WINDOW_US);
    sent.push_back((channel, note, at_us));
}

/// 届いた MIDI が、試しのフレーズで送ったノートが音源から返ってきたものか
/// （音源の Soft Thru が ON のとき。叩いたものとして画面に渡さない）
pub fn is_echo(bytes: &[u8], arrival_us: i64) -> bool {
    let &[status, note, velocity] = bytes else {
        return false;
    };
    if status & 0xF0 != 0x90 || velocity == 0 {
        return false;
    }
    let mut sent = sent_notes();
    if sent.is_empty() {
        return false;
    }
    sent.retain(|&(_, _, t)| arrival_us - t <= ECHO_WINDOW_US);
    let found = sent
        .iter()
        .position(|&(c, n, t)| c == status & 0x0F && n == note && arrival_us >= t);
    if let Some(i) = found {
        sent.remove(i);
    }
    found.is_some()
}

/// 機種の名前の MIDI 出力ポートがあれば、その機種（無ければ、音源を探さない。ほかの機器に聞かないように）
fn supported_port() -> Option<Model> {
    let out = MidiOutput::new("DTXmaniaNXT").ok()?;
    out.ports()
        .iter()
        .filter_map(|p| out.port_name(p).ok())
        .find_map(|n| port_model(&midi::display_name(&n)))
}

/// 覚えておくキットの名前のファイル（機種ごと。V51 なら `v51_kits.toml`）
fn names_path(model: Model) -> Option<PathBuf> {
    Some(Config::dir()?.join(names_file(model)))
}

fn names_file(model: Model) -> String {
    format!("{}_kits.toml", model.name.to_ascii_lowercase())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NamesFile {
    #[serde(default)]
    names: Vec<String>,
}

fn load_names(model: Model) -> Vec<String> {
    let Some(path) = names_path(model) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    match toml::from_str::<NamesFile>(&text) {
        Ok(f) => f.names,
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            Vec::new()
        }
    }
}

fn save_names(model: Model, names: &[String]) -> Result<()> {
    let path = names_path(model).ok_or_else(|| anyhow!("no settings folder"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = toml::to_string(&NamesFile {
        names: names.to_vec(),
    })?;
    let m = model.name;
    let text = format!(
        "# DTXmaniaNXT が {m} から読んだキットの名前です（キット 1 から順に）。\n\
         # Kit names read from the {m} (from kit 1).\n\n{body}"
    );
    fs::write(&path, text).with_context(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_looked_up_by_kit_number() {
        let st = State {
            names: vec!["Studio A".into(), "Rock".into()],
            ..State::default()
        };
        assert_eq!(st.name(1), "Studio A");
        assert_eq!(st.name(2), "Rock");
        assert_eq!(st.name(3), "");
        assert_eq!(st.name(0), "");
        assert!(!st.complete());
    }

    #[test]
    fn settles_after_the_kit_has_had_time_to_change() {
        let mut st = State::default();
        assert!(st.settled());
        st.switched_at = Some(Instant::now());
        assert!(!st.settled());
        st.switched_at = Some(Instant::now() - SETTLE);
        assert!(st.settled());
        st.busy = true;
        assert!(!st.settled());
    }

    /// 作りものの音源（Current の 4 バイトと、書き戻しの記録だけ）
    struct Fake {
        current: Vec<u8>,
        /// 書き戻しの記録（番地の頭と元の値）
        kept: Option<Vec<u8>>,
        writes: Vec<Vec<u8>>,
    }

    impl Fake {
        fn at_kit(kit: u16) -> Self {
            Self {
                current: vseries::KIT_NUM.encode(i32::from(kit) - 1),
                kept: None,
                writes: Vec::new(),
            }
        }
        fn kit(&self) -> i32 {
            vseries::KIT_NUM.read(&self.current).unwrap() + 1
        }
    }

    impl Link for Fake {
        fn read(&mut self, block: vseries::Block) -> Result<Vec<u8>> {
            assert_eq!(block, vseries::current());
            Ok(self.current.clone())
        }
        fn remember(
            &mut self,
            block: vseries::Block,
            data: &[u8],
            offset: u32,
            len: usize,
            _reason: &str,
        ) -> Result<()> {
            assert_eq!(block, vseries::current());
            if self.kept.is_none() {
                self.kept = Some(data[offset as usize..offset as usize + len].to_vec());
            }
            Ok(())
        }
        fn write(&mut self, addr: drum_module::roland::Address, data: &[u8]) -> Result<()> {
            assert_eq!(addr, vseries::current().addr);
            anyhow::ensure!(self.kept.is_some(), "not kept");
            self.current = data.to_vec();
            self.writes.push(data.to_vec());
            Ok(())
        }
        fn pending(&self) -> usize {
            usize::from(self.kept.is_some())
        }
        fn restore(&mut self) -> Result<Restored> {
            if let Some(k) = self.kept.take() {
                self.current = k;
            }
            Ok(Restored {
                written: 1,
                verified: 1,
                failed: Vec::new(),
            })
        }
    }

    #[test]
    fn switching_keeps_the_first_kit_and_restores_it() {
        let mut v = Fake::at_kit(1);
        assert!(switch_to(&mut v, 12).unwrap());
        assert_eq!(v.kit(), 12);
        // KitNum 11 は 4 ビットずつ 00 00 00 0B
        assert_eq!(v.writes[0], vec![0, 0, 0, 0x0B]);
        // 続けてほかのキットにしても、戻す先は最初のキット
        assert!(switch_to(&mut v, 200).unwrap());
        assert_eq!(v.kit(), 200);
        assert_eq!(v.writes[1], vec![0, 0, 0x0C, 0x07]);
        assert_eq!(restore(&mut v).unwrap(), (Some(1), true));
        assert_eq!(v.kit(), 1);
        assert_eq!(v.pending(), 0);
    }

    #[test]
    fn the_same_kit_is_not_written() {
        let mut v = Fake::at_kit(5);
        assert!(!switch_to(&mut v, 5).unwrap());
        assert!(v.writes.is_empty());
        assert_eq!(v.pending(), 0);
        // 戻すものが無くても今のキットは読む
        assert_eq!(restore(&mut v).unwrap(), (Some(5), false));
    }

    #[test]
    fn notes_sent_and_soon_returned_are_echoes() {
        // ほかのテストと取り合わないように、ここでしか使わないノート番号にする
        let t = 1_000_000_000;
        remember_sent(9, 120, t);
        remember_sent(9, 121, t);
        // 返ってきたら 1 回だけ消す
        assert!(is_echo(&[0x99, 120, 100], t + 3_000));
        assert!(!is_echo(&[0x99, 120, 100], t + 4_000));
        // チャンネルが違う、ノートオフ、遅すぎるものは返りではない
        assert!(!is_echo(&[0x98, 121, 100], t + 3_000));
        assert!(!is_echo(&[0x99, 121, 0], t + 3_000));
        assert!(!is_echo(&[0x99, 121, 100], t + ECHO_WINDOW_US + 1));
        assert!(!is_echo(&[0xB9, 4, 127], t));
    }

    #[test]
    fn names_are_kept_per_model() {
        assert_eq!(names_file(vseries::V51), "v51_kits.toml");
        assert_eq!(names_file(vseries::V71), "v71_kits.toml");
    }

    #[test]
    fn the_names_file_round_trips() {
        let f = NamesFile {
            names: vec!["Studio A".into(), String::new(), "Kit \"3\"".into()],
        };
        let text = toml::to_string(&f).unwrap();
        let back: NamesFile = toml::from_str(&text).unwrap();
        assert_eq!(back.names, f.names);
    }
}
