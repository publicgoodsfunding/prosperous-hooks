use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

#[derive(Debug, Clone)]
pub struct TokenClaims {
    pub email: String,
    pub org_id: String,
    pub exp: i64,
}

#[derive(Debug)]
pub enum AuthState {
    NotLoggedIn,
    HasApiKey(String),
    LoggedInCurrent(TokenClaims),
    LoggedInExpired(TokenClaims),
}

#[derive(Debug)]
pub enum ClientError {
    NotLoggedIn,
    TokenExpired(TokenClaims),
    ExchangeFailed,
}

pub struct ClientOptions {
    pub prosperous_key: Option<String>,
    pub base_url: Option<String>,
}

pub struct ProsperousClient {
    options: ClientOptions,
    state: AuthState,
}

impl ProsperousClient {
    pub fn new(options: ClientOptions) -> Self {
        ProsperousClient {
            options,
            state: AuthState::NotLoggedIn,
        }
    }

    pub fn state(&self) -> &AuthState {
        &self.state
    }

    pub async fn initialize(&mut self) -> Result<(), ClientError> {
        // 1. Check for a cached JWT token on disk.
        if let Some(token_str) = find_token_file() {
            if let Some(claims) = parse_jwt_claims(&token_str) {
                if !is_expired(claims.exp) {
                    self.state = AuthState::LoggedInCurrent(claims);
                    return Ok(());
                }

                // Token is expired — try to refresh if we have an API key.
                if let Some(key) = self.effective_api_key() {
                    return match self.do_exchange(&key).await {
                        Some(new_token) if parse_jwt_claims(&new_token).is_some() => {
                            let new_claims = parse_jwt_claims(&new_token).unwrap();
                            self.state = AuthState::LoggedInCurrent(new_claims);
                            Ok(())
                        }
                        _ => {
                            self.state = AuthState::LoggedInExpired(claims);
                            Err(ClientError::ExchangeFailed)
                        }
                    };
                }

                self.state = AuthState::LoggedInExpired(claims.clone());
                return Err(ClientError::TokenExpired(claims));
            }
        }

        // 2. No usable cached token — try exchanging an API key for one.
        if let Some(key) = self.effective_api_key() {
            self.state = AuthState::HasApiKey(key.clone());
            return match self.do_exchange(&key).await {
                Some(token) if parse_jwt_claims(&token).is_some() => {
                    let claims = parse_jwt_claims(&token).unwrap();
                    self.state = AuthState::LoggedInCurrent(claims);
                    Ok(())
                }
                _ => {
                    self.state = AuthState::NotLoggedIn;
                    Err(ClientError::ExchangeFailed)
                }
            };
        }

        // 3. No credentials available at all.
        self.state = AuthState::NotLoggedIn;
        Err(ClientError::NotLoggedIn)
    }

    // Returns the API key from options, falling back to the PROSPEROUS_KEY env var.
    // If options.prosperous_key is explicitly set (even to empty), the env var is not consulted.
    fn effective_api_key(&self) -> Option<String> {
        if let Some(key) = &self.options.prosperous_key {
            let trimmed = key.trim();
            return if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) };
        }
        std::env::var("PROSPEROUS_KEY")
            .ok()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty())
    }

    async fn do_exchange(&self, api_key: &str) -> Option<String> {
        let base_url = self.options.base_url.as_deref()?.trim_end_matches('/');
        let url = format!("{base_url}/auth/exchange");
        let uri: hyper::Uri = url.parse().ok()?;

        let body = serde_json::json!({"api_key": api_key}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri(&uri)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .ok()?;

        let res = Client::builder(TokioExecutor::new())
            .build_http()
            .request(req)
            .await
            .ok()?;

        if !res.status().is_success() {
            return None;
        }

        let bytes = res.into_body().collect().await.ok()?.to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        Some(json["token"].as_str()?.to_owned())
    }
}

fn find_token_file() -> Option<String> {
    let home = dirs::home_dir();
    let mut current = std::env::current_dir().ok()?;

    loop {
        let candidate = current.join(".prosperous").join("token");
        if candidate.exists() {
            if let Ok(contents) = std::fs::read_to_string(&candidate) {
                let trimmed = contents.trim().to_owned();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }

        if home.as_deref() == Some(current.as_path()) {
            break;
        }

        match current.parent().map(PathBuf::from) {
            Some(parent) if parent != current => current = parent,
            _ => break,
        }
    }

    None
}

pub fn parse_jwt_claims(token: &str) -> Option<TokenClaims> {
    let parts: Vec<&str> = token.splitn(4, '.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_b64 = parts[1];
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;

    let json_str = String::from_utf8(decoded).ok()?;
    let value: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    let email = value["email"].as_str()?.to_owned();
    let org_id = value["org_id"].as_str()?.to_owned();
    let exp = value["exp"]
        .as_i64()
        .or_else(|| value["exp"].as_f64().map(|f| f as i64))?;

    Some(TokenClaims { email, org_id, exp })
}

fn is_expired(exp: i64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(i64::MAX);
    now >= exp
}
