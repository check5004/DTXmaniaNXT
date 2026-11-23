# スキン（`assets/skin`）

DTXmaniaNXT の画面の画像と効果音です。リリースのビルドではアプリ（exe）に埋め込むので、exe だけで動きます（読み込みは `crates/app/src/skin.rs`）。DTXManiaNX v2.0 のスキンを元にしていて、段階を分けて自前の素材に置き換えています。

## 置き換えの進み具合

| 段階 | 範囲 | 今の状態 |
|---|---|---|
| 1 | KONAMI の「GITADORA」の絵やロゴが入っていた画像（背景、タイトルのロゴ、シャッター、STAGE FAILED、ランクなど） | 自前の素材に置き換え済み（下の 23 ファイル）。使っていなかった 8 ファイルは消した |
| 2 | 効果音と声（`Sounds/` と `Graphics/Sorting/*.wav`） | DTXManiaNX v2.0 のまま |
| 3 | そのほかの画像（判定の文字、ゲージ、パッド、並べ替えのアイコンなど） | DTXManiaNX v2.0 のまま |

0.8.0 までのリリースと過去のコミットには、置き換える前の画像が入っています。

## 自前の素材

`Graphics/` の次の 23 ファイルは、DTXmaniaNXT のために作ったものです。

- 背景: `1_background.jpg`、`2_background.jpg`、`5_background.jpg`、`6_background.jpg`、`7_background.jpg`、`8_background.jpg`、`9_background.jpg`
- タイトルのロゴとメニューの板: `2_background.png`
- ジャケットの代わり: `5_preimage default.png`、`5_preimage backbox.png`、`5_preimage random.png`
- 演奏: `7_shutter.png`、`7_stage_failed.jpg`
- 結果のランク: `8_rankSS.png`、`8_rankS.png`、`8_rankA.png`、`8_rankB.png`、`8_rankC.png`、`8_rankD.png`、`8_rankE.png`
- 選曲: `SongSelect/back1.png`、`SongSelect/back2.png`、`Sorting/recommended.png`

作り方:

- 画像生成 AI（OpenAI の GPT Image 2.6 Sunburst を OpenRouter から使用、2026-11-22〜23）で 7 枚の絵を作り、それを切り抜き・縮小して、文字を足しました。指示文・日付・費用は `tools/skin/prompts.json`、組み立ては `tools/skin/build_stage1.py`、生成は `tools/skin/generate.py` です。
- 指示文には既存のゲームや製品の名前を入れていません。既存のキャラクターやロゴに似せないように頼んでいます。
- 足した文字（注意書き、「ゲームが終了しました」「カテゴリー内」「Recommended」）は IPA ゴシック（IPA フォントライセンス）と Inter（SIL Open Font License）で描きました。フォントのファイルは入れていません。

ライセンス: [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/deed.ja)。使うときは「DTXmaniaNXT」と表記してください（例:「スキンの一部: DTXmaniaNXT（CC BY 4.0）」）。

## DTXManiaNX v2.0 から写したもの

上の 23 ファイルのほかは、DTXManiaNX v2.0 のスキンです。

- [BocuD/DTXmaniaNX](https://github.com/BocuD/DTXmaniaNX)（DTXManiaNX v2.0）のタグ `V2.0.0-20260121-5`（コミット `76c2c35`、v2.0.0 Beta 20260121）の `Runtime/System`（`Graphics` と `Sounds`）を、中身を変えずにそのまま写したものです。
- 入れていないもの: 背景の動画（`Graphics/5_background.mp4` と `Graphics/2_background.mp4`。同じ 60 秒の動画で 1 つ 75 MB）と、Windows が作る `Thumbs.db`・`desktop.ini`。動画の代わりに、選曲とタイトルは静止画（`5_background.jpg`・`2_background.jpg`）を出します。
- BocuD/DTXmaniaNX は [limyz/DTXmaniaNX](https://github.com/limyz/DTXmaniaNX) 1.4.2 からのフォークで、DTXManiaNX は DTXMania（DTXMania Group）から DTXManiaXG verK などを経て続くフォークです。0.2.0 までは limyz 版の `Runtime/System`（このリポジトリの `master` ブランチのコミット `9cdabe2`）を入れていました。
- 上流に入っていた DTXMania の著作権の表記: Copyright (C) 2000 2010 DTXMania Group

権利:

- これらのファイルには、このリポジトリの MIT ライセンス（`LICENSE`）も、上の CC BY 4.0 も及びません。権利は元の作者にありますが、個々の絵や音の作者の記録はありません。
- 上流の DTXManiaNX のリポジトリ（limyz 版、BocuD 版とも）には、ライセンスの記載がありません。上流の `Runtime/Licenses/dtxmania.txt`（DTXMania Group の使用許諾）は、DTXMania のソフトウェアについてのものです。
- 残っている画面の部品には、KONAMI の「GITADORA」シリーズの画面に似せたものがあります（判定の文字、ゲージ、パッドなど）。音とあわせて、段階 2・3 で置き換えます。「GITADORA」は株式会社コナミデジタルエンタテインメントの商標です。このアプリは KONAMI とは関係ありません。
- 権利をお持ちの方から求めがあれば、取り下げます。[Issue](https://github.com/check5004/DTXmaniaNXT/issues) でお知らせください。

## 差し替えるとき

- ファイル名と置き場所は上流のスキンと同じです（`Graphics/5_background.jpg` など）。名前の大文字・小文字は問いません。
- ファイルを足したり置き換えたりしたら、リリースのビルドで exe に入ります。開発中のビルド（`cargo run`）はこのフォルダを直接読みます。

# アプリのアイコン（`assets/icon`）

exe・窓・タスクバー・ショートカット・インストーラー（Setup.exe）のアイコンです。

- `dtxmanianxt.ico`: 16〜256 の 10 の大きさ。Windows の exe に埋め込み（`crates/app/build.rs`）、窓とタスクバーにも付けます（`crates/app/src/app_icon.rs`）。インストーラーは `.github/release/velopack.sh` の `--icon` で使います。
- `dtxmanianxt.png`: 同じ絵の 256×256。Windows 以外の窓のアイコンです。

## 出どころ

- DTXManiaNX のアイコン `DTXMania/dtx.ico`（32×32・16 色、ドラムと「DTX」の文字の絵）が元です。BocuD 版 v2.0（タグ `V2.0.0-20260121-5`）と limyz 版（`master` ブランチ）で同じファイルで、limyz 版には最初の取り込み（2016-02-28、DTXMania のソースから）から入っています。
- 変えたのは、下の帯の文字を「DTX」から「NXT」にしたことだけです。X と T は元の文字の点をそのまま並べ替え、N は同じ色と太さで描き足しました。NX と NXT を両方入れていても、タスクバーで見分けられるようにするためです。
- 大きさをそろえるのに、32 の整数倍（64・96・128・256）は点のまま拡大し、それ以外は 256 の絵を面積で平均して縮めました。

## 権利とライセンス

- スキンと同じく、このリポジトリの MIT ライセンスは及びません。権利は元の作者にあります。DTXMania の時代からのアイコンなら、上流の `Runtime/Licenses/dtxmania.txt`（Copyright (C) 2000 2010 DTXMania Group、MIT と同じ形の使用許諾）の対象と考えられますが、確かめられてはいません。
- 権利をお持ちの方から求めがあれば、取り下げます。
