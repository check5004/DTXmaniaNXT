//! MIDI 入力。OS が見せる入力ポートを並べ、使うものを開いて、受信したメッセージを時刻付きで渡す。
//!
//! ポート（機器）は名前と OS の ID で見分ける。ID は OS ごとに中身が違う。
//! - Windows（WinMM）: デバイスのパス。USB の VID・PID と、機器のシリアル番号（無ければ挿した USB の口）を含む
//! - macOS（CoreMIDI）: CoreMIDI がポートに付ける一意の番号。挿し直しても多くは同じ
//! - Linux（ALSA）: `client:port` の番号。挿した順で決まるので、挿し直すと変わることがある

use std::sync::mpsc::{Receiver, Sender, channel};

use anyhow::Result;
use midir::{MidiInput, MidiInputConnection};

use crate::time::host_us;

/// 受信した 1 回分
#[derive(Debug, Clone)]
pub struct RawMidi {
    /// [`MidiIn::ports`] の何番目のポートから来たか
    pub port: usize,
    /// デバイス（またはドライバ）の時刻。ポートごとに起点が違う
    pub device_us: i64,
    /// アプリに届いたホスト時刻
    pub arrival_us: i64,
    /// 届いたバイト列（OS によっては複数のメッセージが入っている）
    pub bytes: Vec<u8>,
}

/// OS が見せる入力ポート 1 つ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// OS が出す名前（Linux は末尾の `client:port` の番号を除いたもの）
    pub name: String,
    /// OS のポートの ID（取れなければ空）
    pub id: String,
    /// 開いているか
    pub open: bool,
    /// 開けなかったわけ（ほかのアプリが使っているなど）。使わない設定にしたポートは `None` で `open` も false
    pub error: Option<String>,
}

pub struct MidiIn {
    _connections: Vec<MidiInputConnection<()>>,
    pub ports: Vec<PortInfo>,
    rx: Receiver<RawMidi>,
}

impl MidiIn {
    /// 入力ポートを並べ、`decide`（ポートの名前と ID の並び → 開くか）が true のものを開く。
    /// ポートが 1 つも無くてもエラーにはしない
    pub fn open(decide: impl FnOnce(&[(&str, &str)]) -> Vec<bool>) -> Result<Self> {
        let (tx, rx) = channel::<RawMidi>();
        let probe = MidiInput::new("DTXmaniaNXT")?;
        let found: Vec<(midir::MidiInputPort, String, String)> = probe
            .ports()
            .into_iter()
            .map(|port| {
                let name = probe.port_name(&port).unwrap_or_else(|_| "MIDI".into());
                let id = port.id();
                (port, display_name(&name), id)
            })
            .collect();
        let keys: Vec<(&str, &str)> = found
            .iter()
            .map(|(_, n, id)| (n.as_str(), id.as_str()))
            .collect();
        let use_port = decide(&keys);
        let mut connections = Vec::new();
        let mut ports = Vec::new();
        for (index, (port, name, id)) in found.iter().enumerate() {
            let mut info = PortInfo {
                name: name.clone(),
                id: id.clone(),
                open: false,
                error: None,
            };
            if use_port.get(index).copied().unwrap_or(true) {
                match open_port(port, index, tx.clone()) {
                    Ok(c) => {
                        connections.push(c);
                        info.open = true;
                    }
                    Err(e) => info.error = Some(e.to_string()),
                }
            }
            match (&info.error, info.open) {
                (Some(e), _) => log::warn!("MIDI in {} [{}]: {e}", info.name, info.id),
                (None, true) => log::info!("MIDI in: {} [{}]", info.name, info.id),
                (None, false) => log::info!("MIDI in {} [{}]: not used", info.name, info.id),
            }
            ports.push(info);
        }
        if ports.is_empty() {
            log::info!("no MIDI input ports");
        }
        Ok(Self {
            _connections: connections,
            ports,
            rx,
        })
    }

    /// OS が見せているポートが、開いたときと変わったか（つないだり外したり）
    pub fn changed(&self, now: &[(String, String)]) -> bool {
        now.len() != self.ports.len()
            || now
                .iter()
                .zip(&self.ports)
                .any(|((n, id), p)| *n != p.name || *id != p.id)
    }

    /// 届いたメッセージをすべて取り出す
    pub fn drain(&self) -> impl Iterator<Item = RawMidi> + '_ {
        self.rx.try_iter()
    }

    /// 開いているポートの名前
    pub fn open_names(&self) -> Vec<String> {
        self.ports
            .iter()
            .filter(|p| p.open)
            .map(|p| p.name.clone())
            .collect()
    }

    /// ポートの名前（範囲の外なら "MIDI"）
    pub fn name(&self, port: usize) -> &str {
        self.ports.get(port).map_or("MIDI", |p| p.name.as_str())
    }
}

/// 今 OS が見せている入力ポートの名前と ID（開かずに調べる。つないだり外したりを見つけるのに使う）
pub fn list_ports() -> Vec<(String, String)> {
    let Ok(probe) = MidiInput::new("DTXmaniaNXT") else {
        return Vec::new();
    };
    probe
        .ports()
        .iter()
        .map(|p| {
            let name = probe.port_name(p).unwrap_or_else(|_| "MIDI".into());
            (display_name(&name), p.id())
        })
        .collect()
}

/// 画面とログに出す名前。Linux（ALSA）の名前の末尾に付く `client:port` の番号は、挿すたびに変わるので除く
fn display_name(raw: &str) -> String {
    if cfg!(target_os = "linux")
        && let Some((head, tail)) = raw.rsplit_once(' ')
        && let Some((c, p)) = tail.split_once(':')
        && !c.is_empty()
        && !p.is_empty()
        && c.bytes().chain(p.bytes()).all(|b| b.is_ascii_digit())
    {
        return head.to_owned();
    }
    raw.to_owned()
}

/// 同じ機種の 2 台目以降に OS が付ける印を除いた名前（設定の出発点を選ぶのと、名前で探すのに使う）。
/// Windows は頭に「2- 」（WinMM）か、末尾に「 (2)」（Windows MIDI Services）を付ける
pub fn base_name(name: &str) -> &str {
    let name = name.trim();
    let name = match name.split_once("- ") {
        Some((n, rest))
            if !n.is_empty() && n.len() <= 2 && n.bytes().all(|b| b.is_ascii_digit()) =>
        {
            rest
        }
        _ => name,
    };
    if let Some(head) = name.strip_suffix(')')
        && let Some((head, n)) = head.rsplit_once(" (")
        && !n.is_empty()
        && n.len() <= 2
        && n.bytes().all(|b| b.is_ascii_digit())
        && n != "1"
    {
        return head;
    }
    name
}

fn open_port(
    port: &midir::MidiInputPort,
    index: usize,
    tx: Sender<RawMidi>,
) -> Result<MidiInputConnection<()>> {
    let mut input = MidiInput::new("DTXmaniaNXT")?;
    // クロックやアクティブセンシングは要らない（パッドによってはクロックを 1 秒に何十回も送ってくる）
    input.ignore(midir::Ignore::All);
    let conn = input
        .connect(
            port,
            "dtxmanianxt-in",
            move |stamp_us, bytes, _| {
                let _ = tx.send(RawMidi {
                    port: index,
                    device_us: stamp_us as i64,
                    arrival_us: host_us(),
                    bytes: bytes.to_vec(),
                });
            },
            (),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_name_drops_the_second_unit_marks() {
        assert_eq!(base_name("Finger Drum Pad"), "Finger Drum Pad");
        assert_eq!(base_name("2- Finger Drum Pad"), "Finger Drum Pad");
        assert_eq!(base_name("Finger Drum Pad (2)"), "Finger Drum Pad");
        assert_eq!(base_name("MIDIIN2 (V51)"), "MIDIIN2 (V51)");
        assert_eq!(base_name("TD-27"), "TD-27");
        assert_eq!(base_name("A-B"), "A-B");
        assert_eq!(base_name("Pad (1)"), "Pad (1)");
    }

    #[test]
    fn linux_port_numbers_are_dropped() {
        let n = display_name("Finger Drum Pad:Finger Drum Pad MIDI 1 24:0");
        if cfg!(target_os = "linux") {
            assert_eq!(n, "Finger Drum Pad:Finger Drum Pad MIDI 1");
        } else {
            assert_eq!(n, "Finger Drum Pad:Finger Drum Pad MIDI 1 24:0");
        }
        assert_eq!(display_name("TD-27"), "TD-27");
    }
}
