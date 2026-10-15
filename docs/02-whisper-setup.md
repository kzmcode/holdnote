# 2. whisper.cpp と音声認識モデル

文字起こしは端末内の `whisper-cli`（whisper.cpp）で行います。音声はクラウドに送信されません。

## 2-1. whisper-cli の導入（どちらか一方）

### 方式 1：Homebrew（簡単・推奨の開始方法）

```bash
brew install whisper.cpp
which whisper-cli   # /opt/homebrew/bin/whisper-cli
```

### 方式 2：`.app` に同梱（利用者に Homebrew を要求しない）

whisper.cpp を入れた Mac でビルドします：

```bash
make bundle WHISPER=1          # = scripts/bundle.sh --bundle-whisper
# 任意のバイナリを使う場合
WHISPER_CLI=/path/to/whisper-cli scripts/bundle.sh
```

`HoldNote.app/Contents/Resources/bin/whisper-cli` と依存 dylib（`Contents/Frameworks/`）、
ggml の計算バックエンド（`libggml-cpu-*.so` / `libggml-metal.so` / `libggml-blas.so`、`Contents/Resources/bin/`）がコピーされ、再署名されます。

- ggml は計算バックエンドを実行時に読み込みます。読み込み先は、ビルド時に決まったパス（例：`/opt/homebrew/Cellar/ggml/0.26.0/libexec`）か、実行ファイルと同じフォルダです。
  そのため、バックエンドを同梱しないと Homebrew のない Mac では動かず、`brew upgrade` でパスが変わると動かなくなります（2026-10-15 に修正済み）。
- Homebrew 以外のバックエンドを使う場合は、`GGML_BACKEND_DIR=/path/to/libexec` を指定して `bundle.sh` を実行します。
- **方式1と方式2を両方実行しても問題ありません。** 同梱版が優先されます（2-2 の検出順）。
  なお、Homebrew 版も入っている Mac では、Homebrew 側のバックエンドも併せて読み込まれます。動作に支障はありません（2026-10-15 に確認）。

同梱後は次を確認してください：

- `otool -L dist/HoldNote.app/Contents/Resources/bin/whisper-cli dist/HoldNote.app/Contents/Resources/bin/*.so` で Homebrew のパス（`/opt/homebrew/...`）が残っていないこと
- **Homebrew の入っていない Mac** で文字起こしできること（最終確認。未実施）
- 同梱版で Metal が有効に動くこと、macOS 14 で起動すること
- whisper.cpp / ggml は MIT ライセンス。配布物にライセンス表記を含めること

## 2-2. whisper-cli の検出順

1. 設定画面「音声認識」タブで指定したパス
2. `HoldNote.app/Contents/Resources/bin/whisper-cli`
3. `/opt/homebrew/bin/whisper-cli` → `/usr/local/bin/whisper-cli` → `$PATH`

## 2-3. 音声認識モデル

モデルは同梱しません。設定画面「音声認識」タブ →「モデルをダウンロード」で取得します。
保存先：`~/Library/Application Support/HoldNote/models/`

| モデル | ファイル | サイズ | SHA-256 |
|---|---|---|---|
| base（既定） | `ggml-base.bin` | 147,951,465 B | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |
| small | `ggml-small.bin` | 487,601,967 B | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` |

- 取得元：`https://huggingface.co/ggerganov/whisper.cpp`（多言語モデル。`.en` は使いません）
- ダウンロード後に SHA-256 を検証し、失敗時は既存モデルを保持します。
- オフライン環境では上記ファイルを手動で配置しても構いません。

## 2-4. 要確認

- `whisper-cli` に渡しているフラグ：`-m <model> -f <wav> -l ja -nt -np -otxt -of <tmp>`。
  導入した whisper.cpp のバージョンで `whisper-cli --help` を見て、フラグが有効か確認してください（`src/transcriber.rs`）。
- base / small の速度・精度・メモリを M1/M2 で比較し、既定モデルを決める（NFR-06）。
