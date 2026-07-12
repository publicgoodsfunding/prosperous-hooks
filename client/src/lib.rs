mod client;
mod locale;

// Embed `client/locales/*.yml` for this (library) crate so the `t!` macro
// used inside `client.rs` (e.g. the interactive login prompts) can resolve
// keys. The binary crate (`main.rs`) has its own `i18n!` invocation; the
// active locale is a process-wide global in `rust_i18n`, so a single
// `set_locale` call (see `locale::init_locale`) applies to both.
rust_i18n::i18n!("locales", fallback = "en");

pub use client::{
    parse_jwt_claims, AuthState, ClientError, ClientOptions, ProsperousClient, TokenClaims,
    DEFAULT_REVENUE_THRESHOLD, DEFAULT_REVSHARE_PERCENTAGE,
};
pub use locale::{detect_system_locale, init_locale, SUPPORTED_LOCALES};
