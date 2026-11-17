# このリポジトリで作業するとき

- 返答・コメント・ドキュメントは日本語。コードの識別子は英語。
- `main` が新しい Rust 実装（上流の履歴なし）。`master` は上流 limyz/DTXmaniaNX の C# 版で、読むだけの参考。上流のコードや画像は `main` に持ち込まない（上流に LICENSE が無いため）。
- PR は必ずこのリポジトリ（check5004/DTXmaniaNXT）の `main` 宛てに出す。フォークなので既定の送り先が上流になりやすい。
- 確かめは `cargo fmt --all --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all`。
- DTX の読み方は上流のふるまいと DTX Web Editor（check5004/dtx-mania-web-editor の `src/dtx`）に合わせる。読み方を変えたら `testdata/charts` に譜面と手計算の正解を足す。
- 判定は画面のフレームではなく、MIDI の受信時刻と音の再生位置で行う（計画書: https://claude.ai/artifact/TDiaKDQXkBWz7HGNna8TZe）。
