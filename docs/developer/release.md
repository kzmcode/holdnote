# リリースと配布

公証していないアプリは公式の Homebrew Cask に入れられないため、独自の Homebrew Tap から配布します。
このリポジトリ内の雛形と手順書は、フォークして使えるよう組織名を `YOUR_ORG` と表記しています。

## 構成

```
YOUR_ORG/holdnote              アプリ本体。GitHub Releases に zip を置く
YOUR_ORG/homebrew-tap          Tap。Casks/holdnote.rb だけを置く（他のアプリと共用可）
```

利用者は `brew install --cask YOUR_ORG/tap/holdnote` でインストールします。

## 初回のみ

1. GitHub に `YOUR_ORG/holdnote` と `YOUR_ORG/homebrew-tap` を作成する（どちらも **Public**。Release の zip を誰でもダウンロードできる必要がある）
2. `homebrew-tap` を本リポジトリの隣（`../homebrew-tap`）に clone し、`packaging/homebrew/holdnote.rb` を `Casks/holdnote.rb` にコピーする
3. 準備
   ```bash
   cargo install cargo-audit --locked   # 脆弱性チェック
   brew install whisper.cpp             # whisper-cli の同梱用
   gh auth login                        # GitHub CLI
   ```
4. 推奨する GitHub の設定（両リポジトリ）：Dependabot アラートを有効化、`main` の強制プッシュ・削除を禁止（ruleset）、未使用の Wiki を無効化

## リリース（`make release`）

```bash
make release VERSION=0.2.0 DRY_RUN=1   # まず確認だけ（GitHub には何も送らない）
make release VERSION=0.2.0             # 本番
```

`scripts/release.sh <version> [--dry-run] [--tap-dir PATH] [--no-whisper]` が、次を順に実行します。途中で失敗するとそこで止まります。

| # | 内容 |
|---|---|
| 1 | 事前確認：`main` ブランチ、未コミットの変更なし、`origin/main` と一致、タグが未使用、`gh` にログイン済み |
| 2 | `cargo audit`：脆弱性が1件でもあれば中止 |
| 3 | `cargo test` |
| 4 | `Cargo.toml` のバージョン更新、`THIRD_PARTY_LICENSES.txt` の再生成、コミット |
| 5 | `scripts/bundle.sh --bundle-whisper` でビルドし、`scripts/verify-bundle.sh --transcribe` で検証 |
| 6 | push、タグ `v<version>` の作成と push |
| 7 | GitHub Release の作成（zip と SHA-256 を添付。リリースノートは `packaging/release-notes.md.in` から生成し、前回タグからのコミット一覧を「変更点」に入れる） |
| 8 | Release からダウンロードし直して SHA-256 を照合 |
| 9 | Tap の `Casks/holdnote.rb` を `scripts/update-cask.sh --org <組織名>` で更新し、`brew style --fix` をかけてコミット・push |

完了後、Tap の CI（`brew style` / `brew audit`）が成功することを確認します。

```bash
gh run list --repo YOUR_ORG/homebrew-tap --limit 1
```

### 手動で行う場合

`make release` を使わない場合の個別コマンドです。

```bash
make audit
cargo test
scripts/gen-third-party-licenses.py
make bundle WHISPER=1 && make verify
gh release create v<version> dist/HoldNote-<version>-arm64.zip dist/HoldNote-<version>-arm64.zip.sha256
scripts/update-cask.sh --org YOUR_ORG ../homebrew-tap/Casks/holdnote.rb
```

## 脆弱性チェック

`make audit`（= `cargo audit`）は RustSec のデータベースと `Cargo.lock` の全依存を照合します。`make release` では自動で実行されます。

| 結果 | 対応 |
|---|---|
| `error: N vulnerabilities found!` | 表示された `Solution` に従い `cargo update -p <クレート名>` → `make test` → 再度 `make audit`。0 件になるまでリリースしない |
| 互換バージョンで直らない | `Cargo.toml` の依存バージョンを上げる。上げられない場合は影響範囲を確認し、理由を記録して判断する |
| `warning: ... unmaintained` / `unsound` | macOS 版に含まれるかを `cargo tree -i <クレート名> --target aarch64-apple-darwin` で確認（`nothing to print` なら含まれない） |

2026-10-15 時点の既知の警告（どちらも Linux 版のみの依存で、対応不要）：

- `glib 0.18.5`（RUSTSEC-2024-0429, unsound）
- `proc-macro-error 1.0.4`（RUSTSEC-2024-0370, unmaintained）

## ライセンス表記

- 本体：`LICENSE`（MIT）
- Rust の依存クレート：`THIRD_PARTY_LICENSES.txt`（`scripts/gen-third-party-licenses.py` で生成。`make release` が自動で再生成）
- whisper-cli を同梱した場合：`bundle.sh` が whisper.cpp・ggml（MIT）、libomp（Apache-2.0 WITH LLVM-exception）のライセンス本文を `Contents/Resources/licenses/` にコピーする
- `option-ext`（`dirs` が使用）は MPL-2.0。改変せずに使っており、ソースの入手先を一覧に記載済み。改変する場合は改変部分のソース公開が必要

## 注意事項

- **Bundle ID（`io.github.holdnote.HoldNote`）は変更しない。** 変更すると、マイク権限・キーチェーン・ログイン時起動が引き継がれない。
- アドホック署名のため、利用者が配布物の真正性を確かめる手段はリリースノートの SHA-256 と Cask の照合だけ。リリースノートから SHA-256 を消さないこと。
- Google Cloud の OAuth クライアントは利用者が各自で作る（[Google Cloud の準備](../google-cloud-setup.md)）。
- 新しい Mac でのクリーンインストール・アップデートの確認は [実機検証チェックリスト](verification-checklist.md) の「配布」を参照。
