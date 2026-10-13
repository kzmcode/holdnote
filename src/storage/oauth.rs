//! Google OAuth 2.0 for desktop apps: system browser + loopback redirect (127.0.0.1:<random>) + PKCE (S256)
//! + state check. Also the desktop Google Picker flow (trigger_onepick / allow_folder_selection).
//! OWNER: drive agent. Never log tokens.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::security::{self, StoredTokens};
use crate::storage::SaveError;

#[derive(Debug, Clone)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
    /// Google Cloud project number, used as Picker app id if needed.
    pub project_number: Option<String>,
}

pub const SCOPE_DRIVE_FILE: &str = "https://www.googleapis.com/auth/drive.file";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const FLOW_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct AuthOutcome {
    /// Email of the account if obtainable without extra scopes (e.g. from id_token with "openid email"), else None.
    pub email: Option<String>,
    /// Present when the flow included the Picker and the user picked something.
    pub picked_file_ids: Vec<String>,
}

struct Cached {
    token: String,
    expires_at: i64,
}
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
static EMAIL: Mutex<Option<String>> = Mutex::new(None);

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub(crate) fn http_client() -> Result<reqwest::blocking::Client, SaveError> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| SaveError::Fatal(format!("HTTPクライアントを初期化できません: {e}")))
}

/// Drop the in-memory access token (e.g. after a 401).
pub fn invalidate_cached_token() {
    *CACHE.lock().unwrap() = None;
    // Also drop the stored access token so the next call refreshes.
    if let Ok(Some(mut t)) = security::load_tokens() {
        t.access_token = None;
        t.access_token_expires_at = None;
        let _ = security::save_tokens(&t);
    }
}

/// Email of the connected account if known (from id_token during the non-picker flow; best effort).
pub fn user_email() -> Option<String> {
    EMAIL.lock().unwrap().clone()
}

// ---------- pure helpers ----------

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_token(bytes: usize) -> anyhow::Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("乱数生成に失敗しました: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

pub(crate) fn form_encode(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

pub fn build_auth_url(
    client: &OAuthClient,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
    with_picker: bool,
) -> String {
    // Picker flow: drive.file "can't be combined with any other scope".
    let scope = if with_picker { SCOPE_DRIVE_FILE.to_string() } else { format!("{SCOPE_DRIVE_FILE} openid email") };
    let mut pairs: Vec<(&str, &str)> = vec![
        ("client_id", &client.client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", &scope),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    if with_picker {
        pairs.push(("trigger_onepick", "true"));
        pairs.push(("allow_folder_selection", "true"));
    }
    format!("{AUTH_URL}?{}", form_encode(&pairs))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RedirectParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub picked_file_ids: Vec<String>,
}

/// Parse the HTTP request line (`GET /?code=..&state=.. HTTP/1.1`).
pub fn parse_redirect_request(request: &str) -> Option<RedirectParams> {
    let line = request.lines().next()?;
    let mut it = line.split_whitespace();
    if it.next()? != "GET" {
        return None;
    }
    let target = it.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return None;
    }
    let mut p = RedirectParams::default();
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => p.code = Some(v.into_owned()),
            "state" => p.state = Some(v.into_owned()),
            "error" => p.error = Some(v.into_owned()),
            "picked_file_ids" => {
                p.picked_file_ids = v.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
            }
            _ => {}
        }
    }
    if p.code.is_none() && p.error.is_none() {
        return None;
    }
    Some(p)
}

/// Decode the (unverified) JWT payload and read `email`. Display only.
pub fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("email")?.as_str().map(|s| s.to_string())
}

// ---------- loopback ----------

const DONE_HTML: &str = "<!doctype html><html lang=\"ja\"><head><meta charset=\"utf-8\"><title>HoldNote</title></head><body style=\"font-family:-apple-system,sans-serif;text-align:center;margin-top:20vh\"><h2>認証が完了しました</h2><p>HoldNoteに戻ってください。このタブは閉じて構いません。</p></body></html>";
const FAIL_HTML: &str = "<!doctype html><html lang=\"ja\"><head><meta charset=\"utf-8\"><title>HoldNote</title></head><body style=\"font-family:-apple-system,sans-serif;text-align:center;margin-top:20vh\"><h2>認証に失敗しました</h2><p>HoldNoteに戻ってやり直してください。</p></body></html>";

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn wait_for_redirect(listener: &TcpListener, expected_state: &str) -> anyhow::Result<RedirectParams> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + FLOW_TIMEOUT;
    loop {
        if Instant::now() > deadline {
            bail!("認証がタイムアウトしました(5分)");
        }
        let (mut stream, _) = match listener.accept() {
            Ok(x) => x,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = vec![0u8; 16 * 1024];
        let mut n = 0;
        loop {
            match stream.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => {
                    n += k;
                    if buf[..n].windows(2).any(|w| w == b"\r\n") || n == buf.len() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        match parse_redirect_request(&req) {
            Some(p) => {
                if p.state.as_deref() != Some(expected_state) {
                    respond(&mut stream, "400 Bad Request", FAIL_HTML);
                    continue; // ignore forged/stale requests
                }
                respond(&mut stream, "200 OK", if p.error.is_some() { FAIL_HTML } else { DONE_HTML });
                return Ok(p);
            }
            None => respond(&mut stream, "404 Not Found", "not found"),
        }
    }
}

// ---------- token endpoint ----------

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

fn token_request(form: &str) -> Result<TokenResponse, SaveError> {
    let http = http_client()?;
    let resp = http
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form.to_string())
        .send()
        .map_err(|e| SaveError::Retryable(format!("Google認証サーバーに接続できません: {}", e.without_url())))?;
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    if status == 200 {
        return serde_json::from_str(&text).map_err(|_| SaveError::Retryable("認証サーバーの応答を解釈できません".into()));
    }
    Err(map_token_error(status, &text))
}

pub fn map_token_error(status: u16, body: &str) -> SaveError {
    let err = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    if status == 429 || status >= 500 {
        SaveError::Retryable(format!("Google認証サーバーのエラー (HTTP {status})"))
    } else if err == "invalid_grant" {
        SaveError::AuthRequired
    } else if err == "invalid_client" || err == "unauthorized_client" {
        SaveError::Fatal("OAuthクライアントの設定が正しくありません(クライアントID/シークレットを確認してください)".into())
    } else if status == 401 {
        SaveError::AuthRequired
    } else {
        SaveError::Fatal(format!("認証エラー: {}", if err.is_empty() { format!("HTTP {status}") } else { err }))
    }
}

fn cache_set(token: &str, expires_in: i64) -> i64 {
    let exp = now() + expires_in;
    *CACHE.lock().unwrap() = Some(Cached { token: token.to_string(), expires_at: exp });
    exp
}

/// Full interactive sign-in. Opens the browser, waits (≤5 min) for the loopback redirect,
/// exchanges the code, stores tokens in Keychain via `security`. Blocking — call from a background thread.
/// `with_picker`: request the desktop Picker in the same consent (prompt=consent&trigger_onepick=true&allow_folder_selection=true).
pub fn authorize_interactive(client: &OAuthClient, with_picker: bool) -> anyhow::Result<AuthOutcome> {
    if client.client_id.trim().is_empty() {
        bail!("Google OAuthクライアントIDが設定されていません");
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let verifier = random_token(48)?;
    let state = random_token(24)?;
    let url = build_auth_url(client, &redirect_uri, &state, &pkce_challenge(&verifier), with_picker);

    let status = std::process::Command::new("/usr/bin/open").arg(&url).status()?;
    if !status.success() {
        bail!("ブラウザを開けませんでした");
    }
    let params = wait_for_redirect(&listener, &state)?;
    if let Some(e) = params.error {
        bail!("認証がキャンセルされました ({e})");
    }
    let code = params.code.ok_or_else(|| anyhow!("認可コードが返されませんでした"))?;

    let form = form_encode(&[
        ("code", &code),
        ("client_id", &client.client_id),
        ("client_secret", &client.client_secret),
        ("redirect_uri", &redirect_uri),
        ("grant_type", "authorization_code"),
        ("code_verifier", &verifier),
    ]);
    let tr = token_request(&form).map_err(|e| anyhow!("{e}"))?;
    let access = tr.access_token.ok_or_else(|| anyhow!("アクセストークンが返されませんでした"))?;
    let refresh = tr
        .refresh_token
        .ok_or_else(|| anyhow!("リフレッシュトークンが返されませんでした。再度お試しください"))?;
    let exp = cache_set(&access, tr.expires_in.unwrap_or(3600));
    security::save_tokens(&StoredTokens {
        refresh_token: refresh,
        access_token: Some(access),
        access_token_expires_at: Some(exp),
    })?;
    let email = tr.id_token.as_deref().and_then(email_from_id_token);
    if email.is_some() {
        *EMAIL.lock().unwrap() = email.clone();
    }
    Ok(AuthOutcome { email, picked_file_ids: params.picked_file_ids })
}

/// Valid access token: cached one if not expiring within 60 s, else refresh with the stored refresh token.
/// Err(SaveError::AuthRequired) if no refresh token or refresh rejected (invalid_grant); Retryable on network errors.
pub fn access_token(client: &OAuthClient) -> Result<String, SaveError> {
    if let Some(c) = CACHE.lock().unwrap().as_ref() {
        if c.expires_at - now() > 60 {
            return Ok(c.token.clone());
        }
    }
    let stored = security::load_tokens()
        .map_err(|e| SaveError::Fatal(format!("{e}")))?
        .ok_or(SaveError::AuthRequired)?;
    if let (Some(t), Some(exp)) = (&stored.access_token, stored.access_token_expires_at) {
        if exp - now() > 60 {
            cache_set(t, exp - now());
            return Ok(t.clone());
        }
    }
    let form = form_encode(&[
        ("client_id", &client.client_id),
        ("client_secret", &client.client_secret),
        ("refresh_token", &stored.refresh_token),
        ("grant_type", "refresh_token"),
    ]);
    let tr = token_request(&form)?;
    let access = tr.access_token.ok_or_else(|| SaveError::Retryable("アクセストークンが返されませんでした".into()))?;
    let exp = cache_set(&access, tr.expires_in.unwrap_or(3600));
    let _ = security::save_tokens(&StoredTokens {
        refresh_token: tr.refresh_token.unwrap_or(stored.refresh_token),
        access_token: Some(access.clone()),
        access_token_expires_at: Some(exp),
    });
    Ok(access)
}

/// Revoke the token at Google (best effort) and delete it from Keychain (API-11).
pub fn disconnect() -> anyhow::Result<()> {
    if let Ok(Some(t)) = security::load_tokens() {
        if let Ok(http) = http_client() {
            let _ = http
                .post(REVOKE_URL)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(form_encode(&[("token", &t.refresh_token)]))
                .send();
        }
    }
    *CACHE.lock().unwrap() = None;
    *EMAIL.lock().unwrap() = None;
    security::delete_tokens()
}

/// True if a refresh token exists in Keychain.
pub fn is_connected() -> bool {
    matches!(security::load_tokens(), Ok(Some(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> OAuthClient {
        OAuthClient { client_id: "id x".into(), client_secret: "s".into(), project_number: None }
    }

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn auth_url_picker() {
        let u = build_auth_url(&client(), "http://127.0.0.1:1234", "st", "ch", true);
        assert!(u.contains("trigger_onepick=true"));
        assert!(u.contains("allow_folder_selection=true"));
        assert!(u.contains("prompt=consent"));
        assert!(u.contains("client_id=id+x"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(!u.contains("openid"));
        let u2 = build_auth_url(&client(), "http://127.0.0.1:1234", "st", "ch", false);
        assert!(u2.contains("openid+email"));
        assert!(!u2.contains("trigger_onepick"));
    }

    #[test]
    fn parse_redirect() {
        let p = parse_redirect_request("GET /?code=4%2Fabc&state=xyz&picked_file_ids=a1,b2&scope=s HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert_eq!(p.code.as_deref(), Some("4/abc"));
        assert_eq!(p.state.as_deref(), Some("xyz"));
        assert_eq!(p.picked_file_ids, vec!["a1", "b2"]);
        let e = parse_redirect_request("GET /?error=access_denied&state=xyz HTTP/1.1\r\n").unwrap();
        assert_eq!(e.error.as_deref(), Some("access_denied"));
        assert!(parse_redirect_request("GET /favicon.ico HTTP/1.1\r\n").is_none());
        assert!(parse_redirect_request("POST /?code=a HTTP/1.1\r\n").is_none());
    }

    #[test]
    fn id_token_email() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"a@b.c"}"#);
        assert_eq!(email_from_id_token(&format!("h.{payload}.s")).as_deref(), Some("a@b.c"));
        assert_eq!(email_from_id_token("garbage"), None);
    }

    #[test]
    fn token_error_mapping() {
        assert!(matches!(map_token_error(400, r#"{"error":"invalid_grant"}"#), SaveError::AuthRequired));
        assert!(matches!(map_token_error(503, ""), SaveError::Retryable(_)));
        assert!(matches!(map_token_error(401, ""), SaveError::AuthRequired));
        assert!(matches!(map_token_error(400, r#"{"error":"invalid_client"}"#), SaveError::Fatal(_)));
    }
}
