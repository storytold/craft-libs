//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../support/corpus.rs"]
mod corpus;
#[path = "../support/features.rs"]
mod features;
#[cfg(feature = "synth")]
#[path = "../support/text_fixture.rs"]
mod text_fixture;

#[cfg(feature = "synth")]
mod import_limits;
#[cfg(feature = "synth")]
mod outline_limits;
mod raster;
mod real_files;
#[cfg(feature = "synth")]
mod text;
