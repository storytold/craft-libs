//! Shared test helpers and ffmpeg oracle discovery for Craft media codecs.
//! Unifies with and re-exports [`craft_media_testkit`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_media_testkit::*;

/// Report a skipped test.
pub fn skip(what: &str, reason: &str) {
    eprintln!("SKIPPED ({what}): {reason}");
}
