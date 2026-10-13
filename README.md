# HoldNote

ホットキーを押している間だけ録音し、ローカルの whisper.cpp で文字起こしして、Markdown を **Google Drive API** で指定フォルダに保存する macOS メニューバー常駐アプリ（Rust 製）。UI テーマは Coral Night。

- 要件定義：[requirements.md](requirements.md)（本実装は保存方式 B: Google Drive API 版）
- セットアップと手動作業：[docs/README.md](docs/README.md)

```bash
make test      # 単体テスト
make bundle    # dist/HoldNote.app を作成（アドホック署名）
make run-app   # 起動
```

## 構成

| モジュール | 役割 |
|---|---|
| `src/app.rs` | tao イベントループ、メニューバー、録音フロー、状態管理 |
| `src/hotkey.rs` | グローバルホットキーと解放フェイルセーフ |
| `src/recorder.rs` | cpal 録音 → 16kHz モノラル WAV |
| `src/transcriber.rs` | whisper-cli 実行・モデル管理 |
| `src/markdown.rs` | Markdown 生成 |
| `src/jobs.rs` / `src/worker.rs` | SQLite 永続キューとバックグラウンド処理・再試行 |
| `src/storage/` | OAuth（PKCE・ループバック・Picker）と Drive API v3 |
| `src/security.rs` | Keychain へのトークン保存 |
| `src/ui/` | Coral Night テーマ、録音オーバーレイ、設定・履歴ウィンドウ |
