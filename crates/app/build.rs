//! Windows の exe に、アプリのアイコン（`assets/icon/dtxmanianxt.ico`）と版の情報を埋め込む。
//!
//! - エクスプローラー・スタートメニュー・デスクトップのショートカット（Velopack が exe から写す）は、このアイコンを使う
//! - 窓の左上とタスクバーは、起動したときにこのアイコンを読んで付ける（`src/app_icon.rs`）
//! - 版の情報の「説明」は、タスクマネージャーや音量ミキサーに出るアプリの名前になる
//! - ほかの OS では何もしない

use std::path::PathBuf;

fn main() {
    // このクレート（crates/app）から見た assets/icon/dtxmanianxt.ico。区切りは OS に合わせる（rc.exe に渡す）
    let mut icon = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    icon.extend(["..", "..", "assets", "icon", "dtxmanianxt.ico"]);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    // アイコンの番号は 1（app_icon.rs の RESOURCE_ID と同じ）。番号の一番小さいアイコンが exe のアイコンになる
    res.set_icon(&icon.to_string_lossy())
        .set("ProductName", "DTXmaniaNXT")
        .set("FileDescription", "DTXmaniaNXT")
        .set("OriginalFilename", "DTXmaniaNXT.exe");
    if let Err(e) = res.compile() {
        panic!("exe にアイコンを埋め込めません（Windows SDK の rc.exe が要る）: {e}");
    }
}
