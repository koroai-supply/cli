use std::{
    env,
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use url::Url;

pub const CLIENT_ID: &str = "koroai-cli";
pub const AUTH_URL: &str = "https://koroai.supply";
pub const API_URL: &str = "https://api.koroai.supply";

pub fn origin(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "接続先URLが正しくありません".to_owned())?;
    let localhost = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(localhost && url.scheme() == "http"))
        || url.username() != ""
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("接続先はHTTPSのオリジンを指定してください（localhostのみHTTP可）".into());
    }
    Ok(url)
}

pub fn endpoint(base: &Url, path: &str) -> Result<Url, String> {
    base.join(path)
        .map_err(|_| "接続先URLが正しくありません".into())
}

fn read_json(response: ureq::Response) -> Result<Value, String> {
    let body = response
        .into_string()
        .map_err(|_| "応答を読み取れませんでした".to_owned())?;
    serde_json::from_str(&body).map_err(|_| "APIの応答がJSONではありません".into())
}

fn error_code(response: ureq::Response) -> String {
    response
        .into_string()
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|body| body.get("error")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown_error".into())
}

pub struct Profile {
    pub auth_url: Url,
    pub api_url: Url,
}

impl Profile {
    pub fn new(auth_url: &str, api_url: &str) -> Result<Self, String> {
        Ok(Self {
            auth_url: origin(auth_url)?,
            api_url: origin(api_url)?,
        })
    }

    fn service(&self) -> String {
        format!(
            "koroai-cli:{}:{}",
            self.auth_url.origin().ascii_serialization(),
            self.api_url.origin().ascii_serialization()
        )
    }
    fn account(&self) -> String {
        "live".into()
    }
}

#[derive(Clone)]
pub struct Token {
    pub access: String,
    pub refresh: String,
    pub expires_at: u64,
    pub home_id: String,
    pub scope: String,
    pub mode: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Token {
    fn from_response(value: &Value, mode: &str) -> Result<Self, String> {
        let field = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("OAuth応答に{key}がありません"))
        };
        let expires_in = value
            .get("expires_in")
            .and_then(Value::as_u64)
            .filter(|v| *v > 0)
            .ok_or("OAuth応答に有効期限がありません")?;
        if value.get("token_type").and_then(Value::as_str) != Some("Bearer")
            || value.get("mode").and_then(Value::as_str) != Some(mode)
        {
            return Err("OAuth応答のトークン種別またはモードが一致しません".into());
        }
        Ok(Self {
            access: field("access_token")?,
            refresh: field("refresh_token")?,
            expires_at: now().saturating_add(expires_in),
            home_id: field("home_id")?,
            scope: field("scope")?,
            mode: mode.into(),
        })
    }

    fn to_json(&self) -> Value {
        json!({"access_token":self.access,"refresh_token":self.refresh,"expires_at":self.expires_at,"home_id":self.home_id,"scope":self.scope,"mode":self.mode})
    }

    fn from_stored(value: Value, mode: &str) -> Result<Self, String> {
        let field = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("保存された認証情報に{key}がありません"))
        };
        let token = Self {
            access: field("access_token")?,
            refresh: field("refresh_token")?,
            expires_at: value
                .get("expires_at")
                .and_then(Value::as_u64)
                .ok_or("保存された認証情報に期限がありません")?,
            home_id: field("home_id")?,
            scope: field("scope")?,
            mode: field("mode")?,
        };
        if token.mode != mode {
            return Err("保存された認証情報のモードが一致しません".into());
        }
        Ok(token)
    }
}

#[cfg(target_os = "macos")]
mod store {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };
    pub fn read(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        match get_generic_password(service, account) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err("キーチェーンを読み取れませんでした".into()),
        }
    }
    pub fn write(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
        set_generic_password(service, account, value)
            .map_err(|_| "キーチェーンに保存できませんでした".into())
    }
    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        match delete_generic_password(service, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err("キーチェーンから削除できませんでした".into()),
        }
    }
}

#[cfg(target_os = "linux")]
mod store {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    pub fn read(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        let output = Command::new("secret-tool")
            .args(["lookup", "service", service, "account", account])
            .output()
            .map_err(|_| "secret-tool（libsecret）をインストールしてください".to_owned())?;
        match output.status.code() {
            Some(0) => Ok(Some(
                output
                    .stdout
                    .into_iter()
                    .take_while(|b| *b != b'\n')
                    .collect(),
            )),
            Some(1) => Ok(None),
            _ => Err("資格情報ストアを読み取れませんでした".into()),
        }
    }
    pub fn write(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label=頃合 CLI",
                "service",
                service,
                "account",
                account,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|_| "secret-tool（libsecret）をインストールしてください".to_owned())?;
        child
            .stdin
            .take()
            .ok_or("資格情報ストアへ書き込めませんでした")?
            .write_all(value)
            .map_err(|_| "資格情報ストアへ書き込めませんでした")?;
        if child
            .wait()
            .map_err(|_| "資格情報ストアへ保存できませんでした")?
            .success()
        {
            Ok(())
        } else {
            Err("資格情報ストアへ保存できませんでした".into())
        }
    }
    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        let status = Command::new("secret-tool")
            .args(["clear", "service", service, "account", account])
            .status()
            .map_err(|_| "secret-tool（libsecret）をインストールしてください".to_owned())?;
        if status.success() {
            Ok(())
        } else {
            Err("資格情報ストアから削除できませんでした".into())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod store {
    pub fn read(_: &str, _: &str) -> Result<Option<Vec<u8>>, String> {
        Err("このOSの資格情報ストアは未対応です".into())
    }
    pub fn write(_: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Err("このOSの資格情報ストアは未対応です".into())
    }
    pub fn delete(_: &str, _: &str) -> Result<(), String> {
        Err("このOSの資格情報ストアは未対応です".into())
    }
}

pub fn load(profile: &Profile) -> Result<Option<Token>, String> {
    let Some(bytes) = store::read(&profile.service(), &profile.account())? else {
        return Ok(None);
    };
    let value = serde_json::from_slice(bytes.as_slice())
        .map_err(|_| "保存された認証情報が壊れています".to_owned())?;
    Token::from_stored(value, "live").map(Some)
}

fn save(profile: &Profile, token: &Token) -> Result<(), String> {
    let bytes = serde_json::to_vec(&token.to_json())
        .map_err(|_| "認証情報を保存できませんでした".to_owned())?;
    store::write(&profile.service(), &profile.account(), &bytes)
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(target_os = "linux")]
    let command = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let command = "";
    if !command.is_empty() {
        let _ = Command::new(command).arg(url).spawn();
    }
}

pub fn login(profile: &Profile, no_browser: bool) -> Result<(), String> {
    if load(profile)?.is_some() {
        return Err(
            "この端末はログイン済みです。ホームを変える場合は先に ks logout を実行してください"
                .into(),
        );
    }
    let url = endpoint(&profile.auth_url, "/oauth/public/device_authorization")?;
    let device_name = env::var("HOSTNAME").unwrap_or_else(|_| "ks CLI".into());
    let device_name: String = device_name.chars().take(60).collect();
    let response = ureq::post(url.as_str())
        .send_form(&[
            ("client_id", CLIENT_ID),
            ("mode", "live"),
            ("device_name", &device_name),
        ])
        .map_err(|error| format!("端末認可を開始できませんでした: {}", request_error(error)))?;
    let data = read_json(response)?;
    let field = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("OAuth応答に{key}がありません"))
    };
    let device_code = field("device_code")?.to_owned();
    let code = field("user_code")?.to_owned();
    let uri = field("verification_uri_complete")?.to_owned();
    let verify = Url::parse(&uri).map_err(|_| "承認URLが正しくありません".to_owned())?;
    if verify.origin() != profile.auth_url.origin() {
        return Err("承認URLのドメインが一致しません".into());
    }
    let expires = data
        .get("expires_in")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or("OAuth応答に有効期限がありません")?;
    let mut interval = data
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .max(1);
    println!(
        "Webでこの端末を承認してください。\nURL: {uri}\n確認コード: {code}\n端末名: {device_name}\n有効期限: {expires}秒"
    );
    if !no_browser {
        open_browser(&uri);
    }
    let deadline = now().saturating_add(expires);
    let token_url = endpoint(&profile.auth_url, "/oauth/public/token")?;
    while now() < deadline {
        thread::sleep(Duration::from_secs(
            interval.min(deadline.saturating_sub(now())),
        ));
        if now() >= deadline {
            break;
        }
        let response = ureq::post(token_url.as_str()).send_form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", CLIENT_ID),
            ("device_code", &device_code),
        ]);
        match response {
            Ok(response) => {
                let token = Token::from_response(&read_json(response)?, "live")?;
                save(profile, &token)?;
                println!("ログインしました。");
                return Ok(());
            }
            Err(ureq::Error::Status(_, response)) => match error_code(response).as_str() {
                "authorization_pending" => {}
                "slow_down" => interval = interval.saturating_add(5),
                "access_denied" => return Err("端末の承認が取り消されました".into()),
                "expired_token" => return Err("承認コードの有効期限が切れました".into()),
                code => return Err(format!("端末認可に失敗しました: {code}")),
            },
            Err(ureq::Error::Transport(_)) => interval = interval.saturating_add(5),
        }
    }
    Err("承認コードの有効期限が切れました".into())
}

fn request_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(status, response) => format!("HTTP {status}: {}", error_code(response)),
        ureq::Error::Transport(_) => "通信エラー".into(),
    }
}

pub fn access_token(profile: &Profile) -> Result<Token, String> {
    let token = load(profile)?.ok_or("ログインが必要です。ks login を実行してください")?;
    if token.expires_at > now().saturating_add(60) {
        return Ok(token);
    }
    let url = endpoint(&profile.auth_url, "/oauth/public/token")?;
    let response = ureq::post(url.as_str())
        .send_form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", &token.refresh),
        ])
        .map_err(|error| {
            format!(
                "トークンを更新できませんでした: {}。必要なら ks login を実行してください",
                request_error(error)
            )
        })?;
    let updated = Token::from_response(&read_json(response)?, "live")?;
    if updated.home_id != token.home_id {
        return Err("更新後のホームが一致しません".into());
    }
    save(profile, &updated)?;
    Ok(updated)
}

pub fn logout(profile: &Profile) -> Result<(), String> {
    let Some(token) = load(profile)? else {
        println!("ログインしていません。");
        return Ok(());
    };
    let url = endpoint(&profile.auth_url, "/oauth/public/revoke")?;
    ureq::post(url.as_str())
        .send_form(&[("client_id", CLIENT_ID), ("token", &token.refresh)])
        .map_err(|error| {
            format!(
                "サーバー側の失効を確認できませんでした: {}。資格情報は残しています",
                request_error(error)
            )
        })?;
    store::delete(&profile.service(), &profile.account())?;
    println!("この端末をログアウトしました。");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_token_for_another_mode() {
        let token = json!({"access_token":"access","refresh_token":"refresh","token_type":"Bearer","expires_in":3600,"home_id":"home","scope":"orders:read","mode":"sandbox"});
        assert!(Token::from_response(&token, "live").is_err());
    }

    #[test]
    fn keeps_credentials_separate_by_api_origin() {
        let default = Profile::new(AUTH_URL, API_URL).unwrap();
        let alternate = Profile::new(AUTH_URL, "https://other-api.example.com").unwrap();
        assert_ne!(default.service(), alternate.service());
    }
}
