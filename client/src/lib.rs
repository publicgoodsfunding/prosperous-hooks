use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;

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

pub fn resolve_auth_state() -> AuthState {
    if let Some(token) = find_token_file() {
        if let Some(claims) = parse_jwt_claims(&token) {
            return if is_expired(claims.exp) {
                AuthState::LoggedInExpired(claims)
            } else {
                AuthState::LoggedInCurrent(claims)
            };
        }
    }

    if let Ok(key) = std::env::var("PROSPEROUS_KEY") {
        let key = key.trim().to_owned();
        if !key.is_empty() {
            return AuthState::HasApiKey(key);
        }
    }

    AuthState::NotLoggedIn
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

fn parse_jwt_claims(token: &str) -> Option<TokenClaims> {
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
