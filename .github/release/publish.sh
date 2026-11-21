#!/usr/bin/env bash
# dist/ のファイルを GitHub のリリースに載せる（.github/workflows/release.yml の release から呼ぶ）。
# リリースが無ければタグと一緒に作り、あればファイルを差し替える。
# 引数: タグ（v0.1.0 など）とコミット
set -euo pipefail
tag=$1
sha=$2
export GH_REPO=${GH_REPO:-$GITHUB_REPOSITORY}

if gh release view "$tag" > /dev/null 2>&1; then
  gh release upload "$tag" dist/* --clobber
else
  # --generate-notes はこのリリースまでに入った PR の一覧を notes.md の後ろに足す
  gh release create "$tag" dist/* --target "$sha" --title "DTXmaniaNXT $tag" \
    --notes-file notes.md --generate-notes
fi
gh release view "$tag"
