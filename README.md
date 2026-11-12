# ks — 頃合商店 CLI

Rust製の頃合商店 Public API クライアントです。実行コマンドは `ks` です。

## インストール

macOSではHomebrewでインストールできます。

```sh
brew tap koroai-supply/cli https://github.com/koroai-supply/cli.git
brew install koroai-supply/cli/ks
ks --help
```

ソースからインストールする場合はRust 1.89以降が必要です。macOSではキーチェーン、LinuxではSecret Service（`secret-tool` / libsecret）を使います。Windowsの資格情報ストアは初版では未対応です。

```sh
cargo install --path . --locked
ks --help
```

## ログイン

ログイン時にWebで端末名、ホーム、付与する権限を確認して承認します。確認コードはターミナルに表示しますが、認証トークンは表示しません。

```sh
ks login
ks logout
```

`ks login --no-browser` は承認URLを表示するだけにします。CLIはサーバーが指定した間隔で承認を確認し、`slow_down`では間隔を延ばします。access tokenと回転するrefresh tokenはOSの資格情報ストアに保存します。平文の設定ファイルは作りません。`ks logout`はサーバーで端末を失効した後にローカルの情報を削除します。

## コマンド

長い名前と短い名前は同じ動作です。

| 操作 | コマンド |
| --- | --- |
| 商品名で検索 | `ks products 'お茶'` / `ks p 'お茶'`、`ks p --q 'お茶'` |
| JANで商品を照会 | `ks p get 4901234567894`、`ks p --jan 4901234567894` |
| 見積もり | `ks quote create 4901234567894 --qty 1` / `ks q create ...`、`ks q get <ID>` |
| 注文 | `ks order create 4901234567894 --qty 1 --idempotency-key refill-0001` / `ks o create ...` |
| クイック注文 | `ks quick-order` / `ks qo`（冪等キーは省けば自動で作る。`--idempotency-key qo-0001`で指定も可） |
| クイック注文の商品 | `ks quick-order show` / `ks qo show` |
| 過去の注文 | `ks order list` / `ks o list` |
| 注文状況 | `ks status <ID>` / `ks s <ID>`。IDを省くと一覧 |
| 注文取消 | `ks o cancel <ID>` |
| 取り扱いリクエスト | `ks request create 4901234567894` / `ks r create ...`、`ks r list`、`ks r get <ID>` |

一覧は通常1ページずつ取得します。応答に続きがあるときは `--cursor <next_cursor>`、全件を読むときは `--all` を指定します。既定は表・詳細表示、`--json` で機械処理しやすいJSONを出力します。

```sh
ks o list --all --json
```

注文作成には保存した冪等キーが必須です。通信失敗やタイムアウトは注文の失敗を意味しません。`ks o list` で確認し、必要なら**同じキー・同じJAN・同じ数量**で再送してください。CLIは自動で別のキーを発行したり、注文を自動再送したりしません（クイック注文だけは、キーを省いたときに新しいキーを作ります。自動再送はしません）。

### クイック注文

クイック注文は、ホームに1つ設定した商品を確認なしで1パッケージ注文します。商品は指定しません。`ks qo show` で設定中の商品（JAN・商品名・税込円）を表示し、未設定なら案内を出します。商品の設定・変更・解除はCLIからはできません。頃合商店アプリの連携のタブ（またはMCP）で行ってください。未設定のまま注文すると `quick_order_not_set` のエラーと同じ案内を出します。

```sh
ks qo show
ks qo
ks qo --json
ks qo --idempotency-key qo-20261112-1
```

冪等キーは省けます。省くとCLIが呼ぶたびに新しいキーを作り、標準エラーに出します。通信失敗やタイムアウトのときは `ks o list` で確認し、必要なら表示された**同じキー**を `--idempotency-key` に指定して再送してください（自動では再送しません）。

## 開発

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

## ライセンス

[MIT](LICENSE)
