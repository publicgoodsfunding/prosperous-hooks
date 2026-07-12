use clap::Parser;
use client::{AuthState, ClientError, ClientOptions, ProsperousClient};

#[derive(Parser)]
#[command(about = "Prosperous client")]
struct Args {
    #[arg(long, env = "PROSPEROUS_KEY")]
    prosperous_key: Option<String>,

    #[arg(long, env = "PROSPEROUS_BASE_URL")]
    base_url: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: args.prosperous_key,
        base_url: args.base_url,
    });

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
