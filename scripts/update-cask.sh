#!/usr/bin/env bash
# Fill version + sha256 of the newest dist/HoldNote-<ver>-arm64.zip into a cask.
# Usage: scripts/update-cask.sh [--org NAME] [path/to/holdnote.rb]
#   --org NAME   also replace the YOUR_ORG placeholder (use for the copy in your tap repo;
#                the template in packaging/homebrew/ keeps the placeholder)
#   default cask path: packaging/homebrew/holdnote.rb
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ORG=""
if [ "${1:-}" = "--org" ]; then
  ORG="${2:?--org needs a value}"; shift 2
fi
CASK="${1:-$ROOT/packaging/homebrew/holdnote.rb}"

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ZIP="$ROOT/dist/HoldNote-$VERSION-arm64.zip"
[ -f "$ZIP" ] || { echo "missing $ZIP (run scripts/bundle.sh first)" >&2; exit 1; }
[ -f "$CASK" ] || { echo "missing cask: $CASK" >&2; exit 1; }

SHA="$(shasum -a 256 "$ZIP" | awk '{print $1}')"
sed -i '' -E \
  -e "s/^( *version )\".*\"/\1\"$VERSION\"/" \
  -e "s/^( *sha256 )\".*\"/\1\"$SHA\"/" "$CASK"
if [ -n "$ORG" ]; then
  sed -i '' -e "s/YOUR_ORG/$ORG/g" "$CASK"
fi
echo "Updated $CASK: version=$VERSION sha256=$SHA${ORG:+ org=$ORG}"
