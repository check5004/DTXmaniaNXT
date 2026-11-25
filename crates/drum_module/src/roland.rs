//! Roland の SysEx。
//!
//! - Data Request 1（RQ1）: `F0 41 dev <model> 11 aa bb cc dd ss tt uu vv sum F7` で値を求める
//! - Data Set 1（DT1）: `F0 41 dev <model> 12 aa bb cc dd <data…> sum F7` で値を書く（音源からの返事も同じ形）
//! - Identity Request / Reply（MIDI の決まりの汎用メッセージ）で機種を見分ける
//!
//! 番地と大きさは 7 ビットずつ 4 バイト。チェックサムは番地と値（RQ1 なら大きさ）を足して
//! 128 で割った余りを 128 から引いたもの（余りが 0 なら 0）。

use serde::{Deserialize, Serialize};

/// Roland のメーカー ID
pub const ROLAND_ID: u8 = 0x41;
/// SysEx の始まりと終わり
pub const SOX: u8 = 0xF0;
pub const EOX: u8 = 0xF7;
/// Data Request 1 のコマンド ID
const RQ1: u8 = 0x11;
/// Data Set 1 のコマンド ID
const DT1: u8 = 0x12;
/// どの機器でも受ける Device ID（Identity Request などで使う）
pub const BROADCAST: u8 = 0x7F;

/// 7 ビットずつ 4 バイトの番地（`04 00 52 21` など）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Address(pub [u8; 4]);

impl Address {
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    /// 7 ビットずつの数を 1 つの数にしたもの（`00 01 00 00` は 16384）
    pub const fn to_index(self) -> u32 {
        let [a, b, c, d] = self.0;
        ((a as u32) << 21) | ((b as u32) << 14) | ((c as u32) << 7) | d as u32
    }

    /// [`Address::to_index`] の逆。28 ビットを超える分は捨てる
    pub const fn from_index(n: u32) -> Self {
        Self([
            ((n >> 21) & 0x7F) as u8,
            ((n >> 14) & 0x7F) as u8,
            ((n >> 7) & 0x7F) as u8,
            (n & 0x7F) as u8,
        ])
    }

    /// 7 ビットずつ繰り上げて足す（`00 00 7F` に 1 を足すと `00 01 00`）
    pub const fn offset(self, by: u32) -> Self {
        Self::from_index(self.to_index() + by)
    }

    /// `aa bb cc dd` の形の文字列（記録やログ用）
    pub fn hex(self) -> String {
        hex(&self.0)
    }
}

/// 番地表の「オフセット」（`00 50 00` のように 7 ビットずつ 3 バイトで書かれたもの）を数にする
pub const fn off(a: u8, b: u8, c: u8) -> u32 {
    ((a as u32) << 14) | ((b as u32) << 7) | c as u32
}

/// 大きさ（バイト数）を 7 ビットずつ 4 バイトにする
pub const fn size_bytes(size: u32) -> [u8; 4] {
    Address::from_index(size).0
}

/// チェックサム（足したものを 128 で割った余りを 128 から引く。余りが 0 なら 0）
pub fn checksum(body: &[u8]) -> u8 {
    let sum: u32 = body.iter().map(|&b| u32::from(b & 0x7F)).sum();
    ((128 - sum % 128) % 128) as u8
}

/// Data Request 1。`size` は求めるバイト数
pub fn rq1(device: u8, model: &[u8], addr: Address, size: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&addr.0);
    body.extend_from_slice(&size_bytes(size));
    frame(device, model, RQ1, &body)
}

/// Data Set 1。`data` は番地から順に書く値（どれも 0〜127）
pub fn dt1(device: u8, model: &[u8], addr: Address, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(&addr.0);
    body.extend(data.iter().map(|&b| b & 0x7F));
    frame(device, model, DT1, &body)
}

fn frame(device: u8, model: &[u8], command: u8, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(6 + model.len() + body.len());
    m.push(SOX);
    m.push(ROLAND_ID);
    m.push(device & 0x7F);
    m.extend_from_slice(model);
    m.push(command);
    m.extend_from_slice(body);
    m.push(checksum(body));
    m.push(EOX);
    m
}

/// 音源から届いた Data Set 1
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSet<'a> {
    pub device: u8,
    pub addr: Address,
    pub data: &'a [u8],
}

/// Data Set 1 を読む（機種 `model` のもので、チェックサムが合うものだけ）
pub fn parse_dt1<'a>(msg: &'a [u8], model: &[u8]) -> Option<DataSet<'a>> {
    let head = 3 + model.len() + 1;
    if msg.len() < head + 4 + 2
        || msg[0] != SOX
        || msg[1] != ROLAND_ID
        || &msg[3..3 + model.len()] != model
        || msg[head - 1] != DT1
        || *msg.last()? != EOX
    {
        return None;
    }
    let body = &msg[head..msg.len() - 2];
    if checksum(body) != msg[msg.len() - 2] {
        return None;
    }
    Some(DataSet {
        device: msg[2],
        addr: Address([body[0], body[1], body[2], body[3]]),
        data: &body[4..],
    })
}

/// RQ1 で求めた塊を、届いた Data Set 1 から組み立てる。
///
/// 大きい塊は何回かに分けて届くことがあり、同じものが 2 回届くこともある（入力を 2 つ開いたときなど）。
/// どちらでも、番地の合う所を埋めていき、全部そろったら終わり
#[derive(Debug, Clone)]
pub struct Collector {
    start: u32,
    data: Vec<u8>,
    filled: Vec<bool>,
}

impl Collector {
    pub fn new(addr: Address, size: u32) -> Self {
        Self {
            start: addr.to_index(),
            data: vec![0; size as usize],
            filled: vec![false; size as usize],
        }
    }

    /// 届いた Data Set 1 のうち、求めた範囲に入る所を埋める。全部そろったら true
    pub fn feed(&mut self, set: &DataSet<'_>) -> bool {
        let at = set.addr.to_index();
        for (i, &b) in set.data.iter().enumerate() {
            let pos = i64::from(at) + i as i64 - i64::from(self.start);
            if let Ok(pos) = usize::try_from(pos)
                && pos < self.data.len()
            {
                self.data[pos] = b;
                self.filled[pos] = true;
            }
        }
        self.done()
    }

    pub fn done(&self) -> bool {
        self.filled.iter().all(|&f| f)
    }

    /// そろった値（そろう前は届いていない所が 0）
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

/// Identity Request（`F0 7E dev 06 01 F7`）
pub fn identity_request(device: u8) -> Vec<u8> {
    vec![SOX, 0x7E, device & 0x7F, 0x06, 0x01, EOX]
}

/// Identity Reply の中身
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub device: u8,
    pub manufacturer: u8,
    /// 機種のファミリー（V51 は `01 06`）
    pub family: [u8; 2],
    /// ファミリーの中の番号（V51 は `02 00`）
    pub number: [u8; 2],
    /// ソフトウェアの版（V51 2.00 は `00 02 00 00`）
    pub revision: [u8; 4],
}

/// Identity Reply（`F0 7E dev 06 02 mm ff ff nn nn rr rr rr rr F7`）を読む
pub fn parse_identity_reply(msg: &[u8]) -> Option<Identity> {
    if msg.len() != 15 || msg[0] != SOX || msg[1] != 0x7E || msg[3] != 0x06 || msg[4] != 0x02 {
        return None;
    }
    if msg[14] != EOX {
        return None;
    }
    Some(Identity {
        device: msg[2],
        manufacturer: msg[5],
        family: [msg[6], msg[7]],
        number: [msg[8], msg[9]],
        revision: [msg[10], msg[11], msg[12], msg[13]],
    })
}

/// バイト列を `f0 41 10 …` の形にする（記録やログ用）
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{b:02X}"));
    }
    s
}

/// [`hex`] の逆（`04 00 23 02` → バイト列）。空白で区切った 2 桁の 16 進だけ受ける
pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    s.split_whitespace()
        .map(|t| {
            if t.len() == 2 {
                u8::from_str_radix(t, 16).ok()
            } else {
                None
            }
        })
        .collect()
}

/// 1 つの塊に SysEx がいくつか入っていることがある（OS によって）。F0〜F7 ごとに分ける
pub fn split_sysex(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            SOX => start = Some(i),
            EOX => {
                if let Some(s) = start.take() {
                    out.push(&bytes[s..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// 何回かに分かれて届く SysEx をつなぐ（OS によっては長い SysEx が途中で切れて届く）。
/// ポートごとに 1 つ持つ
#[derive(Debug, Clone, Default)]
pub struct SysexAssembler {
    buf: Option<Vec<u8>>,
}

/// つなぐ途中の SysEx の長さの上限（これを超えたら捨てる）
const SYSEX_LIMIT: usize = 64 * 1024;

impl SysexAssembler {
    /// SysEx の途中まで届いているか
    pub fn in_progress(&self) -> bool {
        self.buf.is_some()
    }

    /// 届いたバイト列を足し、そろった SysEx（F0〜F7）を返す
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &b in bytes {
            match b {
                SOX => self.buf = Some(vec![SOX]),
                EOX => {
                    if let Some(mut m) = self.buf.take() {
                        m.push(EOX);
                        out.push(m);
                    }
                }
                // リアルタイムのメッセージは SysEx の途中に割り込んでよい
                0xF8..=0xFF => {}
                // ほかのステータスが来たら、つないでいた SysEx は途切れたものとして捨てる
                0x80..=0xF6 => self.buf = None,
                _ => {
                    if let Some(m) = self.buf.as_mut() {
                        m.push(b);
                        if m.len() > SYSEX_LIMIT {
                            self.buf = None;
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V51: [u8; 3] = [0x01, 0x06, 0x01];

    #[test]
    fn manual_example_dt1() {
        // V51 MIDI Implementation の例 1: キット 1 の SNARE HEAD のレイヤー A の EQ を ON
        let addr = Address::new(0x04, 0, 0, 0).offset(off(0x00, 0x52, 0x00) + 0x21);
        assert_eq!(addr, Address::new(0x04, 0x00, 0x52, 0x21));
        assert_eq!(
            dt1(0x10, &V51, addr, &[0x01]),
            vec![
                0xF0, 0x41, 0x10, 0x01, 0x06, 0x01, 0x12, 0x04, 0x00, 0x52, 0x21, 0x01, 0x08, 0xF7
            ]
        );
    }

    #[test]
    fn manual_example_rq1() {
        // 例 2: キット 1 の SNARE のパッド・コンプのスイッチを求める
        let addr = Address::new(0x04, 0, 0, 0).offset(off(0x02, 0x11, 0x00) + 0x0D);
        assert_eq!(
            rq1(0x10, &V51, addr, 1),
            vec![
                0xF0, 0x41, 0x10, 0x01, 0x06, 0x01, 0x11, 0x04, 0x02, 0x11, 0x0D, 0x00, 0x00, 0x00,
                0x01, 0x5B, 0xF7
            ]
        );
    }

    #[test]
    fn offsets_carry_every_seven_bits() {
        assert_eq!(
            Address::new(0, 0, 0, 0x7F).offset(1),
            Address::new(0, 0, 1, 0)
        );
        // キット 200 は 0A 1C 00 00
        assert_eq!(
            Address::new(0x04, 0, 0, 0).offset(199 * off(0x04, 0, 0)),
            Address::new(0x0A, 0x1C, 0, 0)
        );
        // レイヤー A の 28 番目は 01 06 00
        assert_eq!(
            off(0, 0x50, 0) + 27 * off(0, 0x02, 0),
            off(0x01, 0x06, 0x00)
        );
        assert_eq!(size_bytes(0x34), [0, 0, 0, 0x34]);
        assert_eq!(size_bytes(off(0x01, 0x51, 0) >> 7), [0, 0, 0x01, 0x51]);
    }

    #[test]
    fn checksum_zero_when_the_sum_is_a_multiple_of_128() {
        assert_eq!(checksum(&[0x40, 0x40]), 0);
        assert_eq!(checksum(&[0x7F]), 1);
    }

    #[test]
    fn reads_a_data_set_and_rejects_a_bad_one() {
        let msg = dt1(0x10, &V51, Address::new(0, 0, 0, 0), &[0, 0, 0, 5]);
        let d = parse_dt1(&msg, &V51).unwrap();
        assert_eq!(
            (d.device, d.addr, d.data),
            (0x10, Address::new(0, 0, 0, 0), &[0, 0, 0, 5][..])
        );
        let mut bad = msg.clone();
        let n = bad.len();
        bad[n - 2] ^= 1;
        assert_eq!(parse_dt1(&bad, &V51), None);
        assert_eq!(parse_dt1(&msg, &[0x01, 0x06, 0x02]), None);
        // RQ1 は返事ではない
        assert_eq!(
            parse_dt1(&rq1(0x10, &V51, Address::new(0, 0, 0, 0), 4), &V51),
            None
        );
    }

    #[test]
    fn collects_a_block_sent_in_pieces() {
        let base = Address::new(0x04, 0, 0x23, 0);
        let mut c = Collector::new(base, 6);
        let first = dt1(0x10, &V51, base, &[1, 2, 3]);
        let second = dt1(0x10, &V51, base.offset(3), &[4, 5, 6, 7]);
        // 範囲の外の番地のものは無視する
        let other = dt1(0x10, &V51, Address::new(0, 0, 0, 0), &[9; 4]);
        assert!(!c.feed(&parse_dt1(&other, &V51).unwrap()));
        assert!(!c.feed(&parse_dt1(&first, &V51).unwrap()));
        // 同じものが 2 回届いてもよい
        assert!(!c.feed(&parse_dt1(&first, &V51).unwrap()));
        assert!(c.feed(&parse_dt1(&second, &V51).unwrap()));
        assert_eq!(c.into_bytes(), vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn reads_the_v51_identity_reply() {
        // V51 MIDI Implementation の Identity Reply
        let msg = [
            0xF0, 0x7E, 0x10, 0x06, 0x02, 0x41, 0x01, 0x06, 0x02, 0x00, 0x00, 0x02, 0x00, 0x00,
            0xF7,
        ];
        let id = parse_identity_reply(&msg).unwrap();
        assert_eq!(id.manufacturer, ROLAND_ID);
        assert_eq!(id.family, [0x01, 0x06]);
        assert_eq!(id.number, [0x02, 0x00]);
        assert_eq!(
            identity_request(BROADCAST),
            vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]
        );
        assert_eq!(parse_identity_reply(&msg[..14]), None);
    }

    #[test]
    fn joins_a_sysex_split_into_chunks() {
        let msg = dt1(0x10, &V51, Address::new(0x04, 0, 0x23, 0), &[1, 2, 3, 4, 5]);
        let mut a = SysexAssembler::default();
        assert!(a.feed(&msg[..5]).is_empty());
        assert!(a.in_progress());
        // 途中にクロックが割り込んでもよい
        assert!(a.feed(&[0xF8]).is_empty());
        assert_eq!(a.feed(&msg[5..]), vec![msg.clone()]);
        // ノートが割り込んだら途切れたものとして捨てる
        assert!(a.feed(&msg[..5]).is_empty());
        assert!(a.feed(&[0x99, 0x30, 0x40]).is_empty());
        assert!(a.feed(&msg[5..]).is_empty());
        // 1 つの塊に 2 つ
        let mut two = msg.clone();
        two.extend_from_slice(&msg);
        assert_eq!(a.feed(&two).len(), 2);
    }

    #[test]
    fn splits_joined_messages() {
        let a = identity_request(0x7F);
        let mut joined = a.clone();
        joined.extend_from_slice(&[0x90, 0x30, 0x40]);
        joined.extend_from_slice(&a);
        assert_eq!(split_sysex(&joined), vec![&a[..], &a[..]]);
        assert_eq!(hex(&[0xF0, 0x0A]), "F0 0A");
        assert_eq!(parse_hex("f0 0A"), Some(vec![0xF0, 0x0A]));
        assert_eq!(parse_hex(""), Some(vec![]));
        assert_eq!(parse_hex("F00A"), None);
        assert_eq!(parse_hex("G0"), None);
    }
}
