# ビルドと開発

利用者向けの説明は [ユーザーマニュアル](../user-manual.md) を参照してください。

## 必要なもの

- Apple Silicon Mac / macOS 14 以降
- Rust（stable）、Xcode Command Line Tools（`xcode-select --install`）
- whisper-cli を同梱する場合：`brew install whisper.cpp`
- Apple Developer Program は **不要**（アドホック署名のみ）

## ビルド

```bash
make test               # 単体テスト
make bundle WHISPER=1   # dist/HoldNote.app と dist/HoldNote-<ver>-arm64.zip を作成（whisper-cli 同梱）
make verify             # 作成した .app の自動チェック（下記）
make run-app            # バンドルして起動
```

`scripts/bundle.sh` は次を行います。

1. release ビルド（ビルド環境のホームディレクトリのパスがバイナリに残らないよう `--remap-path-prefix` を指定）
2. `.app` の組み立て（`Info.plist`、`AppIcon.icns`、`LICENSE`、`THIRD_PARTY_LICENSES.txt`）
3. whisper-cli の同梱（`--bundle-whisper` 指定時。下記）
4. アドホック署名（hardened runtime、マイクの entitlement のみ）
5. zip の作成と SHA-256 の表示

- アイコンを作り直す場合のみ `make assets`（`assets/brand/holdnote-icon.png` から `.icns` とメニューバー用テンプレート画像を生成）。
- `cargo run` で起動すると、マイクの許可は **ターミナルアプリ** に対して求められます。実際の権限の挙動は `.app` で確認してください。

## 自動チェック（`make verify`）

`scripts/verify-bundle.sh [--transcribe] [path/to/HoldNote.app]` が、作成した `.app` について次を確認します。

- 署名が有効で、hardened runtime が付いている
- entitlement がマイク（`com.apple.security.device.audio-input`）だけ
- `Info.plist` の必須キー（Bundle ID、`LSUIElement`、マイクの利用目的、バージョンが `Cargo.toml` と一致）
- 実行ファイルが arm64
- バイナリに `/Users/...`（ビルド環境のパス）が残っていない
- 同梱した whisper-cli と依存ライブラリが、Homebrew などビルド環境のパスを参照していない
- ライセンスファイルがそろっている
- `--transcribe`：合成した日本語音声を同梱の whisper-cli で実際に文字起こしできる（tiny モデルを `~/Library/Caches/holdnote-dev/` に自動ダウンロード）

`make release` はこのチェックを自動で実行します。

## whisper-cli の同梱

`make bundle WHISPER=1`（= `scripts/bundle.sh --bundle-whisper`）は、Homebrew の whisper.cpp から次をコピーし、参照先を書き換えて再署名します。

| コピー先 | 内容 |
|---|---|
| `Contents/Resources/bin/whisper-cli` | 本体 |
| `Contents/Frameworks/` | 依存ライブラリ（libwhisper、libggml、libggml-base、libomp） |
| `Contents/Resources/bin/libggml-*.so` | ggml の計算バックエンド（CPU 各世代・Metal・BLAS） |
| `Contents/Resources/licenses/` | whisper.cpp・ggml（MIT）、libomp（Apache-2.0 WITH LLVM-exception）のライセンス |

- 任意の whisper-cli を使う場合：`WHISPER_CLI=/path/to/whisper-cli scripts/bundle.sh`
- バックエンドの場所を指定する場合：`GGML_BACKEND_DIR=/path/to/libexec scripts/bundle.sh --bundle-whisper`
- ggml は計算バックエンドを **実行時に** ビルド時のパス（例：`/opt/homebrew/Cellar/ggml/<ver>/libexec`）か実行ファイルと同じフォルダから読み込みます。
  バックエンドを同梱しないと、Homebrew のない Mac では動かず、`brew upgrade` でパスが変わると動かなくなります。
- Homebrew 版の whisper.cpp も入っている Mac では、Homebrew 側のバックエンドも併せて読み込まれますが、動作に支障はありません。

### whisper-cli の検出順（アプリ実行時）

1. 設定画面「音声認識」タブで指定したパス
2. `HoldNote.app/Contents/Resources/bin/whisper-cli`
3. `/opt/homebrew/bin/whisper-cli` → `/usr/local/bin/whisper-cli` → `$PATH`

### 呼び出し方

`whisper-cli -m <model> -f <wav> -l ja -nt -np -otxt -of <tmp>`（`src/transcriber.rs`）。
whisper.cpp を更新したら、`make verify` の `--transcribe` でフラグが有効か確認してください。

## 音声認識モデル

モデルは同梱せず、アプリの設定画面からダウンロードします（`~/Library/Application Support/HoldNote/models/`）。
取得元は `https://huggingface.co/ggerganov/whisper.cpp`（多言語モデル。`.en` は使わない）。ダウンロード後に SHA-256 を検証し、失敗時は既存のモデルを残します。

| モデル | ファイル | サイズ | SHA-256 |
|---|---|---|---|
| base（既定） | `ggml-base.bin` | 147,951,465 B | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |
| small | `ggml-small.bin` | 487,601,967 B | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` |

## 開発用オプション

| コマンド | 内容 |
|---|---|
| `holdnote --doctor` | 利用環境の診断（設定やファイルは変更しない） |
| `holdnote --overlay-demo` | 録音オーバーレイの全状態を順に表示して終了 |
| `RUST_LOG=holdnote=debug` | デバッグログ（ターミナルから起動時は標準エラー、それ以外は `logs/holdnote.log`） |
| `HOLDNOTE_GOOGLE_CLIENT_ID` / `HOLDNOTE_GOOGLE_CLIENT_SECRET` | 設定が空のとき、OAuth クライアントを環境変数から読む |
| `cargo test live_mic -- --ignored --nocapture` | 実際のマイクで2秒録音してレベルを表示 |

## 権限と実装メモ

- ホットキーは Carbon のグローバルホットキーで、**入力監視・アクセシビリティ権限は不要**。
  入力監視がない環境では、キーの押下状態を確認する安全装置（修飾キーを先に離したときの停止）は自動的に無効になり、キー解放の通知で録音を止める。
- 録音は macOS の既定の入力デバイスを使う。完全な無音（ピーク 0）は仮想デバイスか権限の問題とみなし、利用者に入力デバイス名を表示する。
- 通知は `osascript` 経由。
- アドホック署名はビルドごとに変わるため、更新後にマイク権限・キーチェーンへのアクセス確認が再度出ることがある。
- ログイン時起動は `~/Library/LaunchAgents/io.github.holdnote.HoldNote.plist`。Bundle ID は変更しないこと（権限・キーチェーン・LaunchAgent が引き継がれなくなる）。
