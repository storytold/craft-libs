#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The fixture faces as an app's crate sees them; read by `craft-fonts-build`'s tests.

use craft_fonts_build::{EmbeddedFace, ServedFace};

pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

pub static CRAFT_FONTS_SERVED: &[ServedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts_served.rs"));
