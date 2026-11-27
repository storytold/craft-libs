//! Time and calendar conversions for release timestamps.
//!
//! Calendar conversion goes through chrono, in accordance with the AGENTS.md shared standard.

use chrono::DateTime;

/// Seconds since the Unix epoch for an RFC 3339 UTC timestamp such as `2026-10-08T14:52:50Z`.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text.trim()).ok().map(|dt| dt.timestamp())
}
