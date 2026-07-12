//! Operating-system language detection for the CLI's user-facing text.
//!
//! Translated strings themselves live in `client/locales/*.yml` and are
//! looked up through the `rust_i18n::t!` macro (see `main.rs`); this module
//! is only responsible for picking *which* of those locales to use, based on
//! the OS's configured language.

use sys_locale::get_locale;

/// Locales this crate ships translations for. Anything else detected from
/// the OS falls back to English — `rust_i18n`'s own `fallback = "en"`
/// (configured alongside its `i18n!` invocation) would catch missing
/// *individual keys* in a supported locale, but an unsupported locale must
/// be rejected here before it's ever passed to `rust_i18n::set_locale`.
pub const SUPPORTED_LOCALES: &[&str] = &["en", "zh", "es", "ar", "hi"];

const DEFAULT_LOCALE: &str = "en";

/// Extracts the primary language subtag from an OS locale string, e.g.
/// `"zh-Hans-CN"` -> `"zh"`, `"en_US"` -> `"en"`. OS locale strings are
/// `-` or `_` separated depending on platform; we only ever care about the
/// leading language component, never the region/script that follows it.
fn primary_language_subtag(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

/// Maps a raw OS locale (as returned by `sys_locale::get_locale`) down to one
/// of `SUPPORTED_LOCALES`, defaulting to English when there's no OS locale to
/// read or when its language isn't one we have translations for. Kept
/// separate from `detect_system_locale` so the mapping logic can be unit
/// tested without depending on the actual host OS.
fn pick_supported(os_locale: Option<String>) -> String {
    os_locale
        .map(|locale| primary_language_subtag(&locale))
        .filter(|lang| SUPPORTED_LOCALES.contains(&lang.as_str()))
        .unwrap_or_else(|| DEFAULT_LOCALE.to_owned())
}

/// Reads the current OS locale and maps it to one of `SUPPORTED_LOCALES`,
/// defaulting to English.
pub fn detect_system_locale() -> String {
    pick_supported(get_locale())
}

/// Detects the OS locale and makes it the active locale for `rust_i18n::t!`
/// lookups everywhere in the process (locale state is a single global inside
/// the `rust_i18n` crate, shared across every crate that depends on it — so
/// calling this once here in the library is enough for `main.rs` to pick it
/// up too). Returns the locale that was applied, mainly so callers can log
/// or test it.
pub fn init_locale() -> String {
    let locale = detect_system_locale();
    rust_i18n::set_locale(&locale);
    locale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_region_and_script_qualified_locales_to_their_language() {
        assert_eq!(pick_supported(Some("zh-Hans-CN".to_owned())), "zh");
        assert_eq!(pick_supported(Some("es-ES".to_owned())), "es");
        assert_eq!(pick_supported(Some("ar-SA".to_owned())), "ar");
        assert_eq!(pick_supported(Some("hi-IN".to_owned())), "hi");
        assert_eq!(pick_supported(Some("en_US".to_owned())), "en");
    }

    #[test]
    fn falls_back_to_english_for_unsupported_or_missing_locale() {
        assert_eq!(pick_supported(Some("fr-FR".to_owned())), "en");
        assert_eq!(pick_supported(Some("".to_owned())), "en");
        assert_eq!(pick_supported(None), "en");
    }
}
