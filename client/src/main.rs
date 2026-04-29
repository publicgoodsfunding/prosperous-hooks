use client::{AuthState, ClientError, ClientOptions, ProsperousClient};

#[tokio::main]
async fn main() {
    let options = parse_options();
    let mut client = ProsperousClient::new(options);

    match client.initialize().await {
        Ok(()) => {
            if let AuthState::LoggedInCurrent(claims) = client.state() {
                println!("Logged in as {} (org: {})", claims.email, claims.org_id);
            }
        }
        Err(ClientError::NotLoggedIn) => {
            eprintln!("Error: not logged in. Provide --prosperous-key or set PROSPEROUS_KEY.");
            std::process::exit(1);
        }
        Err(ClientError::TokenExpired(claims)) => {
            eprintln!(
                "Error: token expired for {}. Provide --prosperous-key to reauthenticate.",
                claims.email
            );
            std::process::exit(1);
        }
        Err(ClientError::ExchangeFailed) => {
            eprintln!("Error: API key exchange failed. Check your key and --base-url.");
            std::process::exit(1);
        }
    }
}

fn parse_options() -> ClientOptions {
    let args: Vec<String> = std::env::args().collect();
    let mut prosperous_key: Option<String> = None;
    let mut base_url: Option<String> = None;

    for arg in &args[1..] {
        if let Some(v) = arg.strip_prefix("--prosperous-key=") {
            prosperous_key = Some(v.to_owned());
        } else if let Some(v) = arg.strip_prefix("--base-url=") {
            base_url = Some(v.to_owned());
        }
    }

    // Fall back to environment variables when flags are not provided.
    if prosperous_key.is_none() {
        prosperous_key = std::env::var("PROSPEROUS_KEY")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
    }
    if base_url.is_none() {
        base_url = std::env::var("PROSPEROUS_BASE_URL")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
    }

    ClientOptions {
        prosperous_key,
        base_url,
    }
}
