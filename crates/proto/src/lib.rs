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
    /// The underlying reason, without its error category prefix.
    pub fn reason(&self) -> &Text {
        match self {
            Self::Io(why) | Self::Protocol(why) | Self::Rejected(why) | Self::Auth(why) => why,
        }
    }

    /// The error as the user is told it: what kind of trouble, and why.
    pub fn said(&self) -> Text {
        match self {
            ProtoError::Io(why) => said!("connection problem: {reason}", reason = Arg::said(why.clone())),
            ProtoError::Protocol(why) => said!("unexpected server response: {reason}", reason = Arg::said(why.clone())),
            ProtoError::Rejected(why) => said!("the server said: {reason}", reason = Arg::said(why.clone())),
            ProtoError::Auth(why) => said!("sign-in failed: {reason}", reason = Arg::said(why.clone())),
        }
    }
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.said(), f)
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
        assert_eq!(err.to_string(), "connection problem: connection problem");
        assert_eq!(err.reason().to_string(), "connection problem");
    }

    #[test]
    fn error_said() {
        let err = ProtoError::Auth(said!("invalid password"));
        assert_eq!(err.said().to_string(), "sign-in failed: invalid password");
    }

    #[test]
    fn xoauth2_format() {
        let result = xoauth2("user@example.com", "token123").unwrap();
        let result = String::from_utf8(STANDARD.decode(result).unwrap()).unwrap();
        assert!(result.contains("user=user@example.com"));
        assert!(result.contains("auth=Bearer token123"));
    }
}

#[cfg(test)]
mod source_regression_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn the_wire_trace_keeps_keywords_and_no_address_name_or_text() {
        // (Line, what the trace shows.)
        for (line, _old_shown) in [
            ("A0001 OK ada@example.org authenticated (Success)\r\n", "A0001 OK … (39 bytes)"),
            ("A0002 LOGIN ***", "A0002 LOGIN ***"),
            ("MAIL FROM:<ada@example.org> SIZE=120", "MAIL FROM: … (26 bytes)"),
            ("RCPT TO:<bob@example.org>", "RCPT TO: … (17 bytes)"),
            ("550 5.1.1 <bob@example.org>: Recipient address rejected", "550 5.1.1 … (45 bytes)"),
            ("250-mx.example.org at your service", "250- … (30 bytes)"),
            ("250-AUTH LOGIN PLAIN XOAUTH2", "250-AUTH LOGIN PLAIN XOAUTH2"),
            ("A0003 SELECT \"Letters from Grace\"", "A0003 SELECT … (20 bytes)"),
            ("* LIST (\\HasNoChildren) \"/\" Grace", "* LIST … (26 bytes)"),
            ("* 12 FETCH (UID 40 FLAGS (\\Seen) BODY[HEADER.FIELDS (SUBJECT)] \"Subject: Quarterly figures\")", "* 12 FETCH … (81 bytes)"),
            ("* 3 EXISTS", "* 3 EXISTS"),
            ("UID FETCH 1:40,42 (UID FLAGS)", "UID FETCH 1:40,42 … (11 bytes)"),
            ("hello there", "(11 bytes)"),
            ("", ""),
        ] {
            let traced = for_trace(line);
            assert!(traced.len() <= line.len().saturating_add(32), "{line:?}");
            for private in ["ada", "bob", "Grace", "Quarterly", "mx.example"] {
                assert!(!traced.contains(private), "{line:?}: {traced}");
            }
        }
        assert!(for_trace(&format!("A0001 OK {}", "X ".repeat(500))).len() < 160);
        assert_eq!(for_trace("ünïcödé ä"), "(14 bytes)");
    }

    #[test]
    fn uppercase_private_text_is_not_protocol_structure() {
        assert_eq!(for_trace("550 CUSTOMER ALICE PAYMENT OVERDUE"), "550 … (30 bytes)");
        assert_eq!(for_trace("A0003 SELECT PRIVATE_MAILBOX"), "A0003 SELECT … (15 bytes)");
        assert_eq!(for_trace("250-PRIVATE_CUSTOMER_DOMAIN"), "250- … (23 bytes)");
        assert_eq!(for_trace("A0001 OK PRIVATE CUSTOMER"), "A0001 OK … (16 bytes)");
    }

    #[test]
    fn secrets_are_hidden_longest_first() {
        let mut s = Secrets::default();
        s.add("hunter2");
        s.add("hunter2-and-more");
        s.add("");
        assert_eq!(s.hide("no hunter2-and-more, no hunter2, nothing"), "no ***, no ***, nothing");
        assert_eq!(Secrets::default().hide("plain"), "plain");
    }

    #[test]
    fn the_xoauth2_response_is_user_and_bearer_token_between_control_a() {
        let decoded = |payload: String| STANDARD.decode(payload).unwrap();
        assert_eq!(decoded(xoauth2("ada@example.org", "tok.en-123").unwrap()), b"user=ada@example.org\x01auth=Bearer tok.en-123\x01\x01");
        // The example of the mechanism's description.
        assert_eq!(
            xoauth2("someuser@example.com", "ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg").unwrap(),
            "dXNlcj1zb21ldXNlckBleGFtcGxlLmNvbQFhdXRoPUJlYXJlciB5YTI5LnZGOWRmdDRxbVRjMk52YjNSbGNrQmhkSFJoZG1semRHRXVZMjl0Q2cBAQ=="
        );
        assert_eq!(decoded(xoauth2("ädä", "t").unwrap()), "user=ädä\x01auth=Bearer t\x01\x01".as_bytes());
        // Nothing that could end the name, the token or the line early.
        for (user, token) in [("ada\x01auth=Bearer other", "t"), ("ada", "t\x01"), ("ada\r\n", "t"), ("ada", "t\nQUIT"), ("a\0", "t"), ("ada", "\0")]
        {
            assert!(matches!(xoauth2(user, token), Err(ProtoError::Auth(_))), "{user:?} {token:?}");
        }
        assert!(matches!(xoauth2("ada", ""), Err(ProtoError::Auth(_))));
        assert!(xoauth2("ada", &"t".repeat(MAX_XOAUTH2 - 3)).is_ok());
        let err = xoauth2("ada", &"t".repeat(MAX_XOAUTH2)).unwrap_err();
        assert!(matches!(&err, ProtoError::Auth(why) if why.to_string().len() < 100), "{err:?}");
    }

    #[test]
    fn server_text_is_tidied() {
        assert_eq!(tidy("  5.7.8\tBad\r\n credentials \u{7}"), "5.7.8 Bad credentials");
        let long = tidy(&"é".repeat(1000));
        assert_eq!(long.chars().count(), 301);
        assert!(long.ends_with('…'));
        assert_eq!(tidy(""), "");
    }
}
