# HoldNote

ホットキーを押している間だけ録音し、ローカルの whisper.cpp で文字起こしして、Markdown を **Google Drive API** で指定フォルダに保存する macOS メニューバー常駐アプリ（Rust 製）。UI テーマは Coral Night。

- 要件定義：[requirements.md](requirements.md)（本実装は保存方式 B: Google Drive API 版）
- **使い方：[ユーザーマニュアル](docs/user-manual.md)**
- 開発・配布：[docs/README.md](docs/README.md)

## インストール（利用者向け）

Apple Silicon Mac（macOS 14 以降）、Homebrew が必要です。

```bash
brew install --cask kzmcode/tap/holdnote
```

- Apple の公証を受けていないため、初回起動時に **システム設定 → プライバシーとセキュリティ →「このまま開く」** で許可してください。
- Google Drive への保存には Google Cloud の OAuth クライアントの準備が必要です（[Google Cloud の準備](docs/google-cloud-setup.md)）。

## 開発

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

## ライセンス

[MIT License](LICENSE)。同梱しているサードパーティ製ソフトウェアのライセンスは [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) と、
アプリ内の `HoldNote.app/Contents/Resources/licenses/`（whisper.cpp・ggml・libomp）を参照してください。
