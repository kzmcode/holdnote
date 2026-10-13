#!/usr/bin/env bash
# Fill version + sha256 of the newest dist/HoldNote-<ver>-arm64.zip into the cask.
# Usage: scripts/update-cask.sh [path/to/holdnote.rb]   (default: packaging/homebrew/holdnote.rb)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CASK="${1:-$ROOT/packaging/homebrew/holdnote.rb}"

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ZIP="$ROOT/dist/HoldNote-$VERSION-arm64.zip"
[ -f "$ZIP" ] || { echo "missing $ZIP (run scripts/bundle.sh first)" >&2; exit 1; }
[ -f "$CASK" ] || { echo "missing cask: $CASK" >&2; exit 1; }

SHA="$(shasum -a 256 "$ZIP" | awk '{print $1}')"
sed -i '' -E \
  -e "s/^( *version )\".*\"/\1\"$VERSION\"/" \
  -e "s/^( *sha256 )\".*\"/\1\"$SHA\"/" "$CASK"
echo "Updated $CASK: version=$VERSION sha256=$SHA"
