//! BandJAM XA の音（`.xa`、ファイルの頭が `KWD1`）を読む。
//! 古い DTX の曲がドラムの音に使っている。上流 DTXMania は xadec.dll で読む。
//!
//! 形式は bjxa（<https://github.com/dridi/bjxa>）の説明 bjxa(5) による。コードは借りていない。
//!
//! - 頭の 32 バイトがヘッダ（リトルエンディアン）。`KWD1`、データの長さ、1 チャンネルのサンプル数、
//!   サンプルレート（16 ビット）、1 サンプルのビット数（4・6・8）、チャンネル数（1・2）。残りは使わない
//! - そのあとに 32 サンプルずつのブロック。ステレオは左のブロック、右のブロックの順に交互に並ぶ
//! - ブロックの頭の 1 バイトは、下位 4 ビットが右へずらす量 R、上位 4 ビットが予測の係数の番号
//! - サンプルは上位のビットから詰めてある。16 ビットの上位に置いて R だけ右へずらし、
//!   前の 2 サンプルから予測した値を足す。前のサンプルはチャンネルごとにブロックをまたいで続く（初めは 0）

use anyhow::{Result, bail, ensure};

/// ファイルの頭
const MAGIC: &[u8; 4] = b"KWD1";
const HEADER_LEN: usize = 32;
const SAMPLES_PER_BLOCK: usize = 32;

/// 予測の係数（1 つ前のサンプル、2 つ前のサンプル）。256 倍してある
const GAINS: [(i32, i32); 5] = [(0, 0), (240, 0), (460, -208), (392, -220), (488, -240)];

/// 読んだ音。サンプルはチャンネルの順に交互に並ぶ
#[derive(Debug, PartialEq)]
pub struct Xa {
    pub rate: u32,
    pub channels: usize,
    pub samples: Vec<i16>,
}

/// BandJAM XA のファイルか（頭だけを見る）
pub fn is_xa(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// BandJAM XA のファイルを読む
pub fn decode(bytes: &[u8]) -> Result<Xa> {
    ensure!(is_xa(bytes), "not a BandJAM XA file");
    ensure!(bytes.len() >= HEADER_LEN, "the XA header is cut short");
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let data_len = u32_at(4) as usize;
    let frames = u32_at(8) as usize;
    let rate = u32::from(u16::from_le_bytes([bytes[12], bytes[13]]));
    let bits = usize::from(bytes[14]);
    let channels = usize::from(bytes[15]);
    ensure!(matches!(bits, 4 | 6 | 8), "{bits} bits per XA sample");
    ensure!(matches!(channels, 1 | 2), "{channels} XA channels");
    ensure!(rate > 0, "no XA sample rate");

    let block_len = 1 + SAMPLES_PER_BLOCK * bits / 8;
    let data = &bytes[HEADER_LEN..];
    let data = &data[..data_len.min(data.len())];
    // 終わりの欠けたブロックは使わない
    let blocks = data.len() / block_len / channels;
    let frames = frames.min(blocks * SAMPLES_PER_BLOCK);
    let mut samples = vec![0i16; frames * channels];
    // チャンネルごとの 1 つ前と 2 つ前のサンプル
    let mut before = [[0i32; 2]; 2];
    for b in 0..blocks {
        for (ch, prev) in before.iter_mut().enumerate().take(channels) {
            let at = (b * channels + ch) * block_len;
            let block = &data[at..at + block_len];
            let shift = u32::from(block[0] & 0x0f);
            let Some(&(k0, k1)) = GAINS.get(usize::from(block[0] >> 4)) else {
                bail!("unknown XA gain {} in block {b}", block[0] >> 4);
            };
            let mut reader = Bits::new(&block[1..]);
            for i in 0..SAMPLES_PER_BLOCK {
                // 上位のビットに置いて符号を付ける
                let a = i32::from((reader.take(bits) << (16 - bits)) as u16 as i16);
                let p = ((a >> shift) + ((prev[0] * k0 + prev[1] * k1) >> 8))
                    .clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                *prev = [p, prev[0]];
                let frame = b * SAMPLES_PER_BLOCK + i;
                if frame < frames {
                    samples[frame * channels + ch] = p as i16;
                }
            }
        }
    }
    Ok(Xa {
        rate,
        channels,
        samples,
    })
}

/// 上位のビットから順に読む
struct Bits<'a> {
    bytes: &'a [u8],
    /// 読んだビット数
    at: usize,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    /// `n`（8 以下）ビット読む
    fn take(&mut self, n: usize) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.bytes.get(self.at / 8).copied().unwrap_or(0);
            v = (v << 1) | u32::from((byte >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ヘッダを作る（データの長さは `data` から）
    fn file(frames: u32, rate: u16, bits: u8, channels: u8, data: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend((data.len() as u32).to_le_bytes());
        v.extend(frames.to_le_bytes());
        v.extend(rate.to_le_bytes());
        v.extend([bits, channels]);
        v.resize(HEADER_LEN, 0);
        v.extend(data);
        v
    }

    /// 8 ビットのブロック（頭の 1 バイトと 32 サンプル。足りない所は 0）
    fn block8(profile: u8, samples: &[u8]) -> Vec<u8> {
        let mut v = vec![profile];
        v.extend(samples);
        v.resize(33, 0);
        v
    }

    #[test]
    fn eight_bit_samples_are_the_upper_byte_shifted_by_the_range() {
        // 予測なし（係数 0）。0x01 は 256、0x80 は -32768、0x7f は 32512、0xff は -256
        let x = decode(&file(
            4,
            44_100,
            8,
            1,
            &block8(0x00, &[0x01, 0x80, 0x7f, 0xff]),
        ))
        .unwrap();
        assert_eq!(x.rate, 44_100);
        assert_eq!(x.channels, 1);
        assert_eq!(x.samples, vec![256, -32768, 32512, -256]);
        // R = 2 なら 4 で割る
        let x = decode(&file(
            4,
            44_100,
            8,
            1,
            &block8(0x02, &[0x01, 0x80, 0x7f, 0xff]),
        ))
        .unwrap();
        assert_eq!(x.samples, vec![64, -8192, 8128, -64]);
    }

    #[test]
    fn prediction_carries_over_to_the_next_block() {
        // 1 つめのブロック: 0x10 を R = 4 で 256。残りは 0 なので予測なしでは 0 のまま
        // 2 つめのブロック: 係数 1（240/256）で、前のブロックの最後の値から減っていく
        let mut first = block8(0x04, &[0x10]);
        first[1..].fill(0);
        first[32] = 0x10; // 最後のサンプルを 256 に
        let second = block8(0x10, &[]);
        let x = decode(&file(36, 48_000, 8, 1, &[first, second].concat())).unwrap();
        assert_eq!(x.samples[31], 256);
        // 256*240/256 = 240、240*240/256 = 225、225*240/256 = 210.9 → 210
        assert_eq!(x.samples[32..], [240, 225, 210, 196]);
    }

    #[test]
    fn second_order_prediction_and_clamping() {
        // 係数 2（460, -208）。1024, 0, 0 → 2 つ目は 1024*460/256 = 1840、
        // 3 つ目は (1840*460 - 1024*208)/256 = 2474.3 → 2474
        let x = decode(&file(3, 44_100, 8, 1, &block8(0x20, &[0x04]))).unwrap();
        assert_eq!(x.samples, vec![1024, 1840, 2474]);
        // 16 ビットを超えたら収める
        let x = decode(&file(2, 44_100, 8, 1, &block8(0x40, &[0x7f, 0x7f]))).unwrap();
        assert_eq!(x.samples, vec![32512, 32767]);
    }

    #[test]
    fn six_bit_stereo_blocks_alternate_left_and_right() {
        // 6 ビットは 3 バイトに 4 サンプル：1、-1、31、-32 は 000001 111111 011111 100000
        let mut left = vec![0x00, 0x07, 0xF7, 0xE0];
        left.resize(25, 0);
        // 右は R = 1。2 は 2048 を 2 で割って 1024
        let mut right = vec![0x01, 0x08];
        right.resize(25, 0);
        // 1 チャンネル 3 サンプルとすれば、ブロックの残りは捨てる
        let x = decode(&file(3, 48_000, 6, 2, &[left, right].concat())).unwrap();
        assert_eq!(x.channels, 2);
        assert_eq!(x.samples, vec![1024, 1024, -1024, 0, 31744, 0]);
    }

    #[test]
    fn four_bit_samples_take_the_high_nibble_first() {
        // 0x7f は 7 と -1。16 ビットの上位 4 ビットに置いて 28672 と -4096
        let mut b = vec![0x00, 0x7f];
        b.resize(17, 0);
        let x = decode(&file(2, 22_050, 4, 1, &b)).unwrap();
        assert_eq!(x.samples, vec![28672, -4096]);
    }

    #[test]
    fn broken_files_are_errors() {
        assert!(!is_xa(b"OggS"));
        assert!(decode(b"KWD1").is_err());
        assert!(decode(&file(1, 44_100, 5, 1, &[0; 33])).is_err());
        assert!(decode(&file(1, 44_100, 8, 3, &[0; 33])).is_err());
        // 係数の番号は 0〜4
        assert!(decode(&file(1, 44_100, 8, 1, &block8(0x50, &[]))).is_err());
        // 欠けたブロックしか無ければ音は空
        let x = decode(&file(32, 44_100, 8, 1, &[0; 20])).unwrap();
        assert!(x.samples.is_empty());
    }
}
