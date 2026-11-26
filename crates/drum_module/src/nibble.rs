//! 4 ビットずつに分けて送る値（番地表で `#` の付いた所。`0000 aaaa` `0000 bbbb` … の並び）。
//!
//! 例: Volume（-601〜60）は 4 バイトで、上の桁から 4 ビットずつ入る。
//! 負の数はその桁数での 2 の補数と見ている（-1 は `0F 0F 0F 0F`）。
//! 番地表の書き方からの推測で、実機ではまだ確かめていない。測る道具で読んだ生の値を記録に残して確かめる。

/// 4 ビットずつの並びを、符号なしの数として読む（どのバイトも下の 4 ビットだけ使う）
pub fn decode_unsigned(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, &b| (acc << 4) | u32::from(b & 0x0F))
}

/// 4 ビットずつの並びを、その桁数での 2 の補数として読む
pub fn decode_signed(bytes: &[u8]) -> i32 {
    let bits = 4 * bytes.len() as u32;
    let raw = decode_unsigned(bytes);
    if bits == 0 || bits >= 32 {
        return raw as i32;
    }
    let sign = 1u32 << (bits - 1);
    if raw & sign != 0 {
        (i64::from(raw) - (1i64 << bits)) as i32
    } else {
        raw as i32
    }
}

/// 数を `nibbles` 桁の 4 ビットずつの並びにする（負の数はその桁数での 2 の補数）
pub fn encode(value: i32, nibbles: usize) -> Vec<u8> {
    let raw = value as u32;
    (0..nibbles)
        .rev()
        .map(|i| ((raw >> (4 * i as u32)) & 0x0F) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_the_kit_number() {
        // Current の KitNum（0〜199）。199 は 00 00 0C 07
        assert_eq!(encode(199, 4), vec![0, 0, 0x0C, 0x07]);
        assert_eq!(decode_unsigned(&[0, 0, 0x0C, 0x07]), 199);
        assert_eq!(decode_signed(&[0, 0, 0x0C, 0x07]), 199);
    }

    #[test]
    fn negative_numbers_are_twos_complement_over_the_width() {
        // Volume の -INF（-601）
        assert_eq!(encode(-601, 4), vec![0x0F, 0x0D, 0x0A, 0x07]);
        assert_eq!(decode_signed(&[0x0F, 0x0D, 0x0A, 0x07]), -601);
        // Pad EQ Low Gain（2 桁、-24〜24）
        assert_eq!(encode(-24, 2), vec![0x0E, 0x08]);
        assert_eq!(decode_signed(&[0x0E, 0x08]), -24);
        assert_eq!(decode_signed(&[0x01, 0x08]), 24);
        assert_eq!(decode_signed(&[0x0F, 0x0F, 0x0F, 0x0F]), -1);
        assert_eq!(encode(60, 4), vec![0, 0, 0x03, 0x0C]);
    }

    #[test]
    fn only_the_low_four_bits_count() {
        assert_eq!(decode_unsigned(&[0x10, 0x21]), 0x01);
        assert_eq!(decode_unsigned(&[]), 0);
    }
}
