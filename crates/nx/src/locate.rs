//! ほかの DTXMania の場所を、ユーザーが指定したもの（ショートカット・exe・フォルダ・設定のファイル）から探す。
//!
//! DTXMania はどの派生も、exe のあるフォルダに設定を置く（Config.ini。本家の 1xx 系だけ Config.xml）。
//! 指定したフォルダに設定が無ければ、すぐ下（2 段まで）の DTXMania を探す（ゲームを集めたフォルダを選んだときなど）。

use std::path::{Path, PathBuf};

use crate::find_file;
use crate::lnk;

/// 設定のファイルの形
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Config.ini（NX、XG、本家 099 系）
    Ini,
    /// Config.xml（本家 1xx 系）
    Xml,
}

/// 見つけた DTXMania
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// DTXMania のフォルダ
    pub root: PathBuf,
    /// 設定のファイル
    pub config: PathBuf,
    pub format: Format,
    /// フォルダにある DTXMania の exe の名前（無ければ `None`）
    pub exe: Option<String>,
    /// ショートカットから見つけたなら、そのショートカット
    pub shortcut: Option<PathBuf>,
}

/// 見つからなかった理由
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocateError {
    /// 指定したものが無い
    Missing(PathBuf),
    /// ショートカットとして読めない
    BadShortcut(PathBuf),
    /// ショートカットの行き先が無い（消したか動かした）
    ShortcutTargetMissing { shortcut: PathBuf, target: PathBuf },
    /// DTXMania の exe はあるが設定のファイルが無い（一度も起動していないなど）
    NoConfig { root: PathBuf },
    /// DTXMania のものではない
    NotDtxmania(PathBuf),
    /// このアプリ（DTXmaniaNXT）のもの
    ItsUs(PathBuf),
    /// 中に DTXMania がいくつもある（どれか選んでもらう）
    Several(Vec<Found>),
}

/// 指定したもの（ショートカット・exe・フォルダ・設定のファイル）から DTXMania を探す
pub fn locate(path: &Path) -> Result<Found, LocateError> {
    if !path.exists() {
        return Err(LocateError::Missing(path.to_owned()));
    }
    let ext = extension(path);
    if path.is_file() && ext == "lnk" {
        return via_shortcut(path);
    }
    if path.is_file() && ext == "desktop" {
        return via_desktop_entry(path);
    }
    locate_target(path).map_err(|e| match e {
        // NXT 自身の設定のフォルダや exe
        LocateError::NotDtxmania(p) if is_nxt(&p) => LocateError::ItsUs(p),
        e => e,
    })
}

/// ショートカットの行き先から探す
fn via_shortcut(lnk_path: &Path) -> Result<Found, LocateError> {
    let bytes =
        std::fs::read(lnk_path).map_err(|_| LocateError::BadShortcut(lnk_path.to_owned()))?;
    let dir = lnk_path.parent().unwrap_or(Path::new("."));
    let shortcut =
        lnk::read(&bytes, dir).map_err(|_| LocateError::BadShortcut(lnk_path.to_owned()))?;
    let with_shortcut = |mut f: Found| {
        f.shortcut = Some(lnk_path.to_owned());
        f
    };
    if let Some(target) = shortcut.existing_target() {
        match locate_target(target) {
            Ok(f) => return Ok(with_shortcut(f)),
            // 行き先のフォルダに設定が無くても、作業フォルダにあればそちら
            Err(e) => {
                if let Some(w) = shortcut.working_dir.as_deref().filter(|w| w.is_dir())
                    && let Ok(f) = locate_target(w)
                {
                    return Ok(with_shortcut(f));
                }
                return Err(match e {
                    LocateError::NotDtxmania(p) if is_nxt(&p) => LocateError::ItsUs(p),
                    e => e,
                });
            }
        }
    }
    if let Some(w) = shortcut.working_dir.as_deref().filter(|w| w.is_dir())
        && let Ok(f) = locate_target(w)
    {
        return Ok(with_shortcut(f));
    }
    let target = shortcut
        .targets
        .first()
        .cloned()
        .or(shortcut.working_dir)
        .unwrap_or_default();
    Err(LocateError::ShortcutTargetMissing {
        shortcut: lnk_path.to_owned(),
        target,
    })
}

/// Linux のデスクトップのエントリ（Wine で動かしている DTXMania など）。作業フォルダ（`Path=`）から探す
fn via_desktop_entry(path: &Path) -> Result<Found, LocateError> {
    let text =
        std::fs::read_to_string(path).map_err(|_| LocateError::BadShortcut(path.to_owned()))?;
    let dir = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Path="))
        .map(|p| PathBuf::from(p.trim()))
        .filter(|p| !p.as_os_str().is_empty());
    match dir {
        Some(d) if d.is_dir() => locate_target(&d).map(|mut f| {
            f.shortcut = Some(path.to_owned());
            f
        }),
        Some(d) => Err(LocateError::ShortcutTargetMissing {
            shortcut: path.to_owned(),
            target: d,
        }),
        None => Err(LocateError::BadShortcut(path.to_owned())),
    }
}

/// ファイルかフォルダから探す
fn locate_target(path: &Path) -> Result<Found, LocateError> {
    if path.is_file() {
        let name = file_name(path).to_lowercase();
        let root = path.parent().unwrap_or(Path::new(".")).to_owned();
        return match name.as_str() {
            "config.ini" => Ok(found(&root, path.to_owned(), Format::Ini)),
            "config.xml" => Ok(found(&root, path.to_owned(), Format::Xml)),
            _ if extension(path) == "exe" => {
                if is_nxt(path) {
                    return Err(LocateError::ItsUs(path.to_owned()));
                }
                match in_folder(&root) {
                    Some(f) => Ok(f),
                    None if is_dtxmania_exe(&name) => Err(LocateError::NoConfig { root }),
                    None => Err(LocateError::NotDtxmania(path.to_owned())),
                }
            }
            _ => Err(LocateError::NotDtxmania(path.to_owned())),
        };
    }
    if let Some(f) = in_folder(path) {
        return Ok(f);
    }
    if exe_in(path).is_some() {
        return Err(LocateError::NoConfig {
            root: path.to_owned(),
        });
    }
    let below = search_below(path, 2);
    match below.len() {
        0 => Err(LocateError::NotDtxmania(path.to_owned())),
        1 => Ok(below.into_iter().next().unwrap_or_else(|| unreachable!())),
        _ => Err(LocateError::Several(below)),
    }
}

/// フォルダそのものが DTXMania のフォルダなら
pub fn in_folder(dir: &Path) -> Option<Found> {
    if let Some(ini) = find_file(dir, "Config.ini").filter(|p| p.is_file())
        && looks_like_dtxmania_ini(&ini)
    {
        return Some(found(dir, ini, Format::Ini));
    }
    let xml = find_file(dir, "Config.xml").filter(|p| p.is_file())?;
    // Config.xml はほかのアプリも使う名前なので、DTXMania の exe があるときだけ
    exe_in(dir)?;
    Some(found(dir, xml, Format::Xml))
}

/// `dir` の下 `depth` 段までの DTXMania のフォルダ（名前の順）
fn search_below(dir: &Path, depth: usize) -> Vec<Found> {
    let mut out = Vec::new();
    if depth == 0 {
        return out;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut subdirs: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .take(500)
        .collect();
    subdirs.sort();
    for sub in subdirs {
        if let Some(f) = in_folder(&sub) {
            out.push(f);
        } else {
            out.extend(search_below(&sub, depth - 1));
        }
    }
    out
}

fn found(root: &Path, config: PathBuf, format: Format) -> Found {
    Found {
        root: root.to_owned(),
        config,
        format,
        exe: exe_in(root),
        shortcut: None,
    }
}

/// Config.ini の頭を読んで、DTXMania のものか確かめる
fn looks_like_dtxmania_ini(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| crate::Ini::parse(&dtx::decode(&b).0).looks_like_dtxmania())
        .unwrap_or(false)
}

/// フォルダにある DTXMania の exe の名前
fn exe_in(dir: &Path) -> Option<String> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| n.to_lowercase().ends_with(".exe") && is_dtxmania_exe(&n.to_lowercase()))
        .collect();
    names.sort();
    names.into_iter().next()
}

/// DTXMania の exe の名前か（`DTXManiaNX.exe`、`DTXManiaGR.exe`、`DTXMania.exe` など。小文字で渡す）
fn is_dtxmania_exe(lower: &str) -> bool {
    lower.starts_with("dtxmania") && !lower.contains("nxt") && lower.ends_with(".exe")
}

/// このアプリ（DTXmaniaNXT）のものか
fn is_nxt(path: &Path) -> bool {
    path.ancestors().take(3).any(|p| {
        let n = file_name(p).to_lowercase();
        n.contains("dtxmanianxt")
    }) || (path.is_dir() && find_file(path, "config.toml").is_some())
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn extension(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// よくある場所から DTXMania を探す（設定の画面の「見つかった DTXMania」）。
/// `shortcut_dirs` はショートカットを探すフォルダ（デスクトップ、スタートメニュー）、`folder_dirs` は
/// DTXMania のフォルダを探すフォルダ（デスクトップ、ドキュメントなど。2 段下まで）。同じフォルダは 1 つにする
pub fn discover(shortcut_dirs: &[PathBuf], folder_dirs: &[PathBuf]) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let push = |f: Found, out: &mut Vec<Found>| {
        if !out.iter().any(|o| same_dir(&o.root, &f.root)) {
            out.push(f);
        }
    };
    for dir in shortcut_dirs {
        for lnk in shortcuts_below(dir, 3) {
            if let Ok(f) = via_shortcut(&lnk) {
                push(f, &mut out);
            }
        }
    }
    for dir in folder_dirs {
        if let Some(f) = in_folder(dir) {
            push(f, &mut out);
        }
        for f in search_below(dir, 2) {
            push(f, &mut out);
        }
    }
    out
}

/// `dir` の下 `depth` 段までの、名前に DTXMania を含むショートカット
fn shortcuts_below(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .take(2000)
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if depth > 1 {
                out.extend(shortcuts_below(&p, depth - 1));
            }
        } else if extension(&p) == "lnk" && file_name(&p).to_lowercase().contains("dtx") {
            out.push(p);
        }
    }
    out
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    canon(a) == canon(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lnk::tests::Builder;

    const INI: &str = "[System]\nVersion=v1.4.2 20240519\n[DrumsKeyAssign]\nHH=M042\n";

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nx-locate-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn install(dir: &Path, exe: &str, ini: Option<&str>) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(exe), b"MZ").unwrap();
        if let Some(ini) = ini {
            std::fs::write(dir.join("Config.ini"), ini).unwrap();
        }
    }

    #[test]
    fn finds_from_a_folder_an_exe_and_the_config() {
        let t = temp("basic");
        let nx = t.join("DTXManiaNX");
        install(&nx, "DTXManiaNX.exe", Some(INI));
        for p in [nx.clone(), nx.join("DTXManiaNX.exe"), nx.join("Config.ini")] {
            let f = locate(&p).unwrap();
            assert_eq!(f.root, nx);
            assert_eq!(f.format, Format::Ini);
            assert_eq!(f.exe.as_deref(), Some("DTXManiaNX.exe"));
        }
        // 上のフォルダを選んでも、下に 1 つだけならそれ
        assert_eq!(locate(&t).unwrap().root, nx);
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn explains_why_nothing_was_found() {
        let t = temp("errors");
        assert_eq!(
            locate(&t.join("nope")),
            Err(LocateError::Missing(t.join("nope")))
        );
        // まだ起動していない DTXMania
        let fresh = t.join("fresh");
        install(&fresh, "DTXManiaGR.exe", None);
        assert_eq!(
            locate(&fresh.join("DTXManiaGR.exe")),
            Err(LocateError::NoConfig {
                root: fresh.clone()
            })
        );
        assert_eq!(locate(&fresh), Err(LocateError::NoConfig { root: fresh }));
        // ほかのアプリの Config.ini
        let other = t.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("Config.ini"), "[Settings]\na=1\n").unwrap();
        std::fs::write(other.join("Game.exe"), b"MZ").unwrap();
        assert_eq!(
            locate(&other.join("Game.exe")),
            Err(LocateError::NotDtxmania(other.join("Game.exe")))
        );
        assert_eq!(locate(&other), Err(LocateError::NotDtxmania(other.clone())));
        // NXT 自身
        let nxt = t.join("DTXmaniaNXT");
        std::fs::create_dir_all(&nxt).unwrap();
        std::fs::write(nxt.join("config.toml"), "").unwrap();
        assert!(matches!(locate(&nxt), Err(LocateError::ItsUs(_))));
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn several_installs_are_listed() {
        let t = temp("several");
        install(&t.join("a").join("NX"), "DTXManiaNX.exe", Some(INI));
        install(&t.join("b"), "DTXManiaGR.exe", Some(INI));
        let Err(LocateError::Several(list)) = locate(&t) else {
            panic!("several");
        };
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].root, t.join("a").join("NX"));
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn follows_shortcuts_to_the_folder() {
        let t = temp("lnk");
        let nx = t.join("Games").join("DTXManiaNX");
        install(&nx, "DTXManiaNX.exe", Some(INI));
        let desktop = t.join("Desktop");
        std::fs::create_dir_all(&desktop).unwrap();
        let exe = nx.join("DTXManiaNX.exe");
        let lnk = Builder {
            local_ansi: Some(exe.to_string_lossy().as_bytes().to_vec()),
            working: Some(nx.to_string_lossy().into_owned()),
            ..Default::default()
        }
        .build();
        let lnk_path = desktop.join("DTXManiaNX.lnk");
        std::fs::write(&lnk_path, &lnk).unwrap();
        let f = locate(&lnk_path).unwrap();
        assert_eq!(f.root, nx);
        assert_eq!(f.shortcut.as_deref(), Some(lnk_path.as_path()));
        // デスクトップのショートカットから見つける
        let list = discover(std::slice::from_ref(&desktop), &[]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].root, nx);

        // 行き先を消したショートカット
        let gone = Builder {
            local_ansi: Some(b"Z:\\Gone\\DTXManiaNX.exe".to_vec()),
            ..Default::default()
        }
        .build();
        let gone_path = desktop.join("Old DTXMania.lnk");
        std::fs::write(&gone_path, &gone).unwrap();
        assert!(matches!(
            locate(&gone_path),
            Err(LocateError::ShortcutTargetMissing { .. })
        ));
        // 壊れたショートカット
        let broken = desktop.join("broken.lnk");
        std::fs::write(&broken, b"not a shortcut").unwrap();
        assert_eq!(
            locate(&broken),
            Err(LocateError::BadShortcut(broken.clone()))
        );
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn config_xml_needs_a_dtxmania_exe() {
        let t = temp("xml");
        let classic = t.join("DTXMania");
        std::fs::create_dir_all(&classic).unwrap();
        std::fs::write(classic.join("Config.xml"), "<CConfigXml/>").unwrap();
        assert!(in_folder(&classic).is_none());
        std::fs::write(classic.join("DTXManiaGR.exe"), b"MZ").unwrap();
        assert_eq!(in_folder(&classic).unwrap().format, Format::Xml);
        std::fs::remove_dir_all(&t).unwrap();
    }
}
