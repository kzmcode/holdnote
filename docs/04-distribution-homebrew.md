# 4. 独自 Homebrew Tap での配布

公証していないアプリは公式 Homebrew Cask に入れられないため、独自 Tap から配布します。

## 4-1. 初回のみ

1. GitHub にアプリ本体のリポジトリ（例：`YOUR_ORG/holdnote`）と Tap リポジトリ **`YOUR_ORG/homebrew-tap`** を作成。
2. `packaging/homebrew/holdnote.rb` を Tap リポジトリの `Casks/holdnote.rb` にコピー。
3. `YOUR_ORG` を実際の組織名/ユーザー名に置換（cask の `url` / `homepage`）。
4. whisper-cli を同梱しない場合は cask の `depends_on formula: "whisper.cpp"` のコメントを外す。

## 4-2. リリースごと

```bash
# 0. 依存ライブラリの脆弱性チェック（4-5 参照）。脆弱性が残っている状態ではリリースしない
make audit
# 1. Cargo.toml の version を更新
make bundle            # 同梱する場合は make bundle WHISPER=1
# 2. GitHub Release v<version> を作り dist/HoldNote-<version>-arm64.zip を添付
#    （SHA-256、変更履歴、ライセンス・依存ライブラリ一覧、対応 OS も記載）
scripts/update-cask.sh path/to/homebrew-tap/Casks/holdnote.rb   # version と sha256 を書き換え
# 3. Tap リポジトリにコミット & push
```

## 4-3. 利用者側

```bash
brew install --cask YOUR_ORG/tap/holdnote
brew upgrade --cask YOUR_ORG/tap/holdnote
brew uninstall --cask YOUR_ORG/tap/holdnote      # --zap で設定・履歴も削除
```

初回起動時は Gatekeeper の許可が必要です（[03](03-build-and-first-run.md#3-3-初回起動)）。cask の caveats にも記載済みです。

## 4-4. 確認事項

- 別の Apple Silicon Mac でクリーンインストール → Gatekeeper 許可 → マイク権限 → 録音・保存できること（AT-18）。
- `brew upgrade --cask` 後に設定・履歴が維持され、権限再要求時に案内できること（AT-19）。
- Bundle ID（`io.github.holdnote.HoldNote`）は変更しないこと（変更すると権限・Keychain・LaunchAgent が引き継がれません）。

## 4-5. 脆弱性チェック（リリースごとに必須）

依存ライブラリの脆弱性は随時公表されるため、**リリースのたびに** 確認します。

### 準備（初回のみ）

```bash
cargo install cargo-audit --locked
```

### 実行

```bash
make audit        # = cargo audit
```

RustSec の脆弱性データベースを取得し、`Cargo.lock` の全依存と照合します。

### 結果への対応

| 結果 | 対応 |
|---|---|
| `error: N vulnerabilities found!` | 表示された `Solution`（例：`Upgrade to >=0.23.46`）に従い `cargo update -p <クレート名>` で更新 → `make test` → 再度 `make audit`。0 件になるまでリリースしない |
| 互換バージョンで直らない | `Cargo.toml` の依存バージョンを上げる。上げられない場合は影響範囲を確認し、理由を記録した上で判断する |
| `warning: ... unmaintained` / `unsound` | 内容を確認する。macOS 版に含まれるかは `cargo tree -i <クレート名> --target aarch64-apple-darwin` で確認（`nothing to print` なら macOS 版には含まれない） |

### 現時点で既知の警告（対応不要）

2026-10-15 時点で次の 2 件が警告として出ますが、どちらも Linux 版でのみ使われる依存で、macOS 版には含まれません。

- `glib 0.18.5`（RUSTSEC-2024-0429, unsound）
- `proc-macro-error 1.0.4`（RUSTSEC-2024-0370, unmaintained）

### あわせて確認すること

- リリースノートに zip の SHA-256 を必ず記載する（アドホック署名のため、利用者が配布物の真正性を確認する手段になる）
- Google Cloud Console で Drive API の割り当て上限を設定しておく（クライアント ID 流用時の被害を抑える。[01](01-google-cloud-setup.md) 参照）
