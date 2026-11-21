#!/usr/bin/env bash
# どのバージョンを、どのコミットから作るかを決める（.github/workflows/release.yml の plan から呼ぶ）。
#
# - バージョンは Cargo.toml の [workspace.package] の version。タグは v<version>（例: v0.1.0）
# - main への push: そのタグがまだ無ければ、push されたコミットから作る。あれば何もしない
# - Run workflow（手で動かす）: タグがあればそのコミットから作り直してファイルを差し替える。
#   無ければ選んだブランチの先頭から作る
set -euo pipefail

version=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p}' Cargo.toml)
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::Cargo.toml の [workspace.package] の version が読めない（'$version'）"
  exit 1
fi
tag="v$version"

# 注釈つきのタグなら ^{} の行（指しているコミット）が後に来る
existing=$(git ls-remote origin "refs/tags/$tag" "refs/tags/$tag^{}" | tail -n1 | cut -f1)
if [ -n "$existing" ]; then
  sha=$existing
  if [ "${GITHUB_EVENT_NAME:-}" = workflow_dispatch ]; then run=true; else run=false; fi
else
  sha=$GITHUB_SHA
  run=true
fi

echo "version=$version tag=$tag sha=$sha run=$run"
{
  echo "tag=$tag"
  echo "sha=$sha"
  echo "run=$run"
} >> "$GITHUB_OUTPUT"
if [ "$run" = false ]; then
  echo "::notice::$tag はもうあるので作らない。新しいリリースは Cargo.toml の version を上げる"
fi
