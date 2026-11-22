# DTXmaniaNXT

DTX 形式の譜面で遊べる、ドラムのゲーム兼練習アプリです。Rust で一から書き直しています。

- **ネイティブ**: Windows を主に、macOS と Linux にも同じコードで対応します（描画は wgpu、音は cpal、MIDI は midir）。
- **電子ドラム**: Roland V-Drums の細かい入力（ハイハットの開き具合、チョーク、リム、ベロシティ）を扱います。
- **練習**: 区間ループ・速度変更・待ちモード・打撃ごとの記録と分析、AI による練習支援を目指します。

譜面づくりは [DTX Web Editor](https://github.com/check5004/dtx-mania-web-editor) で行い、本体とは DTX 形式とテスト譜面で互換を保ちます。

## 使い方（開発中）

### ダウンロード

[リリース](https://github.com/check5004/DTXmaniaNXT/releases/latest)から、Windows は [DTXmaniaNXT.exe](https://github.com/check5004/DTXmaniaNXT/releases/latest/download/DTXmaniaNXT.exe)（いつも最新の版）をダウンロードし、好きなフォルダに置いて起動します。Windows はこの exe 1 つだけで、インストールは要りません。画面の画像と効果音（DTXManiaNX のスキン）も exe に入っているので、DTXManiaNX が無くても NX と同じ見た目で遊べます。macOS（Apple シリコン）と Linux（x86_64）の版も同じページにあります。

- 署名をしていないので、Windows で「Windows によって PC が保護されました」と出たら「詳細情報」→「実行」を押します。
- 新しい版は exe を置き換えるだけです。設定と記録は別のフォルダ（下の `%APPDATA%\DTXmaniaNXT`）にあるので引き継がれます。
- まだリリースしていない変更を試すときは、[Actions](https://github.com/check5004/DTXmaniaNXT/actions) で成功した CI の実行を開き、下の Artifacts から `dtxmanianxt-Windows`（macOS は `dtxmanianxt-macOS`）をダウンロードします。

### 遊び方

1. 起動すると NX と同じタイトルの画面になります（GAME START / CONFIGURATION / EXIT。曲を探しているあいだは左上に印が出ます）。初めは CONFIGURATION の System にある「Add Song Folder」で曲のフォルダを選びます。曲のフォルダをウィンドウにドロップしても足せます（DTXManiaNX の曲を使うなら、NX のフォルダか、その中の曲のフォルダを選びます）。
   - 0.1 で「NX Folder」を選んでいた場合は、0.2 を初めて起動したときに、その NX のパッドの割り当てと表示の設定（StageFailed、ShowLagTime など）を一度だけ引き継ぎます。0.1 が設定のフォルダの `nx` に写した NX のスキンは、もう使わないので消します。
   - 設定の画面は NX と同じ配置です（左のメニュー、右の項目、説明のパネル）。↑↓ で選び、Enter で決め、Esc で戻ります。数の項目は Enter で値を変える状態になり、↑↓ で変えて Enter で戻ります。電子ドラムでは HT / LT で選び、CY か RD で決め、LC で戻ります。選曲の画面から F1 でも開けます。
   - System：曲のフォルダ（Add Song Folder で足し、各フォルダは Enter で一覧から外す）、曲の探し直し、StageFailed（ON / OFF）、ShowLagTime（判定のずれの数字。OFF / ON / GREAT- は PERFECT 以外だけ）、ShowLagTimeColor（ずれの数字の色）、ShowLagHitCount（結果の画面に早かった数と遅かった数）、MIDI の入力のつなぎ直し。
   - Drums：スクロールの速さ、入力の補正、表示の補正、叩いたときの譜面の音、Drums Keys（パッドの割り当て）。Drums Keys でパッドを選んで Enter を押し、そのパッドを叩くと、そのノート番号がそのパッドに割り当たります（`config.toml` の `pad_notes` に残し、Roland の既定より優先します。Reset Keys で消せます）。
2. GAME START で選曲の画面になります。↑↓ で曲を選び、←→ で難易度、Enter で始めます。BOX は Enter で入り、Backspace か Esc で出ます（一番上で Esc を押すとタイトルに戻ります。タイトルで Esc か EXIT を選ぶと終わります）。マウスのホイールでも動かせ、曲バーを押すとそこへ動きます（選んでいる曲を押すと始めます）。始めると NX と同じ読み込みの画面が「Now loading」の音が鳴り終わるまで出て、演奏の画面になります（読み込みの画面で Esc を押すと選曲に戻ります）。
3. 電子ドラムは、つないでおけば起動したときに見つけます。無ければキーボードで叩けます（A S Z D G F H J K L、Q で HH オープン）。
4. 演奏の画面は NX と同じ配置です（スコア、判定の数と達成率・曲別スキル、ゲージ、コンボ、判定の文字とずれ、進み具合のバー、ジャケット）。遊んでいる間は - / = で入力の補正、[ / ] で表示の補正、↑↓ で速さ、M で譜面の音、R でやり直し、Esc で終わって結果へ。I で設定と入力の情報（音の出力、MIDI の入力、最後に叩いたパッドとハイハットの開き具合）を出します。ゲージが尽きると、NX と同じく STAGE FAILED で選曲に戻ります（設定の StageFailed を OFF にすると終わらなくなります。練習では終わりません）。判定の下のずれの数字は設定の ShowLagTime と ShowLagTimeColor に従います。
5. 最後まで叩くと、NX と同じ結果の画面になります（ランク、Excellent・フルコンボ・ステージクリア、判定の数と割合、スコア、達成率と曲別スキル、今回と自己ベストの進み具合のバー、達成率が自己ベストを超えたら New Record、それぞれの音）。設定の ShowLagHitCount が ON なら、早かった数と遅かった数も出します。← → で 2 ページ目（このアプリの分析）と切り替え、Enter か Esc で選曲へ、R でもう一度、P で区間を練習します。電子ドラムは CY か RD か LC で選曲へ、HT か LT でページを切り替えます。

設定（曲のフォルダ・補正値など）は `config.toml` に残ります（Windows は `%APPDATA%\DTXmaniaNXT`）。

アプリのログ（動作の記録）は同じフォルダの `dtxmanianxt.log` に書きます（前回の分は `dtxmanianxt.prev.log`）。Windows ではコンソールの窓を出さないので、うまく動かないときはこのファイルを見てください。起動できなかったときや落ちたときは、その場所をダイアログで知らせます。

遊んだ記録は、音符 1 つごとの判定・ずれ・強さまで同じフォルダの `records.sqlite` に残ります。自己ベスト（達成率、スコア、進み具合のバー）もここから出します（途中でやめたプレイと STAGE FAILED は自己ベストの達成率に入れず、進み具合のバーだけ、最後まで遊んだ記録が無ければ一番先まで進んだものを出します）。結果の画面の 2 ページ目には、直近の記録から出した「苦手な小節」の帯（赤いほど苦手）と、苦手なフレーズの上位 5 つが出ます。小節の番号は DTX の小節番号（`#xxxyy` の `xxx`）です。

### 区間を練習する

選曲の画面か結果の画面で P を押す（結果の画面では苦手フレーズを押しても）と、練習する区間を選ぶ画面になります。

1. 小節の帯をドラッグして区間を選びます（クリックで 1 小節、←→ でずらす、↑↓ で長さ）。苦手フレーズの一覧からも選べます。
2. 周のつなぎ方を選びます。「つなぐ」は区間の終わりから切れ目なく頭へ戻り、「区切る」は次の小節の頭の 1 打（着地）まで叩いて助走からやり直します（T で切り替え）。
3. 速度の決め方を選びます（L で切り替え、, と . で速度）。
   - 段階アップ（既定）: 70% から始め、合格が 2 周続くたびに 5% 上げ、不合格が 3 周続いたら 5% 下げます（始めの速度より下げません）。目標の速度（100%、105%、110% から選ぶ）で 2 周続けて合格したらクリアです。
   - 固定: 25〜150% の決めた速度のまま回ります。
   - 遅くするときの音は「音程を保つ」か「音程ごと遅く（テープ）」から選べます。
4. Enter で始めると、区間の 1 小節前から BGM が流れ、クリックで 4 拍数えてから区間を繰り返します。周ごとに合格（既定は MISS と POOR が 0、PERFECT が 8 割以上）かを出します。判定の幅は遅くしても実際のミリ秒のままなので、遅い周でも判定は甘くなりません。
5. 練習中は左上（スコアの場所）に練習の欄（区間、何周目と速度、段階アップの進み、直近の周の成績）が出て、周の頭には「2 周目」のように出ます。助走の 4 拍はレーンの真ん中に数えます。←→ で区間を 1 小節ずらし、, と . で速度を 5% ずつ変え、R で最初から、Esc で終わって周ごとの成績を見ます（結果の画面の 2 ページ目から開き、← で NX の結果の画面になります。練習はスコアと自己ベストには入りません）。

速度が変わるのは周の切れ目だけです。「つなぐ」では次の周がもう落ちてきているので、決めた速度は次の次の周から使います（練習中の欄に「75% は 5 周目から」のように出ます）。「区切る」では合否が出てから次の周を並べるので、すぐ次の周から変わります。

ドラムでも操作できます（HT / LT で区間をずらす、CY か RD で始める、LC で戻る）。最後まで叩いた周は速度と一緒に記録に残ります。苦手な小節の色に使うのは 100% で叩いた周だけです。

BGM は区間の分だけを 1 本にまとめ、速度ごとに別のスレッドで伸ばしておきます。音程を保つ伸ばしには [timestretch](https://github.com/robmorgan/timestretch-rs)（MIT、Rust だけで書かれていて、ビルドに C++ や LLVM が要らない）を使います。叩いた音とクリックは伸ばしません。

### 克服と叩き方のくせ

- 練習で等速以上の同じ速度で 2 周続けて合格すると、その区間は「クリア」として残ります。そのあとの普通のプレイで、区間を全部叩いて苦手でなく（点が 0.25 未満）、区間の苦手度が練習の前より下がっていれば、結果の画面に「克服！」と出ます。クリアした区間と克服したかは、区間を選ぶ画面にも並びます。
- 結果の画面の「叩き方のくせ」は、すべての曲の等速で叩いた直近 200 回の記録から、レーンと拍の中の位置（表拍、8 分裏、16 分裏、3 連符、細かい位置）ごとにずれの平均とばらつきを出したものです。平均が 8ms 以上ずれている組を、ずれの大きい順に 3 つまで文で出し、全部の組は表で見られます（組ごとに 20 打から）。この版より前の記録は拍の中の位置を持たないので入りません。

### 自分でビルドする

Rust（stable）を入れてから（Linux は `libasound2-dev` も）:

```sh
cargo run -p dtxmanianxt                       # 選曲から
cargo run -p dtxmanianxt -- path/to/song.dtx   # その譜面をすぐ遊ぶ
cargo test --all
```

画面を確かめるときは `DTXMANIANXT_AUTOPLAY=1` を付けて起動すると、音符を自動で叩きます（開発用）。

| 場所 | 中身 |
| --- | --- |
| `crates/dtx` | DTX の読み込み（文字コード判定、ヘッダ、チップ、BPM と小節長から時刻を計算） |
| `crates/engine` | 判定、音と MIDI の時計、電子ドラムの入力の正規化、記録からの苦手・克服・叩き方のくせの分析、練習の区間ループの並べ方と段階アップ |
| `crates/library` | 曲のフォルダの探し方（set.def・box.def・DTXFiles.*） |
| `crates/nx` | DTXManiaNX のフォルダの読み方（0.1 の設定の引き継ぎに使う Config.ini、大文字・小文字を区別しないファイル探し） |
| `crates/records` | 遊んだ記録（SQLite）の読み書き |
| `crates/app` | アプリ本体（winit + wgpu + egui、cpal、midir）。NX のスキンの画像を上流と同じ座標で描く（スキンの読み込み `skin.rs`、`gfx.rs`、タイトル `title_view.rs`、設定 `config_view.rs`、選曲 `select.rs`、読み込み `loading.rs`、演奏 `play_view.rs`、結果 `result_view.rs`） |
| `assets/skin` | DTXManiaNX のスキン（画像と効果音）。リリースのビルドでは exe に埋め込み、開発中のビルドはこのフォルダから読みます。出どころと扱いは [`assets/README.md`](assets/README.md) |
| `testdata/charts` | テスト譜面と手計算の正解。DTX Web Editor と共有する予定 |

## リリースの出し方

1. `Cargo.toml` の `[workspace.package]` の `version` を上げる PR を出して `main` に入れる（[SemVer](https://semver.org/lang/ja/)。1.0 までは、遊び方や記録が変わるときに 0.x の x を、直しだけなら最後の数を上げる）。
2. `main` に入ると [Release](.github/workflows/release.yml) のワークフローが、タグ `v<version>`（例: `v0.2.0`）と GitHub のリリースを作り、Windows の exe（1 つだけ）、macOS と Linux の tar.gz、`THIRD-PARTY-LICENSES.html`、`SHA256SUMS.txt` を載せます。リリースの説明は `.github/release/notes.md` と、前のリリースから入った PR の一覧です。
3. macOS か Linux のビルドだけ落ちたときは、Windows の分だけでリリースします。あとで Actions の画面から Release を Run workflow で動かすと、そのタグのコミットから作り直してファイルを差し替えます。

ビルドと詰め方は `.github/release/` のスクリプトにあります。Windows の exe は C のランタイムを入れて作るので（`.cargo/config.toml` の `crt-static`）、VC++ の再頒布パッケージが無い PC でも起動します。ライセンスの表記（`THIRD-PARTY-LICENSES.html`）は [cargo-about](https://github.com/EmbarkStudios/cargo-about) で作ります。

## 上流について

このリポジトリは [limyz/DTXmaniaNX](https://github.com/limyz/DTXmaniaNX) のフォークです。
`main` ブランチは上流の履歴を持たない新しい実装で、上流のコードは含みません。画面の画像と効果音（`assets/skin`）だけは、上流の `Runtime/System` をそのまま入れています（出どころと扱いは [`assets/README.md`](assets/README.md)）。上流の C# 版は `master` ブランチに参考として残しています。

## ライセンス

MIT（`main` ブランチの新しいコードについて）。`assets/skin` の画像と音、`master` ブランチの上流のコードには適用されません。
