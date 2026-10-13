#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Opt-in update notices and administrator-triggered updates for **self-hosted web
//! deployments** of the Craft apps. Desktop builds don't use this crate; a web build includes it
//! only when built with the app's `self-hosted-update` feature.
//!
//! A web deployment is a folder of static files (the wasm bundle). This crate adds two pieces:
//!
//! - **`craft-updated`** (feature `server`): a small companion process the self-hoster runs next
//!   to the web server, listening on loopback only, behind a reverse-proxied path. It checks the
//!   app's GitHub releases on a timer, emails the administrator about a new release, and, when
//!   the signed-in administrator asks, fetches the release tag, builds it with the configured
//!   command and atomically swaps the served folder. Its configuration (administrator password
//!   hash, SMTP settings, build paths) is a private TOML file written by `craft-updated setup`.
//! - **In the app** (features `web` and `egui`): [`ui::UpdateBanner`] shows every visitor that a
//!   newer version exists, and lets the administrator sign in and start the update.
//!
//! The browser never builds anything and never names what to build: it can only ask the
//! companion to "update to the newest release you found", and the companion checks the
//! administrator's credentials itself ([`protocol`] describes the requests). Setup, deployment and
//! the reverse-proxy configuration are in the crate's README.
//!
//! The shared logic compiles everywhere, wasm included: [`Version`] ordering, GitHub release
//! parsing ([`release`]), the wire types ([`protocol`]), the password hash ([`password`]) and
//! the notification texts ([`mail`]).

pub mod client;
pub mod mail;
pub mod password;
pub mod protocol;
pub mod release;
mod version;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod server;
#[cfg(feature = "egui")]
pub mod ui;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub mod web;

pub use version::Version;

/// What went wrong, with a message for the administrator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The configuration file or a value in it.
    Config(String),
    /// Finding the newest release (network, GitHub's answer).
    Release(String),
    /// Sending an email.
    Mail(String),
    /// Performing an update: fetching, building or swapping.
    Update(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(m) | Self::Release(m) | Self::Mail(m) | Self::Update(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}
