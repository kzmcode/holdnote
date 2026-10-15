# HoldNote ドキュメント

## 使う人向け

| ドキュメント | 内容 |
|---|---|
| **[ユーザーマニュアル](user-manual.md)** | インストール、初期設定、使い方、困ったとき。**まずはこちら** |
| [Google Cloud の準備](google-cloud-setup.md) | Google Drive 連携に必要な OAuth クライアントの作り方（最初に1回だけ） |

## 開発する人向け

| ドキュメント | 内容 |
|---|---|
| [ビルドと開発](developer/build.md) | ビルド、whisper.cpp の同梱、開発用オプション |
| [リリースと配布](developer/release.md) | `make release` によるリリース、Homebrew Tap、脆弱性チェック、ライセンス |
| [実機検証チェックリスト](developer/verification-checklist.md) | 自動テストでは確認できない項目 |

## 主なコマンド

| コマンド | 内容 |
|---|---|
| `make test` | 単体テスト |
| `make bundle` / `make bundle WHISPER=1` | `.app` と配布用 zip を作成（whisper 同梱） |
| `make verify` | 作成した `.app` の自動チェック（署名・権限・同梱ライブラリ・ライセンス） |
| `make release VERSION=x.y.z` | リリースを一括実行（`DRY_RUN=1` で確認のみ） |
| `make audit` | 依存ライブラリの脆弱性チェック |
| `make docs-check` | ドキュメント内のリンク切れチェック |
| `scripts/gcloud-setup.sh` | Google Cloud の準備を半自動化 |
| `holdnote --doctor` | 利用環境の診断 |
