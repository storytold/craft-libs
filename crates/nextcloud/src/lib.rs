#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Nextcloud support for the Craft suite: remote paths, accounts and errors.

mod account;
mod cache;
mod client;
mod dav;
mod error;
mod http;
mod memories;
mod path;
mod signin;
#[cfg(feature = "testing")]
pub mod testing;
mod tls;

pub use account::Account;
pub use cache::{Fetched, Hold, OriginalsCache, Usage};
pub use client::{Client, Entry};
pub use error::{Error, Fingerprint};
pub use memories::{Memories, MemoriesPhoto, RemoteMeta};
pub use path::{MAX_SEGMENTS, RemotePath};
/// The rustls this crate is built with; pass a provider of this version's `Arc<rustls::crypto::CryptoProvider>`.
pub use rustls;
pub use signin::{SIGN_IN_TIMEOUT, SignIn, SignInState};
