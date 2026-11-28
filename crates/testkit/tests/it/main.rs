//! The crate's integration tests, linked into one test binary.

#[cfg(not(target_arch = "wasm32"))]
mod bin;
mod fonts;
mod guards;
mod oracle;
mod source_limits;
