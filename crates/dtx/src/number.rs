//! DTX で使う数値表記。

fn base36_digit(c: u8) -> Option<u32> {
    match c {
        b'0'..=b'9' => Some(u32::from(c - b'0')),
        b'A'..=b'Z' => Some(u32::from(c - b'A') + 10),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 10),
        _ => None,
    }
}

/// 2 文字の 36 進数（00〜ZZ）
pub fn parse_base36x2(s: &str) -> Option<u16> {
    let b = s.as_bytes();
    if b.len() < 2 {
        return None;
    }
    Some((base36_digit(b[0])? * 36 + base36_digit(b[1])?) as u16)
}

/// 2 文字の 16 進数
pub fn parse_hex2(s: &str) -> Option<u8> {
    let b = s.get(..2)?;
    u8::from_str_radix(b, 16).ok()
}

/// 小節番号（3 文字。先頭だけ 36 進数、残り 2 桁は 10 進数。000〜Z99）
pub fn parse_measure(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    if b.len() < 3 || !b[1].is_ascii_digit() || !b[2].is_ascii_digit() {
        return None;
    }
    Some(base36_digit(b[0])? * 100 + u32::from(b[1] - b'0') * 10 + u32::from(b[2] - b'0'))
}

/// 上流と同じ数値の読み方。小数点に '.' と ',' の両方を許し、最後に現れたものを小数点にする。
/// 先頭の数値部分だけを読む（`120 ;コメント` や `1'000` も読める）。
pub fn parse_nx_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let chars: Vec<char> = s.chars().collect();
    let mut decimal_pos = chars.len();
    let mut end = 0;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '0'..='9' | '\'' | ' ' => end = i + 1,
            '.' | ',' => {
                decimal_pos = i;
                end = i + 1;
            }
            _ => break,
        }
    }
    let mut out = String::new();
    for (i, &c) in chars[..end].iter().enumerate() {
        if c.is_ascii_digit() {
            out.push(c);
        } else if (c == '.' || c == ',') && i == decimal_pos {
            out.push('.');
        }
    }
    if !out.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    out.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base36() {
        assert_eq!(parse_base36x2("01"), Some(1));
        assert_eq!(parse_base36x2("ZZ"), Some(1295));
        assert_eq!(parse_base36x2("a0"), Some(360));
        assert_eq!(parse_base36x2("0!"), None);
    }

    #[test]
    fn measure() {
        assert_eq!(parse_measure("000"), Some(0));
        assert_eq!(parse_measure("199"), Some(199));
        assert_eq!(parse_measure("A05"), Some(1005));
        assert_eq!(parse_measure("0A0"), None);
    }

    #[test]
    fn nx_number() {
        assert_eq!(parse_nx_number("120"), Some(120.0));
        assert_eq!(parse_nx_number(" 145.5 "), Some(145.5));
        assert_eq!(parse_nx_number("0,75"), Some(0.75));
        assert_eq!(parse_nx_number("1.000,5"), Some(1000.5));
        assert_eq!(parse_nx_number("150bpm"), Some(150.0));
        assert_eq!(parse_nx_number("abc"), None);
    }
}
