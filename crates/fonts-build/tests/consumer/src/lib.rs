#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The fixture faces as an app's crate sees them; read by `craft-fonts-build`'s tests.

use craft_fonts_build::{EmbeddedFace, ServedFace};

/// The faces the build script embedded.
pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// The faces the build script wrote for serving.
pub static CRAFT_FONTS_SERVED: &[ServedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts_served.rs"));
