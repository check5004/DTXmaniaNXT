#!/usr/bin/env bash
# build で作った実行ファイルを OS ごとにまとめて dist/ に置き、リリースの説明 notes.md を作る
# （.github/workflows/release.yml の release から呼ぶ。build の成果物は bin/bin-<OS>/ にある）。
# 引数: タグ（v0.1.0 など）
set -euo pipefail
tag=$1
here=.github/release
repo=${GITHUB_REPOSITORY:-check5004/DTXmaniaNXT}
download="https://github.com/$repo/releases/download/$tag"

rm -rf dist stage notes.md
mkdir -p dist stage

# このアプリと、使っているライブラリのライセンスの表記。どのまとめにも入れる
command -v cargo-about > /dev/null || cargo install --locked cargo-about --features cli
cargo about generate --locked --fail -m crates/app/Cargo.toml -c "$here/about.toml" \
  -o stage/THIRD-PARTY-LICENSES.html "$here/about.hbs"
cp LICENSE stage/LICENSE.txt

win=bin/bin-Windows/dtxmanianxt.exe
mac=bin/bin-macOS/dtxmanianxt
linux=bin/bin-Linux/dtxmanianxt

if [ ! -f "$win" ]; then
  echo "::error::Windows の exe ができていない。build (windows-latest) のログを見る"
  exit 1
fi
# C のランタイムを exe に入れたか（.cargo/config.toml の crt-static。vcruntime140.dll を読むなら、VC++ の再頒布パッケージが無い PC で起動しない）
if objdump -p "$win" | grep -i 'DLL Name' | tee /dev/stderr | grep -qi 'vcruntime\|msvcp'; then
  echo "::warning::exe が VC++ のランタイムの DLL を読む（crt-static が効いていない）"
fi

# Windows: exe 1 つだけ（どれを落とせばいいか迷わないように、zip などは作らない）。
# 名前は版によらず同じにして、releases/latest/download/DTXmaniaNXT.exe でいつも最新が落とせるようにする
cp "$win" dist/DTXmaniaNXT.exe
# ライセンスの表記は exe と別のファイルで載せる（macOS と Linux はまとめの中にも入れる）
cp stage/THIRD-PARTY-LICENSES.html dist/

# macOS と Linux: 実行の権限を保つため tar.gz（build の成果物は権限が落ちているので付け直す）
pack_unix() {
  local bin=$1 label=$2 dir="DTXmaniaNXT-$tag-$2"
  if [ ! -f "$bin" ]; then
    echo "::warning::$label の実行ファイルができなかったので載せない（あとで Run workflow で足せる）"
    return
  fi
  mkdir "stage/$dir"
  install -m 755 "$bin" "stage/$dir/dtxmanianxt"
  cp stage/LICENSE.txt stage/THIRD-PARTY-LICENSES.html "stage/$dir/"
  tar -czf "dist/$dir.tar.gz" -C stage --owner=0 --group=0 "$dir"
}
pack_unix "$mac" macos-arm64
pack_unix "$linux" linux-x64

(cd dist && sha256sum -- * > SHA256SUMS.txt)
ls -l dist/

# リリースの説明。載せられたファイルだけ並べる
{
  echo "## ダウンロード"
  echo
  echo "- **Windows（64 ビット）**: [DTXmaniaNXT.exe]($download/DTXmaniaNXT.exe)。Windows はこれ 1 つだけです。好きなフォルダに置いてダブルクリックで起動します。"
  if [ -f "dist/DTXmaniaNXT-$tag-macos-arm64.tar.gz" ]; then
    echo "- macOS（Apple シリコン）: [DTXmaniaNXT-$tag-macos-arm64.tar.gz]($download/DTXmaniaNXT-$tag-macos-arm64.tar.gz)"
  fi
  if [ -f "dist/DTXmaniaNXT-$tag-linux-x64.tar.gz" ]; then
    echo "- Linux（x86_64）: [DTXmaniaNXT-$tag-linux-x64.tar.gz]($download/DTXmaniaNXT-$tag-linux-x64.tar.gz)（ALSA の \`libasound2\` が要ります）"
  fi
  echo
  echo "下の一覧のほかのファイルは、遊ぶのには要りません（\`THIRD-PARTY-LICENSES.html\` は使っているライブラリのライセンスの表記、\`SHA256SUMS.txt\` はダウンロードしたファイルの確かめ用、Source code は GitHub が自動で付けるソースです）。"
  echo
  cat "$here/notes.md"
} > notes.md
cat notes.md
