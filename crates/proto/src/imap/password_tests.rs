// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::script::Script;
use base64::{Engine as _, engine::general_purpose::STANDARD};

fn greeting(caps: &str, replies: &str) -> String {
    format!("* OK [CAPABILITY IMAP4rev1 {caps}] ready\r\n{replies}")
}

#[test]
fn plain_uses_an_initial_response_only_when_advertised() {
    let payload = STANDARD.encode(b"\0user@example.org\0invented");
    for inline in [false, true] {
        let caps = if inline { "AUTH=PLAIN SASL-IR LOGINDISABLED" } else { "AUTH=PLAIN LOGINDISABLED" };
        let replies = if inline { "A0001 OK welcome\r\n" } else { "+ continue\r\nA0001 OK welcome\r\n" };
        let mut stream = Script::new(greeting(caps, replies));
        let mut imap = Imap::connect(&mut stream).unwrap();
        imap.authenticate_password("user@example.org", "invented").unwrap();
        assert!(imap.is_authenticated());
        let expected = if inline { format!("A0001 AUTHENTICATE PLAIN {payload}\r\n") } else { format!("A0001 AUTHENTICATE PLAIN\r\n{payload}\r\n") };
        assert_eq!(stream.sent(), expected);
    }
}

#[test]
fn login_remains_available_when_plain_is_not_advertised() {
    let mut stream = Script::new(greeting("SASL-IR", "A0001 OK welcome\r\n"));
    Imap::connect(&mut stream).unwrap().authenticate_password("user@example.org", "invented").unwrap();
    assert_eq!(stream.sent(), "A0001 LOGIN \"user@example.org\" \"invented\"\r\n");
}

#[test]
fn a_plain_refusal_is_redacted_and_never_retried_with_login() {
    let payload = STANDARD.encode(b"\0user@example.org\0invented");
    let mut stream = Script::new(greeting("AUTH=PLAIN SASL-IR", &format!("A0001 NO [AUTHENTICATIONFAILED] invented {payload}\r\n")));
    let mut imap = Imap::connect(&mut stream).unwrap();
    let error = imap.authenticate_password("user@example.org", "invented").unwrap_err();
    assert!(matches!(error, ProtoError::Auth(_)));
    assert!(!format!("{error:?} {imap:?}").contains("invented"));
    assert!(!format!("{error:?} {imap:?}").contains(&payload));
    assert!(!imap.is_authenticated());
    assert!(!stream.sent().contains(" LOGIN "));
}

#[test]
fn a_refusal_before_the_challenge_sends_no_password() {
    let mut stream = Script::new(greeting("AUTH=PLAIN", "A0001 NO unavailable\r\n"));
    assert!(Imap::connect(&mut stream).unwrap().authenticate_password("user@example.org", "invented").is_err());
    assert_eq!(stream.sent(), "A0001 AUTHENTICATE PLAIN\r\n");
}

#[test]
fn repeated_challenges_cancel_and_never_authenticate() {
    for replies in ["+ first\r\n+ second\r\nA0001 OK ignored\r\n", "+ first\r\n+ second\r\n+ third\r\n"] {
        let mut stream = Script::new(greeting("AUTH=PLAIN", replies));
        let mut imap = Imap::connect(&mut stream).unwrap();
        assert!(imap.authenticate_password("user@example.org", "invented").is_err());
        assert!(!imap.is_authenticated());
        assert!(stream.sent().ends_with("\r\n*\r\n"));
    }
}

#[test]
fn premature_success_and_invalid_credentials_are_refused() {
    let mut stream = Script::new(greeting("AUTH=PLAIN", "A0001 OK too early\r\n"));
    assert!(Imap::connect(&mut stream).unwrap().authenticate_password("user@example.org", "invented").is_err());
    for (user, password) in [("", "invented"), ("user@example.org", ""), ("bad\0user", "invented"), ("user@example.org", "bad\0password")] {
        let mut stream = Script::new(greeting("AUTH=PLAIN", ""));
        assert!(Imap::connect(&mut stream).unwrap().authenticate_password(user, password).is_err());
        assert!(stream.sent().is_empty());
    }
    let mut stream = Script::new(greeting("AUTH=PLAIN", ""));
    assert!(Imap::connect(&mut stream).unwrap().authenticate_password("user@example.org", &"x".repeat(32769)).is_err());
    assert!(stream.sent().is_empty());
}

#[test]
fn utf8_and_line_breaks_are_encoded_not_protocol_lines() {
    let mut stream = Script::new(greeting("auth=plain sasl-ir", "A0001 OK welcome\r\n"));
    Imap::connect(&mut stream).unwrap().authenticate_password("user@example.org", "päss\r\nword").unwrap();
    assert_eq!(stream.sent().lines().count(), 1);
    assert!(stream.sent().contains(&STANDARD.encode("\0user@example.org\0päss\r\nword".as_bytes())));
}

#[test]
fn unknown_capabilities_are_queried_once_by_password_authentication() {
    let mut stream = Script::new("* OK ready\r\n* CAPABILITY IMAP4rev1 AUTH=PLAIN SASL-IR\r\nA0001 OK done\r\nA0002 OK welcome\r\n");
    Imap::connect(&mut stream).unwrap().authenticate_password("user@example.org", "invented").unwrap();
    assert!(stream.sent().starts_with("A0001 CAPABILITY\r\nA0002 AUTHENTICATE PLAIN "));
    assert_eq!(stream.sent().matches(" CAPABILITY").count(), 1);
}
#[test]
fn a_missing_password_has_its_own_message_without_network_commands() {
    let mut stream = Script::new("* OK ready\r\n");
    let error = Imap::connect(&mut stream).unwrap().authenticate_password("user@example.org", "").unwrap_err();
    assert!(error.to_string().contains("enter a password"));
    assert!(!error.to_string().contains("NUL"));
    assert!(stream.sent().is_empty());
}

#[test]
fn an_empty_user_name_is_explained_before_any_commands() {
    for text in ["* OK ready\r\n".to_string(), greeting("AUTH=PLAIN", ""), greeting("", "")] {
        let mut stream = Script::new(text);
        let error = Imap::connect(&mut stream).unwrap().authenticate_password("", "invented").unwrap_err();
        assert!(error.to_string().contains("enter a user name"));
        assert!(!error.to_string().contains("NUL"));
        assert!(stream.sent().is_empty());
    }
}
