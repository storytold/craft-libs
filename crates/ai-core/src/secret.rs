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
///
/// Matches are found in `text` itself, never in the markers already written, so the output is at
/// most a fixed multiple of the input whatever the secrets are. Overlapping matches of different
/// secrets become one marker.
pub fn redact(text: &str, secrets: &[&SecretString]) -> String {
    const MARKER: &str = "[redacted]";
    let mut values: Vec<&str> = secrets.iter().map(|s| s.expose()).filter(|v| !v.is_empty()).collect();
    values.sort_unstable();
    values.dedup();
    let mut spans: Vec<(usize, usize)> = values.iter().flat_map(|v| text.match_indices(v).map(|(at, m)| (at, at + m.len()))).collect();
    spans.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    let mut i = 0;
    while i < spans.len() {
        let (start, mut end) = spans[i];
        i += 1;
        while i < spans.len() && spans[i].0 < end {
            end = end.max(spans[i].1);
            i += 1;
        }
        out.push_str(&text[done..start]);
        out.push_str(MARKER);
        done = end;
    }
    out.push_str(&text[done..]);
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
    fn redact_never_searches_its_own_markers() {
        // Each secret matches the marker the one before it wrote; replacing in turn doubled the
        // text for every secret.
        let r = SecretString::new("r");
        let marker = SecretString::new("[redacted]");
        let secrets: Vec<&SecretString> = std::iter::once(&r).chain(std::iter::repeat_n(&marker, 64)).collect();
        assert_eq!(redact("rrrr", &secrets), "[redacted][redacted][redacted][redacted]");
    }

    #[test]
    fn redact_merges_overlapping_secrets() {
        let a = SecretString::new("abc");
        let b = SecretString::new("bcd");
        assert_eq!(redact("xabcdx", &[&a, &b]), "x[redacted]x");
        assert_eq!(redact("abcabc", &[&a]), "[redacted][redacted]");
    }

    #[test]
    fn redact_ignores_empty_secret() {
        let s = SecretString::new("");
        assert_eq!(redact("hello", &[&s]), "hello");
    }
}
