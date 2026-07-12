use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

use client::{AuthState, ClientError, ClientOptions, ProsperousClient};
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

/// Runs `ProsperousClient::initialize` and returns the result as a JSON C string.
/// The caller must free the returned pointer with `prosperous_free_string`.
/// Null pointers for `key` or `base_url` are treated as absent options.
#[no_mangle]
pub extern "C" fn prosperous_initialize(
    key: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let prosperous_key = unsafe { nullable_to_option(key) };
    let base_url = unsafe { nullable_to_option(base_url) };

    let mut client = ProsperousClient::new(ClientOptions {
        prosperous_key,
        base_url,
    });

    let result = runtime().block_on(client.initialize());

    let json = match result {
        Ok(()) => match client.state() {
            AuthState::LoggedInCurrent(claims) => serde_json::json!({
                "ok": true,
                "state": "LoggedInCurrent",
                "email": claims.email,
                "orgId": claims.org_id,
                "exp": claims.exp,
            }),
            _ => serde_json::json!({"ok": true, "state": "Unknown"}),
        },
        Err(ClientError::NotLoggedIn) => serde_json::json!({
            "ok": false,
            "error": "NotLoggedIn",
        }),
        Err(ClientError::TokenExpired(claims)) => serde_json::json!({
            "ok": false,
            "error": "TokenExpired",
            "email": claims.email,
            "orgId": claims.org_id,
        }),
        Err(ClientError::PaymentRequired) => serde_json::json!({
            "ok": false,
            "error": "PaymentRequired",
        }),
        Err(ClientError::InvalidApiKey) => serde_json::json!({
            "ok": false,
            "error": "InvalidApiKey",
        }),
        Err(ClientError::ServerUnreachable) => serde_json::json!({
            "ok": false,
            "error": "ServerUnreachable",
        }),
        Err(ClientError::UnknownServerError) => serde_json::json!({
            "ok": false,
            "error": "UnknownServerError",
        }),
    };

    CString::new(json.to_string())
        .expect("JSON contained null byte")
        .into_raw()
}

/// Frees a string previously returned by `prosperous_initialize`.
#[no_mangle]
pub extern "C" fn prosperous_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

unsafe fn nullable_to_option(s: *const c_char) -> Option<String> {
    if s.is_null() {
        None
    } else {
        Some(CStr::from_ptr(s).to_string_lossy().into_owned())
    }
}
