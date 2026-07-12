use axum::{
    extract::Json,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MOCK_SECRET: &[u8] = b"mock_server_test_secret_for_testing_only";

#[derive(Serialize, Deserialize)]
struct MockClaims {
    email: String,
    org_id: String,
    exp: usize,
}

fn sign_token(email: &str, org_id: &str, ttl_secs: u64) -> String {
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + ttl_secs;
    let claims = MockClaims {
        email: email.to_owned(),
        org_id: org_id.to_owned(),
        exp: exp as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(MOCK_SECRET),
    )
    .expect("JWT signing failed")
}

#[derive(Deserialize)]
struct ExchangeRequest {
    api_key: String,
}

#[derive(Deserialize)]
struct OAuthTokenRequest {
    code: String,
}

#[derive(Serialize)]
struct TokenResponse {
    token: String,
}

async fn exchange_handler(
    Json(body): Json<ExchangeRequest>,
) -> impl IntoResponse {
    if body.api_key.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "api_key is required"}))).into_response();
    }
    let token = sign_token("user@example.com", "test-org", 3600);
    (StatusCode::OK, Json(TokenResponse { token })).into_response()
}

async fn oauth_token_handler(
    Json(body): Json<OAuthTokenRequest>,
) -> impl IntoResponse {
    if body.code.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "code is required"}))).into_response();
    }
    let token = sign_token("user@example.com", "test-org", 3600);
    (StatusCode::OK, Json(TokenResponse { token })).into_response()
}

pub fn create_app() -> Router {
    Router::new()
        .route("/auth/exchange", post(exchange_handler))
        .route("/oauth/token", post(oauth_token_handler))
}
