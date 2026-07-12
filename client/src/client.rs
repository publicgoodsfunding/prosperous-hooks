use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

/// The claims decoded from a Prosperous JWT: who the user is, which
/// organization they belong to, and when the token stops being valid.
#[derive(Debug, Clone)]
pub struct TokenClaims {
    pub email: String,
    pub org_id: String,
    /// Unix timestamp (seconds) after which the token is considered expired.
    pub exp: i64,
}

/// The client's auth state machine. `ProsperousClient::initialize` walks
/// through these states based on what credentials are available: a token
/// cached on disk, an API key, or neither.
#[derive(Debug)]
pub enum AuthState {
    /// No usable credentials were found: no cached token file and no API key
    /// (from `ClientOptions` or the `PROSPEROUS_KEY` env var). This is also
    /// the client's initial state before `initialize` has run.
    NotLoggedIn,
    /// An API key was found and is about to be exchanged for a JWT via the
    /// server. This is a transient state set immediately before the exchange
    /// request is made; `initialize` always moves on to another state (never
    /// returns while still `HasApiKey`).
    HasApiKey(String),
    /// A JWT is cached — either found on disk or freshly obtained via
    /// exchange — and its `exp` claim has not yet passed. This is the only
    /// state in which authentication succeeded.
    LoggedInCurrent(TokenClaims),
    /// A JWT was found on disk but its `exp` claim has passed, and it could
    /// not be refreshed (either no API key was available to retry with, or
    /// the exchange attempt failed).
    LoggedInExpired(TokenClaims),
}

/// The error returned by `ProsperousClient::initialize` when it does not
/// land in `AuthState::LoggedInCurrent`. Each variant carries whatever
/// context is available so callers can report a useful message.
#[derive(Debug)]
pub enum ClientError {
    /// No cached token and no API key were available at all.
    NotLoggedIn,
    /// A cached token was found but has expired, and no API key was
    /// available to refresh it.
    TokenExpired(TokenClaims),
    /// An API key was available but exchanging it for a JWT failed (network
    /// error, non-2xx response, or an unparseable token in the response).
    ExchangeFailed,
}

/// Configuration passed into `ProsperousClient::new`. Both fields are
/// optional so callers can rely on cached credentials alone; when present,
/// `prosperous_key` takes precedence over the `PROSPEROUS_KEY` env var (see
/// `ProsperousClient::effective_api_key`).
pub struct ClientOptions {
    pub prosperous_key: Option<String>,
    /// Base URL of the Prosperous server used for API key exchange (e.g.
    /// `http://localhost:3000`). Required only when an exchange is needed.
    pub base_url: Option<String>,
}

/// Entry point for consumers of this crate. Wraps the current `AuthState`
/// and drives it forward via `initialize`.
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

    /// Resolves the client's `AuthState` by checking, in order: a cached
    /// token on disk, then an API key exchange. Updates `self.state` to
    /// reflect the outcome and returns `Ok(())` only when it lands in
    /// `AuthState::LoggedInCurrent`.
    pub async fn initialize(&mut self) -> Result<(), ClientError> {
        // 1. Prefer a cached token on disk over any API key — if it's still
        // valid there's no need to talk to the server at all.
        if let Some(token_str) = find_token_file() {
            if let Some(claims) = parse_jwt_claims(&token_str) {
                if !is_expired(claims.exp) {
                    self.state = AuthState::LoggedInCurrent(claims);
                    return Ok(());
                }

                // Cached token is expired; try to refresh it if we have a
                // key, otherwise there's nothing more we can do with it.
                return match self.effective_api_key() {
                    Some(key) => {
                        self.try_exchange(&key, AuthState::LoggedInExpired(claims))
                            .await
                    }
                    None => {
                        self.state = AuthState::LoggedInExpired(claims.clone());
                        Err(ClientError::TokenExpired(claims))
                    }
                };
            }
        }

        // 2. No usable cached token — try exchanging an API key for one.
        match self.effective_api_key() {
            Some(key) => {
                self.state = AuthState::HasApiKey(key.clone());
                self.try_exchange(&key, AuthState::NotLoggedIn).await
            }
            None => {
                // 3. No credentials available at all.
                self.state = AuthState::NotLoggedIn;
                Err(ClientError::NotLoggedIn)
            }
        }
    }

    /// Exchanges `api_key` for a JWT via the server and parses its claims.
    /// Returns `None` on any failure: network error, non-2xx response, or an
    /// unparseable token.
    async fn exchange_for_claims(&self, api_key: &str) -> Option<TokenClaims> {
        let token = self.do_exchange(api_key).await?;
        parse_jwt_claims(&token)
    }

    /// Attempts an exchange and updates `self.state` accordingly: to the
    /// freshly obtained `LoggedInCurrent` claims on success, or to
    /// `fallback_state` (paired with `ClientError::ExchangeFailed`) on
    /// failure. Used by both exchange sites in `initialize` — refreshing an
    /// expired cached token and exchanging a key with no cached token — which
    /// differ only in what state to fall back to.
    async fn try_exchange(
        &mut self,
        api_key: &str,
        fallback_state: AuthState,
    ) -> Result<(), ClientError> {
        match self.exchange_for_claims(api_key).await {
            Some(claims) => {
                self.state = AuthState::LoggedInCurrent(claims);
                Ok(())
            }
            None => {
                self.state = fallback_state;
                Err(ClientError::ExchangeFailed)
            }
        }
    }

    /// Returns the API key from `ClientOptions`, falling back to the
    /// `PROSPEROUS_KEY` env var. If `options.prosperous_key` is explicitly
    /// set — even to an empty string — the env var is not consulted; this
    /// lets callers (and tests) force "no key" without ambient env vars
    /// leaking in.
    fn effective_api_key(&self) -> Option<String> {
        if let Some(key) = &self.options.prosperous_key {
            let trimmed = key.trim();
            return if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            };
        }
        std::env::var("PROSPEROUS_KEY")
            .ok()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty())
    }

    /// Calls `POST {base_url}/auth/exchange` with `api_key` and returns the
    /// raw JWT string from the response, or `None` on any failure.
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

/// Walks up from the current working directory to `$HOME`, looking for a
/// `.prosperous/token` file at each level. Returns the trimmed file
/// contents of the first non-empty one found, or `None` if none exists.
/// Stops at `$HOME` (inclusive) so it never searches above the user's home
/// directory.
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

/// Decodes the payload segment of a JWT and extracts `email`, `org_id`, and
/// `exp`. The signature is intentionally not verified — the client treats
/// the token as an opaque credential handed back by a trusted server, not a
/// value it needs to authenticate itself.
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

/// A token is expired once the current time reaches or passes its `exp`
/// claim. Treats a clock read failure as "already expired" so a broken
/// clock fails closed rather than treating every token as perpetually valid.
fn is_expired(exp: i64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(i64::MAX);
    now >= exp
}
