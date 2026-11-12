//! Tests of the `crypt` feature, linked into one test binary. Not built for the web: proptest
//! does not build there.
#![cfg(all(feature = "crypt", not(target_arch = "wasm32")))]

mod permissions_revision;
mod proptests;
mod qpdf_literals;
