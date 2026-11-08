//! Shared test helpers and ffmpeg oracle discovery for Craft media codecs.
//! Unifies with and re-exports [`craft_media_testkit`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_media_testkit::*;

/// Report a skipped test with a context prefix.
pub fn skip_test(what: &str, reason: &str) {
    let _: Option<()> = craft_media_testkit::skip(&format!("{what}: {reason}"));
}
