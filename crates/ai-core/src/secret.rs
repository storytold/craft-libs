//! `SecretString`: a secret value that cannot be printed or serialized by accident.
//!
//! The data model stores only [`crate::SecretRef`] (a locator). The value itself appears only
//! at the moment a provider needs it, wrapped in this type. It has no `Serialize` or
//! `Deserialize` impl, so putting one in a settings struct is a compile error, and `Debug` and
//! `Display` print a placeholder. [`redact`] scrubs a known secret out of text such as a provider
//! error message before it is shown or logged.

use std::fmt;

/// A secret value (API key, token). Clone is allowed; printing and serializing are not.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value. Call this only when building the outgoing request.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Replaces every occurrence of each non-empty secret in `text` with `[redacted]`.
pub fn redact(text: &str, secrets: &[&SecretString]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            out = out.replace(secret.expose(), "[redacted]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_show_the_value() {
        let s = SecretString::new("sk-live-abc123");
        assert!(!format!("{s:?}").contains("abc123"));
        assert!(!format!("{s}").contains("abc123"));
        assert!(!format!("{:?}", Some(&s)).contains("abc123"));
    }

    #[test]
    fn redact_scrubs_every_occurrence() {
        let s = SecretString::new("sk-live-abc123");
        let out = redact("bad key sk-live-abc123 (sk-live-abc123)", &[&s]);
        assert_eq!(out, "bad key [redacted] ([redacted])");
    }

    #[test]
    fn redact_ignores_empty_secret() {
        let s = SecretString::new("");
        assert_eq!(redact("hello", &[&s]), "hello");
    }
}
