//! MIDI 入力。見つかった入力ポートをすべて開き、受信したメッセージを時刻付きで渡す。

use std::sync::mpsc::{Receiver, Sender, channel};

use anyhow::Result;
use midir::{MidiInput, MidiInputConnection};

use crate::time::host_us;

/// 受信した 1 メッセージ
#[derive(Debug, Clone)]
pub struct RawMidi {
    pub port: String,
    /// デバイス（またはドライバ）の時刻。ポートごとに起点が違う
    pub device_us: i64,
    /// アプリに届いたホスト時刻
    pub arrival_us: i64,
    pub bytes: Vec<u8>,
}

pub struct MidiIn {
    _connections: Vec<MidiInputConnection<()>>,
    pub ports: Vec<String>,
    rx: Receiver<RawMidi>,
}

impl MidiIn {
    /// すべての入力ポートを開く。ポートが 1 つも無くてもエラーにはしない
    pub fn open_all() -> Result<Self> {
        let (tx, rx) = channel::<RawMidi>();
        let probe = MidiInput::new("DTXmaniaNXT")?;
        let ports = probe.ports();
        let mut connections = Vec::new();
        let mut names = Vec::new();
        for port in &ports {
            let name = probe.port_name(port).unwrap_or_else(|_| "MIDI".into());
            match open_port(port, &name, tx.clone()) {
                Ok(c) => {
                    log::info!("MIDI in: {name}");
                    connections.push(c);
                    names.push(name);
                }
                Err(e) => log::warn!("MIDI in {name}: {e}"),
            }
        }
        if names.is_empty() {
            log::info!("no MIDI input ports");
        }
        Ok(Self {
            _connections: connections,
            ports: names,
            rx,
        })
    }

    /// 届いたメッセージをすべて取り出す
    pub fn drain(&self) -> impl Iterator<Item = RawMidi> + '_ {
        self.rx.try_iter()
    }
}

fn open_port(
    port: &midir::MidiInputPort,
    name: &str,
    tx: Sender<RawMidi>,
) -> Result<MidiInputConnection<()>> {
    let mut input = MidiInput::new("DTXmaniaNXT")?;
    // クロックやアクティブセンシングは要らない
    input.ignore(midir::Ignore::All);
    let port_name = name.to_owned();
    let conn = input
        .connect(
            port,
            "dtxmanianxt-in",
            move |stamp_us, bytes, _| {
                let _ = tx.send(RawMidi {
                    port: port_name.clone(),
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
