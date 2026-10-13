# 4. 独自 Homebrew Tap での配布

公証していないアプリは公式 Homebrew Cask に入れられないため、独自 Tap から配布します。

## 4-1. 初回のみ

1. GitHub にアプリ本体のリポジトリ（例：`YOUR_ORG/holdnote`）と Tap リポジトリ **`YOUR_ORG/homebrew-tap`** を作成。
2. `packaging/homebrew/holdnote.rb` を Tap リポジトリの `Casks/holdnote.rb` にコピー。
3. `YOUR_ORG` を実際の組織名/ユーザー名に置換（cask の `url` / `homepage`）。
4. whisper-cli を同梱しない場合は cask の `depends_on formula: "whisper.cpp"` のコメントを外す。

## 4-2. リリースごと

```bash
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
