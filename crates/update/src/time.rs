//! Time and calendar conversions for release timestamps.
//!
//! Calendar conversion goes through chrono, in accordance with the AGENTS.md shared standard.

use chrono::DateTime;

/// Seconds since the Unix epoch for an RFC 3339 UTC timestamp such as `2026-10-08T14:52:50Z`.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text.trim()).ok().map(|dt| dt.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_github_timestamps() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-08T14:52:50Z"), Some(1_791_471_170));
        assert_eq!(parse_rfc3339("2026-10-08T16:52:50+02:00"), Some(1_791_471_170));
        assert_eq!(parse_rfc3339("2026-10-08T14:52:50.123Z"), Some(1_791_471_170));
        for bad in ["", "2026-10-08", "2026-13-08T00:00:00Z", "2026-10-08T14:52:50Q", "20x6-10-08T14:52:50Z"] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }
}
