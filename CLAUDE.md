# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Repository layout

| Crate / package | Description |
|---|---|
| `client/` | Rust library + CLI (`ProsperousClient`, auth state machine, `client.rs`) |
| `mock_server/` | Axum HTTP server mocking the OAuth / API key exchange endpoints |
| `node_client/` | npm package (`@prosperous/client`) — native addon wrapping the Rust client |

See `ARCHITECTURE.md` for the client's auth state machine and the client/server protocol.

## Common commands

```bash
cargo build --workspace
cargo test --workspace
cargo run -p mock_server      # start the mock auth server on :3000
cargo run -p client           # run the CLI against it
```

## Code style

- **Internationalization**: any user-facing text in the Rust client (CLI output, prompts, error messages shown to a human) must go through an i18n library (`rust-i18n`, via the `t!` macro and the YAML files under `client/locales/`) — never hardcode string literals for user-facing text directly in Rust source. Add new keys to **every** locale file under `client/locales/`, not just `en.yml`. Supported locales: English (`en`), Mandarin (`zh`), Spanish (`es`), Arabic (`ar`), Hindi (`hi`); anything else falls back to English. Internal/log-only text and non-CLI targets (e.g. `node_client`'s structured JSON error codes) are exempt — those aren't shown to a human as prose.
