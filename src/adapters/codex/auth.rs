use std::env;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{SecondsFormat, Utc};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};

use super::CodexAdapter;
use super::now_ts;
use super::paths::codex_home;
use crate::core::state::LiveIdentity;
use crate::core::storage;

const CHATGPT_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REFRESH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REFRESH_TOKEN_URL_OVERRIDE_ENV: &str = "CODEX_REFRESH_TOKEN_URL_OVERRIDE";
const ACCESS_TOKEN_REFRESH_SKEW_SECS: i64 = 60;

static OAUTH_HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

fn oauth_http_client() -> &'static Client {
    OAUTH_HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build oauth HTTP client")
    })
}

impl CodexAdapter {
    pub(super) fn read_auth_json(&self, path: &Path) -> Result<Value> {
        storage::ensure_exists(path, "auth.json")?;
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let auth: Value = serde_json::from_str(&contents)
            .with_context(|| format!("invalid JSON in {}", path.display()))?;
        Ok(auth)
    }

    /// 过期则续期并写回 stored；成功后若 live 仍是同一 ChatGPT 账号则同步 live。
    pub(super) fn refresh_stored_auth_if_needed(
        &self,
        auth_path: &Path,
        auth: Value,
    ) -> Result<Value> {
        if !auth_needs_token_refresh(&auth, now_ts()) {
            return Ok(auth);
        }
        self.refresh_stored_auth(auth_path, auth)
    }

    pub(super) fn refresh_stored_auth(&self, auth_path: &Path, auth: Value) -> Result<Value> {
        let refreshed = refresh_chatgpt_oauth(auth)?;
        write_auth_json(auth_path, &refreshed)?;
        let _ = self.sync_refreshed_auth_to_live_if_current(&refreshed);
        Ok(refreshed)
    }

    fn sync_refreshed_auth_to_live_if_current(&self, refreshed: &Value) -> Result<()> {
        let live_path = codex_home().join("auth.json");
        if !live_path.exists() {
            return Ok(());
        }
        let live = self.read_auth_json(&live_path)?;
        if !chatgpt_identities_match(&live, refreshed) {
            return Ok(());
        }
        write_auth_json(&live_path, refreshed)
    }
}

pub(super) fn decode_identity(auth: &Value) -> Result<LiveIdentityWithPlan> {
    let id_token = auth
        .pointer("/tokens/id_token")
        .and_then(Value::as_str)
        .context("auth.json is missing tokens.id_token")?;
    let claims = decode_jwt_payload(id_token).context("failed to parse JWT claims")?;
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .context("auth.json is missing email in id_token")?;
    let plan = claims
        .get("https://api.openai.com/auth")
        .and_then(|value| value.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .map(normalize_plan);
    let account_id = auth
        .pointer("/tokens/account_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok(LiveIdentityWithPlan {
        email,
        account_id,
        plan,
    })
}

pub(super) fn normalize_plan(raw: &str) -> String {
    let value = raw.trim().to_ascii_lowercase();
    if value.is_empty() {
        return String::new();
    }
    let mut chars = value.chars();
    let head = chars.next().unwrap().to_ascii_uppercase();
    format!("{head}{}", chars.as_str())
}

pub(super) fn chatgpt_identities_match(left: &Value, right: &Value) -> bool {
    let (Ok(left), Ok(right)) = (decode_identity(left), decode_identity(right)) else {
        return false;
    };
    if left.email.eq_ignore_ascii_case(&right.email) {
        return true;
    }
    matches!(
        (&left.account_id, &right.account_id),
        (Some(left_id), Some(right_id)) if left_id == right_id
    )
}

pub(super) fn live_auth_is_newer(live: &Value, stored: &Value) -> bool {
    match (auth_freshness_ts(live), auth_freshness_ts(stored)) {
        (Some(live_ts), Some(stored_ts)) => live_ts > stored_ts,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => false,
    }
}

pub(super) fn auth_needs_token_refresh(auth: &Value, now: i64) -> bool {
    let has_refresh_token = json_nonempty_str(auth, "/tokens/refresh_token").is_some();
    if !has_refresh_token {
        return false;
    }
    match json_nonempty_str(auth, "/tokens/access_token") {
        None => true,
        Some(token) => jwt_numeric_claim(token, "exp")
            .is_some_and(|exp| exp <= now + ACCESS_TOKEN_REFRESH_SKEW_SECS),
    }
}

fn refresh_chatgpt_oauth(auth: Value) -> Result<Value> {
    let refresh_token = json_nonempty_str(&auth, "/tokens/refresh_token")
        .context("auth.json is missing tokens.refresh_token")?
        .to_owned();
    let url = refresh_token_url();
    let request = json!({
        "client_id": CHATGPT_OAUTH_CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    });

    let response = oauth_http_client()
        .post(&url)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .json(&request)
        .send()
        .with_context(|| format!("POST {url} failed"))?;

    let status = response.status();
    let payload = response
        .text()
        .with_context(|| format!("failed to read OAuth refresh response from {url}"))?;
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::BAD_REQUEST {
        bail!("OAuth token refresh failed: {status}");
    }
    if !status.is_success() {
        bail!("OAuth token refresh failed: {status}");
    }

    let payload: Value =
        serde_json::from_str(&payload).context("invalid JSON in OAuth refresh response")?;
    apply_token_refresh(auth, &payload)
}

pub(super) fn apply_token_refresh(mut auth: Value, payload: &Value) -> Result<Value> {
    let access_token = json_nonempty_str(payload, "/access_token")
        .context("OAuth refresh response is missing access_token")?;
    let tokens = auth
        .pointer_mut("/tokens")
        .and_then(Value::as_object_mut)
        .context("auth.json is missing tokens")?;
    tokens.insert(
        "access_token".into(),
        Value::String(access_token.to_owned()),
    );
    if let Some(refresh_token) = json_nonempty_str(payload, "/refresh_token") {
        tokens.insert(
            "refresh_token".into(),
            Value::String(refresh_token.to_owned()),
        );
    }
    if let Some(id_token) = json_nonempty_str(payload, "/id_token") {
        tokens.insert("id_token".into(), Value::String(id_token.to_owned()));
    }
    let root = auth
        .as_object_mut()
        .context("auth.json root must be an object")?;
    root.insert(
        "last_refresh".into(),
        Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)),
    );
    Ok(auth)
}

pub(super) fn write_auth_json(path: &Path, auth: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{}.tmp",
            path.file_name()
                .and_then(|item| item.to_str())
                .unwrap_or("auth.json")
        ));
    let mut bytes = serde_json::to_vec_pretty(auth).context("failed to serialize auth.json")?;
    bytes.push(b'\n');
    fs::write(&tmp, &bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to restrict permissions on {}", tmp.display()))?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to move {} into place", tmp.display()))?;
    Ok(())
}

fn refresh_token_url() -> String {
    env::var(REFRESH_TOKEN_URL_OVERRIDE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| REFRESH_TOKEN_URL.to_string())
}

fn auth_freshness_ts(auth: &Value) -> Option<i64> {
    auth.get("last_refresh")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_nanos)
        .or_else(|| {
            json_nonempty_str(auth, "/tokens/access_token")
                .and_then(|token| jwt_numeric_claim(token, "iat"))
        })
        .or_else(|| {
            json_nonempty_str(auth, "/tokens/access_token")
                .and_then(|token| jwt_numeric_claim(token, "exp"))
        })
}

fn parse_rfc3339_nanos(value: &str) -> Option<i64> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value.trim()).ok()?;
    parsed
        .timestamp_nanos_opt()
        .or_else(|| parsed.timestamp_millis().checked_mul(1_000_000))
}

fn decode_jwt_payload(token: &str) -> Result<Value> {
    let payload = token
        .split('.')
        .nth(1)
        .context("auth.json id_token is not a valid JWT")?;
    let claims = URL_SAFE_NO_PAD
        .decode(payload)
        .context("failed to decode JWT payload")?;
    serde_json::from_slice(&claims).context("failed to parse JWT claims")
}

fn jwt_numeric_claim(token: &str, claim: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    let value = claims.get(claim)?;
    value
        .as_i64()
        .or_else(|| value.as_u64().map(|item| item as i64))
        .or_else(|| value.as_f64().map(|item| item as i64))
}

fn json_nonempty_str<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

#[derive(Debug)]
pub(super) struct LiveIdentityWithPlan {
    pub(super) email: String,
    pub(super) account_id: Option<String>,
    pub(super) plan: Option<String>,
}

impl From<LiveIdentityWithPlan> for LiveIdentity {
    fn from(value: LiveIdentityWithPlan) -> Self {
        Self {
            email: value.email,
            account_id: value.account_id,
            scodex_account_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use std::fs;

    use anyhow::Result;
    use base64::Engine;
    use serde_json::{Value, json};

    use super::{
        apply_token_refresh, auth_needs_token_refresh, chatgpt_identities_match, decode_identity,
        live_auth_is_newer, refresh_chatgpt_oauth, write_auth_json,
    };
    use crate::adapters::codex::{EnvGuard, TEST_ENV_LOCK};

    fn fake_jwt(payload: &str) -> String {
        let header = super::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = super::URL_SAFE_NO_PAD.encode(payload);
        format!("{header}.{payload}.sig")
    }

    fn spawn_json_server(
        handler: impl Fn(&str, &[u8]) -> (u16, String) + Send + Sync + 'static,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                            if buf.len() > 64 * 1024 {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let header_end = buf
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|index| index + 4)
                    .unwrap_or(buf.len());
                let header_text = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]);
                let request_line = header_text.lines().next().unwrap_or_default().to_string();
                let content_length = header_text
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().ok())
                            .flatten()
                    })
                    .unwrap_or(0usize);
                let mut body = buf.get(header_end..).unwrap_or(&[]).to_vec();
                while body.len() < content_length {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => body.extend_from_slice(&tmp[..n]),
                        Err(_) => break,
                    }
                }
                body.truncate(content_length);
                let (status, resp_body) = handler(&request_line, &body);
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    400 => "Bad Request",
                    _ => "Error",
                };
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                    resp_body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn decode_identity_reads_email_plan_and_account_id() -> Result<()> {
        let auth = json!({
            "tokens": {
                "id_token": fake_jwt(r#"{"email":"a@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#),
                "account_id": "acct-1"
            }
        });

        let identity = decode_identity(&auth)?;

        assert_eq!(identity.email, "a@example.com");
        assert_eq!(identity.account_id.as_deref(), Some("acct-1"));
        assert_eq!(identity.plan.as_deref(), Some("Plus"));
        Ok(())
    }

    #[test]
    fn auth_needs_token_refresh_uses_jwt_exp_and_skew() {
        let expired = json!({
            "tokens": {
                "access_token": fake_jwt(r#"{"exp":100}"#),
                "refresh_token": "rt-1"
            }
        });
        let valid = json!({
            "tokens": {
                "access_token": fake_jwt(r#"{"exp":100000}"#),
                "refresh_token": "rt-1"
            }
        });
        assert!(auth_needs_token_refresh(&expired, 200));
        assert!(!auth_needs_token_refresh(&valid, 200));
        assert!(!auth_needs_token_refresh(
            &json!({"tokens":{"access_token": fake_jwt(r#"{"exp":100}"#)}}),
            200
        ));
        assert!(auth_needs_token_refresh(
            &json!({"tokens":{"refresh_token":"rt-1"}}),
            200
        ));
    }

    #[test]
    fn apply_token_refresh_updates_tokens_and_keeps_account_id() -> Result<()> {
        let auth = json!({
            "OPENAI_API_KEY": "sk-keep",
            "auth_mode": "chatgpt",
            "last_refresh": "2026-08-24T07:28:44Z",
            "tokens": {
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "id_token": "old-id",
                "account_id": "acct-1"
            }
        });
        let updated = apply_token_refresh(
            auth,
            &json!({
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "id_token": "new-id"
            }),
        )?;

        assert_eq!(
            updated
                .pointer("/tokens/access_token")
                .and_then(Value::as_str),
            Some("new-access")
        );
        assert_eq!(
            updated
                .pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("new-refresh")
        );
        assert_eq!(
            updated.pointer("/tokens/id_token").and_then(Value::as_str),
            Some("new-id")
        );
        assert_eq!(
            updated
                .pointer("/tokens/account_id")
                .and_then(Value::as_str),
            Some("acct-1")
        );
        assert_eq!(
            updated.get("OPENAI_API_KEY").and_then(Value::as_str),
            Some("sk-keep")
        );
        let last_refresh = updated
            .get("last_refresh")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(last_refresh.starts_with("20"));
        assert_ne!(last_refresh, "2026-08-24T07:28:44Z");
        Ok(())
    }

    #[test]
    fn apply_token_refresh_keeps_old_refresh_token_when_omitted() -> Result<()> {
        let auth = json!({
            "tokens": {
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "account_id": "acct-1"
            }
        });
        let updated = apply_token_refresh(auth, &json!({ "access_token": "new-access" }))?;
        assert_eq!(
            updated
                .pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("old-refresh")
        );
        Ok(())
    }

    #[test]
    fn live_auth_is_newer_compares_last_refresh() {
        let older = json!({"last_refresh": "2026-08-24T07:28:44.000000000Z"});
        let newer = json!({"last_refresh": "2026-09-04T01:01:08.861768319Z"});
        assert!(live_auth_is_newer(&newer, &older));
        assert!(!live_auth_is_newer(&older, &newer));
        assert!(!live_auth_is_newer(&newer, &newer));
    }

    #[test]
    fn chatgpt_identities_match_by_email() {
        let left = json!({
            "tokens": {
                "id_token": fake_jwt(r#"{"email":"A@example.com"}"#),
                "account_id": "acct-1"
            }
        });
        let right = json!({
            "tokens": {
                "id_token": fake_jwt(r#"{"email":"a@example.com"}"#),
                "account_id": "acct-2"
            }
        });
        assert!(chatgpt_identities_match(&left, &right));
    }

    #[test]
    fn write_auth_json_replaces_target_atomically() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("scodex-auth-write-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir)?;
        let path = dir.join("auth.json");
        write_auth_json(&path, &json!({"tokens":{"access_token":"x"}}))?;
        let parsed: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(
            parsed
                .pointer("/tokens/access_token")
                .and_then(Value::as_str),
            Some("x")
        );
        assert!(!dir.join(".auth.json.tmp").exists());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn refresh_chatgpt_oauth_persists_rotated_tokens_from_http() -> Result<()> {
        let seen_refresh = Arc::new(Mutex::new(String::new()));
        let seen_refresh_clone = Arc::clone(&seen_refresh);
        let base = spawn_json_server(move |request_line, body| {
            assert!(request_line.starts_with("POST /oauth/token"));
            let payload: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            *seen_refresh_clone.lock().unwrap() = payload
                .get("refresh_token")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            (
                200,
                json!({
                    "access_token": "new-access",
                    "refresh_token": "new-refresh",
                    "id_token": "new-id"
                })
                .to_string(),
            )
        });

        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _url = EnvGuard::set(
            "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
            format!("{base}/oauth/token"),
        );
        let auth = json!({
            "tokens": {
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "id_token": "old-id",
                "account_id": "acct-1"
            }
        });
        let updated = refresh_chatgpt_oauth(auth)?;
        assert_eq!(seen_refresh.lock().unwrap().as_str(), "old-refresh");
        assert_eq!(
            updated
                .pointer("/tokens/access_token")
                .and_then(Value::as_str),
            Some("new-access")
        );
        assert_eq!(
            updated
                .pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("new-refresh")
        );
        Ok(())
    }

    #[test]
    fn refresh_chatgpt_oauth_rejects_unauthorized() {
        let base = spawn_json_server(|_, _| (401, r#"{"error":"invalid_grant"}"#.into()));
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _url = EnvGuard::set(
            "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
            format!("{base}/oauth/token"),
        );
        let auth = json!({
            "tokens": {
                "refresh_token": "revoked"
            }
        });
        let err = refresh_chatgpt_oauth(auth).unwrap_err().to_string();
        assert!(err.contains("401"), "{err}");
    }
}
