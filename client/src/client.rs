use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use rust_i18n::t;

/// How many times the interactive login will accept a fresh API key paste
/// before giving up. Only *invalid* keys consume an attempt-and-retry; the
/// other failure reasons abort immediately (retyping the key won't fix them).
const MAX_LOGIN_ATTEMPTS: usize = 3;

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
/// cached on disk, an API key, an interactively pasted key, or neither.
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
    /// not be refreshed (no API key was available and no interactive login
    /// was possible).
    LoggedInExpired(TokenClaims),

    // The four states below all describe a *failed API key exchange*. The
    // key made it to the server, but the server declined to issue a JWT.
    // They are distinguished because the CLI reacts differently to each
    // (see `ProsperousClient::initialize` and the interactive login loop).
    /// The API key itself was accepted, but the account has outstanding dues:
    /// the server refuses to issue a token until payment is settled. Signalled
    /// by an HTTP `402 Payment Required` from the exchange endpoint.
    PaymentRequired,
    /// The server rejected the API key as unusable — deleted, expired, or
    /// simply wrong. Signalled by an HTTP `401`/`403`. This is the only
    /// exchange failure the interactive login retries, since pasting a
    /// different key can resolve it.
    InvalidApiKey,
    /// The server could not be reached at all: no base URL configured, an
    /// unparseable URL, or a transport-level failure (DNS, connection
    /// refused, TLS, timeout). We never received an HTTP response.
    ServerUnreachable,
    /// The server was reachable and returned a response, but not one we can
    /// act on: an unexpected status code, or a `2xx` whose body wasn't a
    /// parseable token. Treated as a transient server-side problem rather
    /// than a credential problem.
    UnknownServerError,
}

/// The error returned by `ProsperousClient::initialize` when it does not
/// land in `AuthState::LoggedInCurrent`. Each variant mirrors the terminal
/// `AuthState` it accompanies so callers can report a useful message.
#[derive(Debug)]
pub enum ClientError {
    /// No cached token and no API key were available, and no key could be
    /// obtained interactively (non-interactive context, or the user provided
    /// nothing).
    NotLoggedIn,
    /// A cached token was found but has expired, no API key was available to
    /// refresh it, and no interactive login was possible.
    TokenExpired(TokenClaims),
    /// The API key was valid but the account owes dues; see
    /// `AuthState::PaymentRequired`.
    PaymentRequired,
    /// The API key was rejected as unusable; see `AuthState::InvalidApiKey`.
    InvalidApiKey,
    /// The server could not be reached; see `AuthState::ServerUnreachable`.
    ServerUnreachable,
    /// The server returned an unusable response; see
    /// `AuthState::UnknownServerError`.
    UnknownServerError,
}

/// The result of a single API key → JWT exchange attempt. Kept private: it's
/// the common currency between `do_exchange` (which classifies the server's
/// response) and the callers that turn a failure into the matching
/// `AuthState`/`ClientError` pair (`classify_failure`).
enum ExchangeOutcome {
    /// The server issued a token and its claims parsed cleanly.
    Success(TokenClaims),
    /// Key valid, but dues are owed (HTTP `402`).
    PaymentRequired,
    /// Key rejected as unusable (HTTP `401`/`403`).
    InvalidApiKey,
    /// No response from the server at all.
    ServerUnreachable,
    /// A response arrived but couldn't be used.
    UnknownServerError,
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
    /// Whether `initialize` may fall back to an interactive login (prompting
    /// the user to paste an API key) when no other credentials are available.
    /// Defaults to `true` (see the `Default` impl). Set it to `false` to keep
    /// initialization strictly non-interactive — the client then reports
    /// `NotLoggedIn`/`TokenExpired` instead of prompting, even on a terminal.
    /// (Interactive login additionally requires stdin to be a terminal, so
    /// leaving this `true` is still safe for piped/CI contexts.)
    pub interactive: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        ClientOptions {
            prosperous_key: None,
            base_url: None,
            interactive: true,
        }
    }
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
    /// token on disk, then an API key exchange, then — as a last resort when
    /// no credentials are available — an interactive login that prompts the
    /// user to paste a freshly generated API key. Updates `self.state` to
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

                // Cached token is expired. Refresh it with an API key if we
                // have one; otherwise fall back to an interactive login,
                // degrading to `LoggedInExpired` when we can't prompt.
                return match self.effective_api_key() {
                    Some(key) => self.try_exchange(&key).await,
                    None => {
                        self.login_or_fail(
                            AuthState::LoggedInExpired(claims.clone()),
                            ClientError::TokenExpired(claims),
                        )
                        .await
                    }
                };
            }
        }

        // 2. No usable cached token — try exchanging an API key for one.
        if let Some(key) = self.effective_api_key() {
            return self.try_exchange(&key).await;
        }

        // 3. No credentials available at all: walk the user through logging
        // in and pasting a key, degrading to `NotLoggedIn` when we can't
        // prompt (piped input, CI, or an embedded non-CLI caller).
        self.login_or_fail(AuthState::NotLoggedIn, ClientError::NotLoggedIn)
            .await
    }

    /// Exchanges `api_key` for a JWT (setting the transient `HasApiKey`
    /// state first) and records the outcome: `LoggedInCurrent` on success, or
    /// the specific failure state paired with its `ClientError` otherwise.
    /// Used for keys supplied non-interactively (env var / flag / cached
    /// token refresh), which get a single attempt with no re-prompt.
    async fn try_exchange(&mut self, api_key: &str) -> Result<(), ClientError> {
        self.state = AuthState::HasApiKey(api_key.to_owned());
        match self.do_exchange(api_key).await {
            ExchangeOutcome::Success(claims) => {
                self.state = AuthState::LoggedInCurrent(claims);
                Ok(())
            }
            failure => {
                let (state, error) = classify_failure(failure);
                self.state = state;
                Err(error)
            }
        }
    }

    /// Runs the interactive login only when it's both enabled
    /// (`ClientOptions.interactive`) and possible (stdin is a terminal);
    /// otherwise records `fallback_state`/`fallback_err` and returns without
    /// prompting. The `interactive` opt-out lets a caller force strictly
    /// non-interactive behavior, while the terminal check keeps the library
    /// usable from `node_client` and piped/CI contexts — in both cases
    /// initialization stays non-blocking and simply reports that no
    /// credentials were found.
    async fn login_or_fail(
        &mut self,
        fallback_state: AuthState,
        fallback_err: ClientError,
    ) -> Result<(), ClientError> {
        if self.options.interactive && stdin_is_interactive() {
            self.interactive_login().await
        } else {
            self.state = fallback_state;
            Err(fallback_err)
        }
    }

    /// Prompts the user to log in to the server, generate an API key, and
    /// paste it back, then exchanges it. Re-prompts (up to
    /// `MAX_LOGIN_ATTEMPTS`) only when the pasted key is rejected as invalid —
    /// the other failure reasons (dues owed, server unreachable, unknown
    /// error) can't be fixed by retyping, so they abort immediately.
    async fn interactive_login(&mut self) -> Result<(), ClientError> {
        // Show the "how to log in" instructions once, naming the server when
        // we know it so the user knows where to go.
        let server = self
            .options
            .base_url
            .clone()
            .unwrap_or_else(|| t!("login_server_generic").into_owned());
        eprintln!("{}", t!("login_intro", url = server));

        for attempt in 1..=MAX_LOGIN_ATTEMPTS {
            eprint!("{}", t!("login_prompt"));
            let _ = io::stderr().flush();

            let key = match read_line_from_stdin() {
                // EOF (e.g. the user hit Ctrl-D): stop asking.
                None => break,
                // Blank line: re-prompt without spending the attempt on an
                // empty key we know can't succeed.
                Some(key) if key.is_empty() => continue,
                Some(key) => key,
            };

            self.state = AuthState::HasApiKey(key.clone());
            match self.do_exchange(&key).await {
                ExchangeOutcome::Success(claims) => {
                    self.state = AuthState::LoggedInCurrent(claims);
                    return Ok(());
                }
                // Only an invalid key is worth another paste.
                ExchangeOutcome::InvalidApiKey => {
                    self.state = AuthState::InvalidApiKey;
                    if attempt < MAX_LOGIN_ATTEMPTS {
                        eprintln!("{}", t!("login_invalid_retry"));
                    }
                }
                // Dues owed / unreachable / unknown: report and stop.
                failure => {
                    let (state, error) = classify_failure(failure);
                    self.state = state;
                    return Err(error);
                }
            }
        }

        // Every attempt was rejected as invalid, or we hit EOF before any key
        // was accepted. Preserve the last invalid-key state if that's how we
        // got here; otherwise there was effectively no credential at all.
        match self.state {
            AuthState::InvalidApiKey => Err(ClientError::InvalidApiKey),
            _ => {
                self.state = AuthState::NotLoggedIn;
                Err(ClientError::NotLoggedIn)
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

    /// Calls `POST {base_url}/auth/exchange` with `api_key` and classifies the
    /// server's response into an `ExchangeOutcome`. The classification is the
    /// heart of the failure-reason distinction:
    ///
    /// - no base URL / unparseable URL / transport error → `ServerUnreachable`
    /// - HTTP `402` → `PaymentRequired`
    /// - HTTP `401`/`403` → `InvalidApiKey`
    /// - any other non-`2xx` → `UnknownServerError`
    /// - `2xx` with a parseable JWT → `Success`; `2xx` otherwise →
    ///   `UnknownServerError`
    async fn do_exchange(&self, api_key: &str) -> ExchangeOutcome {
        // Without a base URL there's nowhere to send the request.
        let Some(base_url) = self.options.base_url.as_deref() else {
            return ExchangeOutcome::ServerUnreachable;
        };
        let base_url = base_url.trim_end_matches('/');
        let url = format!("{base_url}/auth/exchange");
        let Ok(uri) = url.parse::<hyper::Uri>() else {
            return ExchangeOutcome::ServerUnreachable;
        };

        let body = serde_json::json!({ "api_key": api_key }).to_string();
        let request = Request::builder()
            .method("POST")
            .uri(&uri)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body)));
        let Ok(request) = request else {
            // Building the request only fails on inputs we control, so this
            // is an internal hiccup rather than a network condition.
            return ExchangeOutcome::UnknownServerError;
        };

        // A transport-level failure means we never got an answer at all.
        let response = match Client::builder(TokioExecutor::new())
            .build_http()
            .request(request)
            .await
        {
            Ok(response) => response,
            Err(_) => return ExchangeOutcome::ServerUnreachable,
        };

        // Map the HTTP status to a specific outcome before reading the body.
        match response.status() {
            StatusCode::PAYMENT_REQUIRED => return ExchangeOutcome::PaymentRequired,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return ExchangeOutcome::InvalidApiKey
            }
            status if !status.is_success() => return ExchangeOutcome::UnknownServerError,
            _ => {}
        }

        // 2xx: the body should carry a parseable JWT. Anything else here is
        // the server misbehaving rather than a credential problem.
        let Ok(collected) = response.into_body().collect().await else {
            return ExchangeOutcome::UnknownServerError;
        };
        let bytes = collected.to_bytes();
        let token = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|json| json["token"].as_str().map(str::to_owned));
        match token.as_deref().and_then(parse_jwt_claims) {
            Some(claims) => ExchangeOutcome::Success(claims),
            None => ExchangeOutcome::UnknownServerError,
        }
    }
}

/// Maps a *non-success* `ExchangeOutcome` to the `(AuthState, ClientError)`
/// pair that describes it. Both the non-interactive `try_exchange` and the
/// interactive login funnel their failure handling through here so the
/// outcome → state → error mapping lives in exactly one place.
///
/// Panics if handed `Success`; callers always handle the success case
/// themselves (they need the claims), so this is unreachable in practice.
fn classify_failure(outcome: ExchangeOutcome) -> (AuthState, ClientError) {
    match outcome {
        ExchangeOutcome::PaymentRequired => {
            (AuthState::PaymentRequired, ClientError::PaymentRequired)
        }
        ExchangeOutcome::InvalidApiKey => (AuthState::InvalidApiKey, ClientError::InvalidApiKey),
        ExchangeOutcome::ServerUnreachable => {
            (AuthState::ServerUnreachable, ClientError::ServerUnreachable)
        }
        ExchangeOutcome::UnknownServerError => {
            (AuthState::UnknownServerError, ClientError::UnknownServerError)
        }
        ExchangeOutcome::Success(_) => {
            unreachable!("classify_failure is only called with a failed exchange outcome")
        }
    }
}

/// Whether stdin is an interactive terminal. The interactive login only makes
/// sense when a human can actually type a key; in non-interactive contexts
/// (pipes, CI, or the `node_client` addon embedded in another process) we
/// skip the prompt so initialization never blocks on stdin.
fn stdin_is_interactive() -> bool {
    io::stdin().is_terminal()
}

/// Reads a single trimmed line from stdin. Returns `None` on EOF or a read
/// error (so the login loop can stop cleanly) and `Some("")` for a blank
/// line (so the loop can re-prompt rather than treating it as a key).
fn read_line_from_stdin() -> Option<String> {
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_owned()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_failure_maps_each_outcome_to_its_state_and_error() {
        assert!(matches!(
            classify_failure(ExchangeOutcome::PaymentRequired),
            (AuthState::PaymentRequired, ClientError::PaymentRequired)
        ));
        assert!(matches!(
            classify_failure(ExchangeOutcome::InvalidApiKey),
            (AuthState::InvalidApiKey, ClientError::InvalidApiKey)
        ));
        assert!(matches!(
            classify_failure(ExchangeOutcome::ServerUnreachable),
            (AuthState::ServerUnreachable, ClientError::ServerUnreachable)
        ));
        assert!(matches!(
            classify_failure(ExchangeOutcome::UnknownServerError),
            (AuthState::UnknownServerError, ClientError::UnknownServerError)
        ));
    }
}
