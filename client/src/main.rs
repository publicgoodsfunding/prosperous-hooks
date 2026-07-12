use clap::Parser;
use client::{AuthState, ClientError, ClientOptions, ProsperousClient};
use rust_i18n::t;

// Embeds `client/locales/*.yml` into this binary so `t!` below can look
// strings up by key. Must be invoked here too (not just in the library) --
// `t!` expands to a call rooted at `crate::`, so each crate that uses it
// needs its own `i18n!` invocation, even though the locale files and the
// active-locale state are shared with the `client` library.
rust_i18n::i18n!("locales", fallback = "en");

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
    // Pick a display language from the OS locale before printing anything,
    // so every message below -- success or error -- comes out localized.
    client::init_locale();

    let args = Args::parse();
    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key: args.prosperous_key,
        base_url: args.base_url,
    });

    match client.initialize().await {
        Ok(()) => {
            if let AuthState::LoggedInCurrent(claims) = client.state() {
                println!(
                    "{}",
                    t!("logged_in_as", email = claims.email, org_id = claims.org_id)
                );
            }
        }
        Err(ClientError::NotLoggedIn) => {
            eprintln!("{}", t!("error_not_logged_in"));
            std::process::exit(1);
        }
        Err(ClientError::TokenExpired(claims)) => {
            eprintln!("{}", t!("error_token_expired", email = claims.email));
            std::process::exit(1);
        }
        Err(ClientError::ExchangeFailed) => {
            eprintln!("{}", t!("error_exchange_failed"));
            std::process::exit(1);
        }
    }
}
