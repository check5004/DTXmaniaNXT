//! 曲ごとの音源のキット（今は Roland V51 だけ）。
//!
//! - 曲を始めるとき（読み込みの画面）に、その曲に決めたキットへ V51 を切り替える
//! - 選曲に戻るとき・アプリを終えるときに、切り替える前のキットに戻す。書き戻しの記録
//!   （[`crate::module_link`]）を使うので、落ちても次に起動したときに戻る
//! - キットを選んでいるあいだは、選んでいるキットに V51 を切り替える（叩いて音を確かめられる）
//! - キットの名前は V51 から読み、設定のフォルダの `v51_kits.toml` に覚えておく
//!
//! V51 とのやり取りはこのスレッド 1 本で順に行い、画面のスレッドは待たない。V51 とのつながりは
//! 用があるときだけ開き、しばらく用が無ければ閉じる（測る道具と取り合わないように）。
//! 測定（2026-11-28、issue #113）: キットを書いてから鳴る音が変わるまで 0.2〜0.5 秒より長い、RQ1 の往復は 72 ms。

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use drum_module::v51;
use midir::MidiOutput;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::midi;
use crate::module_link::{Restored, V51, looks_like_v51};

/// 曲ごとのキットを記録に残すときの音源の名前
pub const MODULE: &str = "roland-v51";
/// キットを書いてから、鳴る音が変わりきるまで待つ時間（測定では 0.5 秒でもまだ小さく鳴った）
pub const SETTLE: Duration = Duration::from_millis(1000);
/// 読み込みの画面で、キットの切り替えを待つ長さの上限（V51 の返事が遅いときなど）
pub const MAX_WAIT: Duration = Duration::from_millis(3000);
/// 選んでいるあいだ、選ぶのが止まってから切り替えるまで（送るたびに切り替えないように）
const AUDITION_DELAY: Duration = Duration::from_millis(350);
/// 用が無くなってから V51 とのつながりを閉じるまで
const LINK_IDLE: Duration = Duration::from_secs(2);
/// 起動してからキットの名前を読み始めるまで（起動したときの書き戻しのあと）
const NAMES_DELAY: Duration = Duration::from_secs(5);
/// 終えるとき、前のキットに戻すのを待つ長さ
const QUIT_WAIT: Duration = Duration::from_millis(2500);
/// 書き戻しの記録に残すわけ
const REASON: &str = "kit for a song";

/// 今の様子（画面のスレッドが読む）
#[derive(Debug, Clone, Default)]
pub struct State {
    /// 最後に探したとき V51 が見つかったか（まだ探していなければ None）
    pub found: Option<bool>,
    /// キットの名前（キット 1〜200 の順。読めていなければ空の文字）
    pub names: Vec<String>,
    /// 名前を読んでいるところなら、読み終えた数
    pub reading: Option<u16>,
    /// V51 が今鳴らしているキット（このアプリが知っている範囲で）
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

    /// 曲ごとのキットを選べる（V51 が見つかったか、前に見つけて名前を覚えている）
    pub fn available(&self) -> bool {
        self.found == Some(true) || self.names.iter().any(|n| !n.is_empty())
    }
}

enum Cmd {
    /// 曲のためにキットを切り替える（None なら前のキットに戻す）。すぐに
    Apply(Option<u16>),
    /// 選んでいるキットを鳴らす（None なら前のキットに戻す）。選ぶのが止まってから
    Audition(Option<u16>),
    /// キットの名前を読む
    ReadNames,
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
    /// スレッドを始める。V51 らしい MIDI のポートがあれば、少しあとでキットの名前を読む
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(State {
            names: load_names(),
            ..State::default()
        }));
        let (tx, rx) = channel();
        let (done_tx, done) = channel();
        let names_at = v51_port_present().then(|| Instant::now() + NAMES_DELAY);
        let shared = state.clone();
        let worker = std::thread::Builder::new()
            .name("v51-kits".into())
            .spawn(move || {
                Worker::new(shared, names_at).run(&rx);
                let _ = done_tx.send(());
            })
            .map_err(|e| log::warn!("V51 kits thread: {e}"))
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

    /// 選んでいるキット `kit` を鳴らす（None なら前のキットに戻す）
    pub fn audition(&self, kit: Option<u16>) {
        self.state().busy = true;
        self.send(Cmd::Audition(kit));
    }

    /// V51 がまだ見つかっていないか、名前を読み終えていなければ、V51 らしいポートがあるときだけ読み直す
    /// （起動したあとで V51 をつないだとき。簡易設定を開いたときに呼ぶ）
    pub fn check(&self) {
        {
            let st = self.state();
            let complete = st.found == Some(true) && st.names.len() == usize::from(v51::KIT_COUNT);
            if complete || st.reading.is_some() {
                return;
            }
        }
        if v51_port_present() {
            self.send(Cmd::ReadNames);
        }
    }

    /// 演奏しているあいだは V51 に名前を聞かない（変わったときだけ送る）
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
}

struct Worker {
    state: Arc<Mutex<State>>,
    link: Option<V51>,
    /// 最後に V51 とやり取りした時刻
    used: Instant,
    want: Option<Want>,
    /// これから名前を読むキット
    names: VecDeque<u16>,
    /// 名前を読み始める時刻（起動したとき）
    names_at: Option<Instant>,
    quiet: bool,
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
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn run(mut self, rx: &Receiver<Cmd>) {
        loop {
            match rx.recv_timeout(self.wait()) {
                Ok(cmd) => {
                    self.take(cmd);
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => self.step(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // 終えるときは、選んでいたキットも曲のキットも前のキットに戻す
        if self.state().switched {
            self.switch(None);
        }
    }

    fn take(&mut self, cmd: Cmd) {
        let now = Instant::now();
        match cmd {
            Cmd::Apply(kit) => self.want = Some(Want { kit, at: now }),
            Cmd::Audition(kit) => {
                self.want = Some(Want {
                    kit,
                    at: now + AUDITION_DELAY,
                });
            }
            Cmd::ReadNames => self.queue_names(),
            Cmd::Quiet(on) => self.quiet = on,
        }
    }

    /// 次に何かするまで待つ長さ
    fn wait(&self) -> Duration {
        let now = Instant::now();
        let until = |t: Instant| t.saturating_duration_since(now);
        if let Some(w) = self.want {
            return until(w.at);
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
            self.switch(w.kit);
            return;
        }
        if self.want.is_some() {
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
        self.names = (1..=v51::KIT_COUNT).collect();
        self.state().reading = Some(0);
    }

    /// V51 とのつながり（無ければ探す）。見つからなければ None
    fn link(&mut self) -> Option<&mut V51> {
        if self.link.is_none() {
            let mut notes = Vec::new();
            match V51::find(&mut |s| notes.push(s)) {
                Ok(l) => {
                    self.state().found = Some(true);
                    self.link = Some(l);
                }
                Err(e) => {
                    let mut st = self.state();
                    if st.found != Some(false) {
                        log::info!("V51 kits: {e:#} {}", notes.join("; "));
                    }
                    st.found = Some(false);
                    return None;
                }
            }
        }
        self.used = Instant::now();
        self.link.as_mut()
    }

    /// やり取りに失敗した。つながりを閉じ、名前を読むのもやめる
    fn lost(&mut self, e: &anyhow::Error) {
        log::warn!("V51 kits: {e:#}");
        self.link = None;
        self.names.clear();
        let mut st = self.state();
        st.reading = None;
        st.busy = false;
    }

    /// キット `kit` にする（None なら前のキットに戻す）
    fn switch(&mut self, kit: Option<u16>) {
        let Some(link) = self.link() else {
            self.state().busy = false;
            return;
        };
        let result = match kit {
            Some(kit) => switch_to(link, kit).map(|sent| (Some(kit), sent, true)),
            None => restore(link).map(|now| (now, false, false)),
        };
        let pending = link.pending() > 0;
        match result {
            Ok((now, sent, switched)) => {
                let mut st = self.state();
                st.now = now;
                if sent {
                    st.switched_at = Some(Instant::now());
                }
                st.switched = pending && switched;
                // 次の頼みが来ていれば、それが終わるまで busy のまま
                st.busy = self.want.is_some();
            }
            Err(e) => self.lost(&e),
        }
    }

    fn read_name(&mut self, kit: u16) {
        let Some(link) = self.link() else {
            self.names.clear();
            self.state().reading = None;
            return;
        };
        match link.read(v51::kit_common(kit)) {
            Ok(data) => {
                let name = v51::kit_name(&data).unwrap_or_default();
                let done = self.names.is_empty();
                let mut st = self.state();
                let i = usize::from(kit - 1);
                if st.names.len() < usize::from(v51::KIT_COUNT) {
                    st.names.resize(usize::from(v51::KIT_COUNT), String::new());
                }
                st.names[i] = name;
                st.reading = (!done).then_some(kit);
                if done {
                    let names = st.names.clone();
                    drop(st);
                    if let Err(e) = save_names(&names) {
                        log::warn!("V51 kit names: {e:#}");
                    }
                    log::info!("V51: read {} kit names", names.len());
                }
            }
            Err(e) => self.lost(&e),
        }
    }
}

/// キットを切り替えるのに使う、V51 とのやり取り（テストでは作りものの V51 にする）
trait Link {
    fn read(&mut self, block: v51::Block) -> Result<Vec<u8>>;
    fn remember(
        &mut self,
        block: v51::Block,
        data: &[u8],
        offset: u32,
        len: usize,
        reason: &str,
    ) -> Result<()>;
    fn write(&mut self, addr: drum_module::roland::Address, data: &[u8]) -> Result<()>;
    fn pending(&self) -> usize;
    fn restore(&mut self) -> Result<Restored>;
}

impl Link for V51 {
    fn read(&mut self, block: v51::Block) -> Result<Vec<u8>> {
        V51::read(self, block)
    }
    fn remember(
        &mut self,
        block: v51::Block,
        data: &[u8],
        offset: u32,
        len: usize,
        reason: &str,
    ) -> Result<()> {
        V51::remember(self, block, data, offset, len, reason)
    }
    fn write(&mut self, addr: drum_module::roland::Address, data: &[u8]) -> Result<()> {
        V51::write(self, addr, data)
    }
    fn pending(&self) -> usize {
        V51::pending(self)
    }
    fn restore(&mut self) -> Result<Restored> {
        V51::restore(self)
    }
}

/// キットを `kit` にする。前の値を書き戻しの記録に残してから書く。書いたら true（もうそのキットなら false）
fn switch_to(link: &mut impl Link, kit: u16) -> Result<bool> {
    let kit = kit.clamp(1, v51::KIT_COUNT);
    let block = v51::current();
    let data = link.read(block)?;
    let now = v51::KIT_NUM
        .read(&data)
        .ok_or_else(|| anyhow!("no kit number in {:?}", data))?;
    if now + 1 == i32::from(kit) {
        return Ok(false);
    }
    link.remember(block, &data, v51::KIT_NUM.offset, v51::KIT_NUM.len, REASON)?;
    let value = v51::KIT_NUM.encode(i32::from(kit) - 1);
    link.write(block.at(v51::KIT_NUM.offset), &value)?;
    log::info!("V51: kit {} -> {kit}", now + 1);
    Ok(true)
}

/// 書き戻しの記録から前のキットに戻し、今のキットを読み直す
fn restore(link: &mut impl Link) -> Result<Option<u16>> {
    if link.pending() > 0 {
        let r = link.restore()?;
        if !r.failed.is_empty() {
            log::warn!("V51: could not restore {}", r.failed.join("; "));
        }
    }
    let data = link.read(v51::current())?;
    Ok(v51::KIT_NUM
        .read(&data)
        .and_then(|n| u16::try_from(n + 1).ok()))
}

/// V51 の名前の MIDI 出力ポートがあるか（無ければ、起動したときに V51 を探さない）
fn v51_port_present() -> bool {
    let outs = MidiOutput::new("DTXmaniaNXT").map(|o| {
        o.ports()
            .iter()
            .filter_map(|p| o.port_name(p).ok())
            .any(|n| looks_like_v51(&midi::display_name(&n)))
    });
    outs.unwrap_or(false)
}

/// 覚えておくキットの名前のファイル
fn names_path() -> Option<PathBuf> {
    Some(Config::dir()?.join("v51_kits.toml"))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NamesFile {
    #[serde(default)]
    names: Vec<String>,
}

fn load_names() -> Vec<String> {
    let Some(path) = names_path() else {
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

fn save_names(names: &[String]) -> Result<()> {
    let path = names_path().ok_or_else(|| anyhow!("no settings folder"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = toml::to_string(&NamesFile {
        names: names.to_vec(),
    })?;
    let text = format!(
        "# DTXmaniaNXT が V51 から読んだキットの名前です（キット 1 から順に）。\n\
         # Kit names read from the V51 (from kit 1).\n\n{body}"
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
        assert!(st.available());
        assert!(!State::default().available());
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

    /// 作りものの V51（Current の 4 バイトと、書き戻しの記録だけ）
    struct Fake {
        current: Vec<u8>,
        /// 書き戻しの記録（番地の頭と元の値）
        kept: Option<Vec<u8>>,
        writes: Vec<Vec<u8>>,
    }

    impl Fake {
        fn at_kit(kit: u16) -> Self {
            Self {
                current: v51::KIT_NUM.encode(i32::from(kit) - 1),
                kept: None,
                writes: Vec::new(),
            }
        }
        fn kit(&self) -> i32 {
            v51::KIT_NUM.read(&self.current).unwrap() + 1
        }
    }

    impl Link for Fake {
        fn read(&mut self, block: v51::Block) -> Result<Vec<u8>> {
            assert_eq!(block, v51::current());
            Ok(self.current.clone())
        }
        fn remember(
            &mut self,
            block: v51::Block,
            data: &[u8],
            offset: u32,
            len: usize,
            _reason: &str,
        ) -> Result<()> {
            assert_eq!(block, v51::current());
            if self.kept.is_none() {
                self.kept = Some(data[offset as usize..offset as usize + len].to_vec());
            }
            Ok(())
        }
        fn write(&mut self, addr: drum_module::roland::Address, data: &[u8]) -> Result<()> {
            assert_eq!(addr, v51::current().addr);
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
        assert_eq!(restore(&mut v).unwrap(), Some(1));
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
        assert_eq!(restore(&mut v).unwrap(), Some(5));
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
