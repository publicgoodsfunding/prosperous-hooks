# Architecture

## Client state machine

`ProsperousClient` (`client/src/lib.rs`) tracks authentication as an `AuthState` enum:

```rust
enum AuthState {
    NotLoggedIn,
    HasApiKey(String),
    LoggedInCurrent(TokenClaims),
    LoggedInExpired(TokenClaims),
}
```

| State | Meaning |
|---|---|
| `NotLoggedIn` | No credentials found; no token file, no API key. |
| `HasApiKey(key)` | An API key is available and is being exchanged for a JWT. |
| `LoggedInCurrent(claims)` | A valid, non-expired JWT is cached. |
| `LoggedInExpired(claims)` | A cached JWT was found but its `exp` claim has passed. |

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
│  │     ├─ have API key? ── no ───────────────► LoggedInExpired(claims)      [Err: TokenExpired]
│  │     │
│  │     └─ yes → POST /auth/exchange
│  │        ├─ 2xx + valid JWT ─────────────────► LoggedInCurrent(new_claims) [Ok]
│  │        └─ otherwise ───────────────────────► LoggedInExpired(claims)     [Err: ExchangeFailed]
│  │
│  └─ no
│     │
│     ├─ have API key? ── no ───────────────────► NotLoggedIn                 [Err: NotLoggedIn]
│     │
│     └─ yes → state = HasApiKey(key), then POST /auth/exchange
│        ├─ 2xx + valid JWT ────────────────────► LoggedInCurrent(claims)     [Ok]
│        └─ otherwise ──────────────────────────► NotLoggedIn                 [Err: ExchangeFailed]
```

Transition rules, in the order `initialize()` evaluates them:

1. **Token file found, not expired** → `LoggedInCurrent(claims)`, `Ok(())`.
2. **Token file found, expired, API key available** → attempt `POST /auth/exchange`.
   - Success (2xx, parseable JWT) → `LoggedInCurrent(new_claims)`, `Ok(())`.
   - Failure → `LoggedInExpired(claims)`, `Err(ClientError::ExchangeFailed)`.
3. **Token file found, expired, no API key** → `LoggedInExpired(claims)`, `Err(ClientError::TokenExpired(claims))`.
4. **No token file, API key available** → state set to `HasApiKey(key)`, then attempt `POST /auth/exchange`.
   - Success → `LoggedInCurrent(claims)`, `Ok(())`.
   - Failure → `NotLoggedIn`, `Err(ClientError::ExchangeFailed)`.
5. **No token file, no API key** → `NotLoggedIn`, `Err(ClientError::NotLoggedIn)`.

The API key is resolved by `effective_api_key()`: `ClientOptions.prosperous_key` takes precedence when explicitly set (including an explicit empty string, which is treated as "no key" and does **not** fall back to the environment); otherwise the `PROSPEROUS_KEY` environment variable is consulted.

### Auth resolution order

1. Looks for a cached JWT in `.prosperous/token`, walking up from the current directory to `$HOME`.
2. If no token file is found, exchanges `PROSPEROUS_KEY` / `--prosperous-key` for a JWT via `POST /auth/exchange` against `PROSPEROUS_BASE_URL` / `--base-url`.
3. Prints `Error: not logged in` and exits 1 if neither credential is available.

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
