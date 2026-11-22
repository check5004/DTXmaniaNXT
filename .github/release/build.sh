#!/usr/bin/env bash
# リリースの実行ファイルを作り、out/ に置く（.github/workflows/release.yml の build から、各 OS で呼ぶ）。
# Windows は velopack.sh でインストーラーと自動更新の材料も作る。
# Windows の exe に C のランタイムを入れる設定は .cargo/config.toml にある（RUSTFLAGS を設定するとそちらが効かなくなる）
set -euo pipefail

exe=dtxmanianxt
case "${RUNNER_OS:-}" in
  Windows) exe=dtxmanianxt.exe ;;
  # cpal（音声）と midir（MIDI）は Linux では ALSA を使う
  Linux) sudo apt-get update && sudo apt-get install -y libasound2-dev ;;
esac

cargo build --release --locked -p dtxmanianxt
mkdir -p out
cp "target/release/$exe" out/
# Windows はインストーラー（Setup.exe）と自動更新の材料も作る（out/velopack/）
if [ "${RUNNER_OS:-}" = Windows ]; then
  bash .github/release/velopack.sh
fi
ls -l out/
