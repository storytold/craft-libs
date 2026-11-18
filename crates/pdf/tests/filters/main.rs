//! Tests of the `filters` feature. Not built for the web: proptest does not build there.
#![cfg(all(feature = "filters", not(target_arch = "wasm32")))]

mod proptests;
