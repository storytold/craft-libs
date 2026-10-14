#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Nextcloud for the Craft suite: remote paths, sign-in with trust on first use, WebDAV listing, previews, Range reads,
//! downloads, Memories metadata, an originals cache and (feature `keyring`) OS keyring credentials.

mod account;
mod cache;
mod client;
mod dav;
mod error;
mod http;
#[cfg(feature = "keyring")]
mod keyring;
mod memories;
mod path;
mod signin;
#[cfg(feature = "testing")]
pub mod testing;
mod tls;

#[cfg(feature = "keyring")]
pub use self::keyring::{delete_password, load_password, store_password};
pub use account::Account;
pub use cache::{Fetched, Hold, OriginalsCache, Usage};
pub use client::{Client, Entry};
pub use error::{Error, Fingerprint};
pub use memories::{Memories, MemoriesPhoto, RemoteMeta};
pub use path::{MAX_SEGMENTS, RemotePath};
/// The rustls this crate is built with; pass a provider of this version's `Arc<rustls::crypto::CryptoProvider>`.
pub use rustls;
pub use signin::{SIGN_IN_TIMEOUT, SignIn, SignInState};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
