# 1. Google Cloud の準備（OAuth / Drive API / Picker API）

HoldNote は利用者の Google アカウントで OAuth 認証し、Drive API v3 で Markdown をアップロードします。
そのための Google Cloud プロジェクトと **デスクトップ アプリ用 OAuth クライアント** を一度だけ用意します。
同じクライアントを複数の Mac で共有できます（トークンは Mac・ユーザーごとに Keychain に保存されます）。

## 1-1. プロジェクトと API

1. <https://console.cloud.google.com/> でプロジェクトを作成（または既存を選択）。
2. 「API とサービス」→「ライブラリ」で次の 2 つを **有効化**：
   - **Google Drive API**
   - **Google Picker API**（保存先フォルダ選択に使用）

## 1-2. OAuth 同意画面（Google Auth Platform）

1. 「Google Auth Platform」→「ブランディング」：アプリ名 `HoldNote`、サポートメール、デベロッパー連絡先を入力。
2. 「対象」：ユーザーの種類 **外部**（組織内だけで使うなら **内部** でも可）。
3. 「データアクセス」→ スコープを追加：
   - `https://www.googleapis.com/auth/drive.file`（非機密スコープ）
   - `openid`、`email`（接続アカウントのメール表示用）
   - ※ 広範な `drive` スコープは追加しないでください（要件 7.3）。
4. 公開ステータス：
   - **テスト中** のままの場合：「テストユーザー」に利用者の Google アカウントを追加する。
     **テスト中は更新トークンが 7 日で失効**し、毎週再認証が必要になります。
   - 継続利用する場合：「アプリを公開」で **本番環境** にする。非機密スコープのみなので Google の審査は不要ですが、
     ブランディング画面でホームページ URL・プライバシーポリシー URL 等の入力を求められることがあります。
5. Google Workspace アカウントの場合、組織の管理者がサードパーティ アプリのアクセスを制限していると認証できません。管理者に確認してください。

## 1-3. OAuth クライアント（デスクトップ アプリ）

1. 「クライアント」→「クライアントを作成」→ アプリケーションの種類 **デスクトップ アプリ**。
2. 発行された **クライアント ID** と **クライアント シークレット** を控える。
   - デスクトップ アプリではシークレットは秘密として扱われませんが、Google のトークン エンドポイントが要求するため必要です。
   - リダイレクト URI の登録は不要です（`http://127.0.0.1:<ランダムポート>` のループバックを使用）。

## 1-4. HoldNote への設定

次のいずれかで設定します。

- **設定画面（推奨）**：メニューバー →「設定…」→「Google Drive」タブ → クライアント ID / シークレットを入力して「保存」。
  シークレット欄は空のまま保存すると既存値が維持されます。
- **環境変数**（開発時）：`HOLDNOTE_GOOGLE_CLIENT_ID` / `HOLDNOTE_GOOGLE_CLIENT_SECRET`
- **設定ファイル**：`~/Library/Application Support/HoldNote/settings.toml` の `google_client_id` / `google_client_secret`

## 1-5. 接続と保存先フォルダの選択

1. 「Googleに接続」→ ブラウザが開くので同意（アカウントのメールが表示されるようになります）。
2. 「保存先フォルダを選択」→ ブラウザで **Google Picker** が開くので保存先フォルダを 1 つ選ぶ。
   - Picker フローは `drive.file` スコープ単独で動作します。HoldNote がアクセスできるのは、選んだフォルダと HoldNote が作成したファイルだけです。
   - フォルダ以外を選ぶと「フォルダを選択してください」と表示されます。
3. 「保存テスト」→ 選んだフォルダにテストファイルを作成→削除できれば準備完了。

## 1-6. 連携解除

「連携を解除」でトークンを Google 側で失効させ、Keychain（サービス名 `io.github.holdnote.HoldNote` / アカウント `google-oauth`）から削除します。
**Drive 上のファイルは削除されません。**

## 未検証事項（実機で確認すること）

- デスクトップ Picker（`trigger_onepick=true&allow_folder_selection=true`）で任意フォルダを選べ、`picked_file_ids` がループバックに返ること。
- 選んだフォルダに `drive.file` スコープで `files.create` できること（要件 16.2-5）。
