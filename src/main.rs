mod auth;

use std::{collections::HashSet, process::ExitCode, time::Duration};

use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "ks", version, about = "頃合商店 CLI")]
struct Cli {
    /// JSON形式で出力
    #[arg(long, global = true)]
    json: bool,
    /// 公開APIの接続先
    #[arg(long, global = true, env = "KOROAI_API_URL", default_value = auth::API_URL, hide = true)]
    api_url: String,
    /// OAuth認可サーバーの接続先
    #[arg(long, global = true, env = "KOROAI_AUTH_URL", default_value = auth::AUTH_URL, hide = true)]
    auth_url: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Webで承認してこの端末をログイン
    Login {
        #[arg(long)]
        no_browser: bool,
    },
    /// この端末の認可を失効
    Logout,
    /// 商品を検索・照会
    #[command(visible_alias = "p")]
    Products(ProductArgs),
    /// 見積もりを作成・照会
    #[command(visible_alias = "q")]
    Quote {
        #[command(subcommand)]
        action: QuoteAction,
    },
    /// 注文を作成・一覧・照会・取消
    #[command(visible_alias = "o")]
    Order {
        #[command(subcommand)]
        action: OrderAction,
    },
    /// 注文の状況を照会。IDを省くと一覧
    #[command(visible_alias = "s")]
    Status {
        id: Option<String>,
        /// 前の一覧応答のnext_cursor
        #[arg(long)]
        cursor: Option<String>,
        /// 続きのページも取得
        #[arg(long)]
        all: bool,
    },
    /// クイック注文の商品を1パッケージ注文。showで設定中の商品を表示
    #[command(visible_alias = "qo")]
    QuickOrder(QuickOrderArgs),
    /// 取り扱いリクエストを作成・一覧・照会
    #[command(visible_alias = "r")]
    Request {
        #[command(subcommand)]
        action: RequestAction,
    },
}

#[derive(Args)]
struct ProductArgs {
    #[command(subcommand)]
    action: Option<ProductAction>,
    /// 商品名の検索語
    #[arg(conflicts_with_all = ["q", "jan"])]
    query: Option<String>,
    #[arg(long, conflicts_with_all = ["query", "jan"])]
    q: Option<String>,
    #[arg(long, conflicts_with_all = ["query", "q"])]
    jan: Option<String>,
    /// 前の一覧応答のnext_cursor
    #[arg(long)]
    cursor: Option<String>,
    /// 続きのページも取得
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
struct QuickOrderArgs {
    #[command(subcommand)]
    action: Option<QuickOrderAction>,
    /// 再送時も同じ値を使用
    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Subcommand)]
enum QuickOrderAction {
    /// 設定中のクイック注文の商品を表示
    Show,
}

#[derive(Subcommand)]
enum ProductAction {
    Get { jan: String },
}

#[derive(Args)]
struct Purchase {
    jan: String,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=10))]
    qty: u8,
    /// 再送時も同じ値と同じJAN・数量を使用
    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Subcommand)]
enum QuoteAction {
    Create(Purchase),
    Get { id: String },
}

#[derive(Subcommand)]
enum OrderAction {
    /// 注文を作成
    Create(Purchase),
    List {
        /// 前の一覧応答のnext_cursor
        #[arg(long)]
        cursor: Option<String>,
        /// 続きのページも取得
        #[arg(long)]
        all: bool,
    },
    Get {
        id: String,
    },
    Cancel {
        id: String,
    },
}

#[derive(Subcommand)]
enum RequestAction {
    Create {
        jan: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    List {
        /// 前の一覧応答のnext_cursor
        #[arg(long)]
        cursor: Option<String>,
        /// 続きのページも取得
        #[arg(long)]
        all: bool,
    },
    Get {
        id: String,
    },
}

const QUICK_ORDER_NOT_SET: &str =
    "。クイック注文の商品が未設定です。頃合商店アプリの連携のタブで設定してください";

struct Call {
    method: &'static str,
    path: String,
    query: Vec<(&'static str, String)>,
    body: Option<Value>,
    key: Option<String>,
    kind: &'static str,
    all: bool,
}

fn jan(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 8 | 13) || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("JANは8桁または13桁の数字で指定してください".into());
    }
    let digits: Vec<u32> = value.bytes().map(|b| u32::from(b - b'0')).collect();
    let check = digits[digits.len() - 1];
    let sum: u32 = digits[..digits.len() - 1]
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| d * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    if (10 - sum % 10) % 10 != check {
        return Err("JANのチェックディジットが正しくありません".into());
    }
    Ok(())
}

fn id(value: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(value).map_err(|_| "IDとcursorはUUIDで指定してください")?;
    if parsed.is_nil() || parsed.to_string() != value {
        return Err("IDとcursorはUUIDで指定してください".into());
    }
    Ok(())
}

fn cursor(value: &Option<String>) -> Result<(), String> {
    if let Some(v) = value {
        id(v)?;
    }
    Ok(())
}

fn key(value: &str) -> Result<(), String> {
    if value.len() > 128
        || value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err("冪等キーは英数字と._:-で1〜128文字にしてください".into());
    }
    Ok(())
}

fn list(kind: &'static str, cursor_value: Option<String>, all: bool) -> Result<Call, String> {
    cursor(&cursor_value)?;
    Ok(Call {
        method: "GET",
        path: format!("/v1/{kind}"),
        query: cursor_value.into_iter().map(|v| ("cursor", v)).collect(),
        body: None,
        key: None,
        kind,
        all,
    })
}

fn by_id(kind: &'static str, value: String, method: &'static str) -> Result<Call, String> {
    id(&value)?;
    Ok(Call {
        method,
        path: format!("/v1/{kind}/{value}"),
        query: vec![],
        body: None,
        key: None,
        kind,
        all: false,
    })
}

fn purchase(kind: &'static str, p: Purchase, required_key: bool) -> Result<Call, String> {
    jan(&p.jan)?;
    if required_key && p.idempotency_key.is_none() {
        return Err(
            "注文には --idempotency-key が必要です。再送時も同じキー・JAN・数量を使ってください"
                .into(),
        );
    }
    if let Some(v) = &p.idempotency_key {
        key(v)?;
    }
    Ok(Call {
        method: "POST",
        path: format!("/v1/{kind}"),
        query: vec![],
        body: Some(json!({"jan":p.jan,"qty":p.qty})),
        key: p.idempotency_key,
        kind,
        all: false,
    })
}

fn plan(command: Command) -> Result<Call, String> {
    match command {
        Command::Products(p) => match p.action {
            Some(ProductAction::Get { jan: value }) => {
                if p.query.is_some()
                    || p.q.is_some()
                    || p.jan.is_some()
                    || p.cursor.is_some()
                    || p.all
                {
                    return Err("商品の詳細照会に一覧用のオプションは指定できません".into());
                }
                jan(&value)?;
                Ok(Call {
                    method: "GET",
                    path: format!("/v1/products/{value}"),
                    query: vec![],
                    body: None,
                    key: None,
                    kind: "products",
                    all: false,
                })
            }
            None => {
                if let Some(v) = &p.jan {
                    jan(v)?;
                    if p.cursor.is_some() || p.all {
                        return Err("JANの完全一致照会に --cursor / --all は指定できません".into());
                    }
                    return Ok(Call {
                        method: "GET",
                        path: format!("/v1/products/{v}"),
                        query: vec![],
                        body: None,
                        key: None,
                        kind: "products",
                        all: false,
                    });
                }
                let q = p
                    .query
                    .or(p.q)
                    .ok_or("商品名を指定してください。例: ks p お茶")?;
                if q.trim().is_empty() || q.chars().count() > 200 {
                    return Err("検索語は1〜200文字で指定してください".into());
                }
                let mut call = list("products", p.cursor, p.all)?;
                call.path = "/v1/products/search".into();
                call.query.push(("q", q));
                Ok(call)
            }
        },
        Command::Quote { action } => match action {
            QuoteAction::Create(p) => purchase("quotes", p, false),
            QuoteAction::Get { id: value } => by_id("quotes", value, "GET"),
        },
        Command::Order { action } => match action {
            OrderAction::Create(p) => purchase("orders", p, true),
            OrderAction::List { cursor, all } => list("orders", cursor, all),
            OrderAction::Get { id: value } => by_id("orders", value, "GET"),
            OrderAction::Cancel { id: value } => by_id("orders", value, "DELETE"),
        },
        Command::Status {
            id: Some(value),
            cursor: None,
            all: false,
        } => by_id("orders", value, "GET"),
        Command::Status { id: Some(_), .. } => {
            Err("注文IDと --cursor / --all は同時に指定できません".into())
        }
        Command::Status {
            id: None,
            cursor,
            all,
        } => list("orders", cursor, all),
        Command::QuickOrder(q) => match q.action {
            Some(QuickOrderAction::Show) => Ok(Call {
                method: "GET",
                path: "/v1/quick-order".into(),
                query: vec![],
                body: None,
                key: None,
                kind: "quick-order",
                all: false,
            }),
            None => {
                let value = q.idempotency_key.ok_or(
                    "クイック注文には --idempotency-key が必要です。再送時も同じキーを使ってください",
                )?;
                key(&value)?;
                Ok(Call {
                    method: "POST",
                    path: "/v1/quick-order".into(),
                    query: vec![],
                    body: Some(json!({})),
                    key: Some(value),
                    kind: "orders",
                    all: false,
                })
            }
        },
        Command::Request { action } => match action {
            RequestAction::Create {
                jan: value,
                idempotency_key,
            } => {
                jan(&value)?;
                if let Some(v) = &idempotency_key {
                    key(v)?;
                }
                Ok(Call {
                    method: "POST",
                    path: "/v1/product-requests".into(),
                    query: vec![],
                    body: Some(json!({"jan":value})),
                    key: idempotency_key,
                    kind: "product-requests",
                    all: false,
                })
            }
            RequestAction::List { cursor, all } => list("product-requests", cursor, all),
            RequestAction::Get { id: value } => by_id("product-requests", value, "GET"),
        },
        Command::Login { .. } | Command::Logout => unreachable!(),
    }
}

fn request(
    call: &Call,
    base: &Url,
    token: &str,
    cursor_override: Option<&str>,
) -> Result<Value, String> {
    let mut url = auth::endpoint(base, &call.path)?;
    if !call.query.is_empty() || cursor_override.is_some() {
        let mut query = url.query_pairs_mut();
        for (k, v) in &call.query {
            if *k != "cursor" || cursor_override.is_none() {
                query.append_pair(k, v);
            }
        }
        if let Some(v) = cursor_override {
            query.append_pair("cursor", v);
        }
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(30))
        .build();
    let mut req = agent
        .request(call.method, url.as_str())
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json");
    if let Some(value) = &call.key {
        req = req.set("Idempotency-Key", value);
    }
    let response = if let Some(body) = &call.body {
        req.set("Content-Type", "application/json")
            .send_string(&body.to_string())
    } else {
        req.call()
    };
    let body = match response {
        Ok(response) => response
            .into_string()
            .map_err(|_| "API応答を読み取れませんでした".to_owned())?,
        Err(ureq::Error::Status(status, response)) => {
            let retry_after = response.header("Retry-After").map(str::to_owned);
            let value = response
                .into_string()
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok());
            let code = value
                .as_ref()
                .and_then(|v| v.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("unknown_error");
            let hint = if code == "quick_order_not_set" {
                QUICK_ORDER_NOT_SET
            } else if status == 401 {
                "。再ログインしてください: ks login"
            } else if status == 429 {
                "。Retry-Afterの秒数を待ってください"
            } else {
                ""
            };
            return Err(format!(
                "HTTP {status}: {code}{hint}{}",
                retry_after.map(|v| format!(" ({v}秒)")).unwrap_or_default()
            ));
        }
        Err(ureq::Error::Transport(_)) => {
            return Err(match (call.method, call.path.as_str()) {
                ("POST", "/v1/orders") => "通信エラー。注文結果は未確定です。注文一覧を確認し、再送する場合は同じ冪等キー・JAN・数量を使ってください",
                ("POST", "/v1/quick-order") => "通信エラー。注文結果は未確定です。注文一覧を確認し、再送する場合は同じ冪等キーを使ってください",
                _ => "通信エラー",
            }
            .into());
        }
    };
    serde_json::from_str(&body).map_err(|_| "API応答がJSONではありません".into())
}

fn fetch(call: &Call, profile: &auth::Profile) -> Result<Value, String> {
    let token = auth::access_token(profile)?;
    let mut result = request(call, &profile.api_url, &token.access, None)?;
    if !call.all {
        return Ok(result);
    }
    let mut seen = HashSet::new();
    let mut rows = result
        .get_mut("data")
        .and_then(Value::as_array_mut)
        .ok_or("一覧応答が正しくありません")?
        .clone();
    while let Some(next) = result
        .get("next_cursor")
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        if !seen.insert(next.clone()) {
            return Err("同じcursorが繰り返されました".into());
        }
        let page = request(call, &profile.api_url, &token.access, Some(&next))?;
        rows.extend(
            page.get("data")
                .and_then(Value::as_array)
                .ok_or("一覧応答が正しくありません")?
                .iter()
                .cloned(),
        );
        result = page;
    }
    Ok(json!({"data":rows,"next_cursor":null}))
}

fn value(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "-".into(),
        Some(v) => v.to_string(),
    }
}

fn display(result: &Value, kind: &str, json_output: bool) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(result).map_err(|_| "JSONを出力できません".to_owned())?
        );
        return Ok(());
    }
    let data = result.get("data").ok_or("API応答にdataがありません")?;
    if kind == "quick-order" {
        match data.get("product") {
            Some(Value::Null) => println!(
                "クイック注文の商品は未設定です。頃合商店アプリの連携のタブで設定してください。"
            ),
            Some(product @ Value::Object(_)) => {
                for (title, key) in [
                    ("JAN", "jan"),
                    ("商品名", "name"),
                    ("税込円", "unit_price_jpy"),
                    ("単位", "purchase_unit"),
                ] {
                    println!("{title}: {}", value(product, key));
                }
            }
            _ => return Err("API応答のproductが正しくありません".into()),
        }
        return Ok(());
    }
    if let Some(rows) = data.as_array() {
        let columns: &[(&str, &str)] = match kind {
            "products" => &[
                ("JAN", "jan"),
                ("商品名", "name"),
                ("税込円", "unit_price_jpy"),
            ],
            "orders" => &[
                ("ID", "id"),
                ("日時", "created_at"),
                ("JAN", "jan"),
                ("数量", "qty"),
                ("税込円", "total_amount_jpy"),
                ("状態", "status"),
            ],
            _ => &[
                ("ID", "id"),
                ("JAN", "jan"),
                ("状態", "status"),
                ("日時", "created_at"),
            ],
        };
        println!(
            "{}",
            columns
                .iter()
                .map(|(title, _)| *title)
                .collect::<Vec<_>>()
                .join("\t")
        );
        for row in rows {
            println!(
                "{}",
                columns
                    .iter()
                    .map(|(_, key)| value(row, key))
                    .collect::<Vec<_>>()
                    .join("\t")
            );
        }
        if let Some(cursor) = result.get("next_cursor").and_then(Value::as_str) {
            eprintln!("続き: --cursor {cursor}（すべて取得するには --all）");
        }
    } else if let Some(object) = data.as_object() {
        for (key, value) in object {
            println!(
                "{key}: {}",
                if let Some(s) = value.as_str() {
                    s.to_owned()
                } else {
                    value.to_string()
                }
            );
        }
    } else {
        return Err("API応答のdataが正しくありません".into());
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let profile = auth::Profile::new(&cli.auth_url, &cli.api_url)?;
    match cli.command {
        Command::Login { no_browser } => auth::login(&profile, no_browser),
        Command::Logout => auth::logout(&profile),
        command => {
            let call = plan(command)?;
            let result = fetch(&call, &profile)?;
            display(&result, call.kind, cli.json)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ks: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    #[test]
    fn jan_validation() {
        assert!(jan("4901234567894").is_ok());
        assert!(jan("4901234567890").is_err());
        assert!(jan("00000000").is_ok());
    }
    #[test]
    fn aliases_and_order_key() {
        assert!(matches!(
            Cli::try_parse_from(["ks", "o", "list"]).unwrap().command,
            Command::Order {
                action: OrderAction::List { .. }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["ks", "p", "お茶"]).unwrap().command,
            Command::Products(_)
        ));
        assert!(plan(Cli::try_parse_from(["ks", "p"]).unwrap().command).is_err());
        assert_eq!(
            plan(Cli::try_parse_from(["ks", "p", "お茶"]).unwrap().command)
                .unwrap()
                .path,
            "/v1/products/search"
        );
        let missing = plan(
            Cli::try_parse_from(["ks", "o", "create", "4901234567894"])
                .unwrap()
                .command,
        );
        assert!(missing.is_err());
    }
    #[test]
    fn secure_origin() {
        assert!(auth::origin("http://example.com").is_err());
        assert!(auth::origin("https://example.com/path").is_err());
        assert!(auth::origin("http://localhost:3000").is_ok());
    }

    #[test]
    fn list_request_uses_bearer_and_encoded_query() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 2048];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            assert!(request.starts_with("GET /v1/products/search?q=%E8%8C%B6"));
            assert!(
                request.contains("Authorization: Bearer fake-access")
                    || request.contains("authorization: Bearer fake-access")
            );
            let body = r#"{"data":[],"next_cursor":null}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        });
        let mut call = list("products", None, false).unwrap();
        call.path = "/v1/products/search".into();
        call.query.push(("q", "茶".into()));
        let base = auth::origin(&format!("http://127.0.0.1:{port}")).unwrap();
        let result = request(&call, &base, "fake-access", None).unwrap();
        assert_eq!(result["data"], json!([]));
        server.join().unwrap();
    }

    fn serve_once(status: &str, body: &'static str) -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status = status.to_owned();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let mut request = Vec::new();
            loop {
                let size = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..size]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length || size == 0 {
                        break;
                    }
                }
            }
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&request).to_string()
        });
        (port, server)
    }

    #[test]
    fn quick_order_commands() {
        let parse = |args: &[&str]| plan(Cli::try_parse_from(args).unwrap().command);
        let order = parse(&["ks", "quick-order", "--idempotency-key", "qo-0001"]).unwrap();
        assert_eq!(
            (order.method, order.path.as_str()),
            ("POST", "/v1/quick-order")
        );
        assert_eq!(order.body, Some(json!({})));
        assert_eq!(order.key.as_deref(), Some("qo-0001"));
        assert_eq!(order.kind, "orders");
        let show = parse(&["ks", "qo", "show"]).unwrap();
        assert_eq!(
            (show.method, show.path.as_str()),
            ("GET", "/v1/quick-order")
        );
        assert!(show.key.is_none() && show.body.is_none());
        assert!(parse(&["ks", "qo"]).is_err());
        assert!(parse(&["ks", "qo", "--idempotency-key", "bad key"]).is_err());
        assert!(Cli::try_parse_from(["ks", "qo", "show", "--idempotency-key", "qo-0001"]).is_err());
    }

    #[test]
    fn quick_order_sends_key_and_empty_body() {
        let (port, server) = serve_once(
            "201 Created",
            r#"{"data":{"id":"00000000-0000-4000-8000-000000000001","status":"accepted"}}"#,
        );
        let call = plan(
            Cli::try_parse_from(["ks", "qo", "--idempotency-key", "qo-0001"])
                .unwrap()
                .command,
        )
        .unwrap();
        let base = auth::origin(&format!("http://127.0.0.1:{port}")).unwrap();
        let result = request(&call, &base, "fake-access", None).unwrap();
        assert_eq!(result["data"]["status"], "accepted");
        let sent = server.join().unwrap();
        let lower = sent.to_ascii_lowercase();
        assert!(sent.starts_with("POST /v1/quick-order "));
        assert!(lower.contains("idempotency-key: qo-0001"));
        assert!(lower.contains("content-type: application/json"));
        assert!(sent.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn quick_order_not_set_guides_to_app() {
        let (port, server) = serve_once(
            "422 Unprocessable Entity",
            r#"{"error":"quick_order_not_set"}"#,
        );
        let call = plan(
            Cli::try_parse_from(["ks", "qo", "--idempotency-key", "qo-0001"])
                .unwrap()
                .command,
        )
        .unwrap();
        let base = auth::origin(&format!("http://127.0.0.1:{port}")).unwrap();
        let error = request(&call, &base, "fake-access", None).unwrap_err();
        assert!(error.starts_with("HTTP 422: quick_order_not_set"));
        assert!(error.contains("連携のタブ"));
        server.join().unwrap();
    }

    #[test]
    fn quick_order_show_accepts_unset_and_set_product() {
        assert!(display(&json!({"data":{"product":null}}), "quick-order", false).is_ok());
        let set = json!({"data":{"product":{"jan":"4901234567894","name":"お茶","unit_price_jpy":120,"purchase_unit":"package"}}});
        assert!(display(&set, "quick-order", false).is_ok());
        assert!(display(&json!({"data":{}}), "quick-order", false).is_err());
    }
}
