# Google Cloud の準備（OAuth クライアントの作成）

[ユーザーマニュアル](user-manual.md) の「3-3. Google Drive と連携する」で必要な、**クライアント ID** と **クライアント シークレット** を作る手順です。
**最初に1回だけ** 行います。作ったクライアントは、自分の他の Mac でも使い回せます。

> **なぜ必要？**
> HoldNote は、あなた自身の Google アカウントの権限で Google Drive にファイルを保存します。
> そのための「アプリの身分証明書」にあたるものが OAuth クライアントです。費用はかかりません。

---

## かんたんな方法：セットアップスクリプトを使う

ターミナルで次を実行すると、手順 1〜6 のうち **API の有効化を自動で行い**、残りの手順は該当する画面を順番に開いて案内します。
最後に、作ったクライアント ID とシークレットを HoldNote に直接書き込むこともできます。

```bash
curl -fsSLO https://raw.githubusercontent.com/kzmcode/holdnote/main/scripts/gcloud-setup.sh
bash gcloud-setup.sh --create --project holdnote-<好きな英数字>
```

- API の自動有効化には [Google Cloud CLI（gcloud）](https://cloud.google.com/sdk/docs/install?hl=ja) が必要です（`brew install --cask gcloud-cli` → `gcloud auth login`）。
  gcloud がなくても、画面を開いて案内する部分は使えます。
- 実行前に、ダウンロードしたスクリプトの中身を確認することをおすすめします。
- OAuth の同意画面とクライアントの作成は、Google がコマンドでの作成に対応していないため、ブラウザで行う必要があります。

スクリプトを使わない場合は、以下の手順を順番に行ってください。

---

## 手順 1. プロジェクトを作る

1. [Google Cloud コンソール](https://console.cloud.google.com/) を開き、Google アカウントでログインします。
2. 画面上部のプロジェクト選択 →「**新しいプロジェクト**」を押します。
3. プロジェクト名に `HoldNote` などと入力して「**作成**」を押します。
4. 作成したプロジェクトが選ばれていることを、画面上部で確認します。

## 手順 2. API を有効にする

次の2つの API を有効にします。それぞれのページを開いて「**有効にする**」を押してください。

- [Google Drive API](https://console.cloud.google.com/apis/library/drive.googleapis.com)：ファイルの保存に使います
- [Google Picker API](https://console.cloud.google.com/apis/library/picker.googleapis.com)：保存先フォルダの選択に使います

## 手順 3. アプリ情報（ブランディング）を登録する

1. [Google Auth Platform → ブランディング](https://console.cloud.google.com/auth/branding) を開きます。
   初めての場合は「**開始**」を押します。
2. 次のように入力します。

   | 項目 | 入力内容 |
   |---|---|
   | アプリ名 | `HoldNote` |
   | ユーザー サポートメール | 自分のメールアドレス |
   | 対象（ユーザーの種類） | **外部**（Google Workspace の組織内だけで使う場合は「内部」でも可） |
   | 連絡先情報 | 自分のメールアドレス |

3. 規約に同意して「**作成**」を押します。

## 手順 4. アクセス範囲（スコープ）を登録する

1. [Google Auth Platform → データアクセス](https://console.cloud.google.com/auth/scopes) を開きます。
2. 「**スコープを追加または削除**」を押し、次の3つにチェックを入れて「更新」→「保存」します。

   | スコープ | 用途 |
   |---|---|
   | `.../auth/drive.file` | HoldNote が作ったファイルと、あなたが選んだフォルダだけにアクセス |
   | `openid` | ログインの確認 |
   | `.../auth/userinfo.email` | 接続したアカウントのメールアドレスを表示 |

> Google Drive 全体にアクセスする `.../auth/drive` は **追加しないでください**。HoldNote には不要です。

## 手順 5. 公開ステータスを決める

[Google Auth Platform → 対象](https://console.cloud.google.com/auth/audience) を開き、次のどちらかを選びます。

| 選び方 | 操作 | 注意 |
|---|---|---|
| **本番環境（おすすめ）** | 「**アプリを公開**」を押す | HoldNote が使うスコープは審査不要のものだけなので、すぐに使えます |
| テスト中のまま | 「テストユーザー」に自分の Google アカウントを追加 | **7 日ごとに再認証が必要** になります |

> 本番環境にしても、あなたのクライアント ID を知らない人が使うことはありません。

## 手順 6. OAuth クライアントを作る

1. [Google Auth Platform → クライアント → クライアントを作成](https://console.cloud.google.com/auth/clients/create) を開きます。
2. 次のように入力して「**作成**」を押します。

   | 項目 | 入力内容 |
   |---|---|
   | アプリケーションの種類 | **デスクトップ アプリ** |
   | 名前 | `HoldNote` |

3. 表示された **クライアント ID**（`….apps.googleusercontent.com`）と **クライアント シークレット** をコピーします。
   - あとから確認することもできます（クライアント一覧 → HoldNote）。

これで準備は完了です。[ユーザーマニュアルの 3-3 (2)](user-manual.md#3-3-google-drive-と連携する) に戻って、HoldNote に入力してください。

---

## おすすめの追加設定：API の利用上限を下げる

クライアント ID とシークレットは HoldNote の設定ファイルに保存されるため、万一他人に知られると、あなたのプロジェクトの API 利用枠を使われる可能性があります。
念のため、[Google Drive API の割り当て](https://console.cloud.google.com/apis/api/drive.googleapis.com/quotas) で、1 分あたりのリクエスト上限を小さめ（例：100）に下げておくと安心です。
不審な利用に気づいたら、クライアントを削除して作り直してください。

---

## よくある質問

**Q. 「このアプリは Google で確認されていません」と表示される**
自分で作ったクライアントなので問題ありません。「詳細」→「HoldNote（安全ではないページ）に移動」で進んでください。

**Q. 「アクセスをブロック：HoldNote は Google の審査プロセスを完了していません」と表示される**
公開ステータスが「テスト中」で、ログインしたアカウントがテストユーザーに入っていません。手順 5 で本番環境にするか、テストユーザーに追加してください。

**Q. 会社や学校の Google アカウントで接続できない**
組織の管理者が、外部アプリのアクセスを制限している可能性があります。管理者に確認してください。

**Q. 複数の Mac で使いたい**
同じクライアント ID とシークレットを、それぞれの Mac の HoldNote に入力してください。Google への接続（認証）は Mac ごとに行います。

**Q. 連携をやめたい**
HoldNote の設定画面「Google Drive」タブで **「連携を解除」** を押すと、Google 側の許可を取り消し、この Mac から認証情報を削除します。Google Drive に保存したファイルは消えません。
