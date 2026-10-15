#!/usr/bin/env bash
# Pre-release checks of a built HoldNote.app.
# Usage: scripts/verify-bundle.sh [--transcribe] [path/to/HoldNote.app]   (default: dist/HoldNote.app)
#   --transcribe  also run the bundled whisper-cli on synthesized Japanese speech (downloads ggml-tiny once)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRANSCRIBE=0
APP=""
for a in "$@"; do
  case "$a" in
    --transcribe) TRANSCRIBE=1 ;;
    -h|--help) sed -n '2,5p' "$0"; exit 0 ;;
    -*) echo "不明なオプション: $a" >&2; exit 2 ;;
    *) APP="$a" ;;
  esac
done
APP="${APP:-$ROOT/dist/HoldNote.app}"
[ -d "$APP" ] || { echo "✘ アプリが見つかりません: ${APP}（先に scripts/bundle.sh を実行してください）" >&2; exit 1; }
APP="$(cd "$APP" && pwd)"
C="$APP/Contents"

PASS=0; FAIL=0; WARN=0
ok()   { echo "✔ $*"; PASS=$((PASS+1)); }
ng()   { echo "✘ $*"; FAIL=$((FAIL+1)); }
warn() { echo "⚠ $*"; WARN=$((WARN+1)); }

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

plist_get() { /usr/libexec/PlistBuddy -c "Print :$1" "$C/Info.plist" 2>/dev/null || true; }

MAIN="$C/MacOS/holdnote"
BIN="$C/Resources/bin"
FW="$C/Frameworks"

# ---- code signature -----------------------------------------------------
if codesign --verify --deep --strict "$APP" 2>"$TMP/cs.err"; then
  ok "codesign --verify --deep --strict に合格"
else
  ng "codesign 検証に失敗: $(tr '\n' ' ' < "$TMP/cs.err")"
fi
if codesign -dv "$MAIN" 2>&1 | grep -E '^CodeDirectory' | grep -q 'runtime'; then
  ok "メイン実行ファイルに Hardened Runtime (runtime) が有効"
else
  ng "メイン実行ファイルに Hardened Runtime (runtime) フラグがありません"
fi

# ---- entitlements -------------------------------------------------------
codesign -d --entitlements - --xml "$MAIN" > "$TMP/ent.plist" 2>/dev/null || true
if [ -s "$TMP/ent.plist" ] && plutil -lint "$TMP/ent.plist" >/dev/null 2>&1; then
  if [ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.device.audio-input' "$TMP/ent.plist" 2>/dev/null || true)" = true ]; then
    ok "entitlements: com.apple.security.device.audio-input = true"
  else
    ng "entitlements に com.apple.security.device.audio-input = true がありません"
  fi
  extra="$(plutil -convert json -o - "$TMP/ent.plist" | python3 -I -c 'import json,sys
for k in json.load(sys.stdin):
    if k != "com.apple.security.device.audio-input": print(k)')"
  if [ -n "$extra" ]; then
    warn "想定外の entitlement: $(echo "$extra" | tr '\n' ' ')"
  else
    ok "想定外の entitlement なし"
  fi
else
  ng "entitlements を取得できません"
fi

# ---- Info.plist ---------------------------------------------------------
if plutil -lint "$C/Info.plist" >/dev/null 2>&1; then ok "Info.plist の構文が正しい"; else ng "Info.plist が不正です"; fi
if [ "$(plist_get CFBundleIdentifier)" = io.github.holdnote.HoldNote ]; then
  ok "CFBundleIdentifier = io.github.holdnote.HoldNote"
else
  ng "CFBundleIdentifier が想定と異なります: $(plist_get CFBundleIdentifier)"
fi
if [ "$(plist_get LSUIElement)" = true ]; then ok "LSUIElement = true"; else ng "LSUIElement が true ではありません"; fi
if [ -n "$(plist_get NSMicrophoneUsageDescription)" ]; then ok "NSMicrophoneUsageDescription が設定済み"; else ng "NSMicrophoneUsageDescription が空です"; fi
if [ -n "$(plist_get LSMinimumSystemVersion)" ]; then ok "LSMinimumSystemVersion = $(plist_get LSMinimumSystemVersion)"; else ng "LSMinimumSystemVersion がありません"; fi
CARGO_VER="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
if [ "$(plist_get CFBundleShortVersionString)" = "$CARGO_VER" ]; then
  ok "CFBundleShortVersionString = Cargo.toml の version ($CARGO_VER)"
else
  ng "バージョン不一致: Info.plist=$(plist_get CFBundleShortVersionString) / Cargo.toml=$CARGO_VER"
fi

# ---- architecture -------------------------------------------------------
archs="$(lipo -archs "$MAIN" 2>/dev/null || true)"
if [ "$archs" = arm64 ]; then ok "メインバイナリは arm64"; else ng "メインバイナリのアーキテクチャが arm64 ではありません: ${archs:-不明}"; fi

# ---- Mach-O files -------------------------------------------------------
MACHO=()
while IFS= read -r f; do
  if file "$f" | grep -q 'Mach-O'; then MACHO+=("$f"); fi
done < <(find "$C" -type f)
leak=0
for f in "${MACHO[@]}"; do
  if strings -a "$f" | grep -q '/Users/'; then ng "ビルド環境のパス (/Users/) が含まれています: ${f#"$APP"/}"; leak=1; fi
done
if [ "$leak" = 0 ]; then ok "全 Mach-O (${#MACHO[@]} 個) に /Users/ の文字列なし"; fi

# ---- bundled whisper-cli ------------------------------------------------
CLI="$BIN/whisper-cli"
HAS_WHISPER=0
if [ -f "$CLI" ]; then
  HAS_WHISPER=1
  libs=("$CLI")
  for f in "$BIN"/*.so "$FW"/*.dylib; do if [ -f "$f" ]; then libs+=("$f"); fi; done
  bad=0
  for f in "${libs[@]}"; do
    if otool -L "$f" | tail -n +2 | grep -qE '/opt/homebrew|/usr/local|/Users'; then
      ng "外部パス参照あり (otool -L): ${f#"$APP"/}"; bad=1
    fi
    rp="$(otool -l "$f" | awk '/cmd LC_RPATH/{r=1} r&&/ path /{print $2; r=0}' | grep '^/' || true)"
    if [ -n "$rp" ]; then ng "絶対パスの LC_RPATH あり: ${f#"$APP"/} ($(echo "$rp" | tr '\n' ' '))"; bad=1; fi
  done
  if [ "$bad" = 0 ]; then ok "whisper-cli と同梱ライブラリ (${#libs[@]} 個) に外部パス・絶対 rpath の参照なし"; fi

  if "$CLI" --help >/dev/null 2>&1; then ok "whisper-cli --help が正常終了"; else ng "whisper-cli --help が失敗しました"; fi

  for n in whisper.cpp ggml libomp; do
    if [ -s "$C/Resources/licenses/$n-LICENSE.txt" ]; then ok "ライセンス同梱: $n"; else ng "ライセンスがありません: licenses/$n-LICENSE.txt"; fi
  done
else
  warn "whisper-cli は同梱されていません（whisper 関連のチェックをスキップ）"
fi

for f in LICENSE THIRD_PARTY_LICENSES.txt; do
  if [ -s "$C/Resources/$f" ]; then ok "Resources/$f が存在"; else ng "Resources/$f がありません"; fi
done

# ---- transcription smoke test ------------------------------------------
if [ "$TRANSCRIBE" = 1 ]; then
  if [ "$HAS_WHISPER" = 0 ]; then
    warn "--transcribe 指定ですが whisper-cli が未同梱のためスキップ"
  else
    CACHE="$HOME/Library/Caches/holdnote-dev"; MODEL="$CACHE/ggml-tiny.bin"
    mkdir -p "$CACHE"
    if [ ! -f "$MODEL" ] || [ "$(stat -f %z "$MODEL")" -lt 10000000 ]; then
      echo "… ggml-tiny.bin をダウンロード中"
      if curl -fL -sS -o "$MODEL.part" https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin; then
        mv "$MODEL.part" "$MODEL"
      else
        rm -f "$MODEL.part"
      fi
    fi
    if [ ! -f "$MODEL" ] || [ "$(stat -f %z "$MODEL")" -lt 10000000 ]; then
      ng "ggml-tiny.bin を取得できません"
    else
      voice=(-v Kyoko)
      if ! say -v '?' | grep -q '^Kyoko'; then warn "Kyoko の音声がないためデフォルト音声を使用"; voice=(); fi
      WAV="$TMP/speech.wav"
      if say ${voice[@]+"${voice[@]}"} -o "$WAV" --data-format=LEI16@16000 "これは文字起こしのテストです" 2>/dev/null; then
        if "$CLI" -m "$MODEL" -f "$WAV" -l ja -nt -np -otxt -of "$TMP/out" >"$TMP/wh.log" 2>&1 \
           && [ -n "$(tr -d '[:space:]' < "$TMP/out.txt" 2>/dev/null)" ]; then
          ok "文字起こしテストに成功: $(tr -d '\n' < "$TMP/out.txt")"
        else
          ng "文字起こしテストに失敗（出力が空、または whisper-cli がエラー）"
        fi
      else
        ng "say による音声合成に失敗しました"
      fi
    fi
  fi
fi

echo
echo "結果: 合格 $PASS / 失敗 $FAIL / 警告 $WARN"
if [ "$FAIL" -gt 0 ]; then echo "検証に失敗しました。"; exit 1; fi
echo "すべてのチェックに合格しました。"
