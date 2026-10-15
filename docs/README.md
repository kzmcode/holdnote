# HoldNote 手動作業ガイド（Google Drive API版）

実装後に人が手で行う必要がある作業の一覧です。上から順に進めてください。

| # | ドキュメント | 内容 | 必須 |
|---|---|---|---|
| 1 | [01-google-cloud-setup.md](01-google-cloud-setup.md) | Google Cloud プロジェクト・OAuth クライアント・API 有効化 | 必須 |
| 2 | [02-whisper-setup.md](02-whisper-setup.md) | whisper.cpp（`whisper-cli`）の導入と音声認識モデル | 必須 |
| 3 | [03-build-and-first-run.md](03-build-and-first-run.md) | ビルド・`.app` 作成・初回起動・macOS 権限 | 必須 |
| 4 | [04-distribution-homebrew.md](04-distribution-homebrew.md) | 独自 Homebrew Tap での配布・リリース手順・**リリース前の脆弱性チェック** | 配布する場合 |
| 5 | [05-verification-checklist.md](05-verification-checklist.md) | 実機でしか確認できない検証項目（Phase 0 / 受け入れテスト） | 必須 |

## この版の範囲

- 保存方式は **B: Google Drive API への直接保存のみ** を実装しています（要件定義の方式 A「Drive for desktop 同期フォルダ」は未実装）。
- 内部は `SaveBackend` トレイトで抽象化してあるため、方式 A は後から `src/storage/` に追加できます。
