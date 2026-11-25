//! 電子ドラムの音源（Roland V51）と MIDI でやり取りする。
//!
//! - V51 を探す（Identity Request に V51 として返事をした出力ポート）
//! - 塊ごとに値を読む（RQ1 → DT1）
//! - 値を書く（DT1）。書く前に元の値を書き戻しの記録（設定のフォルダの `v51_restore.toml`）に残し、
//!   残していない所には書かない
//! - 書き戻して読み直す。アプリが途中で落ちたら、次に起動したときに戻す（[`restore_on_start`]）
//!
//! 返事の SysEx は [`crate::midi::subscribe_sysex`] で受ける。ゲームの入力が V51 のポートを
//! 開いていなければ、ここで自分でも開く。

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use drum_module::journal::{Entry, Journal};
use drum_module::roland::{self, Address, BROADCAST, Collector, Identity, SysexAssembler};
use drum_module::v51::{self, Block};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::config::Config;
use crate::midi::{self, SysexIn};
use crate::time::host_us;

/// 同時に 2 つが V51 とやり取りしないように（起動したときの書き戻しと測る道具など）
static SESSION: Mutex<()> = Mutex::new(());

/// RQ1 の返事を待つ時間
const REPLY_WAIT: Duration = Duration::from_millis(800);
/// Identity Request の返事を待つ時間（出力ポートごと）
const IDENTITY_WAIT: Duration = Duration::from_millis(500);
/// 書き戻すときの DT1 の間（大きいデータを分けて送るときの決まりに合わせる）
const RESTORE_GAP: Duration = Duration::from_millis(20);

/// 書き戻しの記録のファイル
pub fn journal_path() -> Option<PathBuf> {
    Some(Config::dir()?.join("v51_restore.toml"))
}

/// 名前から V51 のポートらしいか
pub fn looks_like_v51(name: &str) -> bool {
    name.to_ascii_lowercase().contains("v51")
}

/// 書き戻した結果
#[derive(Debug, Clone, Default)]
pub struct Restored {
    /// 書き戻した所の数
    pub written: usize,
    /// 読み直して元の値と合った所の数
    pub verified: usize,
    /// 合わなかった所（記録に残したまま。次に起動したときにもう一度戻す）
    pub failed: Vec<String>,
}

/// 見つけた V51 とのつながり
pub struct V51 {
    out: MidiOutputConnection,
    rx: Receiver<SysexIn>,
    _inputs: Vec<MidiInputConnection<()>>,
    _session: MutexGuard<'static, ()>,
    /// 送っている MIDI 出力ポートの名前
    pub port: String,
    pub identity: Identity,
    journal: Journal,
    /// RQ1 を送ってから返事がそろうまで（µs）
    pub round_trips: Vec<i64>,
    /// DT1 で書いた回数（書き戻しを除く）
    pub writes: usize,
}

impl V51 {
    /// V51 を探す。`note` には見つからなかったわけなどを 1 行ずつ渡す
    pub fn find(note: &mut dyn FnMut(String)) -> Result<Self> {
        let session = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let rx = midi::subscribe_sysex();
        let inputs = open_inputs(note);
        let probe = MidiOutput::new("DTXmaniaNXT")?;
        let mut ports: Vec<(midir::MidiOutputPort, String)> = probe
            .ports()
            .into_iter()
            .map(|p| {
                let name = probe
                    .port_name(&p)
                    .map(|n| midi::display_name(&n))
                    .unwrap_or_default();
                (p, name)
            })
            .collect();
        if ports.is_empty() {
            bail!("no MIDI output ports");
        }
        // V51 の名前のポートから試す
        ports.sort_by_key(|(_, n)| !looks_like_v51(n));
        for (port, name) in &ports {
            let out = MidiOutput::new("DTXmaniaNXT")?;
            let mut conn = match out.connect(port, "dtxmanianxt-v51") {
                Ok(c) => c,
                Err(e) => {
                    note(format!("{name}: {e}"));
                    continue;
                }
            };
            drain(&rx);
            if let Err(e) = conn.send(&roland::identity_request(BROADCAST)) {
                note(format!("{name}: {e}"));
                continue;
            }
            let deadline = Instant::now() + IDENTITY_WAIT;
            while let Some(m) = recv_until(&rx, deadline) {
                let Some(id) = roland::parse_identity_reply(&m.bytes) else {
                    continue;
                };
                if !v51::is_v51(&id) {
                    note(format!("{name}: {}", roland::hex(&m.bytes)));
                    continue;
                }
                log::info!(
                    "V51: out {name}, reply from {}, device {:02X}",
                    m.port,
                    id.device
                );
                return Ok(Self {
                    out: conn,
                    rx,
                    _inputs: inputs,
                    _session: session,
                    port: name.clone(),
                    identity: id,
                    journal: load_journal(),
                    round_trips: Vec::new(),
                    writes: 0,
                });
            }
        }
        let names: Vec<&str> = ports.iter().map(|(_, n)| n.as_str()).collect();
        bail!("V51 did not answer (MIDI out: {})", names.join(", "))
    }

    fn device(&self) -> u8 {
        self.identity.device
    }

    /// MIDI のメッセージをそのまま送る
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.send(bytes).map_err(|e| anyhow!("{e}"))
    }

    /// ノートオン（`channel` は 0〜15）
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        self.send(&[0x90 | (channel & 0x0F), note & 0x7F, velocity.clamp(1, 127)])
    }

    /// 塊を読む
    pub fn read(&mut self, block: Block) -> Result<Vec<u8>> {
        for _ in 0..3 {
            drain(&self.rx);
            let sent = host_us();
            let msg = roland::rq1(self.device(), &v51::MODEL_ID, block.addr, block.size);
            self.send(&msg)?;
            let mut c = Collector::new(block.addr, block.size);
            let deadline = Instant::now() + REPLY_WAIT;
            while let Some(m) = recv_until(&self.rx, deadline) {
                if let Some(set) = roland::parse_dt1(&m.bytes, &v51::MODEL_ID)
                    && set.device == self.device()
                    && c.feed(&set)
                {
                    self.round_trips.push(host_us() - sent);
                    return Ok(c.into_bytes());
                }
            }
        }
        bail!(
            "V51 did not answer RQ1 {} ({} bytes)",
            block.addr.hex(),
            block.size
        )
    }

    /// 書き戻しの記録に残っている所の数
    pub fn pending(&self) -> usize {
        self.journal.entries.len()
    }

    /// `block`（読んだ値は `data`）の `offset` から `len` バイトを書き換える前に、元の値を記録に残して
    /// ファイルに書き出す。同じ所を前に残してあれば、そちらのまま
    pub fn remember(
        &mut self,
        block: Block,
        data: &[u8],
        offset: u32,
        len: usize,
        reason: &str,
    ) -> Result<()> {
        let original = data
            .get(offset as usize..offset as usize + len)
            .ok_or_else(|| anyhow!("{} +{offset:X} is outside the block", block.addr.hex()))?;
        let entry = Entry::new(
            &self.port,
            self.device(),
            block.at(offset),
            original,
            reason,
            &chrono::Local::now().to_rfc3339(),
        )
        .in_block(block.addr, block.size);
        if self.journal.remember(entry) {
            save_journal(&self.journal)?;
        }
        Ok(())
    }

    /// 値を書く。元の値を記録に残していない所には書かない
    pub fn write(&mut self, addr: Address, data: &[u8]) -> Result<()> {
        if !self.journal.covers(self.device(), addr, data.len()) {
            bail!("not writing {} before keeping the original", addr.hex());
        }
        let msg = roland::dt1(self.device(), &v51::MODEL_ID, addr, data);
        self.send(&msg)?;
        self.writes += 1;
        Ok(())
    }

    /// 記録から書き戻し、読み直して確かめる。合ったものは記録から消す
    pub fn restore(&mut self) -> Result<Restored> {
        let mut out = Restored::default();
        let plan: Vec<(Entry, Address, Vec<u8>)> = self
            .journal
            .restore_plan()
            .into_iter()
            .map(|(e, a, b)| (e.clone(), a, b))
            .collect();
        if plan.is_empty() {
            return Ok(out);
        }
        for (e, addr, bytes) in &plan {
            let msg = roland::dt1(e.device, &v51::MODEL_ID, *addr, bytes);
            self.send(&msg)?;
            out.written += 1;
            std::thread::sleep(RESTORE_GAP);
        }
        // 書き戻したあとにあるべき値（同じ所が重なっていれば、一番古い記録の値）
        let mut expected: Vec<(u32, u8)> = Vec::new();
        for (_, addr, bytes) in plan.iter().rev() {
            for (i, &b) in bytes.iter().enumerate() {
                let at = addr.to_index() + i as u32;
                if !expected.iter().any(|&(x, _)| x == at) {
                    expected.push((at, b));
                }
            }
        }
        let mut blocks: Vec<(Address, Vec<u8>)> = Vec::new();
        let mut keep = Journal::default();
        for e in &self.journal.entries.clone() {
            let ok = match e.block() {
                // 確かめる塊が分からないものは、書き戻したところで終わりにする
                None => true,
                Some((baddr, bsize)) => {
                    let data = match blocks.iter().find(|(a, _)| *a == baddr) {
                        Some((_, d)) => Some(d.clone()),
                        None => match self.read(Block {
                            addr: baddr,
                            size: bsize,
                        }) {
                            Ok(d) => {
                                blocks.push((baddr, d.clone()));
                                Some(d)
                            }
                            Err(err) => {
                                log::warn!("V51 restore check: {err:#}");
                                None
                            }
                        },
                    };
                    match (data, e.addr(), e.original_bytes()) {
                        (Some(d), Some(a), Some(b)) => (0..b.len()).all(|i| {
                            let at = a.to_index() + i as u32;
                            let want = expected.iter().find(|&&(x, _)| x == at).map(|&(_, v)| v);
                            let pos = (at - baddr.to_index()) as usize;
                            d.get(pos).copied() == want
                        }),
                        _ => false,
                    }
                }
            };
            if ok {
                out.verified += 1;
            } else {
                out.failed
                    .push(format!("{} ({}) = {}", e.address, e.reason, e.original));
                keep.entries.push(e.clone());
            }
        }
        self.journal = keep;
        save_journal(&self.journal)?;
        Ok(out)
    }
}

/// 記録が残っていれば V51 を探して書き戻す。記録が無ければ `None`
pub fn restore_pending(note: &mut dyn FnMut(String)) -> Result<Option<Restored>> {
    if load_journal().is_empty() {
        return Ok(None);
    }
    let mut link = V51::find(note)?;
    Ok(Some(link.restore()?))
}

/// 起動したとき、前に V51 を書き換えたまま終わっていたら（落ちたなど）書き戻す。
/// ゲームの MIDI 入力が開くのを少し待ってから、別のスレッドで行う
pub fn restore_on_start() {
    if !journal_path().is_some_and(|p| p.exists()) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("v51-restore".into())
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(3));
            let mut notes = Vec::new();
            match restore_pending(&mut |s| notes.push(s)) {
                Ok(Some(r)) if r.failed.is_empty() => {
                    log::info!("V51: restored {} values left from last time", r.verified);
                }
                Ok(Some(r)) => log::warn!("V51: could not restore {}", r.failed.join("; ")),
                Ok(None) => {}
                Err(e) => log::warn!(
                    "V51: values left from last time are not restored yet ({e:#}) {}",
                    notes.join("; ")
                ),
            }
        });
    if let Err(e) = spawned {
        log::warn!("V51 restore thread: {e}");
    }
}

/// 書き戻しの記録を読む。読めないファイルは横に除けて（`.broken`）、空として扱う
fn load_journal() -> Journal {
    let Some(path) = journal_path() else {
        return Journal::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Journal::default();
    };
    match Journal::from_toml(&text) {
        Ok(j) => {
            if j.broken() > 0 {
                log::warn!("{}: {} broken entries", path.display(), j.broken());
            }
            j
        }
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            let _ = fs::rename(&path, path.with_extension("toml.broken"));
            Journal::default()
        }
    }
}

/// 書き戻しの記録を書き出す（空ならファイルを消す）。途中で落ちても壊れないよう、別のファイルに書いてから置き換える
fn save_journal(journal: &Journal) -> Result<()> {
    let path = journal_path().ok_or_else(|| anyhow!("no settings folder"))?;
    if journal.is_empty() {
        if path.exists() {
            fs::remove_file(&path).with_context(|| path.display().to_string())?;
        }
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = fs::File::create(&tmp).with_context(|| tmp.display().to_string())?;
        f.write_all(
            "# DTXmaniaNXT が V51 の値を書き換える前の値です。次に起動したときに V51 へ書き戻します。\n\
             # Values the V51 had before DTXmaniaNXT changed them. They are written back on the next start.\n\n"
                .as_bytes(),
        )?;
        f.write_all(journal.to_toml().as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path).with_context(|| path.display().to_string())?;
    Ok(())
}

/// 返事を受ける入力を開く。ゲームの入力（[`crate::midi::MidiIn`]）が開いていない V51 のポートだけ。
/// V51 の名前のポートが無ければ、開いていないポートをすべて開く
fn open_inputs(note: &mut dyn FnMut(String)) -> Vec<MidiInputConnection<()>> {
    let Ok(probe) = MidiInput::new("DTXmaniaNXT") else {
        return Vec::new();
    };
    let tapped = midi::tapped_ports();
    let ports: Vec<(String, midir::MidiInputPort)> = probe
        .ports()
        .into_iter()
        .map(|p| {
            let name = probe
                .port_name(&p)
                .map(|n| midi::display_name(&n))
                .unwrap_or_default();
            (name, p)
        })
        .collect();
    let any_v51 = ports.iter().any(|(n, _)| looks_like_v51(n));
    let mut out = Vec::new();
    for (name, port) in ports {
        if (any_v51 && !looks_like_v51(&name)) || tapped.contains(&name) {
            continue;
        }
        match open_input(&port, name.clone()) {
            Ok(c) => out.push(c),
            Err(e) => note(format!("{name}: {e:#}")),
        }
    }
    out
}

fn open_input(port: &midir::MidiInputPort, name: String) -> Result<MidiInputConnection<()>> {
    let mut input = MidiInput::new("DTXmaniaNXT")?;
    input.ignore(midir::Ignore::TimeAndActiveSense);
    let mut sysex = SysexAssembler::default();
    input
        .connect(
            port,
            "dtxmanianxt-v51-in",
            move |_, bytes, _| {
                for m in sysex.feed(bytes) {
                    midi::publish_sysex(&name, m);
                }
            },
            (),
        )
        .map_err(|e| anyhow!("{e}"))
}

fn drain(rx: &Receiver<SysexIn>) {
    while rx.try_recv().is_ok() {}
}

/// `deadline` まで 1 つ待つ
fn recv_until(rx: &Receiver<SysexIn>, deadline: Instant) -> Option<SysexIn> {
    let left = deadline.checked_duration_since(Instant::now())?;
    rx.recv_timeout(left).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v51_port_names() {
        assert!(looks_like_v51("V51"));
        assert!(looks_like_v51("MIDIIN2 (V51)"));
        assert!(looks_like_v51("2- v51"));
        assert!(!looks_like_v51("Finger Drum Pad"));
    }
}
