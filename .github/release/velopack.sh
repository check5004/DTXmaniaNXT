#!/usr/bin/env bash
# Windows のインストーラー（Setup.exe）と自動更新の材料を Velopack の vpk で作り、out/velopack/ に置く
# （build.sh から Windows のときに呼ぶ。cargo build を済ませてから）。アプリ側は crates/app/src/update.rs。
#
# - 前のリリースの更新の材料を落としてきて、前の版との差分（delta）も作る。落とせなければ全体だけ
# - Setup.exe は管理者の権限なしで %LocalAppData%\DTXmaniaNXT に入れ、スタートメニューとデスクトップに置く
# - Setup.exe と Update.exe（アプリと機能の一覧）のアイコンは --icon。ショートカットは exe に埋め込んだアイコン（crates/app/build.rs）
# - リリースに載せるのは package.sh（Setup.exe、この版の .nupkg、releases.win.json）
set -euo pipefail

version=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p}' Cargo.toml)
repo="https://github.com/${GITHUB_REPOSITORY:-check5004/DTXmaniaNXT}"
# vpk はアプリの velopack クレートと同じ版にそろえる（Cargo.lock）
vpk_version=$(sed -n '/^name = "velopack"$/{n;s/^version = "\(.*\)"$/\1/p}' Cargo.lock)
if [ -z "$vpk_version" ]; then
  echo "::error::Cargo.lock に velopack が無い"
  exit 1
fi

dotnet --list-sdks
dotnet tool install -g vpk --version "$vpk_version"
export PATH="$PATH:$HOME/.dotnet/tools"

pack=stage/velopack
out=out/velopack
rm -rf "$pack" "$out"
mkdir -p "$pack" "$out"
cp target/release/dtxmanianxt.exe "$pack/DTXmaniaNXT.exe"

# 前の版の材料（差分を作るのに使う）
if ! vpk download github --repoUrl "$repo" -o "$out"; then
  echo "::warning::前のリリースの更新の材料を落とせなかったので、差分なしで作る"
  rm -rf "$out"
  mkdir -p "$out"
fi
# 同じ版を作り直すとき（Run workflow）は、落としてきたこの版を使わない
if grep -Eq "\"Version\": *\"$version\"" "$out/releases.win.json" 2> /dev/null; then
  echo "$version はもうリリースにあるので、差分なしで作り直す"
  rm -rf "$out"
  mkdir -p "$out"
fi

vpk pack \
  --packId DTXmaniaNXT \
  --packVersion "$version" \
  --packDir "$pack" \
  --mainExe DTXmaniaNXT.exe \
  --packTitle DTXmaniaNXT \
  --packAuthors check5004 \
  --icon assets/icon/dtxmanianxt.ico \
  --noPortable \
  --outputDir "$out"
ls -l "$out"
