//! Windows のショートカット（`.lnk`、Shell Link Binary File Format [MS-SHLLINK]）の行き先を読む。
//!
//! OS の機能（COM の IShellLink）は使わず、ファイルの中身から読むので、どの OS でもテストできる。
//! 行き先は書いてある順に候補を並べ、呼ぶ側が「あるもの」を選ぶ:
//!
//! 1. LinkInfo の PC の中のパス（Unicode があればそれ、無ければ ANSI。後ろに CommonPathSuffix を足す）
//! 2. LinkInfo のネットワークのパス（`\\サーバー\共有` ＋ CommonPathSuffix）
//! 3. EnvironmentVariableDataBlock のパス（`%USERPROFILE%` などを今の環境で置き換える）
//! 4. RELATIVE_PATH（ショートカットのあるフォルダから）
//!
//! 作業フォルダ（WORKING_DIR）は別に返す（DTXMania のショートカットなら、たいてい DTXMania のフォルダ）。
//! ANSI の文字列は書いた PC の文字コードで、日本語の Windows なら Shift_JIS。UTF-8 として読めなければ Shift_JIS として読む。

use std::path::{Path, PathBuf};

/// `.lnk` のはじめの 20 バイト（HeaderSize と LinkCLSID）
const MAGIC: [u8; 20] = [
    0x4C, 0x00, 0x00, 0x00, 0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x46,
];

const HAS_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const HAS_NAME: u32 = 0x4;
const HAS_RELATIVE_PATH: u32 = 0x8;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const HAS_ICON_LOCATION: u32 = 0x40;
const IS_UNICODE: u32 = 0x80;
const FORCE_NO_LINK_INFO: u32 = 0x100;

/// EnvironmentVariableDataBlock の印
const ENV_BLOCK: u32 = 0xA000_0001;

/// 読んだショートカット
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shortcut {
    /// 行き先の候補（確からしい順）
    pub targets: Vec<PathBuf>,
    /// 作業フォルダ（書いていなければ `None`）
    pub working_dir: Option<PathBuf>,
}

impl Shortcut {
    /// 行き先の候補のうち、今あるもの
    pub fn existing_target(&self) -> Option<&Path> {
        self.targets
            .iter()
            .map(PathBuf::as_path)
            .find(|p| p.exists())
    }
}

/// ショートカットとして読めなかった理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LnkError {
    /// `.lnk` の形ではない
    NotShortcut,
    /// 途中で切れている、または長さが合わない
    Broken,
    /// 行き先が 1 つも書いていない（コントロールパネルの項目など、ファイルではないもの）
    NoTarget,
}

/// `.lnk` の中身を読む。`lnk_dir` はショートカットのあるフォルダ（相対パスの起点）
pub fn read(bytes: &[u8], lnk_dir: &Path) -> Result<Shortcut, LnkError> {
    if bytes.len() < 0x4C || bytes[..20] != MAGIC {
        return Err(LnkError::NotShortcut);
    }
    let flags = u32_at(bytes, 0x14).ok_or(LnkError::Broken)?;
    let unicode = flags & IS_UNICODE != 0;
    let mut pos = 0x4C;
    if flags & HAS_ID_LIST != 0 {
        let size = usize::from(u16_at(bytes, pos).ok_or(LnkError::Broken)?);
        pos = pos.checked_add(2 + size).ok_or(LnkError::Broken)?;
    }
    let mut out = Shortcut::default();
    if flags & HAS_LINK_INFO != 0 && flags & FORCE_NO_LINK_INFO == 0 {
        let size = u32_at(bytes, pos).ok_or(LnkError::Broken)? as usize;
        let info = bytes.get(pos..pos + size).ok_or(LnkError::Broken)?;
        out.targets.extend(link_info_paths(info));
        pos += size;
    } else if flags & HAS_LINK_INFO != 0 {
        // 使わないと書いてあっても、場所は取っている
        let size = u32_at(bytes, pos).ok_or(LnkError::Broken)? as usize;
        pos += size;
    }
    // StringData（この順に、フラグの立っているものだけ並ぶ）
    let mut strings = [None, None, None, None, None];
    let bits = [
        HAS_NAME,
        HAS_RELATIVE_PATH,
        HAS_WORKING_DIR,
        HAS_ARGUMENTS,
        HAS_ICON_LOCATION,
    ];
    for (slot, bit) in strings.iter_mut().zip(bits) {
        if flags & bit == 0 {
            continue;
        }
        let count = usize::from(u16_at(bytes, pos).ok_or(LnkError::Broken)?);
        pos += 2;
        let len = if unicode { count * 2 } else { count };
        let raw = bytes.get(pos..pos + len).ok_or(LnkError::Broken)?;
        *slot = Some(if unicode { utf16(raw) } else { ansi(raw) });
        pos += len;
    }
    let [_, relative, working, _, _]: [Option<String>; 5] = strings;
    // 追加のデータ（ExtraData）。大きさが 4 より小さいものが終わりの印
    while let Some(size) = u32_at(bytes, pos).map(|s| s as usize) {
        if size < 8 {
            break;
        }
        let Some(block) = bytes.get(pos..pos + size) else {
            break;
        };
        if u32_at(block, 4) == Some(ENV_BLOCK) && block.len() >= 8 + 260 + 520 {
            let wide = utf16(&block[8 + 260..8 + 260 + 520]);
            let target = if wide.is_empty() {
                ansi(&block[8..8 + 260])
            } else {
                wide
            };
            if !target.is_empty() {
                out.targets.push(windows_path(&expand_env(&target)));
            }
        }
        pos += size;
    }
    if let Some(rel) = relative.filter(|r| !r.is_empty()) {
        out.targets.push(join_windows(lnk_dir, &rel));
    }
    out.working_dir = working
        .filter(|w| !w.is_empty())
        .map(|w| windows_path(&expand_env(&w)));
    dedup(&mut out.targets);
    if out.targets.is_empty() && out.working_dir.is_none() {
        return Err(LnkError::NoTarget);
    }
    Ok(out)
}

/// LinkInfo の中のパス（PC の中のパス、ネットワークのパスの順）
fn link_info_paths(info: &[u8]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let (Some(header), Some(flags)) = (u32_at(info, 4), u32_at(info, 8)) else {
        return out;
    };
    let header = header as usize;
    let offset = |at: usize| u32_at(info, at).map(|o| o as usize).filter(|&o| o > 0);
    let suffix = || {
        let wide = (header >= 0x24)
            .then(|| offset(0x20))
            .flatten()
            .map(|o| utf16_z(info, o));
        wide.filter(|s| !s.is_empty())
            .or_else(|| offset(0x18).map(|o| ansi_z(info, o)))
            .unwrap_or_default()
    };
    if flags & 0x1 != 0 {
        let wide = (header >= 0x24)
            .then(|| offset(0x1C))
            .flatten()
            .map(|o| utf16_z(info, o))
            .filter(|s| !s.is_empty());
        let base = wide.or_else(|| offset(0x10).map(|o| ansi_z(info, o)));
        if let Some(base) = base.filter(|b| !b.is_empty()) {
            out.push(windows_path(&concat(&base, &suffix())));
        }
    }
    if flags & 0x2 != 0
        && let Some(net) = offset(0x14)
        && let Some(net_name) = u32_at(info, net + 8).map(|o| net + o as usize)
    {
        let name = ansi_z(info, net_name);
        if !name.is_empty() {
            out.push(windows_path(&concat(&name, &suffix())));
        }
    }
    out
}

/// パスの後ろに続きを足す（区切りが重ならないように）
fn concat(base: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return base.to_owned();
    }
    if base.ends_with('\\') || base.ends_with('/') {
        format!("{base}{suffix}")
    } else {
        format!("{base}\\{suffix}")
    }
}

/// `%NAME%` を今の環境変数で置き換える（無い名前はそのまま）
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        let (head, tail) = rest.split_at(start);
        out.push_str(head);
        let Some(end) = tail[1..].find('%') else {
            out.push_str(tail);
            return out;
        };
        let name = &tail[1..1 + end];
        match std::env::var(name).ok().filter(|_| !name.is_empty()) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&tail[..end + 2]),
        }
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Windows の書き方のパスを、この OS のパスにする（Windows 以外では `\` を `/` に）
fn windows_path(s: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(s)
    } else {
        PathBuf::from(s.replace('\\', "/"))
    }
}

/// `base` から Windows の書き方の相対パスをたどる
fn join_windows(base: &Path, rel: &str) -> PathBuf {
    crate::resolve(base, rel)
}

fn dedup(paths: &mut Vec<PathBuf>) {
    let mut seen = Vec::new();
    paths.retain(|p| {
        let key = p.to_string_lossy().to_lowercase();
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// UTF-16LE（最初の NUL まで）
fn utf16(raw: &[u8]) -> String {
    let units: Vec<u16> = raw
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// `at` から NUL までの UTF-16LE
fn utf16_z(b: &[u8], at: usize) -> String {
    b.get(at..).map(utf16).unwrap_or_default()
}

/// ANSI（最初の NUL まで。UTF-8 として読めなければ Shift_JIS）
fn ansi(raw: &[u8]) -> String {
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    dtx::decode(&raw[..end]).0
}

/// `at` から NUL までの ANSI
fn ansi_z(b: &[u8], at: usize) -> String {
    b.get(at..).map(ansi).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// テスト用にショートカットを組み立てる
    #[derive(Default)]
    pub struct Builder {
        pub local_ansi: Option<Vec<u8>>,
        pub local_wide: Option<String>,
        pub suffix: String,
        pub net_name: Option<String>,
        pub relative: Option<String>,
        pub working: Option<String>,
        pub env_target: Option<String>,
        pub id_list: bool,
    }

    fn wide_z(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    impl Builder {
        pub fn build(&self) -> Vec<u8> {
            let mut flags = IS_UNICODE;
            let mut b = MAGIC.to_vec();
            b.resize(0x4C, 0);
            if self.id_list {
                flags |= HAS_ID_LIST;
            }
            let has_info =
                self.local_ansi.is_some() || self.local_wide.is_some() || self.net_name.is_some();
            if has_info {
                flags |= HAS_LINK_INFO;
            }
            if self.relative.is_some() {
                flags |= HAS_RELATIVE_PATH;
            }
            if self.working.is_some() {
                flags |= HAS_WORKING_DIR;
            }
            b[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
            if self.id_list {
                // 中身は読まないので適当な 6 バイト
                b.extend_from_slice(&6u16.to_le_bytes());
                b.extend_from_slice(&[4, 0, 0xAA, 0xBB, 0, 0]);
            }
            if has_info {
                let wide = self.local_wide.is_some();
                let header: u32 = if wide { 0x24 } else { 0x1C };
                let mut body = Vec::new();
                let mut info_flags = 0u32;
                let mut fields = [0u32; 8];
                let base = header as usize;
                // VolumeID（中身は読まないので最小）
                let volume_at = base;
                body.extend_from_slice(&16u32.to_le_bytes());
                body.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0, 0]);
                if self.local_ansi.is_some() || wide {
                    info_flags |= 1;
                    fields[2] = volume_at as u32;
                    fields[3] = (base + body.len()) as u32;
                    body.extend(self.local_ansi.clone().unwrap_or_default());
                    body.push(0);
                }
                if let Some(net) = &self.net_name {
                    info_flags |= 2;
                    fields[4] = (base + body.len()) as u32;
                    let mut nb = Vec::new();
                    nb.extend_from_slice(&(0x14u32 + net.len() as u32 + 1).to_le_bytes());
                    nb.extend_from_slice(&2u32.to_le_bytes());
                    nb.extend_from_slice(&0x14u32.to_le_bytes());
                    nb.extend_from_slice(&0u32.to_le_bytes());
                    nb.extend_from_slice(&0u32.to_le_bytes());
                    nb.extend_from_slice(net.as_bytes());
                    nb.push(0);
                    body.extend(nb);
                }
                fields[5] = (base + body.len()) as u32;
                body.extend_from_slice(self.suffix.as_bytes());
                body.push(0);
                if let Some(w) = &self.local_wide {
                    fields[6] = (base + body.len()) as u32;
                    body.extend(wide_z(w));
                    fields[7] = (base + body.len()) as u32;
                    body.extend(wide_z(&self.suffix));
                }
                let size = header as usize + body.len();
                fields[0] = size as u32;
                fields[1] = header;
                let mut info = Vec::new();
                info.extend_from_slice(&fields[0].to_le_bytes());
                info.extend_from_slice(&fields[1].to_le_bytes());
                info.extend_from_slice(&info_flags.to_le_bytes());
                for f in &fields[2..6] {
                    info.extend_from_slice(&f.to_le_bytes());
                }
                if wide {
                    info.extend_from_slice(&fields[6].to_le_bytes());
                    info.extend_from_slice(&fields[7].to_le_bytes());
                }
                info.extend(body);
                b.extend(info);
            }
            for s in [&self.relative, &self.working].into_iter().flatten() {
                let units: Vec<u16> = s.encode_utf16().collect();
                b.extend_from_slice(&(units.len() as u16).to_le_bytes());
                b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
            }
            if let Some(t) = &self.env_target {
                let mut block = Vec::new();
                block.extend_from_slice(&0x314u32.to_le_bytes());
                block.extend_from_slice(&ENV_BLOCK.to_le_bytes());
                let mut a = t.as_bytes().to_vec();
                a.resize(260, 0);
                block.extend(a);
                let mut w = wide_z(t);
                w.resize(520, 0);
                block.extend(w);
                b.extend(block);
            }
            b.extend_from_slice(&0u32.to_le_bytes());
            b
        }
    }

    fn p(s: &str) -> PathBuf {
        windows_path(s)
    }

    #[test]
    fn reads_the_local_path_and_working_dir() {
        let lnk = Builder {
            local_ansi: Some(b"C:\\Games\\DTXManiaNX\\DTXManiaNX.exe".to_vec()),
            working: Some("C:\\Games\\DTXManiaNX".into()),
            relative: Some("..\\..\\Games\\DTXManiaNX\\DTXManiaNX.exe".into()),
            id_list: true,
            ..Default::default()
        }
        .build();
        let s = read(&lnk, Path::new("/home/u/Desktop")).unwrap();
        assert_eq!(s.targets[0], p("C:\\Games\\DTXManiaNX\\DTXManiaNX.exe"));
        assert_eq!(s.working_dir, Some(p("C:\\Games\\DTXManiaNX")));
        // 相対パスはショートカットのあるフォルダから
        assert_eq!(
            s.targets[1],
            Path::new("/home/Games/DTXManiaNX/DTXManiaNX.exe")
        );
    }

    #[test]
    fn prefers_unicode_and_reads_shift_jis() {
        // 「ゲーム」の Shift_JIS
        let mut ansi = b"D:\\".to_vec();
        ansi.extend_from_slice(&[0x83, 0x51, 0x81, 0x5B, 0x83, 0x80]);
        let lnk = Builder {
            local_ansi: Some(ansi.clone()),
            suffix: "NX\\DTXManiaNX.exe".into(),
            ..Default::default()
        }
        .build();
        let s = read(&lnk, Path::new(".")).unwrap();
        assert_eq!(s.targets, vec![p("D:\\ゲーム\\NX\\DTXManiaNX.exe")]);

        let lnk = Builder {
            local_ansi: Some(b"D:\\????\\".to_vec()),
            local_wide: Some("D:\\ドラム\\".into()),
            suffix: "DTXManiaGR.exe".into(),
            ..Default::default()
        }
        .build();
        let s = read(&lnk, Path::new(".")).unwrap();
        assert_eq!(s.targets[0], p("D:\\ドラム\\DTXManiaGR.exe"));
    }

    #[test]
    fn reads_network_and_environment_paths() {
        let lnk = Builder {
            net_name: Some("\\\\NAS\\share".into()),
            suffix: "DTX\\DTXManiaNX.exe".into(),
            env_target: Some("%NXT_TEST_NO_SUCH_VAR%\\DTX\\DTXManiaNX.exe".into()),
            ..Default::default()
        }
        .build();
        let s = read(&lnk, Path::new(".")).unwrap();
        assert_eq!(s.targets[0], p("\\\\NAS\\share\\DTX\\DTXManiaNX.exe"));
        // 無い環境変数はそのまま残す
        assert_eq!(
            s.targets[1],
            p("%NXT_TEST_NO_SUCH_VAR%\\DTX\\DTXManiaNX.exe")
        );
    }

    #[test]
    fn rejects_other_files_and_cut_shortcuts() {
        assert_eq!(
            read(b"[System]\r\n", Path::new(".")),
            Err(LnkError::NotShortcut)
        );
        let lnk = Builder {
            local_ansi: Some(b"C:\\a.exe".to_vec()),
            ..Default::default()
        }
        .build();
        assert_eq!(read(&lnk[..0x60], Path::new(".")), Err(LnkError::Broken));
        let empty = Builder::default().build();
        assert_eq!(read(&empty, Path::new(".")), Err(LnkError::NoTarget));
    }

    #[test]
    fn expands_environment_variables() {
        // PATH はどの OS にもある
        let path = std::env::var("PATH").unwrap();
        assert_eq!(expand_env("%PATH%\\x"), format!("{path}\\x"));
        assert_eq!(expand_env("100%"), "100%");
        assert_eq!(expand_env("%%"), "%%");
    }
}
