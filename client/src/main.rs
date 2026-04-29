use client::{resolve_auth_state, AuthState};
use http_body_util::BodyExt;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

#[tokio::main]
async fn main() {
    match resolve_auth_state() {
        AuthState::NotLoggedIn => eprintln!("Auth: not logged in"),
        AuthState::HasApiKey(_) => println!("Auth: API key found, token exchange required"),
        AuthState::LoggedInCurrent(c) => {
            println!("Auth: logged in as {} (org: {})", c.email, c.org_id)
        }
        AuthState::LoggedInExpired(c) => {
            eprintln!("Auth: token expired for {} (org: {})", c.email, c.org_id)
        }
    }

    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:3000/".to_string());

    let uri: hyper::Uri = url.parse().expect("invalid URL");

    let client = Client::builder(TokioExecutor::new()).build_http();

    let req = Request::builder()
        .uri(&uri)
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .unwrap();

    let res = client.request(req).await.expect("request failed");

    println!("Status: {}", res.status());

    let body = res.into_body().collect().await.expect("failed to read body");
    let bytes = body.to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    println!("Body: {body}");
}
