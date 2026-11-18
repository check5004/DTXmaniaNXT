//! DTX ファイルの文字コード。BOM が無ければ UTF-8 として読めるか試し、だめなら Shift_JIS（CP932）。

use encoding_rs::{SHIFT_JIS, UTF_16BE, UTF_16LE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtxEncoding {
    ShiftJis,
    Utf8Bom,
    Utf8,
    Utf16Le,
    Utf16Be,
}

pub fn decode(bytes: &[u8]) -> (String, DtxEncoding) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (
            String::from_utf8_lossy(rest).into_owned(),
            DtxEncoding::Utf8Bom,
        );
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (
            UTF_16LE.decode_without_bom_handling(rest).0.into_owned(),
            DtxEncoding::Utf16Le,
        );
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return (
            UTF_16BE.decode_without_bom_handling(rest).0.into_owned(),
            DtxEncoding::Utf16Be,
        );
    }
    if bytes.is_ascii() {
        return (
            String::from_utf8_lossy(bytes).into_owned(),
            DtxEncoding::ShiftJis,
        );
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_owned(), DtxEncoding::Utf8);
    }
    (
        SHIFT_JIS.decode_without_bom_handling(bytes).0.into_owned(),
        DtxEncoding::ShiftJis,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_jis_title() {
        // 「テスト」の Shift_JIS
        let bytes = [b'#', b'T', b' ', 0x83, 0x65, 0x83, 0x58, 0x83, 0x67];
        let (s, enc) = decode(&bytes);
        assert_eq!(s, "#T テスト");
        assert_eq!(enc, DtxEncoding::ShiftJis);
    }

    #[test]
    fn utf8_bom() {
        let (s, enc) = decode("\u{feff}#T テスト".as_bytes());
        assert_eq!(s, "#T テスト");
        assert_eq!(enc, DtxEncoding::Utf8Bom);
    }
}
