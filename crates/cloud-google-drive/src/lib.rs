// Desktop only. A browser can't listen on a loopback port for Google's redirect, and the suite's
// keychain store doesn't build there, so on wasm the whole crate is empty and an app's web build
// connects no backend.
#![cfg(not(target_arch = "wasm32"))]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Google Drive as a [`craft_ui::cloud::CloudBackend`]: the shared Open and Save pages, against a
//! person's own Google Drive, with no service in between.
//!
//! - [`oauth`] signs in the way Google documents for installed apps: the system browser opens the
//!   consent page, Google redirects to a one-shot listener on `127.0.0.1` ([`loopback`]), and the
//!   code is exchanged with a PKCE verifier ([`pkce`], RFC 7636). No embedded web view.
//! - [`drive`] makes the Drive v3 calls: list a folder or what was shared, download (or export a
//!   Google Docs, Sheets or Slides file as Office Open XML), create, and update after checking the
//!   file's version.
//! - The refresh token is kept in the operating system's keychain through
//!   [`craft_suite_account::TokenStore`], never in a file.
//! - [`http`] is the transport seam: [`http::Transport`] is implemented with `ureq` on native
//!   targets, and by fakes in tests, so no test touches the network or the keychain.
//! - [`GoogleDriveBackend`] answers the shared pages' jobs on worker threads.
//!
//! There is no OAuth client built in. Each person, or their organisation, registers a "Desktop
//! app" client in their own Google Cloud project ([`ClientConfig`]); the application reads it
//! from its profile folder or the environment and hands it over. Nothing here names an
//! application.
//!
//! Written from Google's public OAuth 2.0 and Drive v3 REST documentation.

mod backend;
pub mod drive;
pub mod encode;
pub mod http;
pub mod loopback;
pub mod oauth;
pub mod pkce;

#[cfg(test)]
mod tests;

pub use backend::GoogleDriveBackend;
pub use drive::{Drive, DriveFile, SaveTarget};
pub use oauth::{Auth, ClientConfig, Tokens, downloads_dir, import_client, newest_client_file, write_client};

/// Errors from signing in or talking to Drive. Messages are English; an application translates
/// its own summary and shows the detail as is.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// No OAuth client is configured.
    #[error("Google Drive isn't set up: no OAuth client ID ({0})")]
    NotConfigured(String),
    /// Not signed in, or the refresh token was revoked.
    #[error("not signed in to Google Drive")]
    NotSignedIn,
    /// The person closed the consent page, denied access, or sign-in timed out or was cancelled.
    #[error("sign-in didn't finish: {0}")]
    SignIn(String),
    /// The file changed in Drive since it was opened; nothing was overwritten.
    #[error("{name} was changed in Google Drive since you opened it (Drive version {remote}, opened {opened})")]
    Conflict { name: String, opened: String, remote: String },
    /// Drive answered with an error status.
    #[error("Google Drive returned {status}: {message}")]
    Api { status: u16, message: String },
    /// The response was malformed or larger than allowed.
    #[error("unexpected response from Google: {0}")]
    Protocol(String),
    /// A parameter was rejected before anything was sent (bad file id, empty name, too large).
    #[error("{0}")]
    Invalid(String),
    /// The network or TLS failed.
    #[error("couldn't reach Google: {0}")]
    Network(String),
    /// The keychain failed.
    #[error("keychain: {0}")]
    Keychain(String),
}

pub type Result<T> = std::result::Result<T, Error>;
