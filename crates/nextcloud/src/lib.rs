//! Nextcloud for the Craft apps: files over WebDAV, sharing through the OCS Share API, and
//! contacts over CardDAV.
//!
//! The crate builds requests and parses responses. It opens no sockets. The caller supplies a
//! transport by implementing [`Http`], so the same code serves a native app and a WebAssembly
//! build. That also means every test here runs without a network.
//!
//! Written from the public specifications: WebDAV ([RFC 4918]), CardDAV ([RFC 6352]), vCard 4.0
//! ([RFC 6350]) and Nextcloud's published OCS Share API. No Nextcloud code is used.
//!
//! Like every Craft parser, the readers are lenient and bounded. Unknown elements are skipped,
//! missing properties fall back to defaults, and sizes are capped, so a hostile or simply odd
//! server yields an error or a best-effort answer, never a panic.
//!
//! ```no_run
//! use craft_nextcloud::{Account, Files};
//! # fn demo<H: craft_nextcloud::Http>(http: &H) -> craft_nextcloud::Result<()> {
//! let acct = Account::new("https://cloud.example.net", "alice", "app-password");
//! let files = Files::new(&acct);
//! for entry in files.list(http, "/Documents")? {
//!     println!("{} ({} bytes)", entry.name, entry.size);
//! }
//! # Ok(()) }
//! ```
//!
//! [RFC 4918]: https://www.rfc-editor.org/rfc/rfc4918
//! [RFC 6352]: https://www.rfc-editor.org/rfc/rfc6352
//! [RFC 6350]: https://www.rfc-editor.org/rfc/rfc6350
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod contacts;
mod dav;
mod error;
mod http;
mod login;
mod provider;
mod share;
mod xml;

#[cfg(test)]
mod tests;

pub use contacts::{AddressBook, Contact, Contacts};
pub use dav::{Entry, Files, Lock};
pub use error::{Error, Result};
pub use http::{Http, Method, Request, Response};
pub use login::{AppPassword, LoginFlow, begin, collab_token};
pub use provider::Nextcloud;
pub use share::{NewShare, Permissions, Share, ShareKind, Shares};

use std::fmt;

/// Largest response body any parser here will walk, as a guard against a server that answers a
/// small request with something enormous.
pub(crate) const MAX_BODY: usize = 32 * 1024 * 1024;

/// Where to reach a Nextcloud, and as whom.
///
/// `password` should be an **app password**, not the account's login password. Nextcloud issues
/// them per client, they can be revoked one at a time, and they sidestep two-factor prompts.
///
/// [`fmt::Debug`] is implemented by hand so the secret cannot reach a log through a derived
/// formatter.
#[derive(Clone)]
pub struct Account {
    base: String,
    user: String,
    password: String,
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account").field("base", &self.base).field("user", &self.user).field("password", &"<redacted>").finish()
    }
}

impl Account {
    /// `base` is the server root, with or without a trailing slash, as in
    /// `https://cloud.example.net`.
    pub fn new(base: impl Into<String>, user: impl Into<String>, password: impl Into<String>) -> Account {
        let mut base = base.into();
        while base.ends_with('/') {
            base.pop();
        }
        Account { base, user: user.into(), password: password.into() }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// The value for an `Authorization` header.
    ///
    /// Basic auth, which is what Nextcloud expects for an app password. The encoder is local
    /// because this crate carries no dependency for it.
    pub(crate) fn authorization(&self) -> String {
        let raw = format!("{}:{}", self.user, self.password);
        format!("Basic {}", base64(raw.as_bytes()))
    }

    /// The WebDAV files root for this user, without a trailing slash.
    pub(crate) fn files_root(&self) -> String {
        format!("{}/remote.php/dav/files/{}", self.base, encode_segment(&self.user))
    }

    /// The CardDAV address-book home for this user.
    pub(crate) fn addressbook_root(&self) -> String {
        format!("{}/remote.php/dav/addressbooks/users/{}", self.base, encode_segment(&self.user))
    }

    /// The OCS Share API root.
    pub(crate) fn ocs_share_root(&self) -> String {
        format!("{}/ocs/v2.php/apps/files_sharing/api/v1", self.base)
    }

    /// A request with authentication and the headers Nextcloud wants on every call.
    pub(crate) fn request(&self, method: Method, url: impl Into<String>) -> Request {
        Request::new(method, url)
            .header("authorization", self.authorization())
            // Nextcloud rejects OCS calls without this, as a CSRF guard. It is harmless elsewhere.
            .header("ocs-apirequest", "true")
            .header("accept", "application/xml")
    }
}

/// Percent-encode one path segment.
///
/// Everything outside the unreserved set of [RFC 3986] is encoded, which is stricter than
/// necessary and never wrong. A `/` in the input is encoded too, so a segment can never widen into
/// a path. Callers that mean to pass several segments split them first, see [`encode_path`].
///
/// [RFC 3986]: https://www.rfc-editor.org/rfc/rfc3986#section-2.3
pub(crate) fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(*byte as char),
            other => {
                out.push('%');
                out.push(hex_digit(other >> 4));
                out.push(hex_digit(other & 0x0f));
            }
        }
    }
    out
}

/// Percent-encode a `/`-separated path, segment by segment, keeping the separators.
///
/// Leading and trailing slashes are dropped, and empty segments collapse, so `//a//b/` and `a/b`
/// give the same result. A `.` or `..` segment is rejected: a remote path is built from user input,
/// and traversal has no legitimate meaning here.
pub(crate) fn encode_path(path: &str) -> Result<String> {
    let mut out = String::with_capacity(path.len());
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." {
            return Err(Error::invalid("path", format!("traversal segment {segment:?} in {path:?}")));
        }
        out.push('/');
        out.push_str(&encode_segment(segment));
    }
    Ok(out)
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        // `nibble` is masked to 4 bits by both callers, so this arm covers 10..=15.
        n => (b'A' + (n - 10)) as char,
    }
}

/// Standard base64, with padding. Local because the crate carries no dependency for it.
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied();
        let b2 = chunk.get(2).copied();
        let n = u32::from(b0) << 16 | u32::from(b1.unwrap_or(0)) << 8 | u32::from(b2.unwrap_or(0));
        out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
        out.push(if b1.is_some() { ALPHABET[(n >> 6 & 0x3f) as usize] as char } else { '=' });
        out.push(if b2.is_some() { ALPHABET[(n & 0x3f) as usize] as char } else { '=' });
    }
    out
}

/// Send a request and reject a non-2xx status, with 207 Multi-Status allowed through.
pub(crate) fn send<H: Http>(http: &H, req: Request) -> Result<Response> {
    let method = req.method;
    let url = req.url.clone();
    let resp = http.send(req).map_err(|e| Error::Transport(e.to_string()))?;
    if resp.body.len() > MAX_BODY {
        return Err(Error::malformed("response", format!("body of {} bytes exceeds the cap", resp.body.len())));
    }
    if !resp.is_success() {
        return Err(Error::status(method, &url, &resp));
    }
    Ok(resp)
}
