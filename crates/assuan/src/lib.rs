//! The Assuan protocol and gpg-agent, without sockets.
//!
//! OpenPGP private keys stay in GnuPG (on disk or on a smartcard) and are used through
//! gpg-agent, which asks for a PIN or passphrase in its own pinentry: the caller never sees one.
//! This crate speaks to the agent over any `Read + Write` stream, written from the public
//! documentation of the Assuan protocol and of gpg-agent's commands:
//!
//! - [`Client`]: Assuan's line framing (lines of at most [`MAX_LINE`] bytes, percent-escaping),
//!   the server's `OK`, `ERR`, `S`, `D`, `INQUIRE` and `#` lines, answers to an inquiry (`D` …
//!   `END`, or `CAN`), and caps on everything a server may send.
//! - [`GpgAgent`]: the operations a mail program needs: whether a key is there (`HAVEKEY`, `KEYINFO`,
//!   `READKEY`), signing a digest (`SIGKEY`, `SETHASH`, `PKSIGN`; SHA-256, SHA-384 and SHA-512
//!   only, never MD5 or SHA-1), decrypting a session key (`SETKEY`, `PKDECRYPT`, answering the
//!   agent's `INQUIRE CIPHERTEXT`), and the hints (`OPTION display=…`, `ttyname`…) that let the
//!   agent's own pinentry show on the owner's screen.
//! - [`sexp`]: a bounded reader and writer of canonical S-expressions, the form the agent hands
//!   keys, signatures and session keys over in.
//! - [`socket_file`]: the two kinds of file that may stand where the agent's socket is named: a
//!   redirect, and (on Windows) the loopback port and nonce of Assuan's socket emulation.
//!
//! Finding the agent and opening the socket is the caller's: this crate does
//! no I/O of its own and builds for the browser, which never uses it.
//!
//! Everything the agent sends is treated as hostile: lines, data, status lines, inquiries and
//! S-expressions are capped, and nothing here panics on junk. A session key that comes back is
//! wiped from memory when it is dropped, and never shown by `Debug`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod agent;
mod client;
mod line;
pub mod sexp;
pub mod socket_file;

#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod line_tests;
#[cfg(test)]
mod sexp_tests;
#[cfg(test)]
mod socket_file_tests;

pub use agent::{
    Digest, GpgAgent, HashAlgorithm, KeyInfo, KeyStorage, Keygrip, MAX_DESCRIPTION, MAX_KEYS, OwnerWait, PinentryHints, Protection, SessionKey,
    Signature,
};
pub use client::{Answer, Client, Inquiry, Response, Status};
pub use sexp::Sexp;

/// Longest line either side may send, its line feed included (the Assuan specification's limit).
pub const MAX_LINE: usize = 1000;
/// Most bytes of `D` data one answer may carry: keys, signatures and session keys are far smaller.
pub const MAX_DATA: usize = 256 * 1024;
/// Most lines one answer may take (comments and status lines included), so that a server that
/// talks for ever is stopped.
pub const MAX_LINES: usize = 16 * 1024;
/// Most status lines kept from one answer. `KEYINFO --list`, which has a line for each key, reads
/// its lines as they come instead and keeps at most [`MAX_KEYS`].
pub const MAX_STATUS: usize = 256;
/// Most inquiries one command may answer.
pub const MAX_INQUIRIES: usize = 16;
/// Most bytes sent in answer to one inquiry.
pub const MAX_INQUIRY_DATA: usize = 1024 * 1024;

/// What went wrong talking to gpg-agent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The stream failed (its kind, never what was being sent).
    #[error("connection problem with gpg-agent: {0}")]
    Io(String),
    /// The agent closed the connection.
    #[error("gpg-agent closed the connection")]
    Closed,
    /// The agent sent a line longer than the protocol allows.
    #[error("gpg-agent sent a line longer than the protocol allows")]
    LineTooLong,
    /// The agent sent more lines, data, status lines or inquiries than one answer may have.
    #[error("gpg-agent sent more than one answer may hold")]
    TooMuch,
    /// The agent sent something the protocol doesn't have, or not what the command asks for.
    #[error("gpg-agent sent something unexpected: {0}")]
    Protocol(String),
    /// The agent answered with `ERR`.
    #[error("gpg-agent refused: {0}")]
    Server(ServerError),
    /// Nothing was sent: what was asked can't be said in the protocol (a line break in an
    /// argument, a line too long, a digest of the wrong size…).
    #[error("not sent to gpg-agent: {0}")]
    Refused(String),
    /// An earlier failure left the connection in an unknown state; a new one is needed.
    #[error("the connection to gpg-agent can't be used any more")]
    Broken,
}

impl Error {
    /// The agent's refusal, when that is what this is.
    pub fn server(&self) -> Option<&ServerError> {
        match self {
            Error::Server(e) => Some(e),
            _ => None,
        }
    }
}

/// An `ERR` line: the agent's error code and its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerError {
    /// The whole code (an error source in the high byte, the error itself in the low 16 bits).
    pub code: u32,
    /// The agent's description, one line of printable text, capped.
    pub text: String,
}

/// What an [`ServerError`] means, for the errors told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The agent has no secret key for the keygrip.
    NoSecretKey,
    /// The owner cancelled the pinentry.
    Canceled,
    /// A wrong passphrase was typed.
    BadPassphrase,
    /// A wrong PIN was typed.
    BadPin,
    /// What was asked for isn't there.
    NotFound,
    /// The pinentry waited too long.
    Timeout,
    /// No pinentry could be shown.
    NoPinentry,
    /// Any other error.
    Other,
}

impl ServerError {
    /// The error itself, without its source.
    pub fn number(&self) -> u32 {
        self.code & 0xFFFF
    }

    /// What it means.
    pub fn kind(&self) -> ErrorKind {
        // The numbers of the GnuPG error codes (libgpg-error's published list).
        match self.number() {
            11 => ErrorKind::BadPassphrase,
            17 => ErrorKind::NoSecretKey,
            27 => ErrorKind::NotFound,
            62 => ErrorKind::Timeout,
            85 => ErrorKind::NoPinentry,
            87 => ErrorKind::BadPin,
            99 | 198 => ErrorKind::Canceled,
            _ => ErrorKind::Other,
        }
    }
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.text.is_empty() { write!(f, "error {}", self.code) } else { write!(f, "{} (error {})", self.text, self.code) }
    }
}

/// Bytes the agent sent, as one line of printable text of at most `max` characters.
pub(crate) fn tidy(bytes: &[u8], max: usize) -> String {
    String::from_utf8_lossy(bytes).chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_string()
}

/// Append `more` to `buf` without leaving a copy behind: when `buf` must grow, its bytes are moved
/// to a new allocation and the old one is wiped before it is given back.
pub(crate) fn extend_wiped(buf: &mut Vec<u8>, more: &[u8]) {
    let needed = buf.len().saturating_add(more.len());
    if needed > buf.capacity() {
        let mut old = grow(buf, needed);
        wipe(&mut old);
    }
    buf.extend_from_slice(more);
}

/// Move `buf` to an allocation of at least `needed` bytes; the old buffer, its bytes wiped, is
/// handed back (to be dropped).
pub(crate) fn grow(buf: &mut Vec<u8>, needed: usize) -> Vec<u8> {
    let mut bigger = Vec::with_capacity(needed.max(buf.capacity().saturating_mul(2)));
    bigger.extend_from_slice(buf);
    let mut old = std::mem::replace(buf, bigger);
    // The whole allocation: bytes past the length may be left from before.
    let len = old.len();
    old.resize(old.capacity(), 0);
    wipe(&mut old);
    old.truncate(len);
    old
}

/// Drop the first `n` bytes of `buf`, wiping the copies that moving the rest down leaves behind.
pub(crate) fn discard_front(buf: &mut Vec<u8>, n: usize) {
    let n = n.min(buf.len());
    let len = buf.len();
    buf.copy_within(n.., 0);
    let kept = len.saturating_sub(n);
    if let Some(tail) = buf.get_mut(kept..) {
        wipe(tail);
    }
    buf.truncate(kept);
}

/// Overwrite bytes that held a secret before their memory is given back.
pub(crate) fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        *b = 0;
    }
    std::hint::black_box(bytes);
}
