//! 開発者モードで作る報告（GitHub の issue の下書き）。
//!
//! 電子ドラムをつないだ PC と開発に使う PC が別のとき、ドラム側の PC で調べた結果（MIDI の記録、ログ、
//! 道具で測った数字）を issue にして開発側へ渡す（2026-11-25 ユーザー依頼）。
//!
//! - 送り方: 本文を入れた「新しい issue」の画面をブラウザで開き、中身を確かめて送ってもらう。exe に
//!   GitHub のトークンは入れない。URL が長すぎるときは本文をクリップボードに入れ、貼り付けてもらう
//!   （VS Code の issue の画面と同じやり方）
//! - 伏せるもの: ユーザー名を含むフォルダの場所、PC の名前、メールアドレス。公開のリポジトリなので、
//!   伏せたあとの本文をそのまま画面で見せ、送る前に確かめてもらう
//! - 報告はいつも設定のフォルダの `reports` にも保存する（ファイルを issue に添えたり、別の手段で運んだりできる）
//!
//! 調べる道具は [`TOOLS`] に並べる。道具は別のスレッドで動く同期の処理で、終わったら [`Finding`] を返す。

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use i18n::{Tr, t, tf};

/// issue を作るリポジトリの「新しい issue」の画面
pub const NEW_ISSUE_URL: &str = "https://github.com/check5004/DTXmaniaNXT/issues/new";
/// ブラウザに渡す URL の長さの上限。GitHub は 8 KB ほどで 414 を返すので余裕を見る
pub const MAX_URL: usize = 7000;
/// 道具が返す本文の上限（文字）。issue の本文の上限は 65,536 文字で、環境の節を足す分を空けておく
pub const MAX_FINDING: usize = 50_000;
/// issue の題の頭に付ける印（あとで探しやすいように）
pub const TITLE_MARK: &str = "[dev]";
/// 報告を保存するフォルダ（設定のフォルダの中）
pub const REPORTS_DIR: &str = "reports";

/// 道具が調べた結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// issue の題（[`TITLE_MARK`] はこちらで付ける）
    pub title: String,
    /// issue の本文（Markdown、[`MAX_FINDING`] 文字まで。超えた分は切る）
    pub body: String,
}

/// 開発者メニューに並べる道具 1 つ
pub struct Tool {
    /// 保存するファイルの名前に使う（英小文字とハイフン）
    pub key: &'static str,
    /// 一覧に出す名前
    pub name: Tr,
    /// 説明（右の板に出す）
    pub desc: Tr,
    /// 始める前に確かめること（機器のつなぎ方、音が鳴るなど）。あれば確かめの板を出す
    pub confirm: Option<Tr>,
    /// issue に付けるラベル（リポジトリに無いものは GitHub が無視する）
    pub labels: &'static [&'static str],
    /// 調べる。別のスレッドで呼ぶ。`cancel` が立ったらできるだけ早く（後片付けをしてから）戻る。
    /// `progress` には画面に出す 1 行の進み具合を渡す
    pub run: fn(&AtomicBool, &mut dyn FnMut(String)) -> Finding,
}

/// 開発者メニューの道具（組み込みの「環境の情報」「MIDI を記録」「ログ」のほか）。
/// 新しい道具はここに足す
pub const TOOLS: &[Tool] = &[crate::module_probe::TOOL];

/// 環境の節に出す行（名前と値）。App が今の設定と機器から作る
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    pub rows: Vec<(String, String)>,
}

impl Environment {
    /// 環境の節の Markdown
    pub fn markdown(&self) -> String {
        let mut s = format!(
            "### {}\n\n| {} | {} |\n|---|---|\n",
            t("Environment", "環境"),
            t("Item", "項目"),
            t("Value", "値")
        );
        for (k, v) in &self.rows {
            let _ = writeln!(s, "| {} | {} |", cell(k), cell(v));
        }
        s
    }
}

/// 表の 1 マスに入れられるようにする（縦棒と改行）
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

/// 伏せるもの（この PC のホームフォルダと PC の名前）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    pub home: Option<String>,
    pub host: Option<String>,
}

impl Names {
    /// この PC のもの
    pub fn current() -> Self {
        let host = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .ok()
            .or_else(|| {
                std::fs::read_to_string("/etc/hostname")
                    .ok()
                    .map(|s| s.trim().to_owned())
            });
        Self {
            home: dirs::home_dir().map(|p| p.to_string_lossy().into_owned()),
            host,
        }
        .usable()
    }

    /// 短すぎて関係の無い所まで伏せてしまうものは使わない
    fn usable(mut self) -> Self {
        self.home = self
            .home
            .filter(|h| h.trim_end_matches(['/', '\\']).len() >= 4);
        self.host = self.host.filter(|h| h.chars().count() >= 3);
        self
    }
}

/// 送る前に、名前の分かるものを伏せる
/// - ホームフォルダ → `~`
/// - `\Users\名前\`・`/Users/名前/`・`/home/名前/` の名前 → `<user>`（ほかの人のフォルダも）
/// - PC の名前 → `<pc>`
/// - メールアドレス → `<email>`
pub fn redact(text: &str, names: &Names) -> String {
    let mut s = text.to_owned();
    if let Some(home) = &names.home {
        let home = home.trim_end_matches(['/', '\\']);
        for h in [
            home.to_owned(),
            home.replace('\\', "/"),
            home.replace('/', "\\"),
        ] {
            s = replace_ci(&s, &h, "~");
        }
    }
    s = mask_user_dirs(&s);
    if let Some(host) = &names.host {
        s = replace_word_ci(&s, host, "<pc>");
    }
    mask_emails(&s)
}

/// 大文字と小文字を区別せずに（ASCII だけ）置き換える
fn replace_ci(s: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return s.to_owned();
    }
    let hay = s.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    while let Some(i) = hay[at..].find(&needle) {
        out.push_str(&s[at..at + i]);
        out.push_str(to);
        at += i + needle.len();
    }
    out.push_str(&s[at..]);
    out
}

/// 語として出てくる所だけ置き換える（前後が英数字・`-`・`_` でない）
fn replace_word_ci(s: &str, from: &str, to: &str) -> String {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_');
    let hay = s.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    let mut from_i = 0;
    while let Some(i) = hay[from_i..].find(&needle) {
        let start = from_i + i;
        let end = start + needle.len();
        if !word(s[..start].chars().next_back()) && !word(s[end..].chars().next()) {
            out.push_str(&s[at..start]);
            out.push_str(to);
            at = end;
        }
        from_i = end;
    }
    out.push_str(&s[at..]);
    out
}

/// ユーザーのフォルダの名前を伏せる
fn mask_user_dirs(s: &str) -> String {
    const MARKS: [&str; 3] = ["\\users\\", "/users/", "/home/"];
    const USER: &str = "<user>";
    let hay = s.to_ascii_lowercase();
    // 伏せる範囲（名前の始まりと終わり）
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for mark in MARKS {
        let mut from = 0;
        while let Some(i) = hay[from..].find(mark) {
            let start = from + i + mark.len();
            let end = s[start..]
                .find(|c: char| {
                    matches!(
                        c,
                        '\\' | '/' | '"' | '\'' | '`' | '|' | ')' | ']' | '>' | '<'
                    ) || c.is_whitespace()
                })
                .map_or(s.len(), |j| start + j);
            if end > start && &s[start..end] != USER {
                spans.push((start, end));
            }
            from = start;
        }
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for (start, end) in spans {
        if start < at {
            continue;
        }
        out.push_str(&s[at..start]);
        out.push_str(USER);
        at = end;
    }
    out.push_str(&s[at..]);
    out
}

/// メールアドレスを伏せる
fn mask_emails(s: &str) -> String {
    let local = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
    let domain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    let mut search = 0;
    while let Some(i) = s[search..].find('@') {
        let atmark = search + i;
        let start = s[..atmark]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| local(c))
            .last()
            .map_or(atmark, |(j, _)| j);
        let end = s[atmark + 1..]
            .char_indices()
            .take_while(|&(_, c)| domain(c))
            .last()
            .map_or(atmark + 1, |(j, c)| atmark + 1 + j + c.len_utf8());
        let host = s[atmark + 1..end].trim_end_matches('.');
        let looks_like = start < atmark
            && start >= at
            && host.contains('.')
            && !host.starts_with('.')
            && host.rsplit('.').next().is_some_and(|tld| tld.len() >= 2);
        if looks_like {
            out.push_str(&s[at..start]);
            out.push_str("<email>");
            at = atmark + 1 + host.len();
        }
        search = atmark + 1;
    }
    out.push_str(&s[at..]);
    out
}

/// 本文を `max` 文字までにする（行の切れ目で切り、切ったことを書き添える）
pub fn clip(body: &str, max: usize) -> String {
    if body.chars().count() <= max {
        return body.to_owned();
    }
    let cut = body.char_indices().nth(max).map_or(body.len(), |(i, _)| i);
    let head = &body[..cut];
    let head = head.rfind('\n').map_or(head, |i| &head[..i]);
    // コードの囲みの途中で切ったら閉じておく
    let fences = head
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count();
    let close = if fences % 2 == 1 { "\n```" } else { "" };
    format!(
        "{head}{close}\n\n{}",
        t(
            "_(Cut because it was too long. The saved file has the rest.)_",
            "_（長いので切りました。残りは保存したファイルにあります）_"
        )
    )
}

/// issue の下書き 1 つ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// 保存するファイルの名前に使う
    pub key: String,
    /// issue の題（伏せたあと、印つき）
    pub title: String,
    pub labels: Vec<String>,
    /// issue の本文（伏せたあと、環境の節つき、上限に収めたもの）
    pub body: String,
    /// 保存した本文（伏せたあと、切っていないもの）
    full: String,
    /// 保存したファイル
    pub saved: Option<PathBuf>,
}

/// ブラウザで issue を作るときの開き方
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Send {
    /// 本文ごと URL に入る
    Url(String),
    /// URL に入らないので、本文をクリップボードに入れてから、貼り付けを頼む本文で開く
    Paste { url: String, body: String },
}

impl Report {
    pub fn new(
        key: &str,
        finding: Finding,
        labels: &[&str],
        env: &Environment,
        names: &Names,
    ) -> Self {
        let title = format!("{TITLE_MARK} {}", finding.title.trim());
        let compose = |body: &str| {
            format!(
                "{}\n\n---\n\n{}\n{}\n",
                body.trim_end(),
                env.markdown(),
                tf!(
                    "_Made in the developer menu of DTXmaniaNXT {}._",
                    "_DTXmaniaNXT {} の開発者メニューで作りました。_",
                    env!("CARGO_PKG_VERSION")
                )
            )
        };
        let full = redact(&compose(&finding.body), names);
        let body = redact(&compose(&clip(&finding.body, MAX_FINDING)), names);
        Self {
            key: key.to_owned(),
            title: redact(&title, names),
            labels: labels.iter().map(|&l| l.to_owned()).collect(),
            body,
            full,
            saved: None,
        }
    }

    /// 題と本文をまとめた Markdown（コピーと保存に使う）
    pub fn markdown(&self) -> String {
        format!("# {}\n\n{}", self.title, self.full)
    }

    /// ブラウザで開く URL
    pub fn send(&self) -> Send {
        let url = issue_url(&self.title, &self.body, &self.labels);
        if url.len() <= MAX_URL {
            return Send::Url(url);
        }
        let note = t(
            "(The report was too long for the link, so the game copied it to your clipboard. Select this text and paste with Ctrl+V.)",
            "（本文が長いので、ゲームがクリップボードに入れました。この文を選んで Ctrl+V で貼り付けてください）",
        );
        Send::Paste {
            url: issue_url(&self.title, note, &self.labels),
            body: self.body.clone(),
        }
    }

    /// `dir` に保存する（ファイルの名前は日時と道具の名前）
    pub fn save(&mut self, dir: &Path) -> io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = dir.join(format!("{stamp}-{}.md", self.key));
        std::fs::write(&path, self.markdown())?;
        self.saved = Some(path.clone());
        Ok(path)
    }
}

/// 「新しい issue」の画面の URL（題・本文・ラベルを入れたもの）
pub fn issue_url(title: &str, body: &str, labels: &[String]) -> String {
    let mut url = format!(
        "{NEW_ISSUE_URL}?title={}&body={}",
        encode(title),
        encode(body)
    );
    if !labels.is_empty() {
        url.push_str("&labels=");
        url.push_str(&encode(&labels.join(",")));
    }
    url
}

/// URL の値に入れられるようにする（英数字と `-._~` のほかは %XX）
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// クリップボード（Linux ではこれを持っているあいだだけ中身が残るので、閉じずに持っておく）
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// クリップボードに文字を入れる
pub fn copy(text: &str) -> Result<(), String> {
    let mut slot = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
    }
    slot.as_mut()
        .expect("the clipboard was just opened")
        .set_text(text)
        .map_err(|e| e.to_string())
}

/// ファイルをエクスプローラー（Finder、ファイルマネージャー）で見せる
pub fn reveal(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .spawn()?;
    }
    Ok(())
}

/// ログのファイルの最後の `lines` 行から報告を作る（`prev` なら前回のログ）
pub fn log_finding(dir: Option<&Path>, prev: bool, lines: usize) -> Finding {
    let file = if prev {
        crate::logging::PREV_LOG_FILE
    } else {
        crate::logging::LOG_FILE
    };
    let title = if prev {
        t("Previous log", "前回のログ")
    } else {
        t("Log", "ログ")
    }
    .to_owned();
    let text = dir.map(|d| d.join(file)).map(std::fs::read);
    let body = match text {
        Some(Ok(bytes)) => {
            let text = String::from_utf8_lossy(&bytes);
            let all: Vec<&str> = text.lines().collect();
            let from = all.len().saturating_sub(lines);
            let mut tail = all[from..].join("\n");
            // 本文の上限に収まるよう、古い方から削る
            while tail.chars().count() > MAX_FINDING - 500 {
                match tail.find('\n') {
                    Some(i) => tail.drain(..=i),
                    None => break,
                };
            }
            let shown = tail.lines().count();
            format!(
                "{}\n\n```text\n{tail}\n```",
                tf!(
                    "Last {shown} of {} lines of `{file}`.",
                    "`{file}` の {} 行のうち最後の {shown} 行です。",
                    all.len()
                )
            )
        }
        Some(Err(e)) => tf!(
            "Couldn't read `{file}`: {e}",
            "`{file}` を読めませんでした: {e}"
        ),
        None => t(
            "There is no settings folder, so no log is saved.",
            "設定のフォルダが無いので、ログは残っていません。",
        )
        .to_owned(),
    };
    Finding { title, body }
}

/// 記録した MIDI 1 つ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiEntry {
    /// 記録を始めてから届くまで（マイクロ秒）
    pub at_us: i64,
    /// 機器の時刻（マイクロ秒、ポートごとに起点が違う）
    pub device_us: i64,
    /// ポートの何番目か
    pub port: usize,
    pub bytes: Vec<u8>,
}

/// 一覧に出さず数だけ数えるメッセージ（クロックとアクティブセンシング）
pub fn is_noise(bytes: &[u8]) -> bool {
    matches!(bytes.first(), Some(0xF8 | 0xFE))
}

/// MIDI の記録の一覧に出す最大の数
const MIDI_LIST_MAX: usize = 700;

/// 記録した MIDI から報告を作る。`ports` はポートの名前
pub fn midi_finding(
    entries: &[MidiEntry],
    ports: &[String],
    seconds: f64,
    noise: usize,
) -> Finding {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "{}\n",
        tf!(
            "Every MIDI message received in {seconds:.0} s ({} messages; {noise} clock / active sensing messages are only counted).",
            "{seconds:.0} 秒のあいだに届いた MIDI です（{} 件。クロックとアクティブセンシングの {noise} 件は数だけ）。",
            entries.len()
        )
    );
    if ports.is_empty() {
        let _ = writeln!(
            body,
            "{}",
            t(
                "No MIDI input device is connected.",
                "MIDI の入力機器がつながっていません。"
            )
        );
    } else {
        // ポートごとの数
        let _ = writeln!(
            body,
            "| # | {} | {} |\n|---|---|---|",
            t("Device", "機器"),
            t("Messages", "件数")
        );
    }
    for (i, name) in ports.iter().enumerate() {
        let n = entries.iter().filter(|e| e.port == i).count();
        let _ = writeln!(body, "| {} | {} | {n} |", i + 1, cell(name));
    }
    // ノートオンのノートごとの数と強さ
    let mut notes: std::collections::BTreeMap<(usize, u8, u8), (usize, u8, u8)> =
        std::collections::BTreeMap::new();
    for e in entries {
        if let [status, note, vel, ..] = e.bytes[..]
            && status & 0xF0 == 0x90
            && vel > 0
        {
            let s = notes
                .entry((e.port, status & 0x0F, note))
                .or_insert((0, vel, vel));
            s.0 += 1;
            s.1 = s.1.min(vel);
            s.2 = s.2.max(vel);
        }
    }
    if !notes.is_empty() {
        let _ = writeln!(
            body,
            "\n#### {}\n\n| # | ch | {} | {} | {} |\n|---|---|---|---|---|",
            t("Note On", "ノートオン"),
            t("Note", "ノート"),
            t("Count", "回数"),
            t("Velocity", "強さ")
        );
        for ((port, ch, note), (n, lo, hi)) in &notes {
            let _ = writeln!(
                body,
                "| {} | {} | {note} | {n} | {lo}–{hi} |",
                port + 1,
                ch + 1
            );
        }
    }
    if entries.is_empty() {
        let _ = writeln!(
            body,
            "\n{}",
            t("Nothing was received.", "何も届きませんでした。")
        );
        return Finding {
            title: tf!(
                "MIDI recording ({seconds:.0} s, nothing received)",
                "MIDI の記録（{seconds:.0} 秒、届かず）"
            ),
            body,
        };
    }
    let _ = writeln!(
        body,
        "\n#### {}\n\n{}\n\n```text",
        t("Messages", "届いた順"),
        t(
            "ms = since the start, dev = device time since its first message, # = device",
            "ms = 始めてから、dev = その機器の最初のメッセージからの機器の時刻、# = 機器"
        )
    );
    let mut first: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();
    for e in entries.iter().take(MIDI_LIST_MAX) {
        let base = *first.entry(e.port).or_insert(e.device_us);
        let hex: Vec<String> = e.bytes.iter().map(|b| format!("{b:02X}")).collect();
        let _ = writeln!(
            body,
            "{:>9.1} {:>9.1} #{} {:<12} {}",
            e.at_us as f64 / 1000.0,
            (e.device_us - base) as f64 / 1000.0,
            e.port + 1,
            short_hex(&hex),
            describe(&e.bytes)
        );
    }
    body.push_str("```\n");
    if entries.len() > MIDI_LIST_MAX {
        let _ = writeln!(
            body,
            "\n{}",
            tf!(
                "The last {} messages are left out.",
                "残りの {} 件は省きました。",
                entries.len() - MIDI_LIST_MAX
            )
        );
    }
    Finding {
        title: tf!(
            "MIDI recording ({seconds:.0} s, {} messages)",
            "MIDI の記録（{seconds:.0} 秒、{} 件）",
            entries.len()
        ),
        body,
    }
}

/// バイト列を一覧の 1 列に収める（SysEx は長いので頭と長さだけ）
fn short_hex(hex: &[String]) -> String {
    if hex.len() <= 4 {
        hex.join(" ")
    } else {
        format!("{} … ({} B)", hex[..3].join(" "), hex.len())
    }
}

/// メッセージの中身を短く
pub fn describe(bytes: &[u8]) -> String {
    let Some(&status) = bytes.first() else {
        return String::new();
    };
    let ch = (status & 0x0F) + 1;
    let a = bytes.get(1).copied().unwrap_or(0);
    let b = bytes.get(2).copied().unwrap_or(0);
    match status & 0xF0 {
        0x90 if b > 0 => format!("ch{ch} note on {a} vel {b}"),
        0x80 | 0x90 => format!("ch{ch} note off {a}"),
        0xA0 => format!("ch{ch} aftertouch {a} = {b}"),
        0xB0 => format!("ch{ch} CC{a} = {b}"),
        0xC0 => format!("ch{ch} program {}", a as u16 + 1),
        0xD0 => format!("ch{ch} channel pressure {a}"),
        0xE0 => format!(
            "ch{ch} pitch bend {}",
            (i32::from(b) << 7 | i32::from(a)) - 8192
        ),
        _ => match status {
            0xF0 => format!("SysEx {} B", bytes.len()),
            0xF8 => "clock".to_owned(),
            0xFA => "start".to_owned(),
            0xFC => "stop".to_owned(),
            0xFE => "active sensing".to_owned(),
            _ => String::new(),
        },
    }
}

/// 設定の「バージョン」を続けて決定した回数を数える（7 回で開発者モード）
#[derive(Debug, Clone, Copy, Default)]
pub struct Taps {
    last: Option<(u8, Instant)>,
}

/// 開発者モードになるまでの回数
pub const TAPS_TO_UNLOCK: u8 = 7;
/// これより間が空いたら数え直す
const TAP_GAP: Duration = Duration::from_secs(3);

impl Taps {
    /// 1 回決定した。今が何回目かを返す（[`TAPS_TO_UNLOCK`] に届いたら数え直す）
    pub fn tap(&mut self, now: Instant) -> u8 {
        let n = match self.last {
            Some((n, at)) if now.duration_since(at) < TAP_GAP => n + 1,
            _ => 1,
        };
        self.last = (n < TAPS_TO_UNLOCK).then_some((n, now));
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        Names {
            home: Some(r"C:\Users\taro".to_owned()),
            host: Some("DESKTOP-AB12CD".to_owned()),
        }
    }

    #[test]
    fn home_folders_and_other_users_are_hidden() {
        let s = redact(
            r"songs C:\Users\taro\Desktop\DTXFiles and c:\users\TARO\x and D:\Users\hanako\Music",
            &names(),
        );
        assert_eq!(
            s,
            r"songs ~\Desktop\DTXFiles and ~\x and D:\Users\<user>\Music"
        );
        let s = redact(
            "/home/someone/.config/x and C:/Users/taro/AppData",
            &names(),
        );
        assert_eq!(s, "/home/<user>/.config/x and ~/AppData");
        // 日本語の名前のフォルダも
        let s = redact(r"C:\Users\楯\Desktop", &Names::default());
        assert_eq!(s, r"C:\Users\<user>\Desktop");
    }

    #[test]
    fn the_pc_name_and_emails_are_hidden() {
        let s = redact(
            "host DESKTOP-AB12CD, desktop-ab12cd: mail me@example.com or a.b+c@mail.example.co.jp",
            &names(),
        );
        assert_eq!(s, "host <pc>, <pc>: mail <email> or <email>");
        // 語の一部や、メールアドレスでない @ はそのまま
        let s = redact("XDESKTOP-AB12CD @here user@host ch10", &names());
        assert_eq!(s, "XDESKTOP-AB12CD @here user@host ch10");
    }

    #[test]
    fn short_names_are_not_used() {
        let n = Names {
            home: Some("/".to_owned()),
            host: Some("pc".to_owned()),
        }
        .usable();
        assert_eq!(n, Names::default());
    }

    #[test]
    fn short_reports_go_into_the_link() {
        let env = Environment {
            rows: vec![("OS".to_owned(), "windows x86_64".to_owned())],
        };
        let f = Finding {
            title: "V51 test".to_owned(),
            body: "result: 12 ms\nC:\\Users\\taro\\x".to_owned(),
        };
        let r = Report::new("v51", f, &["dev-report"], &env, &names());
        assert_eq!(r.title, "[dev] V51 test");
        assert!(r.body.contains("result: 12 ms"));
        assert!(r.body.contains("~\\x"));
        assert!(!r.body.contains("taro"));
        assert!(r.body.contains("| OS | windows x86_64 |"));
        let Send::Url(url) = r.send() else {
            panic!("should fit")
        };
        assert!(url.starts_with(NEW_ISSUE_URL));
        assert!(url.contains("title=%5Bdev%5D%20V51%20test"));
        assert!(url.contains("&labels=dev-report"));
        assert!(!url.contains(' ') && !url.contains('\n'));
    }

    #[test]
    fn long_reports_are_pasted() {
        let f = Finding {
            title: "long".to_owned(),
            body: "あ".repeat(2000),
        };
        let r = Report::new("x", f, &[], &Environment::default(), &Names::default());
        match r.send() {
            Send::Paste { url, body } => {
                assert!(url.len() <= MAX_URL);
                assert_eq!(body, r.body);
            }
            Send::Url(_) => panic!("should not fit"),
        }
    }

    #[test]
    fn huge_bodies_are_clipped_but_saved_whole() {
        let line = "0123456789".repeat(9) + "\n";
        let body = format!("```text\n{}```", line.repeat(1000));
        let f = Finding {
            title: "huge".to_owned(),
            body,
        };
        let r = Report::new("x", f, &[], &Environment::default(), &Names::default());
        assert!(r.body.chars().count() < 65_536);
        assert!(r.markdown().chars().count() > 90_000);
        // 切った所でコードの囲みを閉じる
        let fences = r.body.lines().filter(|l| l.starts_with("```")).count();
        assert_eq!(fences % 2, 0);
    }

    #[test]
    fn encode_keeps_only_safe_characters() {
        assert_eq!(encode("a b/あ\n"), "a%20b%2F%E3%81%82%0A");
    }

    #[test]
    fn midi_reports_list_messages_and_count_notes() {
        let e = |at_us, bytes: &[u8]| MidiEntry {
            at_us,
            device_us: at_us + 5000,
            port: 0,
            bytes: bytes.to_vec(),
        };
        let entries = vec![
            e(0, &[0x99, 38, 90]),
            e(1500, &[0x89, 38, 0]),
            e(250_000, &[0x99, 38, 40]),
            e(260_000, &[0xF0, 0x41, 0x10, 0x01, 0x06, 0xF7]),
        ];
        let f = midi_finding(&entries, &["TD-17".to_owned()], 10.0, 3);
        assert!(f.title.contains('4'));
        assert!(f.body.contains("| 1 | TD-17 | 4 |"));
        assert!(f.body.contains("| 1 | 10 | 38 | 2 | 40–90 |"));
        assert!(f.body.contains("ch10 note on 38 vel 90"));
        assert!(f.body.contains("F0 41 10 … (6 B)"));
        assert!(f.body.contains("250.0"));
    }

    #[test]
    fn seven_quick_taps_unlock() {
        let mut taps = Taps::default();
        let t0 = Instant::now();
        for i in 1..TAPS_TO_UNLOCK {
            assert_eq!(taps.tap(t0 + Duration::from_millis(u64::from(i) * 300)), i);
        }
        assert_eq!(taps.tap(t0 + Duration::from_millis(2400)), TAPS_TO_UNLOCK);
        // 届いたら数え直す。間が空いても数え直す
        assert_eq!(taps.tap(t0 + Duration::from_millis(2500)), 1);
        assert_eq!(taps.tap(t0 + Duration::from_secs(10)), 1);
    }

    #[test]
    fn logs_keep_the_last_lines() {
        let dir = std::env::temp_dir().join(format!("nxt-dev-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join(crate::logging::LOG_FILE), text).unwrap();
        let f = log_finding(Some(&dir), false, 10);
        assert!(f.body.contains("line 41\n"));
        assert!(f.body.contains("line 50\n"));
        assert!(!f.body.contains("line 40\n"));
        let missing = log_finding(Some(&dir), true, 10);
        assert!(!missing.body.contains("```"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
