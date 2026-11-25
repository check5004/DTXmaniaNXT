# スキン（`assets/skin`）

DTXmaniaNXT の画面の画像と効果音です。リリースのビルドではアプリ（exe）に埋め込むので、exe だけで動きます（読み込みは `crates/app/src/skin.rs`）。ファイル名と配置（アプリが絵を切り出す位置）は DTXManiaNX v2.0 のスキンに合わせていますが、中身は段階を分けてすべて DTXmaniaNXT のために作り直しました（2026-11-23 に終わり）。

## 作り直しの経過

| 段階 | 範囲 | 状態 |
|---|---|---|
| 1 | KONAMI の「GITADORA」の絵やロゴが入っていた画像（背景、タイトルのロゴ、シャッター、STAGE FAILED、ランクなど） | 自前の素材に置き換え済み（下の 23 ファイル）。使っていなかった 8 ファイルは消した |
| 2 | 効果音と声（`Sounds/` と `Graphics/Sorting/*.wav`） | 全部を自前の音に置き換え済み（下の 25 ファイル）。使っていなかった 5 ファイルは消した |
| 3 | そのほかの画像（レーン、チップ、パッド、判定の文字、数字、ゲージ、選曲の帯、並べ替えのアイコンなど） | 自前の素材に置き換え済み（下の 116 ファイル）。使っていなかった 164 ファイルは消した |

置き換える前のファイルは、段階 1 の画像が 0.8.0 まで、音が 0.9.1 まで、そのほかの画像が 0.10.0 までのリリースと、過去のコミットに入っています。

## 自前の素材

### 絵（段階 1）

`Graphics/` の次の 23 ファイルは、DTXmaniaNXT のために作ったものです。

- 背景: `1_background.jpg`、`2_background.jpg`、`5_background.jpg`、`6_background.jpg`、`7_background.jpg`、`8_background.jpg`、`9_background.jpg`
- タイトルのロゴとメニューの板: `2_background.png`（板はあとで `2_menu_plate.png` に分けた。アプリがメニューの行の数に合わせて縦に伸ばして描く）
- ジャケットの代わり: `5_preimage default.png`、`5_preimage backbox.png`、`5_preimage random.png`
- 演奏: `7_shutter.png`、`7_stage_failed.jpg`
- 結果のランク: `8_rankSS.png`、`8_rankS.png`、`8_rankA.png`、`8_rankB.png`、`8_rankC.png`、`8_rankD.png`、`8_rankE.png`
- 選曲: `SongSelect/back1.png`、`SongSelect/back2.png`、`Sorting/recommended.png`

作り方:

- 画像生成 AI（OpenAI の GPT Image 2.6 Sunburst を OpenRouter から使用、2026-11-22〜23）で 7 枚の絵を作り、それを切り抜き・縮小して、文字を足しました。指示文・日付・費用は `tools/skin/prompts.json`、組み立ては `tools/skin/build_stage1.py`、生成は `tools/skin/generate.py` です。
- 指示文には既存のゲームや製品の名前を入れていません。既存のキャラクターやロゴに似せないように頼んでいます。
- 足した文字（注意書き、「ゲームが終了しました」「カテゴリー内」「Recommended」）は IPA ゴシック（IPA フォントライセンス）と Inter（SIL Open Font License）で描きました。フォントのファイルは入れていません。

### 絵（段階 3）

`Graphics/` の次の 116 ファイルも、DTXmaniaNXT のために作ったものです。

- 演奏: `7_Paret.png`、`7_Paret_Dark.png`、`7_chips_drums.png`、`7_pads.png`、`ScreenPlayDrums pads flush.png`、`7_judge strings.png`、`ScreenPlayDrums hit-bar.png`、`ScreenPlayDrums combo.png`、`ScreenPlayDrums combo_2.png`、`7_combobomb.png`、`ScreenPlayDrums chip fire_*.png`（11）、`ScreenPlayDrums chip star_*.png`（10）、`ScreenPlayDrums lane flush *.png`（20）、`7_SkillPanel.png`、`7_Difficulty.png`、`7_LevelNumber.png`、`7_Ratenumber_l.png`、`7_Ratenumber_s.png`、`7_RatePercent_l.png`、`7_skill max.png`、`7_score numbersGD.png`、`7_lag numbers.png`、`7_Gauge.png`、`7_gauge_bar.png`、`7_gauge_bar.jpg`、`7_panel_icons.jpg`、`7_Drum_Progress_bg.png`、`7_JacketPanel.png`、`7_Excellent.png`
- 選曲: `5_bar.png`、`5_box_closed.png`、`5_box_open.png`、`5_bpm font.png`、`5_difficulty_panel.png`、`5_difficultyframe.png`、`5_level number.png`、`5_skillbar.png`、`5_skillbar_fill.png`、`5_sortmenu_bg.png`、`SongSelect/graph_fg_drums.png`、`SongSelect/graph_panel_drums.png`、`SongSelect/top_bar.png`、`Lamp/00.png`〜`05.png`、`Rank/rank_0.png`〜`rank_6.png`、`Rank/skill.png`、`Sorting/` の `allmusic.png`・`artist.png`・`difficulty.png`・`level.png`・`player.png`・`skill.png`・`title.png`
- 結果: `ScreenResult Excellent.png`、`ScreenResult fullcombo.png`、`ScreenResult StageCleared.png`、`8_New Record.png`、`8_progress_bar_panel.png`
- 読み込み: `6_Difficulty.png`、`6_LevelNumber.png`、`6_Part.png`
- タイトル・設定・共通: `2_menu.png`、`4_background.png`、`4_header panel.png`、`4_footer panel.png`、`Screen font dfp.png`、`Screen font dfp em.png`、`logo_small.png`

作り方:

- ほとんどは図形の合成とフォントで描きました（`tools/skin/build_stage3.py`）。数字や文字を並べた絵は、アプリが切り出す位置を変えずに描いています。レーンとチップの色は、叩く場所を色で覚えている人のために v2.0 と同じ色にしました。
- 画像生成 AI（段階 1 と同じモデル、2026-11-23）で作ったのは 2 枚だけです。結果の「EXCELLENT」「FULL COMBO」「STAGE CLEARED」の文字と、並べ替えのアイコン 7 つ・練習の合格のメダル・スキルの対象の印です（`tools/skin/prompts.json` の `result_words` と `badges`）。
- ランクの小さい印（`Rank/rank_*.png`）は段階 1 の結果のランクの絵を縮めたもの、設定の背景（`4_background.png`）は段階 1 の背景をぼかしたもの、シャッターに出る小さいロゴ（`logo_small.png`）は段階 1 のタイトルのロゴです。
- 文字は Rubik Spray Paint と Inter / Inter Display（どちらも SIL Open Font License）で描きました。フォントのファイルは入れていません。
- v2.0 でも何も描かれていなかった画像（透明の `7_JacketPanel.png`・`6_Part.png`・`4_footer panel.png`、黒で加算しても何も出ない `7_combobomb.png`）は、同じ大きさの空の画像として作り直しました。

絵のライセンス（段階 1 と 3）: [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/deed.ja)。使うときは「DTXmaniaNXT」と表記してください（例:「スキンの一部: DTXmaniaNXT（CC BY 4.0）」）。

### 音（段階 2）

次の 25 ファイルは、DTXmaniaNXT のために作ったものです。作り方の道具と設定は `tools/skin/`（`sounds.json` に指示文・言葉・日付・費用）にあります。

- 操作音: `Sounds/Move.ogg`、`Sounds/Change.ogg`、`Sounds/Decide.ogg`、`Sounds/Cancel.ogg`。FM 合成のベルのような音に響きを足したもので、`tools/skin/ui_sounds.py` で作りました。ライセンスは絵と同じ CC BY 4.0 です。
- タイトル曲: `Sounds/Title.ogg`。Google の音楽生成モデル Lyria 3（`google/lyria-3-clip-preview` を OpenRouter から使用、2026-11-23）で作った 30 秒の曲の終わりを、3 秒かけて消しました（`tools/skin/lyria.py`）。指示文には既存の曲や演奏者の名前を入れていません。Lyria の出力には、聞こえない透かし（SynthID）が入っています。ライセンスは CC BY 4.0 です。
- 結果・開始・終了・読み込み中の音: `Sounds/stage clear.ogg`、`Sounds/Full combo.ogg`、`Sounds/Excellent.ogg`、`Sounds/Stage failed.ogg`、`Sounds/New Record.ogg`、`Sounds/Game start.ogg`、`Sounds/Game end.ogg`、`Sounds/Now loading.ogg`。タイトル曲と同じく Lyria 3 で作った曲から数秒を切り出して終わりを消し、読み込み中のほかは春日部つむぎの声を重ねました（`tools/skin/jingles.py`）。曲の部分は CC BY 4.0 ですが、声を重ねた 7 つのファイルは、ファイルとしては下の「声の扱い」に従います。
- 声: `Sounds/Basic.ogg`、`Sounds/Advanced.ogg`、`Sounds/Extreme.ogg`、`Sounds/Master.ogg` と、`Graphics/Sorting/` の `recommended.wav`・`title.wav`・`artist.wav`・`difficulty.wav`・`level.wav`・`player.wav`・`allmusic.wav`・`skill.wav`。結果・開始・終了の音の声も同じです。[VOICEVOX](https://voicevox.hiroshiba.jp/) の春日部つむぎで読み上げ、響きを足しました（`tools/skin/voice.py`）。アクセントは NHK のアクセント辞典の型に合わせて指定しています（`sounds.json` の `kana`）。

声の扱い:

- 表記: **VOICEVOX:春日部つむぎ**。アプリでは、設定の「アプリ」にあるクレジットの行に出しています。
- 声のファイルには CC BY 4.0 も MIT ライセンスも及びません。[VOICEVOX の利用規約](https://voicevox.hiroshiba.jp/term/)と[春日部つむぎの利用規約](https://tsumugi-official.studio.site/rule)に従います。
- 声のファイルは DTXmaniaNXT の中で鳴らすためのものです。取り出してほかの作品に使ったり、配ったりしないでください（春日部つむぎの規約で二次配布が禁じられています）。声が要るときは、VOICEVOX で作ってください。

## もとにした DTXManiaNX v2.0 のスキン

0.10.0 までのリリースには、作り直していない画像が DTXManiaNX v2.0 のスキンのまま入っていました。今の `assets/skin` には、v2.0 から写したファイルは残っていません。ファイル名と、アプリが絵を切り出す位置は v2.0 に合わせています。

- 写していたのは [BocuD/DTXmaniaNX](https://github.com/BocuD/DTXmaniaNX)（DTXManiaNX v2.0）のタグ `V2.0.0-20260121-5`（コミット `76c2c35`、v2.0.0 Beta 20260121）の `Runtime/System`（`Graphics` と `Sounds`）です。0.2.0 までは limyz 版の `Runtime/System`（このリポジトリの `master` ブランチのコミット `9cdabe2`）でした。
- BocuD/DTXmaniaNX は [limyz/DTXmaniaNX](https://github.com/limyz/DTXmaniaNX) 1.4.2 からのフォークで、DTXManiaNX は DTXMania（DTXMania Group）から DTXManiaXG verK などを経て続くフォークです。上流に入っていた DTXMania の著作権の表記: Copyright (C) 2000 2010 DTXMania Group
- 過去のリリースとコミットに入っている v2.0 のファイルには、このリポジトリの MIT ライセンスも CC BY 4.0 も及びません。権利は元の作者にあります。上流のリポジトリにはライセンスの記載がなく、`Runtime/Licenses/dtxmania.txt`（DTXMania Group の使用許諾）は DTXMania のソフトウェアについてのものです。
- 画面の配置は KONAMI の「GITADORA」シリーズにならっています。「GITADORA」は株式会社コナミデジタルエンタテインメントの商標です。このアプリは KONAMI とは関係ありません。
- 権利をお持ちの方から求めがあれば、対応します。[Issue](https://github.com/check5004/DTXmaniaNXT/issues) でお知らせください。

## 差し替えるとき

- ファイル名と置き場所は DTXManiaNX v2.0 のスキンと同じです（`Graphics/5_background.jpg` など）。名前の大文字・小文字は問いません。
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

- このリポジトリの MIT ライセンスは及びません。権利は元の作者にあります。DTXMania の時代からのアイコンなら、上流の `Runtime/Licenses/dtxmania.txt`（Copyright (C) 2000 2010 DTXMania Group、MIT と同じ形の使用許諾）の対象と考えられますが、確かめられてはいません。
- 権利をお持ちの方から求めがあれば、取り下げます。
