#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The fixture catalogs as an app's crate sees them; read by `craft-i18n-build`'s tests.

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));
