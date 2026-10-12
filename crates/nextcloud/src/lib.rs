#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Nextcloud support for the Craft suite: remote paths, accounts and errors.

mod account;
mod error;
mod path;

pub use account::Account;
pub use error::{Error, Fingerprint};
pub use path::{MAX_SEGMENTS, RemotePath};
