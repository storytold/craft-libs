// SPDX-FileCopyrightText: Craft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Mail protocols without sockets.
//!
//! [`smtp::Smtp`] and [`imap::Imap`] speak SMTP (RFC 5321, 3207, 4954) and IMAP4rev1 (RFC 3501,
//! 4315, 4959, 6154, 6851) over any `Read + Write` stream: a TLS socket on the desktop, a scripted
//! transcript in the tests. Everything a server sends is treated as hostile: lines, literals and
//! response counts are capped, and nothing here panics on junk.
//!
//! Passwords and access tokens never reach a log or an error text, even when a server echoes
//! them back.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod i18n;
pub mod imap;
mod line;
pub mod smtp;

#[cfg(test)]
mod script;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use i18n::{Arg, Text};

pub(crate) use i18n::{for_trace, tidy};

/// Longest line a server may send.
pub const MAX_LINE: usize = 64 * 1024;
/// Largest counted block (an IMAP literal, so: the largest message) a server may send.
pub const MAX_LITERAL: usize = 64 * 1024 * 1024;
/// Longest user name and access token, together, that one sign-in may carry.
const MAX_XOAUTH2: usize = 32 * 1024;

/// What went wrong. Each holds the reason as a [`Text`]: Craft's own words are a template
/// displayed in its language ([`said!`]), and what a server or the system said is kept as
/// it said it ([`Text::verbatim`]). The English ([`Display`](std::fmt::Display)) is what the CLI, MCP
/// and the log read, and it is the same either way.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    Io(Text),
    Protocol(Text),
    /// The server refused the command (IMAP NO/BAD, SMTP 4xx/5xx); the text is the server's.
    Rejected(Text),
    Auth(Text),
}

impl ProtoError {
    /// The error as the user is told it: what kind of trouble, and why.
    pub fn said(&self) -> Text {
        match self {
            ProtoError::Io(why) => said!("connection problem: {reason}", reason = Arg::said(why.clone())),
            ProtoError::Protocol(why) => said!("unexpected server response: {reason}", reason = Arg::said(why.clone())),
            ProtoError::Rejected(why) => said!("server rejected: {reason}", reason = Arg::said(why.clone())),
            ProtoError::Auth(why) => said!("authentication failed: {reason}", reason = Arg::said(why.clone())),
        }
    }
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Io(why) => why.fmt(f),
            ProtoError::Protocol(why) => why.fmt(f),
            ProtoError::Rejected(why) => why.fmt(f),
            ProtoError::Auth(why) => why.fmt(f),
        }
    }
}

impl std::error::Error for ProtoError {}

/// Secrets: passwords and tokens, never shown in logs or errors.
#[derive(Default)]
pub(crate) struct Secrets(Vec<zeroize::Zeroizing<String>>);

impl Secrets {
    pub(crate) fn add(&mut self, secret: &str) {
        if !secret.is_empty() && !self.0.iter().any(|s| s.as_str() == secret) {
            self.0.push(zeroize::Zeroizing::new(secret.to_owned()));
            // Longer overlapping secrets must be hidden before their prefixes.
            self.0.sort_by_key(|s| std::cmp::Reverse(s.len()));
        }
    }

    pub(crate) fn hide(&self, text: &str) -> String {
        self.0.iter().fold(text.to_owned(), |text, secret| text.replace(secret.as_str(), "***"))
    }
}

/// Build a bounded, base64-encoded SASL XOAUTH2 initial response.
pub(crate) fn xoauth2(user: &str, token: &str) -> Result<String, ProtoError> {
    if user.is_empty() || token.is_empty() || user.chars().chain(token.chars()).any(char::is_control) {
        return Err(ProtoError::Auth(said!("the user name or token has a character that can't be sent")));
    }
    if user.len().saturating_add(token.len()) > MAX_XOAUTH2 {
        return Err(ProtoError::Auth(said!("the user name or token is too long")));
    }
    let plain = zeroize::Zeroizing::new(format!("user={user}\x01auth=Bearer {token}\x01\x01"));
    Ok(STANDARD.encode(plain.as_bytes()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn secrets_redact_overlapping_values() {
        let mut secrets = Secrets::default();
        secrets.add("token");
        secrets.add("token-long");
        assert_eq!(secrets.hide("token-long token"), "*** ***");
    }

    #[test]
    fn xoauth2_rejects_injection_and_oversized_values() {
        assert!(xoauth2("user\x01auth=Bearer injected", "token").is_err());
        assert!(xoauth2("user", &"x".repeat(MAX_XOAUTH2)).is_err());
    }

    #[test]
    fn error_display() {
        let err = ProtoError::Io(said!("connection problem"));
        assert_eq!(err.to_string(), "connection problem");
    }

    #[test]
    fn error_said() {
        let err = ProtoError::Auth(said!("invalid password"));
        assert_eq!(err.said().to_string(), "authentication failed: invalid password");
    }

    #[test]
    fn xoauth2_format() {
        let result = xoauth2("user@example.com", "token123").unwrap();
        let result = String::from_utf8(STANDARD.decode(result).unwrap()).unwrap();
        assert!(result.contains("user=user@example.com"));
        assert!(result.contains("auth=Bearer token123"));
    }
}
