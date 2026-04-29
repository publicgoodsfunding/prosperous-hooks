use std::future::IntoFuture;

use bytes::Bytes;
use client::{exchange_api_key, parse_jwt_claims};
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tokio::net::TcpListener;

async fn start_mock_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, mock_server::create_app()).into_future());
    format!("http://{}", addr)
}

async fn post_json(url: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
    let uri: hyper::Uri = url.parse().unwrap();
    let req = Request::builder()
        .method("POST")
        .uri(&uri)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap();

    let res = Client::builder(TokioExecutor::new())
        .build_http()
        .request(req)
        .await
        .unwrap();

    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// --- /auth/exchange ---

#[tokio::test]
async fn api_key_exchange_returns_200_with_token() {
    let base = start_mock_server().await;
    let (status, body) = post_json(
        &format!("{base}/auth/exchange"),
        serde_json::json!({"api_key": "my-test-key"}),
    )
    .await;

    assert_eq!(status, 200);
    assert!(body["token"].as_str().is_some(), "response should contain a token");
}

#[tokio::test]
async fn api_key_exchange_rejects_empty_key() {
    let base = start_mock_server().await;
    let (status, _) = post_json(
        &format!("{base}/auth/exchange"),
        serde_json::json!({"api_key": ""}),
    )
    .await;

    assert_eq!(status, 400);
}

// --- /oauth/token ---

#[tokio::test]
async fn oauth_token_returns_200_with_token() {
    let base = start_mock_server().await;
    let (status, body) = post_json(
        &format!("{base}/oauth/token"),
        serde_json::json!({"code": "test-oauth-code"}),
    )
    .await;

    assert_eq!(status, 200);
    assert!(body["token"].as_str().is_some(), "response should contain a token");
}

#[tokio::test]
async fn oauth_token_rejects_empty_code() {
    let base = start_mock_server().await;
    let (status, _) = post_json(
        &format!("{base}/oauth/token"),
        serde_json::json!({"code": ""}),
    )
    .await;

    assert_eq!(status, 400);
}

// --- Client library round-trip ---

#[tokio::test]
async fn client_exchange_api_key_returns_parseable_jwt() {
    let base = start_mock_server().await;
    let token = exchange_api_key(&base, "my-test-key").await;
    assert!(token.is_some(), "exchange_api_key should return a token");

    let claims = parse_jwt_claims(token.as_deref().unwrap());
    assert!(claims.is_some(), "token should be a valid JWT with claims");

    let claims = claims.unwrap();
    assert_eq!(claims.email, "user@example.com");
    assert_eq!(claims.org_id, "test-org");
}

#[tokio::test]
async fn exchanged_token_is_not_expired() {
    let base = start_mock_server().await;
    let token = exchange_api_key(&base, "my-test-key").await.unwrap();
    let claims = parse_jwt_claims(&token).unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    assert!(claims.exp > now, "token should not be expired");
}

#[tokio::test]
async fn client_exchange_with_empty_key_returns_none() {
    let base = start_mock_server().await;
    let token = exchange_api_key(&base, "").await;
    assert!(token.is_none(), "empty api key should not return a token");
}
