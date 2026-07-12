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

### Auth resolution order

1. Looks for a cached JWT in `.prosperous/token`, walking up from the current directory to `$HOME`.
2. If no token file is found, exchanges `PROSPEROUS_KEY` / `--prosperous-key` for a JWT via `POST /auth/exchange` against `PROSPEROUS_BASE_URL` / `--base-url`.
3. Prints `Error: not logged in` and exits 1 if neither credential is available.

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
  // err.code: 'NotLoggedIn' | 'TokenExpired' | 'ExchangeFailed'
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

## JWT token format

Tokens issued by the mock server (and expected by the client) are HS256 JWTs with the following claims:

```json
{
  "email":  "user@example.com",
  "org_id": "test-org",
  "exp":    1234567890
}
```

To cache a token manually, write it to `.prosperous/token` in any ancestor directory up to `$HOME`:

```bash
mkdir -p .prosperous
echo "<jwt>" > .prosperous/token
```
