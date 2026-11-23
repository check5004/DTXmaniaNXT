#!/usr/bin/env bash
# dist/ のファイルを GitHub のリリースに載せる（.github/workflows/release.yml の release から呼ぶ）。
# リリースが無ければタグと一緒に作り、あればファイルと説明を差し替える。
# 引数: タグ（v0.1.0 など）とコミット
set -euo pipefail
tag=$1
sha=$2
export GH_REPO=${GH_REPO:-$GITHUB_REPOSITORY}

if gh release view "$tag" > /dev/null 2>&1; then
  gh release upload "$tag" dist/* --clobber
  # あとから OS のファイルを足したとき（Run workflow や、実行機が付かなかったジョブのやり直し）に、
  # 説明のダウンロードの一覧も載っているファイルに合わせる。PR の一覧は作ったときと同じ形で作り直す
  {
    cat notes.md
    echo
    gh api "repos/$GH_REPO/releases/generate-notes" -f tag_name="$tag" --jq .body
  } > notes-full.md
  gh release edit "$tag" --notes-file notes-full.md
else
  # --generate-notes はこのリリースまでに入った PR の一覧を notes.md の後ろに足す
  gh release create "$tag" dist/* --target "$sha" --title "DTXmaniaNXT $tag" \
    --notes-file notes.md --generate-notes
fi
gh release view "$tag"
