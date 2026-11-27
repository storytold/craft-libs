#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The Craft apps' add-in contract.
//!
//! An add-in is a separate program, written in any language, described by a `plugin.json`
//! manifest. An app starts it and talks JSON-RPC 2.0 with it over its standard input and output,
//! one message per line. The manifest and the protocol are the same for every app, so one add-in
//! can serve several. This crate is the part every app would otherwise write again:
//!
//! - [`Manifest`]: `plugin.json`, read for one app ([`Host`]) and checked against every limit.
//! - [`version`] and [`requires`]: versions, ranges, and resolving what add-ins need (app
//!   versions, protocol, host features, each other) over every installed add-in together.
//! - [`github`]: the GitHub addresses add-ins are installed from and the release asset that suits
//!   a platform. Parsing only: nothing here touches the network.
//! - [`Connection`]: the process, with reading and writing on their own threads, so an add-in
//!   that hangs, floods or exits costs a timeout or an error, never a hung app.
//! - [`push`]: what add-ins say on their own, queued within limits for the app's main loop.
//!
//! What an add-in's functions, commands and documents mean stays with the app: its registry,
//! its commands, its menus and its file formats are not here.

pub mod github;
mod manifest;
mod process;
pub mod push;
pub mod requires;
pub mod version;

use std::sync::{Mutex, MutexGuard};

pub use manifest::{
    ButtonDecl, ButtonSize, CollabDecl, CommandDecl, FunctionDecl, Host, MAX_ARGS, MAX_DECLARATIONS, MAX_LABEL, MacrosDecl, Manifest, MenuItemDecl,
    RibbonGroupDecl, RunSpec, UiDecl,
};
pub use process::{Connection, HostError, HostHandler, Launch, Notifier, RequestError};

/// The protocol version an app sends in `initialize`. Protocols 1.1 to 2.2 add to 1 without
/// changing it; an add-in checks `host.protocol` and `host.features` instead.
pub const PROTOCOL_VERSION: u32 = 1;
/// The manifest's file name inside an add-in's folder.
pub const MANIFEST: &str = "plugin.json";
/// Largest manifest read from a file or an archive.
pub const MAX_MANIFEST_BYTES: u64 = 1 << 20;

/// Whether `s` can be an add-in, app or command id: 1 to 128 letters, digits, `.`, `-` or `_`,
/// starting with a letter or digit (an id is also the folder an installed add-in goes in).
pub fn valid_id(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.len() <= 128
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Locks `m`; a thread that panicked while holding it doesn't take the queue with it.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}
