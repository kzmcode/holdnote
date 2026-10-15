#!/usr/bin/env bash
# Release automation (docs/04 "リリースごと"): audit, test, version bump, build, verify, tag, GitHub Release, tap update.
# Usage: scripts/release.sh <version> [--dry-run] [--tap-dir PATH] [--no-whisper] [--changes FILE]
#   --dry-run     run read-only checks and the local build; only print what git/GitHub steps would do.
#                 Cargo.toml / Cargo.lock / THIRD_PARTY_LICENSES.txt are restored afterwards.
#   --tap-dir     homebrew-tap checkout (default: ../homebrew-tap)
#   --no-whisper  build without bundling whisper-cli
#   --changes     Markdown file with the user-facing "変更点" list (default: commit subjects since the last tag)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION=""; DRY=0; TAP_DIR="$ROOT/../homebrew-tap"; WHISPER=1; CHANGES_FILE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --no-whisper) WHISPER=0 ;;
    --changes) CHANGES_FILE="${2:?--changes にはファイルが必要です}"; shift ;;
    --tap-dir) TAP_DIR="${2:?--tap-dir にはパスが必要です}"; shift ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    -*) echo "不明なオプション: $1" >&2; exit 2 ;;
    *) [ -z "$VERSION" ] || { echo "バージョンは1つだけ指定してください" >&2; exit 2; }; VERSION="$1" ;;
  esac
  shift
done
[ -n "$VERSION" ] || { echo "使い方: scripts/release.sh <version> [--dry-run] [--tap-dir PATH] [--no-whisper] [--changes FILE]" >&2; exit 2; }
if [ -n "${CHANGES_FILE:-}" ] && [ ! -f "$CHANGES_FILE" ]; then echo "--changes のファイルがありません: $CHANGES_FILE" >&2; exit 2; fi

TAG="v$VERSION"
STEP_N=0
step() { STEP_N=$((STEP_N+1)); echo; echo "==> [$STEP_N] $*"; }
die()  { echo "エラー: $*" >&2; exit 1; }
# Precondition failure: fatal normally, only a warning in dry-run so the rest can be exercised.
need() { if [ "$DRY" = 1 ]; then echo "  (dry-run) 警告: $*"; else die "$*"; fi; }
# Run a state-changing command, or just print it in dry-run.
act() { if [ "$DRY" = 1 ]; then echo "  [dry-run] 実行予定: $*"; else "$@"; fi; }

TMP="$(mktemp -d)"
RESTORE=()
cleanup() {
  local rc=$?
  if [ "$DRY" = 1 ] && [ ${#RESTORE[@]} -gt 0 ]; then
    for f in "${RESTORE[@]}"; do cp "$TMP/bak/$f" "$ROOT/$f"; done
    echo "(dry-run) ${RESTORE[*]} を元に戻しました"
  fi
  rm -rf "$TMP"
  if [ $rc -ne 0 ] && [ "$DRY" = 0 ]; then
    echo "リリース処理が途中で失敗しました。git status で状態を確認してください。" >&2
  fi
  exit $rc
}
trap cleanup EXIT
if [ "$DRY" = 1 ]; then
  mkdir -p "$TMP/bak"
  for f in Cargo.toml Cargo.lock THIRD_PARTY_LICENSES.txt; do cp "$f" "$TMP/bak/$f"; RESTORE+=("$f"); done
  echo "*** dry-run: git / GitHub への変更は行いません ***"
fi

# ---- 1. preconditions ----------------------------------------------------
step "事前チェック"
command -v gh >/dev/null || die "gh (GitHub CLI) が見つかりません"
gh auth status >/dev/null 2>&1 || die "gh にログインしていません。gh auth login を実行してください"
cargo audit --version >/dev/null 2>&1 || die "cargo-audit が未インストールです (cargo install cargo-audit --locked)"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "バージョンは x.y.z 形式で指定してください: $VERSION"

branch="$(git branch --show-current)"
[ "$branch" = main ] || need "main ブランチ上で実行してください（現在: ${branch}）"
[ -z "$(git status --porcelain)" ] || need "作業ツリーがクリーンではありません"
git fetch origin --tags --quiet || die "git fetch に失敗しました"
if [ "$branch" = main ]; then
  [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || need "ローカルの main が origin/main と一致しません（pull / push してください）"
fi
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then die "タグ $TAG はローカルに既に存在します"; fi
if [ -n "$(git ls-remote --tags origin "refs/tags/$TAG")" ]; then die "タグ $TAG は origin に既に存在します"; fi
REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner)" || die "リポジトリ情報を取得できません"
ORG="${REPO%%/*}"
echo "  リポジトリ: $REPO / バージョン: $VERSION / ブランチ: $branch"

# ---- 2. cargo audit ------------------------------------------------------
step "cargo audit"
if ! cargo audit >"$TMP/audit.log" 2>&1; then
  tail -30 "$TMP/audit.log" >&2
  die "cargo audit が失敗しました（脆弱性あり、またはDB取得エラー）。docs/04 の 4-5 を参照してください"
fi
grep -E 'warning:' "$TMP/audit.log" | sort | uniq -c | sed 's/^/  /' || true
echo "  脆弱性なし"

# ---- 3. tests ------------------------------------------------------------
step "cargo test"
cargo test >"$TMP/test.log" 2>&1 || { tail -30 "$TMP/test.log" >&2; die "cargo test が失敗しました"; }
echo "  OK"

# ---- 4. version bump + licenses + commit ----------------------------------
step "バージョン更新 ($VERSION) とライセンス一覧の再生成"
awk -v v="$VERSION" '
  /^\[/ { in_pkg = ($0 == "[package]") }
  in_pkg && !done && /^version *=/ { print "version = \"" v "\""; done=1; next }
  { print }
  END { if (!done) exit 3 }' Cargo.toml > "$TMP/Cargo.toml" || die "Cargo.toml の [package] version を更新できません"
cp "$TMP/Cargo.toml" Cargo.toml
cargo update -p holdnote --offline >/dev/null 2>&1 || cargo check --offline >/dev/null 2>&1 || cargo check >/dev/null 2>&1 \
  || die "Cargo.lock を更新できません"
python3 scripts/gen-third-party-licenses.py >/dev/null || die "THIRD_PARTY_LICENSES.txt の生成に失敗しました"
git --no-pager diff --stat -- Cargo.toml Cargo.lock THIRD_PARTY_LICENSES.txt | sed 's/^/  /'
if [ -n "$(git status --porcelain -- Cargo.toml Cargo.lock THIRD_PARTY_LICENSES.txt)" ]; then
  act git add Cargo.toml Cargo.lock THIRD_PARTY_LICENSES.txt
  act git commit -q -m "Release $TAG"
else
  echo "  変更なし（コミットしません）"
fi

# ---- 5. build + verify -----------------------------------------------------
step "ビルドと検証"
if [ "$WHISPER" = 1 ]; then
  scripts/bundle.sh --bundle-whisper || die "bundle.sh が失敗しました"
  scripts/verify-bundle.sh --transcribe || die "verify-bundle.sh が失敗しました"
else
  scripts/bundle.sh || die "bundle.sh が失敗しました"
  scripts/verify-bundle.sh || die "verify-bundle.sh が失敗しました"
fi
ZIP="$ROOT/dist/HoldNote-$VERSION-arm64.zip"
[ -f "$ZIP" ] && [ -f "$ZIP.sha256" ] || die "成果物が見つかりません: $ZIP"
SHA="$(shasum -a 256 "$ZIP" | awk '{print $1}')"
echo "  zip: $ZIP"
echo "  SHA-256: $SHA"

# ---- 6. push + tag -----------------------------------------------------------
step "push とタグ作成"
act git push origin main
act git tag -a "$TAG" -m "HoldNote $VERSION"
act git push origin "$TAG"

# ---- 7. GitHub Release ---------------------------------------------------------
step "GitHub Release の作成"
PREV="$(git describe --tags --abbrev=0 --match 'v*' "$( [ "$DRY" = 1 ] && echo HEAD || echo "$TAG^" )" 2>/dev/null || true)"
if [ -n "$PREV" ]; then RANGE="$PREV..HEAD"; else RANGE="HEAD"; fi
if [ -n "${CHANGES_FILE:-}" ]; then
  CHANGES="$(cat "$CHANGES_FILE")"
else
  # Fallback: commit subjects (no merge commits). Prefer --changes with user-facing Japanese text.
  CHANGES="$(git log --no-merges --pretty='- %s' --invert-grep --grep='^Release v' "$RANGE")"
fi
[ -n "$CHANGES" ] || CHANGES="- （変更履歴なし）"
echo "  前回タグ: ${PREV:-なし}"
NOTES="$TMP/notes.md"
export VERSION SHA REPO CHANGES
python3 -I - packaging/release-notes.md.in "$NOTES" <<'PY'
import os, sys
t = open(sys.argv[1], encoding="utf-8").read()
for k in ("VERSION", "SHA256", "REPO", "CHANGES"):
    t = t.replace("{{%s}}" % k, os.environ["SHA" if k == "SHA256" else k])
open(sys.argv[2], "w", encoding="utf-8").write(t)
PY
if [ "$DRY" = 1 ]; then
  echo "  --- リリースノート (抜粋) ---"; sed -n '1,12p' "$NOTES" | sed 's/^/  | /'; echo "  | ..."
fi
act gh release create "$TAG" "$ZIP" "$ZIP.sha256" --title "HoldNote $VERSION" --notes-file "$NOTES"
RELEASE_URL="https://github.com/$REPO/releases/tag/$TAG"

# ---- 8. verify download ---------------------------------------------------------
step "公開アセットの SHA-256 検証"
ASSET_URL="https://github.com/$REPO/releases/download/$TAG/$(basename "$ZIP")"
if [ "$DRY" = 1 ]; then
  echo "  [dry-run] $ASSET_URL をダウンロードして SHA-256 を照合予定"
else
  curl -fL --retry 3 -sS -o "$TMP/asset.zip" "$ASSET_URL" || die "アセットをダウンロードできません: $ASSET_URL"
  GOT="$(shasum -a 256 "$TMP/asset.zip" | awk '{print $1}')"
  [ "$GOT" = "$SHA" ] || die "SHA-256 が一致しません (local=$SHA, remote=$GOT)"
  echo "  一致しました"
fi

# ---- 9. tap update ----------------------------------------------------------------
step "Homebrew tap の更新"
CASK_REL="Casks/holdnote.rb"
if [ ! -d "$TAP_DIR/.git" ]; then
  need "tap ディレクトリが git リポジトリではありません: ${TAP_DIR}（--tap-dir で指定）"
  TAP_OK=0
else
  TAP_DIR="$(cd "$TAP_DIR" && pwd)"; TAP_OK=1
  [ "$(git -C "$TAP_DIR" branch --show-current)" = main ] || need "tap が main ブランチではありません"
  [ -z "$(git -C "$TAP_DIR" status --porcelain)" ] || need "tap の作業ツリーがクリーンではありません"
  [ -f "$TAP_DIR/$CASK_REL" ] || need "cask がありません: $TAP_DIR/$CASK_REL"
fi
if [ "$DRY" = 1 ]; then
  echo "  [dry-run] 実行予定: git -C $TAP_DIR pull --ff-only"
  echo "  [dry-run] 実行予定: scripts/update-cask.sh --org $ORG $TAP_DIR/$CASK_REL"
  command -v brew >/dev/null && echo "  [dry-run] 実行予定: brew style --fix $TAP_DIR/$CASK_REL"
  echo "  [dry-run] 実行予定: git -C $TAP_DIR commit -am 'holdnote $VERSION' && git push"
else
  git -C "$TAP_DIR" pull --ff-only -q || die "tap の git pull --ff-only に失敗しました"
  scripts/update-cask.sh --org "$ORG" "$TAP_DIR/$CASK_REL" || die "update-cask.sh が失敗しました"
  if command -v brew >/dev/null; then brew style --fix "$TAP_DIR/$CASK_REL" || die "brew style が失敗しました"; fi
  git -C "$TAP_DIR" add "$CASK_REL"
  git -C "$TAP_DIR" commit -q -m "holdnote $VERSION"
  git -C "$TAP_DIR" push origin main
fi
: "$TAP_OK"

# ---- 10. summary ----------------------------------------------------------------------
echo
if [ "$DRY" = 1 ]; then echo "=== dry-run 完了（何も公開していません） ==="; else echo "=== リリース完了 ==="; fi
echo "  バージョン : $VERSION"
echo "  Release    : $RELEASE_URL"
echo "  SHA-256    : $SHA"
echo "  次の作業   :"
echo "    - tap の CI を確認: gh run list --repo $ORG/homebrew-tap --limit 1"
echo "    - 動作確認: brew upgrade --cask $ORG/tap/holdnote"
