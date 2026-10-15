# 3. ビルド・初回起動・macOS 権限

## 3-1. 必要なもの

- Apple Silicon Mac / macOS 14 以降
- Rust（stable）、Xcode Command Line Tools（`xcode-select --install`）
- Apple Developer Program は **不要**（アドホック署名のみ）

## 3-2. ビルド

```bash
make test          # 単体テスト
make bundle        # dist/HoldNote.app と dist/HoldNote-<ver>-arm64.zip を作成
make run-app       # バンドルして起動
```

- `scripts/bundle.sh` は release ビルド → `.app` 組み立て（`Info.plist`、`AppIcon.icns`）→ `codesign -s -`（アドホック署名、hardened runtime、マイク entitlement）→ zip 作成 → SHA-256 表示を行います。
- アイコンを作り直す場合のみ `make assets`（`assets/brand/holdnote-icon.png` から `.icns` とメニューバー用テンプレート画像を生成）。
- 開発中に `cargo run` で起動すると、マイク権限は **ターミナル アプリ** に対して要求されます。実際の権限挙動は `.app` で確認してください。

## 3-3. 初回起動

1. `dist/HoldNote.app` を `/Applications` に置いて起動。
   アドホック署名のため Gatekeeper にブロックされた場合は、**システム設定 → プライバシーとセキュリティ →「このまま開く」** で許可します（Gatekeeper を無効化するコマンドは使わないでください）。
2. Dock には表示されず、メニューバーにマイクのアイコンが出ます。初回は設定ウィンドウが自動で開きます。
3. **マイク権限** のダイアログが出たら「許可」。拒否した場合は **システム設定 → プライバシーとセキュリティ → マイク** で HoldNote をオンにします。
4. 「Google Drive」タブ：[01-google-cloud-setup.md](01-google-cloud-setup.md) の手順で接続・フォルダ選択・保存テスト。
5. 「音声認識」タブ：モデルをダウンロード（[02-whisper-setup.md](02-whisper-setup.md)）。
6. 任意のアプリ使用中に **`⌃⌥R`（Control + Option + R）を押している間** 録音されます。離すと文字起こし → Drive に保存されます。

### 録音デバイスについて

HoldNote は **macOS の既定の入力デバイス** で録音します（アプリ内でのマイク選択は未対応）。
BlackHole などの仮想オーディオデバイスが既定の入力になっていると、無音で録音され「音声が入力されていません」と表示されます。
**システム設定 → サウンド → 入力** で使いたいマイク（例：MacBook Pro のマイク）を選んでください。
現在の入力デバイスは、設定画面「一般」タブのマイク欄に表示されます。

### 権限について

- ホットキーは Carbon のグローバルホットキーで実装しており、**入力監視・アクセシビリティ権限は不要** です。
  入力監視が許可されていない場合、キーの押下状態を確認する安全装置（修飾キーを先に離したときの停止）は自動的に無効になり、キーを離したときの通知で録音を止めます。
- 通知は `osascript` 経由で出すため、通知が出ない場合は **システム設定 → 通知** で「スクリプトエディタ」の通知を許可してください。
- アドホック署名はビルドごとに変わるため、**アップデート後にマイク権限を再度求められることがあります**。

## 3-4. ログイン時起動

`.app` から起動した初回に、設定「ログイン時に起動」（既定オン）に従って
`~/Library/LaunchAgents/io.github.holdnote.HoldNote.plist` を作成します。設定画面からオン/オフできます。
アプリを移動した場合は一度オフ→オンにしてパスを更新してください。

## 3-5. データの場所

| 内容 | 場所 |
|---|---|
| 設定 | `~/Library/Application Support/HoldNote/settings.toml` |
| ジョブ DB | `~/Library/Application Support/HoldNote/jobs.sqlite3` |
| 録音 WAV | `~/Library/Application Support/HoldNote/audio/`（保存成功後、既定 24 時間で削除） |
| 生成 Markdown | `~/Library/Application Support/HoldNote/queue/` |
| モデル | `~/Library/Application Support/HoldNote/models/` |
| ログ | `~/Library/Application Support/HoldNote/logs/holdnote.log`（1MB でローテーション。文字起こし本文・トークンは記録しない） |
| OAuth トークン | macOS Keychain（`io.github.holdnote.HoldNote` / `google-oauth`） |

## 3-6. 開発用オプション

- `HoldNote.app/Contents/MacOS/holdnote --overlay-demo`：録音オーバーレイの全状態を順に表示して終了（見た目の確認用）。
- `RUST_LOG=holdnote=debug` でデバッグログ（トークン・文字起こし本文はログに出しません）。
