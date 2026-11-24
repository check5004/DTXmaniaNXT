# リリースの出し方

機能や直しの PR では `version` を上げません。いくつかの変更が `main` に入ってから、版を上げる PR を 1 つ出して、まとめて 1 回でリリースします（同時にいくつも作業が進んでいるとき、リリースのワークフローで Actions が詰まらないように）。

1. `Cargo.toml` の `[workspace.package]` の `version` を上げる PR を出して `main` に入れる（[SemVer](https://semver.org/lang/ja/)。1.0 までは、遊び方や記録が変わるときに 0.x の x を、直しだけなら最後の数を上げる）。
2. `main` に入ると [Release](../.github/workflows/release.yml) のワークフローが、タグ `v<version>`（例: `v0.2.0`）と GitHub のリリースを作り、Windows のインストーラー `DTXmaniaNXT-Setup.exe`（落とすのはこれ 1 つだけ）と自動更新の材料（その版の `.nupkg` と `releases.win.json`）、macOS と Linux の tar.gz、`THIRD-PARTY-LICENSES.html`、`SHA256SUMS.txt` を載せます。リリースの説明は `.github/release/notes.md` と、前のリリースから入った PR の一覧です。
3. macOS か Linux のビルドだけ落ちたときは、Windows の分だけでリリースします。あとで Actions の画面から Release を Run workflow で動かすと、そのタグのコミットから作り直してファイルと説明のダウンロードの一覧を差し替えます。
   - GitHub の実行機（とくに macOS）が足りないと、ジョブは実行機が付かないまま 15 分で取り消されます。そうなったジョブは [Retry without runner](../.github/workflows/retry-runner.yml) が 2 回までやり直し、通ればそのファイルがリリースに足されます（CI も同じ）。

ビルドと詰め方は `.github/release/` のスクリプトにあります。Windows の exe は C のランタイムを入れて作るので（`.cargo/config.toml` の `crt-static`）、VC++ の再頒布パッケージが無い PC でも起動します。ライセンスの表記（`THIRD-PARTY-LICENSES.html`）は [cargo-about](https://github.com/EmbarkStudios/cargo-about) で作ります。

Windows のインストーラーと自動更新は [Velopack](https://velopack.io) で作ります（`.github/release/velopack.sh`。アプリの側は `crates/app/src/update.rs`）。前のリリースの材料を落としてきて、前の版との差分の `.nupkg` も作るので、更新で落とすのは変わったところだけです。インストールしたアプリは、GitHub の新しい 10 個のリリースにある `releases.win.json` を読んで新しい版を探します。
