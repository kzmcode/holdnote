#!/usr/bin/env bash
# Build dist/HoldNote.app (ad-hoc signed) and dist/HoldNote-<ver>-arm64.zip.
# Usage: scripts/bundle.sh [--bundle-whisper]
#   WHISPER_CLI=/path/to/whisper-cli   bundle that binary (+ dylibs)
#   --bundle-whisper                   bundle Homebrew's whisper-cli ($(brew --prefix whisper.cpp))
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BUNDLE_WHISPER=0
for a in "$@"; do
  case "$a" in
    --bundle-whisper) BUNDLE_WHISPER=1 ;;
    -h|--help) sed -n '2,6p' "$0"; exit 0 ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$VERSION" ] || { echo "cannot read version from Cargo.toml" >&2; exit 1; }

DIST="$ROOT/dist"
APP="$DIST/HoldNote.app"
ZIP="$DIST/HoldNote-$VERSION-arm64.zip"

echo "==> cargo build --release (arm64)"
if [ "$(uname -m)" = arm64 ]; then
  cargo build --release --bin holdnote
  BIN="$ROOT/target/release/holdnote"
else
  cargo build --release --bin holdnote --target aarch64-apple-darwin
  BIN="$ROOT/target/aarch64-apple-darwin/release/holdnote"
fi
file "$BIN" | grep -q arm64 || { echo "binary is not arm64: $BIN" >&2; exit 1; }

echo "==> assembling $APP"
rm -rf "$APP" "$ZIP" "$ZIP.sha256"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/holdnote"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
sed "s/@VERSION@/$VERSION/g" packaging/Info.plist.in > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"

# ---- optional whisper-cli bundling -------------------------------------
WHISPER_SRC="${WHISPER_CLI:-}"
if [ -z "$WHISPER_SRC" ] && [ "$BUNDLE_WHISPER" = 1 ]; then
  WHISPER_SRC="$(brew --prefix whisper.cpp)/bin/whisper-cli"
fi

SEARCH_DIRS=""   # newline-separated dirs used to resolve @rpath/ etc.

is_system() { case "$1" in /usr/lib/*|/System/*) return 0 ;; *) return 1 ;; esac; }

resolve_dep() { # $1 = install name as printed by otool
  local dep="$1" base d
  base="$(basename "$dep")"
  case "$dep" in /*) if [ -e "$dep" ]; then echo "$dep"; return 0; fi ;; esac
  while IFS= read -r d; do
    if [ -n "$d" ] && [ -e "$d/$base" ]; then echo "$d/$base"; return 0; fi
  done <<< "$SEARCH_DIRS"
  return 1
}

rpaths_of() { otool -l "$1" | awk '/cmd LC_RPATH/{f=1} f&&/ path /{print $2; f=0}'; }

if [ -n "$WHISPER_SRC" ]; then
  [ -x "$WHISPER_SRC" ] || { echo "whisper-cli not found/executable: $WHISPER_SRC" >&2; exit 1; }
  echo "==> bundling whisper-cli from $WHISPER_SRC"
  FW="$APP/Contents/Frameworks"
  CLI="$APP/Contents/Resources/bin/whisper-cli"
  mkdir -p "$APP/Contents/Resources/bin" "$FW"
  cp -L "$WHISPER_SRC" "$CLI"
  chmod u+w "$CLI"

  SRC_DIR="$(cd "$(dirname "$WHISPER_SRC")" && pwd -P)"
  PREFIX="$(brew --prefix 2>/dev/null || echo /opt/homebrew)"
  SEARCH_DIRS="$SRC_DIR/../lib
$SRC_DIR
$PREFIX/lib"
  while IFS= read -r rp; do
    case "$rp" in
      /*) SEARCH_DIRS="$SEARCH_DIRS
$rp" ;;
      @loader_path/*|@executable_path/*) SEARCH_DIRS="$SEARCH_DIRS
$SRC_DIR/${rp#*/}" ;;
    esac
  done < <(rpaths_of "$WHISPER_SRC")

  # worklist lines: "<bundled file>|<kind exe|lib>|<original file>"
  QUEUE="$(mktemp)"; DONE="$(mktemp)"; trap 'rm -f "$QUEUE" "$DONE"' EXIT
  echo "$CLI|exe|$WHISPER_SRC" > "$QUEUE"
  while [ -s "$QUEUE" ]; do
    line="$(head -1 "$QUEUE")"; sed -i '' '1d' "$QUEUE"
    file="${line%%|*}"; rest="${line#*|}"; kind="${rest%%|*}"; orig="${rest#*|}"
    if grep -qxF "$file" "$DONE"; then continue; fi
    echo "$file" >> "$DONE"
    while IFS= read -r dep; do
      is_system "$dep" && continue
      base="$(basename "$dep")"
      [ "$base" = "$(basename "$file")" ] && continue   # the dylib's own id line
      if real="$(resolve_dep "$dep")"; then
        if [ ! -e "$FW/$base" ]; then
          cp -L "$real" "$FW/$base"; chmod u+w "$FW/$base"
          install_name_tool -id "@loader_path/$base" "$FW/$base"
          # dylib rpaths may also help resolve its own @rpath deps
          while IFS= read -r rp; do
            case "$rp" in /*) SEARCH_DIRS="$SEARCH_DIRS
$rp" ;; esac
          done < <(rpaths_of "$real")
          SEARCH_DIRS="$SEARCH_DIRS
$(dirname "$real")"
          echo "$FW/$base|lib|$real" >> "$QUEUE"
        fi
        if [ "$kind" = exe ]; then new="@executable_path/../../Frameworks/$base"; else new="@loader_path/$base"; fi
        install_name_tool -change "$dep" "$new" "$file"
      else
        echo "WARNING: cannot resolve $dep (needed by $file)" >&2
      fi
    done < <(otool -L "$orig" | tail -n +2 | awk '{print $1}')
  done
  # strip rpaths that point at the build machine
  for f in "$CLI" "$FW"/*; do
    [ -f "$f" ] || continue
    for rp in $(rpaths_of "$f"); do
      install_name_tool -delete_rpath "$rp" "$f" 2>/dev/null || true
    done
  done
fi

rmdir "$APP/Contents/Frameworks" 2>/dev/null || true
# ---- ad-hoc signing (inside-out; helpers without hardened runtime) --------
echo "==> codesign (ad-hoc)"
if [ -d "$APP/Contents/Frameworks" ]; then
  for f in "$APP"/Contents/Frameworks/*; do [ -f "$f" ] && codesign --force -s - "$f"; done
fi
if [ -f "$APP/Contents/Resources/bin/whisper-cli" ]; then
  codesign --force -s - "$APP/Contents/Resources/bin/whisper-cli"
fi
codesign --force --deep -s - --options runtime \
  --entitlements packaging/HoldNote.entitlements "$APP"
codesign --verify --deep --strict "$APP"
codesign -dv "$APP" 2>&1 | sed 's/^/    /'

echo "==> zip"
mkdir -p "$DIST"
ditto -c -k --keepParent "$APP" "$ZIP"
SHA="$(shasum -a 256 "$ZIP" | awk '{print $1}')"
echo "$SHA  $(basename "$ZIP")" > "$ZIP.sha256"
echo "Created: $ZIP"
echo "SHA-256: $SHA"
