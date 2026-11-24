//! Tests of the `cos` feature, linked into one test binary. Not built for the web: proptest and
//! the oracle processes are not there.
#![cfg(all(feature = "cos", not(target_arch = "wasm32")))]

mod bounded_writer;
mod corpus;
mod efopen;
mod encryption;
mod file_identifier;
mod generations;
mod oracle;
mod proptests;
mod serialize_format;
mod shared_streams;
#[path = "../../src/cos/testkit.rs"]
mod testkit;
mod writer;
