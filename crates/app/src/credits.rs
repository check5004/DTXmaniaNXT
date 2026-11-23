//! クレジット（スキンの声・音楽・絵の出どころ）。設定の画面の「アプリ」に出し、決定で一覧のページを開く。
//! 声は VOICEVOX の春日部つむぎで、規約で「ユーザーが探して見つけられる場所」への表記を求められている
//! （ゲームならエンディングなど）。詳しい一覧はリポジトリの `assets/README.md`。

/// 声の表記（規約の書き方のまま）
pub const VOICE: &str = "VOICEVOX:春日部つむぎ";

/// 一覧のページ（スキンの出どころと権利）
pub const URL: &str = "https://github.com/check5004/DTXmaniaNXT/blob/main/assets/README.md";

/// 既定のブラウザで一覧を開く
pub fn open() -> std::io::Result<crate::editor::Opened> {
    crate::editor::open_url(URL)
}
