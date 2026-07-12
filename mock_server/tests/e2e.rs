use std::future::IntoFuture;

use bytes::Bytes;
use client::{AuthState, ClientError, ClientOptions, ProsperousClient, parse_jwt_claims};
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

// --- ProsperousClient round-trip ---

#[tokio::test]
async fn client_initialize_with_api_key_returns_ok() {
    let base = start_mock_server().await;
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: Some("my-test-key".to_owned()),
        base_url: Some(base),
    });

    assert!(client.initialize().await.is_ok());

    let claims = match client.state() {
        AuthState::LoggedInCurrent(c) => c,
        other => panic!("expected LoggedInCurrent, got {other:?}"),
    };
    assert_eq!(claims.email, "user@example.com");
    assert_eq!(claims.org_id, "test-org");
}

#[tokio::test]
async fn initialized_token_is_not_expired() {
    let base = start_mock_server().await;
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: Some("my-test-key".to_owned()),
        base_url: Some(base),
    });

    client.initialize().await.unwrap();

    let claims = match client.state() {
        AuthState::LoggedInCurrent(c) => c,
        other => panic!("expected LoggedInCurrent, got {other:?}"),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    assert!(claims.exp > now, "token should not be expired");
}

#[tokio::test]
async fn initialize_with_empty_key_returns_not_logged_in() {
    let base = start_mock_server().await;
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: Some("".to_owned()),
        base_url: Some(base),
    });

    assert!(matches!(client.initialize().await, Err(ClientError::NotLoggedIn)));
}

// --- Exchange-failure reasons ---

#[tokio::test]
async fn unpaid_dues_maps_to_payment_required() {
    let base = start_mock_server().await;
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: Some("unpaid-dues".to_owned()),
        base_url: Some(base),
    });

    assert!(matches!(
        client.initialize().await,
        Err(ClientError::PaymentRequired)
    ));
    assert!(matches!(client.state(), AuthState::PaymentRequired));
}

#[tokio::test]
async fn rejected_key_maps_to_invalid_api_key() {
    let base = start_mock_server().await;
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: Some("invalid-key".to_owned()),
        base_url: Some(base),
    });

    assert!(matches!(
        client.initialize().await,
        Err(ClientError::InvalidApiKey)
    ));
    assert!(matches!(client.state(), AuthState::InvalidApiKey));
}

#[tokio::test]
async fn server_5xx_maps_to_unknown_server_error() {
    let base = start_mock_server().await;
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: Some("server-error".to_owned()),
        base_url: Some(base),
    });

    assert!(matches!(
        client.initialize().await,
        Err(ClientError::UnknownServerError)
    ));
    assert!(matches!(client.state(), AuthState::UnknownServerError));
}

#[tokio::test]
async fn no_reachable_server_maps_to_server_unreachable() {
    // Port 1 is not listening, so the connection is refused before any HTTP
    // response comes back.
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: Some("any-key".to_owned()),
        base_url: Some("http://127.0.0.1:1".to_owned()),
    });

    assert!(matches!(
        client.initialize().await,
        Err(ClientError::ServerUnreachable)
    ));
    assert!(matches!(client.state(), AuthState::ServerUnreachable));
}

#[tokio::test]
async fn parse_jwt_claims_round_trips_server_token() {
    let base = start_mock_server().await;
    let (_, body) = post_json(
        &format!("{base}/auth/exchange"),
        serde_json::json!({"api_key": "any-key"}),
    )
    .await;

    let token = body["token"].as_str().unwrap();
    let claims = parse_jwt_claims(token).expect("token from server should be parseable");
    assert_eq!(claims.email, "user@example.com");
    assert_eq!(claims.org_id, "test-org");
}
