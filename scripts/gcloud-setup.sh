#!/usr/bin/env bash
# HoldNote: Google Cloud setup helper.
# Automates what gcloud officially supports (project, API enablement) and guides
# the rest (consent screen / Desktop OAuth client) through console deep links.
#
# Why the rest is manual (verified 2026-10):
#  - The IAP OAuth Admin API (gcloud iap oauth-brands/oauth-clients) was shut down on
#    2026-03-19 and only ever created IAP-locked web clients:
#    https://docs.cloud.google.com/iap/docs/programmatic-oauth-clients
#  - Google Auth Platform (branding / audience / data access / clients) is documented
#    as console-only: https://support.google.com/cloud/answer/15549257
set -euo pipefail

CONSOLE="https://console.cloud.google.com"
PROJECT=""
CREATE=0
OPEN=1
GCLOUD_OK=0

usage() {
  cat <<'USAGE'
使い方: scripts/gcloud-setup.sh [--project PROJECT_ID] [--create] [--no-open] [--help]

HoldNote 用の Google Cloud 設定を支援します。

  --project ID   対象のプロジェクト ID（省略時は gcloud の現在のプロジェクト、なければ選択）
  --create       --project で指定した ID のプロジェクトを新規作成（既存ならそれを使用）
  --no-open      ブラウザでコンソールのページを開かず、URL の表示のみ行う
  -h, --help     このヘルプを表示

自動化する内容 (gcloud が必要):
  - プロジェクト作成 / 選択
  - Google Drive API と Google Picker API の有効化と確認

手動のまま (Google が公式に API を提供していないため):
  - OAuth 同意画面 (Google Auth Platform: ブランディング / 対象 / データアクセス)
  - デスクトップ アプリ用 OAuth クライアントの作成
  - API 割り当て上限の設定
  このスクリプトが各ページを順に開き、入力内容を案内します。

最後に、クライアント ID / シークレットを settings.toml へ書き込むことも選べます。
USAGE
}

info() { printf '%s\n' "$*"; }
warn() { printf '警告: %s\n' "$*" >&2; }
die()  { printf 'エラー: %s\n' "$*" >&2; exit 1; }

is_tty() { [[ -t 0 ]]; }

# ---- gcloud detection -------------------------------------------------------
gcloud_ready() {
  command -v gcloud >/dev/null 2>&1 || return 1
  local acct
  acct=$(gcloud auth list --filter=status:ACTIVE --format='value(account)' 2>/dev/null || true)
  [[ -n "$acct" ]]
}

explain_install() {
  cat <<'MSG'

gcloud CLI が見つからないか、ログインしていません。
  インストール:  brew install --cask gcloud-cli
  ログイン:      gcloud auth login
  (インストール後はシェルを開き直してください)

gcloud なしでも「コンソールのみ」モードで続行できます
(API の有効化も含め、すべてブラウザ上で手動操作になります)。
MSG
}

# ---- project handling -------------------------------------------------------
choose_project() {
  local current list
  current=$(gcloud config get-value project 2>/dev/null || true)
  current=${current//\(unset\)/}
  if [[ -n "$current" ]]; then PROJECT="$current"; info "現在の gcloud プロジェクトを使用します: $PROJECT"; return; fi
  is_tty || die "プロジェクトが未指定です。--project PROJECT_ID を指定するか、gcloud config set project ID を実行してください。"
  list=$(gcloud projects list --format='value(projectId)' 2>/dev/null || true)
  [[ -n "$list" ]] || die "アクセス可能なプロジェクトがありません。--create --project 新しいID で作成してください。"
  info "利用可能なプロジェクト:"
  local -a arr=()
  while IFS= read -r line; do arr+=("$line"); done <<<"$list"
  local i=1 p
  for p in "${arr[@]}"; do printf '  %d) %s\n' "$i" "$p"; i=$((i+1)); done
  local n
  read -r -p "番号を選んでください: " n
  [[ "$n" =~ ^[0-9]+$ && "$n" -ge 1 && "$n" -le ${#arr[@]} ]] || die "無効な選択です。"
  PROJECT="${arr[$((n-1))]}"
}

create_project() {
  if [[ -z "$PROJECT" ]]; then
    is_tty || die "--create には --project PROJECT_ID が必要です。"
    read -r -p "作成するプロジェクト ID (小文字・数字・ハイフン、6〜30 文字): " PROJECT
  fi
  [[ "$PROJECT" =~ ^[a-z][a-z0-9-]{4,28}[a-z0-9]$ ]] || die "プロジェクト ID の形式が不正です: $PROJECT"
  local out
  if out=$(gcloud projects create "$PROJECT" --name="HoldNote" 2>&1); then
    info "プロジェクトを作成しました: $PROJECT"
  elif grep -qi "already exists" <<<"$out"; then
    if gcloud projects describe "$PROJECT" >/dev/null 2>&1; then
      info "プロジェクト $PROJECT は既に存在します。そのまま使用します。"
    else
      die "ID $PROJECT は既に他者に使われています。別の ID を指定してください。"
    fi
  else
    printf '%s\n' "$out" >&2
    die "プロジェクトの作成に失敗しました。"
  fi
}

enable_apis() {
  local apis=(drive.googleapis.com picker.googleapis.com)
  info "API を有効化します: ${apis[*]}"
  gcloud services enable "${apis[@]}" --project="$PROJECT" || die "API の有効化に失敗しました。"
  local missing=0 api
  for api in "${apis[@]}"; do
    if [[ -n "$(gcloud services list --enabled --project="$PROJECT" --filter="config.name=$api" --format='value(config.name)' 2>/dev/null)" ]]; then
      info "  OK: $api は有効です"
    else
      warn "$api が有効になっていません"; missing=1
    fi
  done
  [[ $missing -eq 0 ]] || die "有効化の確認に失敗しました。"
}

# ---- console guidance -------------------------------------------------------
console_url() { # $1 = path under console root
  if [[ -n "$PROJECT" ]]; then printf '%s/%s?project=%s' "$CONSOLE" "$1" "$PROJECT"
  else printf '%s/%s' "$CONSOLE" "$1"; fi
}

step() { # $1 title, $2 url, $3.. instructions
  local title=$1 url=$2; shift 2
  printf '\n== %s ==\n' "$title"
  local l; for l in "$@"; do printf '   - %s\n' "$l"; done
  printf '   URL: %s\n' "$url"
  if [[ $OPEN -eq 1 ]]; then
    open "$url" >/dev/null 2>&1 || warn "ブラウザを開けませんでした。上の URL を手動で開いてください。"
    if is_tty; then read -r -p "   完了したら Enter を押してください... " _ || true; fi
  fi
}

console_guide() {
  info ""
  info "以下が手動で行う手順のチェックリストです。"
  local n=1
  if [[ $GCLOUD_OK -eq 0 ]]; then
    step "$n. Google Drive API を有効化" "$(console_url apis/library/drive.googleapis.com)" \
      "「有効にする」をクリック"; n=$((n+1))
    step "$n. Google Picker API を有効化" "$(console_url apis/library/picker.googleapis.com)" \
      "「有効にする」をクリック"; n=$((n+1))
  fi
  step "$n. ブランディング (OAuth 同意画面)" "$(console_url auth/branding)" \
    "「開始」→ アプリ名: HoldNote" "ユーザー サポートメール / デベロッパーの連絡先メールを入力" \
    "(初回は「対象」で ユーザーの種類: 外部 を選ぶウィザードが出ます)"; n=$((n+1))
  step "$n. 対象 (Audience)" "$(console_url auth/audience)" \
    "ユーザーの種類: 外部 (組織内のみなら 内部 でも可)" \
    "公開ステータス: 「テスト中」ならテストユーザーに自分の Google アカウントを追加" \
    "  注意: テスト中は更新トークンが 7 日で失効し、毎週再認証が必要です" \
    "  継続利用するなら「アプリを公開」で本番環境へ (drive.file は非機密スコープのため審査不要)"; n=$((n+1))
  step "$n. データアクセス (スコープ)" "$(console_url auth/scopes)" \
    "「スコープを追加または削除」で次の 3 つを追加して保存:" \
    "  https://www.googleapis.com/auth/drive.file" "  openid" "  email" \
    "広範な drive スコープは追加しないでください"; n=$((n+1))
  step "$n. OAuth クライアントを作成" "$(console_url auth/clients/create)" \
    "アプリケーションの種類: デスクトップ アプリ" "名前: HoldNote (任意)" \
    "「作成」後に表示されるクライアント ID とクライアント シークレットを控える" \
    "(シークレットは作成直後にしか全文表示されない場合があります)"; n=$((n+1))
  step "$n. (推奨) Drive API の割り当て上限を下げる" "$(console_url apis/api/drive.googleapis.com/quotas)" \
    "ユーザーあたり・プロジェクトあたりのリクエスト上限を利用規模に見合う値へ"
  info ""
  info "HoldNote「設定」→「Google Drive」タブへクライアント ID / シークレットを入力するか、"
  info "このスクリプトの次の手順で settings.toml に書き込めます。"
}

# ---- settings.toml writer ---------------------------------------------------
settings_path() { printf '%s/Library/Application Support/HoldNote/settings.toml' "$HOME"; }

valid_client_id() { [[ "$1" =~ ^[A-Za-z0-9._-]+\.apps\.googleusercontent\.com$ ]]; }

# Reads GN_ID and GN_SECRET from the environment (never argv). Preserves other keys.
write_settings() {
  local file; file=$(settings_path)
  mkdir -p "$(dirname "$file")"
  python3 -I - "$file" <<'PY'
import os, sys, re, tempfile, tomllib
path = sys.argv[1]
vals = {"google_client_id": os.environ["GN_ID"], "google_client_secret": os.environ["GN_SECRET"]}
lines = open(path, encoding="utf-8").read().splitlines() if os.path.exists(path) else []
if lines:
    tomllib.loads("\n".join(lines))  # refuse to touch an already-invalid file
def q(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'
first_table = next((i for i, l in enumerate(lines) if re.match(r"\s*\[", l)), len(lines))
for key, val in vals.items():
    pat = re.compile(r"\s*" + re.escape(key) + r"\s*=")
    idx = next((i for i in range(first_table) if pat.match(lines[i])), None)
    new = f"{key} = {q(val)}"
    if idx is not None:
        lines[idx] = new
    else:
        lines.insert(first_table, new)
        first_table += 1
text = "\n".join(lines) + "\n"
parsed = tomllib.loads(text)
assert parsed["google_client_id"] == vals["google_client_id"]
assert parsed["google_client_secret"] == vals["google_client_secret"]
d = os.path.dirname(path)
fd, tmp = tempfile.mkstemp(dir=d, prefix=".settings.")
os.fchmod(fd, 0o600)
with os.fdopen(fd, "w", encoding="utf-8") as f:
    f.write(text)
os.replace(tmp, path)
PY
}

offer_write_settings() {
  is_tty || return 0
  local ans
  read -r -p $'\nクライアント ID / シークレットを settings.toml に書き込みますか? [y/N] ' ans
  [[ "$ans" =~ ^[Yy]$ ]] || { info "スキップしました。HoldNote の設定画面から入力してください。"; return 0; }
  command -v python3 >/dev/null 2>&1 || die "python3 が必要です。"
  while pgrep -x holdnote >/dev/null 2>&1; do
    warn "HoldNote が起動中です。終了してから続行してください (起動中に書き込むと上書きされます)。"
    read -r -p "終了したら Enter (中止は Ctrl-C)... " _ || return 1
  done
  local id secret
  read -r -p "クライアント ID: " id
  valid_client_id "$id" || die "クライアント ID の形式が不正です (例: 123-abc.apps.googleusercontent.com)。"
  read -r -s -p "クライアント シークレット (入力は表示されません): " secret; echo
  [[ -n "$secret" ]] || die "シークレットが空です。"
  GN_ID="$id" GN_SECRET="$secret" write_settings || die "settings.toml の書き込みに失敗しました。"
  info "書き込みました: $(settings_path) (権限 600)"
}

# ---- main -------------------------------------------------------------------
main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --project) [[ $# -ge 2 ]] || die "--project には値が必要です。"; PROJECT="$2"; shift 2 ;;
      --project=*) PROJECT="${1#*=}"; shift ;;
      --create) CREATE=1; shift ;;
      --no-open) OPEN=0; shift ;;
      -h|--help) usage; exit 0 ;;
      *) usage >&2; die "不明なオプション: $1" ;;
    esac
  done

  if gcloud_ready; then
    GCLOUD_OK=1
    info "gcloud を検出しました (アカウント: $(gcloud auth list --filter=status:ACTIVE --format='value(account)' | head -n1))"
    if [[ $CREATE -eq 1 ]]; then create_project; elif [[ -z "$PROJECT" ]]; then choose_project; fi
    info "対象プロジェクト: $PROJECT"
    enable_apis
    info ""
    info "同意画面とデスクトップ OAuth クライアントは gcloud / 公式 API では作成できません。"
    info "(IAP OAuth Admin API は 2026-03-19 に終了。Google Auth Platform はコンソール専用)"
  else
    explain_install
    if is_tty; then
      local ans
      read -r -p "コンソールのみモードで続行しますか? [y/N] " ans
      [[ "$ans" =~ ^[Yy]$ ]] || exit 1
      [[ -n "$PROJECT" ]] || read -r -p "プロジェクト ID (不明なら空 Enter): " PROJECT
    else
      die "gcloud が利用できません。上記の手順で準備するか、対話端末で実行してください。"
    fi
  fi

  console_guide
  offer_write_settings
  info ""
  info "完了。HoldNote を起動し「Googleに接続」→「保存先フォルダを選択」へ進んでください。"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then main "$@"; fi
