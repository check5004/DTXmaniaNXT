# DTXmaniaNXT

DTX 形式の譜面で遊べる、ドラムのゲーム兼練習アプリです。Rust で一から書き直しています。

- **ネイティブ**: Windows を主に、macOS と Linux にも同じコードで対応します（描画は wgpu、音は cpal、MIDI は midir）。
- **電子ドラム**: Roland V-Drums の細かい入力（ハイハットの開き具合、チョーク、リム、ベロシティ）を扱います。
- **練習**: 区間ループ・速度変更・待ちモード・打撃ごとの記録と分析、AI による練習支援を目指します。

譜面づくりは [DTX Web Editor](https://github.com/check5004/dtx-mania-web-editor) で行い、本体とは DTX 形式とテスト譜面で互換を保ちます。

## 使い方（開発中）

### 実行ファイルを使う

GitHub の [Actions](https://github.com/check5004/DTXmaniaNXT/actions) で、成功した CI の実行を開き、下の Artifacts から `dtxmanianxt-Windows`（macOS は `dtxmanianxt-macOS`）をダウンロードして展開します。

1. 起動して「フォルダを追加…」で、DTXMania の曲のフォルダ（DTXFiles など）を選びます。フォルダをウィンドウにドロップしても追加できます。
2. ↑↓ で曲を選び、←→ で難易度、Enter で始めます。BOX は Enter で入り、Backspace で出ます。
3. 電子ドラムは、つないでおけば起動したときに見つけます。無ければキーボードで叩けます（A S Z D G F H J K L、Q で HH オープン）。
4. 遊んでいる間は - / = で入力の補正、[ / ] で表示の補正、↑↓ で速さ、M で譜面の音、R でやり直し、Esc で終わります。

設定（曲のフォルダ・補正値など）は `config.toml` に残ります（Windows は `%APPDATA%\DTXmaniaNXT`）。

### 自分でビルドする

Rust（stable）を入れてから（Linux は `libasound2-dev` も）:

```sh
cargo run -p dtxmanianxt                       # 選曲から
cargo run -p dtxmanianxt -- path/to/song.dtx   # その譜面をすぐ遊ぶ
cargo test --all
```

| 場所 | 中身 |
| --- | --- |
| `crates/dtx` | DTX の読み込み（文字コード判定、ヘッダ、チップ、BPM と小節長から時刻を計算） |
| `crates/engine` | 判定、音と MIDI の時計、電子ドラムの入力の正規化 |
| `crates/library` | 曲のフォルダの探し方（set.def・box.def・DTXFiles.*） |
| `crates/app` | アプリ本体（winit + wgpu + egui、cpal、midir） |
| `testdata/charts` | テスト譜面と手計算の正解。DTX Web Editor と共有する予定 |

## 上流について

このリポジトリは [limyz/DTXmaniaNX](https://github.com/limyz/DTXmaniaNX) のフォークです。
`main` ブランチは上流の履歴を持たない新しい実装で、上流のコードや画像は含みません。上流の C# 版は `master` ブランチに参考として残しています。

## ライセンス

MIT（`main` ブランチの新しいコードについて）。`master` ブランチの上流のコードには適用されません。
