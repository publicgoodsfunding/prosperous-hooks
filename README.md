# prosperous-hooks

Hooks for prosperous software.

## Repository layout

| Crate / package | Description |
|---|---|
| `client/` | Rust library — `ProsperousClient` with auth state machine |
| `mock_server/` | Axum HTTP server — mocks OAuth and API key exchange endpoints for local development and tests |
| `node_client/` | npm package (`@prosperous/client`) — node-gyp native addon wrapping the Rust client |

---

## Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) ≥ 18
- [pnpm](https://pnpm.io/) — `npm install -g pnpm`
- A C++ compiler for node-gyp (`gcc`/`clang` on Linux/macOS, MSVC on Windows)
- Python 3 (required by node-gyp)

---

## Running the mock server

The mock server exposes two endpoints:

| Method | Path | Body | Returns |
|---|---|---|---|
| `POST` | `/auth/exchange` | `{"api_key":"…"}` | `{"token":"<JWT>"}` |
| `POST` | `/oauth/token` | `{"code":"…"}` | `{"token":"<JWT>"}` |

```bash
cargo run -p mock_server
# Listening on 0.0.0.0:3000
```

`/auth/exchange` recognizes a few **sentinel API keys** to exercise the client's failure paths: `unpaid-dues` → `402 Payment Required`, `invalid-key` → `401 Unauthorized`, `server-error` → `500`. Any other non-empty key is exchanged for a valid token.

---

## Running the Rust client

The `client` binary authenticates and prints the result. Options can be passed as flags or environment variables.

```bash
# Via environment variables
PROSPEROUS_KEY=my-api-key \
PROSPEROUS_BASE_URL=http://localhost:3000 \
cargo run -p client

# Via command-line flags
cargo run -p client -- --prosperous-key my-api-key --base-url http://localhost:3000

# See all options
cargo run -p client -- --help
```

If you have neither a cached token nor an API key, the client walks you through logging in: it prints instructions to sign in on the server and generate an API key, then waits for you to paste it. An invalid key re-prompts (up to 3 times); dues owed, an unreachable server, or an unknown server error are reported without retrying. This interactive prompt only appears when stdin is a terminal — piped/CI runs and the Node addon report `NotLoggedIn` instead of blocking.

To disable the interactive prompt entirely (even on a terminal), pass `--interactive false` or set `PROSPEROUS_INTERACTIVE=false`; the client then reports `NotLoggedIn` rather than prompting. It defaults to `true`.

The login message explains the Prosperous Software movement's contribution terms. The two figures it quotes are configurable: `--revenue-threshold <USD>` (`PROSPEROUS_REVENUE_THRESHOLD`, default `1000000`) is the annual revenue above which a company is asked to contribute, and `--revshare-percentage <PCT>` (`PROSPEROUS_REVSHARE_PERCENTAGE`, default `1`) is the share requested. Everyone under the threshold uses the software for free, and registering once covers all software in the movement.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full auth resolution order and state machine.

Output is localized based on the OS locale (checked via `LANG`/`LC_ALL` on Linux/macOS). Supported languages: English (`en`), Mandarin (`zh`), Spanish (`es`), Arabic (`ar`), Hindi (`hi`); anything else falls back to English:

```bash
LANG=es_ES.UTF-8 cargo run -p client
# Error: no se ha iniciado sesión. Proporcione --prosperous-key o defina PROSPEROUS_KEY.
```

---

## Using the Node.js client

### Build

```bash
cd node_client
pnpm install        # installs deps, compiles Rust static lib, then compiles the C++ addon
```

The install script runs `cargo build --release` followed by `node-gyp configure && node-gyp build` automatically. Subsequent Rust-only changes can be rebuilt with:

```bash
pnpm run build:rust   # recompile Rust only
pnpm run build:node   # recompile C++ addon only (links existing .a)
pnpm run build        # both
```

### API

```js
const { ProsperousClient } = require('@prosperous/client');

const client = new ProsperousClient({
  prosperousKey: process.env.PROSPEROUS_KEY,   // optional
  baseUrl: process.env.PROSPEROUS_BASE_URL,     // optional
});

try {
  const state = await client.initialize();
  // state: { type: 'LoggedInCurrent', email, orgId, exp }
  console.log(`Logged in as ${state.email} (org: ${state.orgId})`);
} catch (err) {
  // err.code: 'NotLoggedIn' | 'TokenExpired' | 'PaymentRequired'
  //         | 'InvalidApiKey' | 'ServerUnreachable' | 'UnknownServerError'
  console.error(`Auth failed [${err.code}]: ${err.message}`);
}

// Current state is also accessible after initialize() resolves:
console.log(client.state);
```

---

## Developing against the mock server

### End-to-end flow (Rust client → mock server)

```bash
# Terminal 1 — start the mock server
cargo run -p mock_server

# Terminal 2 — run the client against it
PROSPEROUS_KEY=any-key PROSPEROUS_BASE_URL=http://localhost:3000 cargo run -p client
# Logged in as user@example.com (org: test-org)
```

### End-to-end flow (Node.js client → mock server)

```bash
# Terminal 1
cargo run -p mock_server

# Terminal 2
cd node_client && node -e "
const { ProsperousClient } = require('.');
const c = new ProsperousClient({
  prosperousKey: 'any-key',
  baseUrl: 'http://localhost:3000',
});
c.initialize().then(s => console.log('Logged in:', s)).catch(console.error);
"
```

### Running the test suite

The integration tests in `mock_server/tests/e2e.rs` spin up the mock server on a random port in-process — no running server needed.

```bash
cargo test
```

---

## Further reading

See [ARCHITECTURE.md](ARCHITECTURE.md) for the client's auth state machine and the client/server protocol, including the JWT token format.
