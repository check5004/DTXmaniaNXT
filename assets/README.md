# DTXManiaNX のスキン（`assets/skin`）

DTXmaniaNXT の画面の画像と効果音です。リリースのビルドではアプリ（exe）に埋め込むので、DTXManiaNX を持っていなくても、exe だけで NX と同じ見た目で動きます（読み込みは `crates/app/src/skin.rs`）。

## 出どころ

- [limyz/DTXmaniaNX](https://github.com/limyz/DTXmaniaNX) の `Runtime/System`（`Graphics` と `Sounds`）を、中身を変えずにそのまま写したものです。写した時点の上流は、このリポジトリの `master` ブランチのコミット `9cdabe2` です。
- DTXManiaNX は、DTXMania（DTXMania Group）から DTXManiaXG verK などを経て続くフォークです。このスキンも、それらの作者が作り、受け継いできたものです。
- 上流に入っていた DTXMania の著作権の表記: Copyright (C) 2000 2010 DTXMania Group

## 権利とライセンス

- このフォルダのファイルには、このリポジトリの MIT ライセンス（`LICENSE`）は及びません。権利はそれぞれの作者にあります。
- 上流の DTXManiaNX のリポジトリには、ライセンスの記載がありません。上流の `Runtime/Licenses/dtxmania.txt`（DTXMania Group の使用許諾）は、DTXMania のソフトウェアについてのものです。
- 画面のデザインは KONAMI の「GITADORA」シリーズに倣ったもので、「GITADORA」「Tri-Boost」の名前やロゴを含む画像があります。「GITADORA」は株式会社コナミデジタルエンタテインメントの商標です。このアプリは KONAMI とは関係ありません。
- 権利をお持ちの方から求めがあれば、取り下げます。[Issue](https://github.com/check5004/DTXmaniaNXT/issues) でお知らせください。

## 差し替えるとき

- ファイル名と置き場所は上流のスキンと同じです（`Graphics/5_background.jpg` など）。名前の大文字・小文字は問いません。
- ファイルを足したり置き換えたりしたら、リリースのビルドで exe に入ります。開発中のビルド（`cargo run`）はこのフォルダを直接読みます。
