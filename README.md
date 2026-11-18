# DTXmaniaNXT

DTX 形式の譜面で遊べる、ドラムのゲーム兼練習アプリです。Rust で一から書き直しています。

- **ネイティブ**: Windows を主に、macOS と Linux にも同じコードで対応します（描画は wgpu、音は cpal、MIDI は midir）。
- **電子ドラム**: Roland V-Drums の細かい入力（ハイハットの開き具合、チョーク、リム、ベロシティ）を扱います。
- **練習**: 区間ループ・速度変更・待ちモード・打撃ごとの記録と分析、AI による練習支援を目指します。

譜面づくりは [DTX Web Editor](https://github.com/check5004/dtx-mania-web-editor) で行い、本体とは DTX 形式とテスト譜面で互換を保ちます。

## 動かし方（開発中）

Rust（stable）を入れてから:

```sh
cargo run -p dtxmanianxt -- path/to/song.dtx   # ウィンドウを開き、譜面を読んだ結果をログに出す
cargo test --all
```

| 場所 | 中身 |
| --- | --- |
| `crates/dtx` | DTX の読み込み（文字コード判定、ヘッダ、チップ、BPM と小節長から時刻を計算） |
| `crates/app` | アプリ本体（winit + wgpu） |
| `testdata/charts` | テスト譜面と手計算の正解。DTX Web Editor と共有する予定 |

## 上流について

このリポジトリは [limyz/DTXmaniaNX](https://github.com/limyz/DTXmaniaNX) のフォークです。
`main` ブランチは上流の履歴を持たない新しい実装で、上流のコードや画像は含みません。上流の C# 版は `master` ブランチに参考として残しています。

## ライセンス

MIT（`main` ブランチの新しいコードについて）。`master` ブランチの上流のコードには適用されません。
