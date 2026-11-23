# このリポジトリで作業するとき

- 返答・コメント・ドキュメントは日本語。コードの識別子は英語。
- 画面に出す文は英語と日本語の両方を書く（既定は英語）。`i18n::t("Back", "戻る")`、format の形は `i18n::tf!("{n} bars", "{n} 小節")`、表に置く文は `i18n::tr("Back", "戻る")`（どれも英語が先）。英語は直訳にせず、英語のゲームで普通の短い言い方にする（項目名・ボタン・見出しは Title Case）。ログ・コメント・テストの文は日本語のままでよい。詳しくは `crates/i18n/src/lib.rs`。
- `main` が新しい Rust 実装（上流の履歴なし）。`master` は上流 limyz/DTXmaniaNX の C# 版で、読むだけの参考。上流のコードは `main` に持ち込まない（上流に LICENSE が無いため）。
- スキン（`assets/skin` の画像と音）は exe に埋め込む。ファイル名と配置は上流の v2.0 のスキン（`Runtime/System`）に合わせ、中身は段階を分けてすべて自前の素材に置き換えた（1 KONAMI の絵が入っていた画像 → 2 音 → 3 そのほか、2026-11-23 に終わり）。ユーザーに NX を持たせる前提の設定や仕組みは作らない。出どころと扱いは `assets/README.md`。自前の素材は CC BY 4.0。画像生成の指示文に既存のゲームや製品の名前を入れない。生成の指示文と組み立ては `tools/skin/`、進み具合は `assets/README.md`。声は VOICEVOX の春日部つむぎで、表記「VOICEVOX:春日部つむぎ」をアプリ（`crates/app/src/credits.rs`）と `assets/README.md` に出す。声のファイルは CC BY ではない（キャラの規約で二次配布が禁止）。
- PR は必ずこのリポジトリ（check5004/DTXmaniaNXT）の `main` 宛てに出す。フォークなので既定の送り先が上流になりやすい。
- 確かめは `cargo fmt --all --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all`。
- DTX の読み方は上流のふるまいと DTX Web Editor（check5004/dtx-mania-web-editor の `src/dtx`）に合わせる。読み方を変えたら `testdata/charts` に譜面と手計算の正解を足す。
- 判定は画面のフレームではなく、MIDI の受信時刻と音の再生位置で行う（計画書: https://claude.ai/artifact/TDiaKDQXkBWz7HGNna8TZe）。
