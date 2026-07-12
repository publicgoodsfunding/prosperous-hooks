# Architecture

## Client state machine

`ProsperousClient` (`client/src/lib.rs`) tracks authentication as an `AuthState` enum:

```rust
enum AuthState {
    NotLoggedIn,
    HasApiKey(String),
    LoggedInCurrent(TokenClaims),
    LoggedInExpired(TokenClaims),
    // API key exchange failed — one variant per distinguishable reason:
    PaymentRequired,
    InvalidApiKey,
    ServerUnreachable,
    UnknownServerError,
}
```

| State | Meaning |
|---|---|
| `NotLoggedIn` | No credentials found; no token file, no API key, no key obtained interactively. |
| `HasApiKey(key)` | An API key is available and is being exchanged for a JWT. |
| `LoggedInCurrent(claims)` | A valid, non-expired JWT is cached. |
| `LoggedInExpired(claims)` | A cached JWT was found but its `exp` claim has passed, and it could not be refreshed. |
| `PaymentRequired` | The API key is valid, but the account owes dues; the server refused to issue a token (HTTP `402`). |
| `InvalidApiKey` | The server rejected the API key as deleted/expired/wrong (HTTP `401`/`403`). |
| `ServerUnreachable` | No HTTP response at all — missing/unparseable base URL, or a transport failure (DNS, connection refused, timeout). |
| `UnknownServerError` | A response arrived but was unusable — unexpected status, or a `2xx` whose body wasn't a parseable token. |

The four exchange-failure states all describe a request that reached the server but did not yield a JWT. They are kept distinct because the CLI reacts differently to each (see the interactive login below), and only `InvalidApiKey` is worth retrying with a different key.

`initialize()` starts from `NotLoggedIn` and drives the transitions below, evaluated in order:

```
start: NotLoggedIn
│
├─ token file found in .prosperous/token?
│  │
│  ├─ yes → parse claims
│  │  │
│  │  ├─ not expired ──────────────────────────► LoggedInCurrent(claims)      [Ok]
│  │  │
│  │  └─ expired
│  │     │
│  │     ├─ have API key? ── yes → exchange (see below)
│  │     │
│  │     └─ no → interactive login (see below), falling back to
│  │            ────────────────────────────────► LoggedInExpired(claims)     [Err: TokenExpired] when non-interactive
│  │
│  └─ no
│     │
│     ├─ have API key? ── yes → exchange (see below)
│     │
│     └─ no → interactive login (see below), falling back to
│            ────────────────────────────────────► NotLoggedIn                [Err: NotLoggedIn] when non-interactive
│
exchange: state = HasApiKey(key), then POST /auth/exchange, classified by response:
│  ├─ 2xx + parseable JWT ─────────────────────► LoggedInCurrent(claims)      [Ok]
│  ├─ 402 Payment Required ────────────────────► PaymentRequired              [Err: PaymentRequired]
│  ├─ 401 / 403 ───────────────────────────────► InvalidApiKey                [Err: InvalidApiKey]
│  ├─ no response (transport error/bad URL) ───► ServerUnreachable            [Err: ServerUnreachable]
│  └─ any other response ──────────────────────► UnknownServerError           [Err: UnknownServerError]
│
interactive login (only when the `interactive` option is set AND stdin is a
terminal): prompt the user to log in and paste a freshly generated API key,
then run the same exchange classification:
   ├─ success ────────────────────────────────► LoggedInCurrent(claims)      [Ok]
   ├─ InvalidApiKey ──► re-prompt, up to 3 attempts, then ─► [Err: InvalidApiKey]
   └─ PaymentRequired / ServerUnreachable / UnknownServerError ─► report immediately (no retry)
```

Transition rules, in the order `initialize()` evaluates them:

1. **Token file found, not expired** → `LoggedInCurrent(claims)`, `Ok(())`.
2. **Token file found, expired, API key available** → attempt exchange (see the exchange classification above).
3. **Token file found, expired, no API key** → interactive login; if stdin is not a terminal, degrade to `LoggedInExpired(claims)`, `Err(ClientError::TokenExpired(claims))`.
4. **No token file, API key available** → attempt exchange.
5. **No token file, no API key** → interactive login; if stdin is not a terminal, degrade to `NotLoggedIn`, `Err(ClientError::NotLoggedIn)`.

The exchange (used by both rules 2/4 and the interactive login) classifies the server's response into one of the outcomes shown above; each maps to the matching `AuthState` and `ClientError` variant.

### Interactive login fallback

When no valid token and no API key are available, and stdin is an interactive terminal, `initialize()` walks the user through logging in: it prints a welcome message explaining the Prosperous Software movement and how to register, then reads a pasted API key from stdin and exchanges it. A pasted key that comes back `InvalidApiKey` is re-prompted (up to 3 attempts total); the other failure reasons abort immediately, since retyping the key cannot fix dues owed, an unreachable server, or an unknown server error.

The welcome message quotes two configurable figures — `ClientOptions.revenue_threshold` (CLI `--revenue-threshold` / `PROSPEROUS_REVENUE_THRESHOLD`, default `1_000_000`) and `ClientOptions.revshare_percentage` (CLI `--revshare-percentage` / `PROSPEROUS_REVSHARE_PERCENTAGE`, default `1.0`) — describing the revenue above which a company is asked to contribute, and the share requested. They only affect the displayed text, not the auth logic.

The prompt is guarded by two conditions: the `ClientOptions.interactive` flag (default `true`; exposed on the CLI as `--interactive <bool>` / `PROSPEROUS_INTERACTIVE`) must be set, **and** `stdin().is_terminal()` must be true. Setting `interactive` to `false` forces strictly non-interactive behavior even on a terminal; the terminal check independently covers piped input, CI, and the `node_client` addon (which sets `interactive: false` outright). When either condition fails the login is skipped, `initialize()` never blocks on stdin, and the client reports the plain `NotLoggedIn` / `TokenExpired` outcome instead.

The API key is resolved by `effective_api_key()`: `ClientOptions.prosperous_key` takes precedence when explicitly set (including an explicit empty string, which is treated as "no key" and does **not** fall back to the environment); otherwise the `PROSPEROUS_KEY` environment variable is consulted. Note that a key supplied this way (env var / flag / cached-token refresh) gets a single exchange attempt with no interactive re-prompt — the paste loop only applies when no key was available up front.

### Auth resolution order

1. Looks for a cached JWT in `.prosperous/token`, walking up from the current directory to `$HOME`.
2. If no token file is found, exchanges `PROSPEROUS_KEY` / `--prosperous-key` for a JWT via `POST /auth/exchange` against `PROSPEROUS_BASE_URL` / `--base-url`.
3. If neither a token nor an API key is available, the `interactive` option is enabled (the default), and stdin is a terminal, prompts the user to log in and paste a freshly generated API key, then exchanges it.
4. Prints a localized error and exits 1 if authentication cannot be completed (not logged in, token expired, payment required, invalid key, server unreachable, or unknown server error).

---

## Protocol between client and server

The client and mock server communicate over plain HTTP with JSON bodies. The client currently only calls `POST /auth/exchange`; `POST /oauth/token` is served by the mock for future OAuth-based login flows and shares the same request/response shape.

### `POST /auth/exchange`

Exchanges a long-lived API key for a short-lived JWT.

**Request**

```json
{ "api_key": "my-api-key" }
```

**Response — `200 OK`**

```json
{ "token": "<JWT>" }
```

**Response — `400 Bad Request`** (empty/missing `api_key`)

```json
{ "error": "api_key is required" }
```

**Other responses.** The client distinguishes exchange-failure reasons by HTTP status: `402` (dues owed) → `PaymentRequired`, `401`/`403` (bad key) → `InvalidApiKey`, any other non-`2xx` → `UnknownServerError`, and a transport failure with no response → `ServerUnreachable`. The mock server drives these paths via sentinel API keys — `unpaid-dues` → `402`, `invalid-key` → `401`, `server-error` → `500` — while any other non-empty key is exchanged for a token.

### `POST /oauth/token`

Exchanges an OAuth authorization code for a JWT. Same response shape as `/auth/exchange`.

**Request**

```json
{ "code": "auth-code-from-oauth-redirect" }
```

**Response — `200 OK`**

```json
{ "token": "<JWT>" }
```

**Response — `400 Bad Request`** (empty/missing `code`)

```json
{ "error": "code is required" }
```

### JWT token format

Tokens issued by the mock server (and expected by the client) are HS256 JWTs with the following claims:

```json
{
  "email":  "user@example.com",
  "org_id": "test-org",
  "exp":    1234567890
}
```

The client decodes the payload (base64url) and reads `email`, `org_id`, and `exp` without verifying the signature — the mock server is a local development/test double, not a trust boundary. `exp` is a Unix timestamp (seconds); the client treats a token as expired once `now >= exp`.

To cache a token manually, write it to `.prosperous/token` in any ancestor directory up to `$HOME`:

```bash
mkdir -p .prosperous
echo "<jwt>" > .prosperous/token
```
