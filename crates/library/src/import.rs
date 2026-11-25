//! 曲の追加。ウィンドウにドロップされた zip や曲のフォルダを、曲のフォルダの中へ写す。
//!
//! zip は外から来るファイルなので、中の名前をそのまま使わない。
//! - 名前を `/` と `\` で区切り、`..`・空・`.` の部分は使わない（展開先の外へは書かない）。
//!   ドライブ名（`C:`）や Windows で使えない文字は `_` に、`CON` などの予約名は前に `_` を付ける
//! - UTF-8 の印が無い名前は、UTF-8 として読めなければ Shift_JIS（CP932）として読む。
//!   日本の Windows で作った zip は名前が CP932 で、そのままだと文字化けする
//! - リンク、暗号化した中身、展開すると大きすぎるものは書かない
//!
//! 写す先は、いったん隠しの作業フォルダに書き、全部そろってから名前を付ける。途中で失敗したら作業フォルダを消すので、
//! 曲のフォルダに半端なものを残さない。同じ名前のフォルダがあれば、中身が同じなら写さずにそれを使い、
//! 違えば「名前 (2)」にする（前からある曲は上書きしない）。

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use encoding_rs::SHIFT_JIS;

/// 譜面として数える拡張子（[`crate::scan`] と同じ）
const CHART_EXTENSIONS: &[&str] = &["dtx"];

/// 中身の数の上限（壊れた zip や、わざと大きく作った zip で止まらないように）
pub const MAX_FILES: usize = 100_000;

/// 写す大きさの合計の上限
pub const MAX_BYTES: u64 = 32 << 30;

/// フォルダをたどる深さの上限
const MAX_DEPTH: usize = 16;

/// 作業フォルダの名前の頭。曲を探すときは飛ばす（[`is_work_dir`]）
const WORK_PREFIX: &str = ".dtxnxt-adding-";

/// 1 回に書く大きさ（このたびに「やめる」を見る）
const CHUNK: usize = 256 * 1024;

/// 追加できないわけ
#[derive(Debug)]
pub enum ImportError {
    /// 読めない種類のファイル（拡張子。`rar` など）
    Unsupported(String),
    /// 中に譜面（.dtx）が無い
    NoCharts,
    /// パスワード付きの zip
    Encrypted,
    /// 読めない圧縮のしかた
    Method(String),
    /// 壊れている
    Broken(String),
    /// 大きすぎる（中身が多すぎる）
    TooLarge,
    /// ドロップしたフォルダの中に、曲のフォルダ（追加先）がある
    ContainsSongFolder,
    /// 読み書きの失敗
    Io(io::Error),
    /// 途中でやめた
    Stopped,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(ext) => write!(f, "unsupported file type: .{ext}"),
            Self::NoCharts => write!(f, "no DTX charts inside"),
            Self::Encrypted => write!(f, "password-protected"),
            Self::Method(m) => write!(f, "unsupported compression: {m}"),
            Self::Broken(e) => write!(f, "broken archive: {e}"),
            Self::TooLarge => write!(f, "too large"),
            Self::ContainsSongFolder => write!(f, "the folder contains a song folder"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<io::Error> for ImportError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<zip::result::ZipError> for ImportError {
    fn from(e: zip::result::ZipError) -> Self {
        use zip::result::ZipError;
        match e {
            ZipError::Io(e) => Self::Io(e),
            ZipError::UnsupportedArchive(m) if m.contains("assword") => Self::Encrypted,
            ZipError::UnsupportedArchive(m) => Self::Method(m.to_owned()),
            other => Self::Broken(other.to_string()),
        }
    }
}

/// 何をドロップされたか
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Zip,
    Folder,
}

/// ドロップされたものの種類。追加できない種類なら、そのわけ
pub fn source_of(path: &Path) -> Result<Source, ImportError> {
    if path.is_dir() {
        return Ok(Source::Folder);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "zip" => Ok(Source::Zip),
        _ => Err(ImportError::Unsupported(ext)),
    }
}

/// 写すファイル 1 つ
#[derive(Debug, Clone)]
struct PlanFile {
    /// 作るフォルダの中での場所（`/` 区切りの部分）
    rel: Vec<String>,
    from: Origin,
    size: u64,
}

#[derive(Debug, Clone)]
enum Origin {
    /// zip の何番目か
    Zip(usize),
    File(PathBuf),
}

/// 追加の下見（何という名前のフォルダに、何を写すか）
#[derive(Debug, Clone)]
pub struct Plan {
    pub source: PathBuf,
    pub kind: Source,
    /// 曲のフォルダの中に作るフォルダの名前
    pub name: String,
    /// 譜面（.dtx）の数
    pub charts: usize,
    /// 譜面のあるフォルダの数（だいたいの曲の数）
    pub song_dirs: usize,
    /// 写す大きさの合計
    pub bytes: u64,
    files: Vec<PlanFile>,
}

impl Plan {
    /// 写すファイルの数
    pub fn files(&self) -> usize {
        self.files.len()
    }
}

/// 追加した結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// 曲のフォルダの中に作った（同じものがあったなら、その）フォルダ
    pub dir: PathBuf,
    /// 譜面のパス（名前の順）
    pub charts: Vec<PathBuf>,
    /// 同じ中身のフォルダがすでにあったので写さなかった
    pub already: bool,
}

/// フォルダの中の、譜面のあるフォルダ（だいたいの曲）を数える。`limit` を超えたらそこでやめる
/// （曲のたくさん入ったフォルダを、写す前に見分けるため）
pub fn count_song_dirs(dir: &Path, limit: usize) -> usize {
    fn walk(dir: &Path, depth: usize, limit: usize, n: &mut usize) {
        if depth > MAX_DEPTH || *n > limit {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut has_chart = false;
        let mut dirs = Vec::new();
        for e in entries.flatten() {
            let Ok(meta) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if meta.is_dir() {
                dirs.push(e.path());
            } else if meta.is_file() && is_chart(&e.file_name().to_string_lossy()) {
                has_chart = true;
            }
        }
        if has_chart {
            *n += 1;
        }
        for d in dirs {
            walk(&d, depth + 1, limit, n);
        }
    }
    let mut n = 0;
    walk(dir, 0, limit, &mut n);
    n
}

/// 下見をする（zip は中身の目録だけ読む。フォルダはたどってファイルを数える）
pub fn plan(path: &Path) -> Result<Plan, ImportError> {
    match source_of(path)? {
        Source::Zip => plan_zip(path),
        Source::Folder => plan_folder(path),
    }
}

fn plan_zip(path: &Path) -> Result<Plan, ImportError> {
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    if zip.len() > MAX_FILES {
        return Err(ImportError::TooLarge);
    }
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let f = zip.by_index_raw(i)?;
        if f.is_dir() || f.is_symlink() {
            continue;
        }
        let name = decode_name(f.name_raw(), f.name());
        let Some(rel) = safe_parts(&name) else {
            log::warn!("import: skipped an unsafe name in the zip: {name:?}");
            continue;
        };
        if skipped(&rel) {
            continue;
        }
        files.push(PlanFile {
            rel,
            from: Origin::Zip(i),
            size: f.size(),
        });
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    finish_plan(path, Source::Zip, &stem, files, true)
}

fn plan_folder(path: &Path) -> Result<Plan, ImportError> {
    let mut files = Vec::new();
    walk(path, &mut Vec::new(), 0, &mut files)?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    finish_plan(path, Source::Folder, &name, files, false)
}

fn walk(
    dir: &Path,
    rel: &mut Vec<String>,
    depth: usize,
    out: &mut Vec<PlanFile>,
) -> Result<(), ImportError> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        // リンクはたどらない（輪になったり、思わぬ所を写したりしないように）
        let Ok(meta) = fs::symlink_metadata(e.path()) else {
            continue;
        };
        let name = e.file_name().to_string_lossy().into_owned();
        rel.push(name);
        if meta.is_dir() {
            if !skipped(rel) {
                walk(&e.path(), rel, depth + 1, out)?;
            }
        } else if meta.is_file() && !skipped(rel) {
            if out.len() >= MAX_FILES {
                return Err(ImportError::TooLarge);
            }
            out.push(PlanFile {
                rel: rel.clone(),
                from: Origin::File(e.path()),
                size: meta.len(),
            });
        }
        rel.pop();
    }
    Ok(())
}

/// 中身の並びから下見を仕上げる。zip で全部が 1 つのフォルダに入っていれば、そのフォルダの名前を使う。
/// そうでなければ（zip の一番上に譜面がじかにあるなど）`fallback`（zip の名前）のフォルダを作って入れる
fn finish_plan(
    source: &Path,
    kind: Source,
    fallback: &str,
    mut files: Vec<PlanFile>,
    strip_single_root: bool,
) -> Result<Plan, ImportError> {
    let mut name = sanitize_part(fallback);
    if strip_single_root
        && let Some(first) = files.first().map(|f| f.rel[0].clone())
        && files
            .iter()
            .all(|f| f.rel.len() > 1 && f.rel[0].to_lowercase() == first.to_lowercase())
    {
        name = first;
        for f in &mut files {
            f.rel.remove(0);
        }
    }
    let name = match name.as_str() {
        "" => "Song".to_owned(),
        _ => name,
    };
    // 曲を探すときは名前が `._` で始まるファイルを譜面としない
    let chart_dirs: Vec<&[String]> = files
        .iter()
        .filter(|f| is_chart(f.rel.last().map_or("", String::as_str)))
        .map(|f| &f.rel[..f.rel.len() - 1])
        .collect();
    if chart_dirs.is_empty() {
        return Err(ImportError::NoCharts);
    }
    let charts = chart_dirs.len();
    let song_dirs = chart_dirs.iter().collect::<HashSet<_>>().len();
    let bytes = files.iter().map(|f| f.size).sum::<u64>();
    if bytes > MAX_BYTES {
        return Err(ImportError::TooLarge);
    }
    Ok(Plan {
        source: source.to_owned(),
        kind,
        name,
        charts,
        song_dirs,
        bytes,
        files,
    })
}

/// 写さないもの（macOS が zip に入れる影のファイルなど）
fn skipped(rel: &[String]) -> bool {
    rel.iter().any(|p| p == "__MACOSX" || p == ".DS_Store")
        || rel.last().is_some_and(|n| n.starts_with(WORK_PREFIX))
}

fn is_chart(name: &str) -> bool {
    if name.starts_with("._") {
        return false;
    }
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| CHART_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// 曲を探すときに飛ばす、追加の途中の作業フォルダか
pub fn is_work_dir(name: &str) -> bool {
    name.starts_with(WORK_PREFIX)
}

/// zip の中の名前を文字にする。UTF-8 として読めればそれ（UTF-8 の印付きか、Info-ZIP の Unicode の名前があれば
/// zip のクレートがすでに UTF-8 にしている）、だめなら Shift_JIS、それもだめなら zip のクレートの読み方（CP437）
pub fn decode_name(raw: &[u8], fallback: &str) -> String {
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.to_owned();
    }
    match SHIFT_JIS.decode_without_bom_handling_and_without_replacement(raw) {
        Some(s) => s.into_owned(),
        None => fallback.to_owned(),
    }
}

/// zip の中の名前を、展開先の中だけを指す部分の並びにする。`..` があれば使わない（`None`）
pub fn safe_parts(name: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p => {
                let s = sanitize_part(p);
                if s.is_empty() {
                    return None;
                }
                out.push(s);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// ファイルやフォルダの名前 1 つを、どの OS でも使える形にする
pub fn sanitize_part(part: &str) -> String {
    let mut s: String = part
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows は名前の終わりの点と空白を黙って落とすので、ほかの名前と重ならないよう先に落とす
    while s.ends_with(['.', ' ']) {
        s.pop();
    }
    let s = s.trim_start().to_owned();
    if s.is_empty() || s.chars().all(|c| c == '.') {
        return String::new();
    }
    if is_reserved(&s) { format!("_{s}") } else { s }
}

/// Windows の予約名（CON、NUL、COM1 など。拡張子が付いていても使えない）
fn is_reserved(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or_default().trim_end();
    let upper = base.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.chars().count() == 4
            && upper
                .chars()
                .nth(3)
                .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)))
}

/// 写す。`dest_root`（曲のフォルダ）の中に [`Plan::name`] のフォルダを作る。
/// `progress` に写した大きさと合計を渡す。`stop` が立ったらやめて、作りかけを消す
pub fn import(
    plan: &Plan,
    dest_root: &Path,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Added, ImportError> {
    fs::create_dir_all(dest_root)?;
    let first = dest_root.join(&plan.name);
    if same_contents(plan, &first) {
        return Ok(Added {
            charts: chart_paths(plan, &first),
            dir: first,
            already: true,
        });
    }
    let work = work_dir(dest_root)?;
    let result = write_all(plan, &work, stop, progress).and_then(|()| {
        let dir = place(&work, dest_root, &plan.name)?;
        Ok(Added {
            charts: chart_paths(plan, &dir),
            dir,
            already: false,
        })
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&work);
    }
    result
}

/// 中身が同じフォルダがあるか（写すファイルが全部、同じ大きさであればそうとみなす）
fn same_contents(plan: &Plan, dir: &Path) -> bool {
    dir.is_dir()
        && plan.files.iter().all(|f| {
            fs::metadata(join(dir, &f.rel))
                .map(|m| m.is_file() && m.len() == f.size)
                .unwrap_or(false)
        })
}

fn join(dir: &Path, rel: &[String]) -> PathBuf {
    let mut p = dir.to_owned();
    p.extend(rel);
    p
}

fn chart_paths(plan: &Plan, dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = plan
        .files
        .iter()
        .filter(|f| is_chart(f.rel.last().map_or("", String::as_str)))
        .map(|f| join(dir, &f.rel))
        .collect();
    v.sort();
    v
}

/// 作業フォルダを作る（ほかと重ならない名前で）
fn work_dir(dest_root: &Path) -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let p = dest_root.join(format!("{WORK_PREFIX}{}-{n}", std::process::id()));
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// 作業フォルダに名前を付ける。同じ名前があれば「名前 (2)」「名前 (3)」…
fn place(work: &Path, dest_root: &Path, name: &str) -> io::Result<PathBuf> {
    for n in 1..1000 {
        let candidate = if n == 1 {
            dest_root.join(name)
        } else {
            dest_root.join(format!("{name} ({n})"))
        };
        if candidate.exists() {
            continue;
        }
        match rename_dir(work, &candidate) {
            Ok(()) => return Ok(candidate),
            // ほかが同じ名前を先に作った
            Err(_) if candidate.exists() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many folders with the same name",
    ))
}

/// フォルダの名前を変える。Windows では書いたばかりのファイルをウイルス対策ソフトなどが開いていて
/// 少しのあいだ断られることがあるので、何度か待ってやり直す
fn rename_dir(from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            r => return r,
        }
    }
}

fn write_all(
    plan: &Plan,
    work: &Path,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), ImportError> {
    let total = plan.bytes;
    let mut done = 0;
    progress(0, total);
    let mut zip = match plan.kind {
        Source::Zip => Some(zip::ZipArchive::new(File::open(&plan.source)?)?),
        Source::Folder => None,
    };
    for f in &plan.files {
        if stop.load(Ordering::Relaxed) {
            return Err(ImportError::Stopped);
        }
        let to = join(work, &f.rel);
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut out = File::create(&to)?;
        match (&f.from, &mut zip) {
            (Origin::Zip(i), Some(zip)) => {
                let entry = zip.by_index(*i)?;
                copy(entry, &mut out, f.size, stop, &mut |n| {
                    done += n;
                    progress(done, total);
                })?;
            }
            (Origin::File(p), _) => {
                copy(File::open(p)?, &mut out, f.size, stop, &mut |n| {
                    done += n;
                    progress(done, total);
                })?;
            }
            (Origin::Zip(_), None) => unreachable!("zip の下見に zip が無い"),
        }
        out.flush()?;
    }
    Ok(())
}

/// `limit` を超えて書こうとしたら壊れているとみなす（目録の大きさより大きく展開される zip で、
/// ディスクを埋めないように）
fn copy(
    mut from: impl Read,
    to: &mut File,
    limit: u64,
    stop: &AtomicBool,
    wrote: &mut dyn FnMut(u64),
) -> Result<(), ImportError> {
    let mut buf = vec![0u8; CHUNK];
    let mut n_total = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(ImportError::Stopped);
        }
        let n = match from.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return Err(ImportError::Broken(e.to_string()));
            }
            Err(e) => return Err(e.into()),
        };
        n_total += n as u64;
        if n_total > limit {
            return Err(ImportError::Broken("larger than its stated size".into()));
        }
        to.write_all(&buf[..n])?;
        wrote(n as u64);
    }
}

/// `inner` が `outer` の中（同じものも含む）にあるか。実際の場所（リンクをたどり、Windows では
/// `\\?\` の付いた形）で比べる。まだ無いフォルダは、あるところまでを実際の場所にして残りを足す
pub fn is_within(inner: &Path, outer: &Path) -> bool {
    real_path(inner).starts_with(real_path(outer))
}

fn real_path(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut base = p;
    loop {
        if let Ok(real) = fs::canonicalize(base) {
            let mut out = real;
            out.extend(rest.iter().rev());
            return out;
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                base = parent;
            }
            _ => return p.to_owned(),
        }
    }
}

/// フォルダを写すときに、写す先がそのフォルダの中なら写せない（自分の中へ写し続けることになる）
pub fn check_destination(plan: &Plan, dest_root: &Path) -> Result<(), ImportError> {
    if plan.kind == Source::Folder && is_within(dest_root, &plan.source) {
        return Err(ImportError::ContainsSongFolder);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試しの作業場所（終わったら消す）
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "nxt-import-{name}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// 中身 1 つ（名前はバイトのまま。`utf8` なら UTF-8 の印を立てる）
    struct Entry<'a> {
        name: &'a [u8],
        data: &'a [u8],
        utf8: bool,
    }

    fn entry<'a>(name: &'a str, data: &'a [u8]) -> Entry<'a> {
        Entry {
            name: name.as_bytes(),
            data,
            utf8: !name.is_ascii(),
        }
    }

    /// 無圧縮の zip を手で作る（名前のバイトと印を思いどおりにするため）
    fn make_zip(path: &Path, entries: &[Entry]) {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for e in entries {
            let offset = out.len() as u32;
            let flags: u16 = if e.utf8 { 1 << 11 } else { 0 };
            let crc = crc32(e.data);
            let size = e.data.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend(20u16.to_le_bytes());
            out.extend(flags.to_le_bytes());
            out.extend(0u16.to_le_bytes()); // 無圧縮
            out.extend(0u32.to_le_bytes()); // 時刻
            out.extend(crc.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend((e.name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(e.name);
            out.extend(e.data);

            central.extend(0x0201_4b50u32.to_le_bytes());
            out_central(&mut central, flags, crc, size, e.name, offset);
        }
        let cd_offset = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        fs::write(path, out).unwrap();
    }

    fn out_central(c: &mut Vec<u8>, flags: u16, crc: u32, size: u32, name: &[u8], offset: u32) {
        c.extend(20u16.to_le_bytes()); // 作った OS（DOS）と版
        c.extend(20u16.to_le_bytes());
        c.extend(flags.to_le_bytes());
        c.extend(0u16.to_le_bytes());
        c.extend(0u32.to_le_bytes());
        c.extend(crc.to_le_bytes());
        c.extend(size.to_le_bytes());
        c.extend(size.to_le_bytes());
        c.extend((name.len() as u16).to_le_bytes());
        c.extend(0u16.to_le_bytes()); // extra
        c.extend(0u16.to_le_bytes()); // comment
        c.extend(0u16.to_le_bytes()); // disk
        c.extend(0u16.to_le_bytes()); // internal
        c.extend(0u32.to_le_bytes()); // external
        c.extend(offset.to_le_bytes());
        c.extend(name);
    }

    fn sjis(s: &str) -> Vec<u8> {
        SHIFT_JIS.encode(s).0.into_owned()
    }

    fn run(plan: &Plan, dest: &Path) -> Result<Added, ImportError> {
        import(plan, dest, &AtomicBool::new(false), &mut |_, _| {})
    }

    /// `dir` の下のファイルを、`dir` からの相対パス（`/` 区切り）で
    fn tree(dir: &Path) -> Vec<String> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(base, &p, out);
                } else {
                    let rel = p.strip_prefix(base).unwrap();
                    let parts: Vec<_> = rel.iter().map(|s| s.to_string_lossy()).collect();
                    out.push(parts.join("/"));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn shift_jis_names_in_a_windows_zip() {
        let t = Temp::new("sjis");
        let zip = t.0.join("pack.zip");
        let folder = sjis("春の歌");
        let chart = [folder.clone(), b"/".to_vec(), sjis("譜面.dtx")].concat();
        let sound = [folder, b"/".to_vec(), sjis("スネア.wav")].concat();
        make_zip(
            &zip,
            &[
                Entry {
                    name: &chart,
                    data: b"#TITLE test",
                    utf8: false,
                },
                Entry {
                    name: &sound,
                    data: b"RIFF",
                    utf8: false,
                },
            ],
        );
        let plan = plan(&zip).unwrap();
        assert_eq!(plan.name, "春の歌");
        assert_eq!(plan.charts, 1);
        let dest = t.0.join("songs");
        let added = run(&plan, &dest).unwrap();
        assert_eq!(added.dir, dest.join("春の歌"));
        assert_eq!(added.charts, vec![dest.join("春の歌").join("譜面.dtx")]);
        assert!(!added.already);
        assert_eq!(tree(&dest), ["春の歌/スネア.wav", "春の歌/譜面.dtx"]);
        assert_eq!(
            fs::read(dest.join("春の歌/譜面.dtx")).unwrap(),
            b"#TITLE test"
        );
    }

    #[test]
    fn utf8_names_are_kept() {
        let t = Temp::new("utf8");
        let zip = t.0.join("x.zip");
        make_zip(&zip, &[entry("曲 ♪/master.dtx", b"a")]);
        let plan = plan(&zip).unwrap();
        assert_eq!(plan.name, "曲 ♪");
    }

    #[test]
    fn names_never_point_outside_the_destination() {
        let t = Temp::new("slip");
        let zip = t.0.join("evil.zip");
        make_zip(
            &zip,
            &[
                entry("song.dtx", b"ok"),
                entry("../escape.dtx", b"x"),
                entry("a/../../escape2.dtx", b"x"),
                entry("..\\escape3.dtx", b"x"),
                entry("/abs/root.wav", b"r"),
                entry("C:\\Windows\\win.wav", b"w"),
                entry("\\\\server\\share\\unc.wav", b"u"),
                entry("CON.wav", b"c"),
                entry("trail. ", b"t"),
                entry("__MACOSX/._song.dtx", b"m"),
                entry(".DS_Store", b"d"),
            ],
        );
        let plan = plan(&zip).unwrap();
        // 中身が 1 つのフォルダに入っていないので、zip の名前のフォルダを作る
        assert_eq!(plan.name, "evil");
        let dest = t.0.join("songs");
        run(&plan, &dest).unwrap();
        assert_eq!(
            tree(&t.0),
            [
                "evil.zip",
                "songs/evil/C_/Windows/win.wav",
                "songs/evil/_CON.wav",
                "songs/evil/abs/root.wav",
                "songs/evil/server/share/unc.wav",
                "songs/evil/song.dtx",
                "songs/evil/trail",
            ]
        );
    }

    #[test]
    fn a_zip_without_charts_is_refused() {
        let t = Temp::new("nochart");
        let zip = t.0.join("music.zip");
        make_zip(&zip, &[entry("a/song.ogg", b"x")]);
        assert!(matches!(plan(&zip), Err(ImportError::NoCharts)));
    }

    #[test]
    fn other_archives_are_unsupported() {
        let t = Temp::new("rar");
        let rar = t.0.join("song.rar");
        fs::write(&rar, b"Rar!").unwrap();
        assert!(matches!(plan(&rar), Err(ImportError::Unsupported(e)) if e == "rar"));
    }

    #[test]
    fn a_broken_zip_is_reported() {
        let t = Temp::new("broken");
        let zip = t.0.join("broken.zip");
        fs::write(&zip, b"PK\x03\x04 not really a zip").unwrap();
        assert!(matches!(plan(&zip), Err(ImportError::Broken(_))));
    }

    #[test]
    fn the_same_zip_twice_reuses_the_folder_and_a_different_one_gets_a_new_name() {
        let t = Temp::new("twice");
        let dest = t.0.join("songs");
        let a = t.0.join("a.zip");
        make_zip(&a, &[entry("Song/x.dtx", b"one")]);
        let first = run(&plan(&a).unwrap(), &dest).unwrap();
        let again = run(&plan(&a).unwrap(), &dest).unwrap();
        assert!(again.already);
        assert_eq!(again.dir, first.dir);

        let b = t.0.join("b.zip");
        make_zip(&b, &[entry("Song/x.dtx", b"other")]);
        let other = run(&plan(&b).unwrap(), &dest).unwrap();
        assert!(!other.already);
        assert_eq!(other.dir, dest.join("Song (2)"));
        // 前からある曲は書き換えない
        assert_eq!(fs::read(dest.join("Song/x.dtx")).unwrap(), b"one");
        assert_eq!(tree(&dest), ["Song (2)/x.dtx", "Song/x.dtx"]);
    }

    #[test]
    fn stopping_leaves_nothing_behind() {
        let t = Temp::new("stop");
        let zip = t.0.join("s.zip");
        make_zip(&zip, &[entry("S/x.dtx", b"x")]);
        let dest = t.0.join("songs");
        let r = import(
            &plan(&zip).unwrap(),
            &dest,
            &AtomicBool::new(true),
            &mut |_, _| {},
        );
        assert!(matches!(r, Err(ImportError::Stopped)));
        assert!(tree(&dest).is_empty());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn a_folder_is_copied_and_links_are_not_followed() {
        let t = Temp::new("folder");
        let src = t.0.join("My Song");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("bsc.dtx"), b"b").unwrap();
        fs::write(src.join("sub/bgm.ogg"), b"o").unwrap();
        fs::write(src.join(".DS_Store"), b"d").unwrap();
        let plan = plan(&src).unwrap();
        assert_eq!(plan.kind, Source::Folder);
        assert_eq!(plan.name, "My Song");
        assert_eq!((plan.charts, plan.song_dirs, plan.files()), (1, 1, 2));
        let dest = t.0.join("songs");
        check_destination(&plan, &dest).unwrap();
        let added = run(&plan, &dest).unwrap();
        assert_eq!(added.charts, vec![dest.join("My Song/bsc.dtx")]);
        assert_eq!(tree(&dest), ["My Song/bsc.dtx", "My Song/sub/bgm.ogg"]);
        // 元のフォルダはそのまま
        assert!(src.join("bsc.dtx").is_file());
    }

    #[test]
    fn a_folder_holding_the_destination_is_refused() {
        let t = Temp::new("inside");
        fs::write(t.0.join("a.dtx"), b"a").unwrap();
        let plan = plan(&t.0).unwrap();
        // 追加先がまだ無くても、ドロップしたフォルダの中なら分かる
        let dest = t.0.join("not yet").join("songs");
        assert!(is_within(&dest, &t.0));
        assert!(!is_within(&t.0, &dest));
        assert!(matches!(
            check_destination(&plan, &dest),
            Err(ImportError::ContainsSongFolder)
        ));
    }

    #[test]
    fn work_folders_are_hidden_from_the_song_scan() {
        let t = Temp::new("scan");
        let work = t.0.join(format!("{WORK_PREFIX}1-0"));
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("a.dtx"), b"#TITLE a").unwrap();
        let lib = crate::scan(
            std::slice::from_ref(&t.0),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(lib.songs, 0);
    }

    #[test]
    fn copying_more_than_stated_is_an_error() {
        let t = Temp::new("bomb");
        let mut out = File::create(t.0.join("out")).unwrap();
        let r = copy(
            &[0u8; 100][..],
            &mut out,
            10,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(matches!(r, Err(ImportError::Broken(_))));
    }

    #[test]
    fn names_are_made_safe_for_windows() {
        assert_eq!(sanitize_part("a:b*c?"), "a_b_c_");
        assert_eq!(sanitize_part("name. . "), "name");
        assert_eq!(sanitize_part("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_part("COM1"), "_COM1");
        assert_eq!(sanitize_part("COM10"), "COM10");
        assert_eq!(sanitize_part("console"), "console");
        assert_eq!(sanitize_part("..."), "");
        assert_eq!(
            safe_parts("a/./b//c"),
            Some(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(safe_parts("a/../b"), None);
    }

    #[test]
    fn names_are_read_as_utf8_then_shift_jis() {
        assert_eq!(decode_name("曲".as_bytes(), "?"), "曲");
        assert_eq!(decode_name(&sjis("曲"), "?"), "曲");
        // どちらでも読めなければ zip のクレートの読み方
        assert_eq!(decode_name(&[0x81], "fallback"), "fallback");
    }
}
