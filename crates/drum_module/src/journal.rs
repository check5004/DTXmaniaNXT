//! 音源の値を書き換える前の値の記録。
//!
//! V51 は書き換えた値をそのまま本体に保存するので、書く前に元の値を読んでここに残し、
//! ファイルに書き出してから DT1 を送る。用が済んだら新しいものから順に書き戻す。
//! アプリが途中で落ちても、次に起動したときにこの記録から戻せる。
//!
//! ファイルは人が読んでも分かるよう TOML にし、番地と値は `04 00 23 02` の形で書く。

use serde::{Deserialize, Serialize};

use crate::roland::{Address, hex, parse_hex};

/// 書き換え 1 回分
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// 送った MIDI 出力ポートの名前（探す手がかり。挿し直すと変わることがある）
    pub port: String,
    /// 送った先の Device ID
    pub device: u8,
    /// 番地（`04 00 23 02`）
    pub address: String,
    /// 書き換える前の値（`0F 0D 0A 07`）
    pub original: String,
    /// 何のために書き換えたか
    #[serde(default)]
    pub reason: String,
    /// いつ（RFC 3339。入れるのはアプリ）
    #[serde(default)]
    pub at: String,
    /// 書き戻したあとに確かめに読む塊の番地（空なら確かめない）と大きさ
    #[serde(default)]
    pub block: String,
    #[serde(default)]
    pub block_size: u32,
}

impl Entry {
    pub fn new(
        port: &str,
        device: u8,
        addr: Address,
        original: &[u8],
        reason: &str,
        at: &str,
    ) -> Self {
        Self {
            port: port.to_owned(),
            device,
            address: addr.hex(),
            original: hex(original),
            reason: reason.to_owned(),
            at: at.to_owned(),
            block: String::new(),
            block_size: 0,
        }
    }

    /// 書き戻したあとに確かめに読む塊（値を含むもの）を添える
    pub fn in_block(mut self, addr: Address, size: u32) -> Self {
        self.block = addr.hex();
        self.block_size = size;
        self
    }

    /// 確かめに読む塊。値がその中に入っていなければ `None`
    pub fn block(&self) -> Option<(Address, u32)> {
        let b: [u8; 4] = parse_hex(&self.block)?.try_into().ok()?;
        let addr = Address(b);
        let (start, end) = self.range()?;
        let b0 = addr.to_index();
        (b.iter().all(|&x| x < 0x80) && b0 <= start && end <= b0 + self.block_size)
            .then_some((addr, self.block_size))
    }

    pub fn addr(&self) -> Option<Address> {
        let b = parse_hex(&self.address)?;
        let b: [u8; 4] = b.try_into().ok()?;
        b.iter().all(|&x| x < 0x80).then_some(Address(b))
    }

    pub fn original_bytes(&self) -> Option<Vec<u8>> {
        parse_hex(&self.original).filter(|b| !b.is_empty() && b.iter().all(|&x| x < 0x80))
    }

    /// 番地の範囲（[`Address::to_index`] で、終わりは含まない）
    fn range(&self) -> Option<(u32, u32)> {
        let start = self.addr()?.to_index();
        let len = self.original_bytes()?.len() as u32;
        Some((start, start + len))
    }
}

/// 書き戻しの記録
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default, rename = "entry")]
    pub entries: Vec<Entry>,
}

impl Journal {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `device` の `addr` から `len` バイトが、どれも記録のどれかに入っているか
    /// （元の値を残さずに書くのを防ぐ）
    pub fn covers(&self, device: u8, addr: Address, len: usize) -> bool {
        let start = addr.to_index();
        (start..start + len as u32).all(|i| {
            self.entries
                .iter()
                .any(|e| e.device == device && e.range().is_some_and(|(s, end)| s <= i && i < end))
        })
    }

    /// 書き換える前の値を残す。同じ所を前に残してあれば、そちら（本当の元の値）を残したままにする。
    /// 足したら true
    pub fn remember(&mut self, entry: Entry) -> bool {
        let Some(addr) = entry.addr() else {
            return false;
        };
        let Some(len) = entry.original_bytes().map(|b| b.len()) else {
            return false;
        };
        if self.covers(entry.device, addr, len) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// 書き戻す順（新しいものから。同じ所が重なっていても、最後に一番古い元の値が残る）
    pub fn restore_plan(&self) -> Vec<(&Entry, Address, Vec<u8>)> {
        self.entries
            .iter()
            .rev()
            .filter_map(|e| Some((e, e.addr()?, e.original_bytes()?)))
            .collect()
    }

    /// 読めない行（番地や値の形がおかしいもの）の数
    pub fn broken(&self) -> usize {
        self.entries.iter().filter(|e| e.range().is_none()).count()
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn from_toml(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(addr: Address, original: &[u8]) -> Entry {
        Entry::new(
            "V51",
            0x10,
            addr,
            original,
            "TOM1 HEAD の音量",
            "2026-11-25T00:00:00Z",
        )
    }

    #[test]
    fn keeps_the_first_original() {
        let vol = Address::new(0x04, 0, 0x23, 0x02);
        let mut j = Journal::default();
        assert!(!j.covers(0x10, vol, 4));
        assert!(j.remember(entry(vol, &[0, 0, 0, 0])));
        assert!(j.covers(0x10, vol, 4));
        assert!(j.covers(0x10, vol.offset(1), 2));
        assert!(!j.covers(0x10, vol, 5));
        assert!(!j.covers(0x11, vol, 4));
        // -INF にしたあとでもう一度残そうとしても、最初の値のまま
        assert!(!j.remember(entry(vol, &[0x0F, 0x0D, 0x0A, 0x07])));
        assert_eq!(j.entries.len(), 1);
        assert_eq!(j.entries[0].original, "00 00 00 00");
    }

    #[test]
    fn knows_the_block_to_read_back() {
        let block = Address::new(0x04, 0, 0x23, 0);
        let e = entry(block.offset(2), &[0, 0, 0, 0]).in_block(block, 0x34);
        assert_eq!(e.block(), Some((block, 0x34)));
        // 値が塊からはみ出すなら確かめられない
        assert_eq!(
            entry(block.offset(0x32), &[0; 4])
                .in_block(block, 0x34)
                .block(),
            None
        );
        assert_eq!(entry(block, &[0]).block(), None);
    }

    #[test]
    fn restores_newest_first() {
        let a = Address::new(0x04, 0, 0x56, 0x01);
        let mut j = Journal::default();
        j.remember(entry(a, &[0, 0, 1, 2]));
        // 一部が重なる書き換え（後から広く残した）
        j.remember(entry(a.offset(2), &[9, 9, 0, 0]));
        let plan = j.restore_plan();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].1, a.offset(2));
        assert_eq!(plan[1].1, a);
        assert_eq!(plan[1].2, vec![0, 0, 1, 2]);
    }

    #[test]
    fn round_trips_through_toml() {
        let mut j = Journal::default();
        j.remember(entry(Address::new(0x04, 0, 0x23, 0x02), &[0, 0, 0, 0]));
        j.remember(entry(Address::new(0, 0, 0, 0), &[0, 0, 0, 5]));
        let text = j.to_toml();
        assert!(text.contains("address = \"04 00 23 02\""), "{text}");
        assert_eq!(Journal::from_toml(&text).unwrap(), j);
        assert_eq!(Journal::from_toml("").unwrap(), Journal::default());
        assert!(Journal::from_toml("entry = 3").is_err());
    }

    #[test]
    fn ignores_broken_lines() {
        let j = Journal {
            entries: vec![Entry {
                address: "04 00 23".into(),
                ..entry(Address::new(0, 0, 0, 0), &[1])
            }],
        };
        assert_eq!(j.broken(), 1);
        assert!(j.restore_plan().is_empty());
        let mut j = Journal::default();
        assert!(!j.remember(Entry {
            original: "80".into(),
            ..entry(Address::new(0, 0, 0, 0), &[1])
        }));
    }
}
