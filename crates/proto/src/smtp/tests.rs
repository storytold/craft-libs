// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use std::io::ErrorKind;

use proptest::prelude::*;

use super::*;
use crate::script::{Broken, Script};

const EHLO: &str =
    "250-mail.example.org greets you\r\n250-SIZE 35882577\r\n250-8BITMIME\r\n250-STARTTLS\r\n250-AUTH LOGIN PLAIN XOAUTH2\r\n250 HELP\r\n";

fn rejected(r: Result<(), ProtoError>) -> String {
    match r {
        Err(ProtoError::Rejected(text)) => text.to_string(),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn a_whole_session() {
    let mut s = Script::new(format!(
        "220-mail.example.org ESMTP\r\n220 ready\r\n{EHLO}235 2.7.0 Accepted\r\n250 OK\r\n250 OK\r\n251 will forward\r\n354 go ahead\r\n250 queued as 1\r\n221 bye\r\n"
    ));
    let mut smtp = Smtp::connect(&mut s).unwrap();
    let ext = smtp.hello("[127.0.0.1]").unwrap();
    assert_eq!(
        ext,
        Extensions { starttls: true, auth_plain: true, auth_login: true, auth_xoauth2: true, size: Some(35_882_577), eight_bit_mime: true }
    );
    assert_eq!(smtp.extensions(), &ext);
    smtp.auth("ada@example.org", "hunter2").unwrap();
    smtp.send("ada@example.org", &["bob@example.org", "carol@example.org"], b"Subject: Hi\r\n\r\nHello.\r\n").unwrap();
    smtp.quit().unwrap();
    assert!(s.finished());
    assert_eq!(
        s.sent(),
        "EHLO [127.0.0.1]\r\n\
         AUTH PLAIN AGFkYUBleGFtcGxlLm9yZwBodW50ZXIy\r\n\
         MAIL FROM:<ada@example.org> SIZE=23\r\n\
         RCPT TO:<bob@example.org>\r\n\
         RCPT TO:<carol@example.org>\r\n\
         DATA\r\n\
         Subject: Hi\r\n\r\nHello.\r\n.\r\n\
         QUIT\r\n"
    );
}

#[test]
fn the_plain_payload_is_nul_user_nul_password() {
    let mut s = Script::new("250-x\r\n250 AUTH PLAIN\r\n235 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    smtp.auth("grace", "pässword \"1\"").unwrap();
    let sent = s.sent();
    let payload = sent.lines().nth(1).unwrap().strip_prefix("AUTH PLAIN ").unwrap();
    assert_eq!(STANDARD.decode(payload).unwrap(), "\0grace\0pässword \"1\"".as_bytes());
}

#[test]
fn a_long_plain_payload_waits_for_the_server_to_ask() {
    let mut s = Script::new("250-x\r\n250 AUTH PLAIN\r\n334 \r\n235 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    let password = "p".repeat(600);
    smtp.auth("grace", &password).unwrap();
    let sent = s.sent();
    let lines: Vec<&str> = sent.lines().collect();
    assert_eq!(lines[1], "AUTH PLAIN");
    assert_eq!(STANDARD.decode(lines[2]).unwrap(), format!("\0grace\0{password}").as_bytes());
}

#[test]
fn login_is_used_when_plain_is_not_offered() {
    let mut s = Script::new("250-x\r\n250 AUTH=LOGIN CRAM-MD5\r\n334 VXNlcm5hbWU6\r\n334 UGFzc3dvcmQ6\r\n235 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    let ext = smtp.hello("localhost").unwrap();
    assert!(ext.auth_login && !ext.auth_plain);
    smtp.auth("ada", "hunter2").unwrap();
    assert_eq!(s.sent(), "EHLO localhost\r\nAUTH LOGIN\r\nYWRh\r\naHVudGVyMg==\r\n");
}

#[test]
fn a_wrong_password_is_an_auth_error_that_never_shows_it() {
    // A careless server repeats what it was sent.
    let echo = "535-5.7.8 no such password: hunter2\r\n535 5.7.8 (AGFkYQBodW50ZXIy)\r\n";
    let mut s = Script::new(format!("250-x\r\n250 AUTH PLAIN\r\n{echo}"));
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    let err = smtp.auth("ada", "hunter2").unwrap_err();
    assert_eq!(err, ProtoError::Auth("535 5.7.8 no such password: *** 5.7.8 (***)".into()));
    assert!(!format!("{err} {err:?} {smtp:?}").contains("hunter2"));

    // The same over LOGIN, where the password travels alone.
    let mut s = Script::new("250-x\r\n250 AUTH LOGIN\r\n334 VXNlcm5hbWU6\r\n334 UGFzc3dvcmQ6\r\n535 bad aHVudGVyMg== / hunter2\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert_eq!(smtp.auth("ada", "hunter2"), Err(ProtoError::Auth("535 bad *** / ***".into())));
}

#[test]
fn sign_in_needs_a_method_and_sendable_text() {
    let mut s = Script::new("250-x\r\n250 AUTH XOAUTH2\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert!(matches!(smtp.auth("ada", "hunter2"), Err(ProtoError::Auth(_))));
    assert!(matches!(smtp.auth("ada", "hun\0ter2"), Err(ProtoError::Auth(_))));
    // Neither attempt put anything on the wire.
    assert_eq!(s.sent(), "EHLO localhost\r\n");
}

/// What an XOAUTH2 response says under its base64.
fn bearer(payload: &str) -> String {
    String::from_utf8(STANDARD.decode(payload).unwrap()).unwrap()
}

#[test]
fn a_token_signs_in_with_xoauth2() {
    let mut s = Script::new("250-x\r\n250 AUTH LOGIN XOAUTH2\r\n235 2.7.0 Authentication successful\r\n");
    let mut smtp = Smtp::resume(&mut s);
    assert!(smtp.hello("localhost").unwrap().auth_xoauth2);
    smtp.auth_xoauth2("ada@example.org", "tok.en-123").unwrap();
    assert!(s.finished());
    let sent = s.sent();
    let lines: Vec<&str> = sent.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(bearer(lines[1].strip_prefix("AUTH XOAUTH2 ").unwrap()), "user=ada@example.org\x01auth=Bearer tok.en-123\x01\x01");
}

#[test]
fn a_long_token_waits_for_the_server_to_ask() {
    // Real tokens run to a couple of thousand characters.
    let token = "t".repeat(2_000);
    let mut s = Script::new("250-x\r\n250 AUTH XOAUTH2\r\n334 \r\n235 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    smtp.auth_xoauth2("ada@example.org", &token).unwrap();
    let sent = s.sent();
    let lines: Vec<&str> = sent.lines().collect();
    assert_eq!(lines[1], "AUTH XOAUTH2");
    assert_eq!(bearer(lines[2]), format!("user=ada@example.org\x01auth=Bearer {token}\x01\x01"));

    // The longest that still fits goes in one line.
    let token = "t".repeat(349);
    let mut s = Script::new("250-x\r\n250 AUTH XOAUTH2\r\n235 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    smtp.auth_xoauth2("ada", &token).unwrap();
    let sent = s.sent();
    assert_eq!((sent.lines().count(), sent.lines().nth(1).unwrap().len()), (2, MAX_COMMAND - 1));

    // A server that won't ask is not shown the token.
    let token = "t".repeat(2_000);
    let mut s = Script::new("250-x\r\n250 AUTH XOAUTH2\r\n504 5.5.4 not that way\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert_eq!(smtp.auth_xoauth2("ada", &token), Err(ProtoError::Auth("504 5.5.4 not that way".into())));
    assert_eq!(s.sent(), "EHLO localhost\r\nAUTH XOAUTH2\r\n");
}

#[test]
fn a_refused_token_is_an_auth_error_that_never_shows_it() {
    let payload = xoauth2("ada", "tok.en-123").unwrap();
    let reasons = STANDARD.encode(r#"{"status":"401","schemes":"bearer","scope":"https://mail.example.org/"}"#);
    // The reasons come first and the verdict after an empty line; a careless server repeats what it was sent.
    let mut s = Script::new(format!("250-x\r\n250 AUTH XOAUTH2\r\n334 {reasons}\r\n535-5.7.3 no good: tok.en-123\r\n535 5.7.3 ({payload})\r\n"));
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    let err = smtp.auth_xoauth2("ada", "tok.en-123").unwrap_err();
    assert_eq!(err, ProtoError::Auth("535 5.7.3 no good: *** 5.7.3 (***)".into()));
    assert!(!format!("{err} {err:?} {smtp:?}").contains("tok.en-123"));
    assert_eq!(s.sent(), format!("EHLO localhost\r\nAUTH XOAUTH2 {payload}\r\n\r\n"));
    assert!(s.finished());

    // The same when the token went on a line of its own.
    let token = "t".repeat(2_000);
    let mut s = Script::new(format!("250-x\r\n250 AUTH XOAUTH2\r\n334 \r\n334 {reasons}\r\n535 5.7.3 Authentication unsuccessful\r\n"));
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert_eq!(smtp.auth_xoauth2("ada", &token), Err(ProtoError::Auth("535 5.7.3 Authentication unsuccessful".into())));
    assert_eq!(s.sent(), format!("EHLO localhost\r\nAUTH XOAUTH2\r\n{}\r\n\r\n", xoauth2("ada", &token).unwrap()));
    assert!(s.finished());

    // Refused in one word, without reasons.
    let mut s = Script::new("250-x\r\n250 AUTH XOAUTH2\r\n535 5.7.3 Authentication unsuccessful\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert_eq!(smtp.auth_xoauth2("ada", "tok.en-123"), Err(ProtoError::Auth("535 5.7.3 Authentication unsuccessful".into())));
    assert_eq!(s.sent(), format!("EHLO localhost\r\nAUTH XOAUTH2 {payload}\r\n"));

    // A question after the reasons has no answer: the exchange is cancelled.
    let mut s = Script::new(format!("250-x\r\n250 AUTH XOAUTH2\r\n334 {reasons}\r\n334 more?\r\n501 cancelled\r\n"));
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert!(matches!(smtp.auth_xoauth2("ada", "tok.en-123"), Err(ProtoError::Auth(_))));
    assert!(s.sent().ends_with("\r\n\r\n*\r\n"));
}

#[test]
fn a_token_echoed_in_a_long_reply_is_hidden_whole() {
    // The reply is shortened for the error text: not before the token is out of it.
    let token = format!("head.{}.tail", "x".repeat(2_000));
    let mut s = Script::new(format!("250-x\r\n250 AUTH XOAUTH2\r\n334 \r\n535 5.7.3 {} bad token {token}\r\n", "no ".repeat(80)));
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    let err = smtp.auth_xoauth2("ada", &token).unwrap_err();
    let shown = format!("{err} {err:?}");
    assert!(matches!(err, ProtoError::Auth(_)));
    assert!(!shown.contains("head.") && !shown.contains("xxxx"), "{shown}");
}

#[test]
fn a_token_needs_a_server_that_takes_one_and_sendable_text() {
    let mut s = Script::new("250-x\r\n250 AUTH PLAIN LOGIN\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert_eq!(smtp.auth_xoauth2("ada", "tok.en-123"), Err(ProtoError::Auth(said!("the server offers no way to sign in with an access token"))));
    assert_eq!(s.sent(), "EHLO localhost\r\n");

    let mut s = Script::new("250-x\r\n250 AUTH XOAUTH2\r\n235 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    for (user, token) in
        [("ada", ""), ("ada", "tok\r\nQUIT"), ("ada", "tok\x01"), ("ada\x01auth=Bearer x", "tok"), ("a\0da", "tok"), ("ada\n", "tok")]
    {
        assert!(matches!(smtp.auth_xoauth2(user, token), Err(ProtoError::Auth(_))), "{user:?} {token:?}");
    }
    assert_eq!(s.sent(), "EHLO localhost\r\n");
}

#[test]
fn an_unexpected_second_question_cancels_the_sign_in() {
    let mut s = Script::new("250-x\r\n250 AUTH PLAIN\r\n334 more?\r\n501 cancelled\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    assert!(matches!(smtp.auth("ada", "hunter2"), Err(ProtoError::Auth(_))));
    assert!(s.sent().ends_with("\r\n*\r\n"));
}

#[test]
fn helo_is_the_fallback() {
    let mut s = Script::new("220 old.example.org\r\n502 what?\r\n250 old.example.org\r\n");
    let mut smtp = Smtp::connect(&mut s).unwrap();
    assert_eq!(smtp.hello("localhost").unwrap(), Extensions::default());
    assert_eq!(s.sent(), "EHLO localhost\r\nHELO localhost\r\n");

    // A server on its way out is not asked twice.
    let mut s = Script::new("421 shutting down\r\n");
    assert_eq!(Smtp::resume(&mut s).hello("localhost"), Err(ProtoError::Rejected("421 shutting down".into())));
    assert_eq!(s.sent(), "EHLO localhost\r\n");
}

#[test]
fn a_client_name_that_could_break_the_line_is_replaced() {
    for bad in ["", "two words", "evil\r\nMAIL FROM:<x>", "nön-ascii"] {
        let mut s = Script::new("250 ok\r\n");
        Smtp::resume(&mut s).hello(bad).unwrap();
        assert_eq!(s.sent(), "EHLO localhost\r\n", "{bad:?}");
    }
}

#[test]
fn extensions_are_read_whatever_their_case() {
    let mut s = Script::new("250-x\r\n250-starttls\r\n250-Size\r\n250-auth plain\r\n250 8bitmime\r\n");
    let ext = Smtp::resume(&mut s).hello("localhost").unwrap();
    assert_eq!(ext, Extensions { starttls: true, auth_plain: true, auth_login: false, auth_xoauth2: false, size: Some(0), eight_bit_mime: true });
    let mut s = Script::new("250-x\r\n250 auth=login xoauth2 OAUTHBEARER\r\n");
    let ext = Smtp::resume(&mut s).hello("localhost").unwrap();
    assert_eq!(ext, Extensions { auth_login: true, auth_xoauth2: true, ..Default::default() });
}

#[test]
fn starttls_hands_the_stream_back() {
    let mut s = Script::new("220 hi\r\n250-x\r\n250 STARTTLS\r\n220 go ahead\r\n");
    let mut smtp = Smtp::connect(&mut s).unwrap();
    assert!(smtp.hello("localhost").unwrap().starttls);
    let stream = smtp.starttls().unwrap();
    assert_eq!(stream.sent(), "EHLO localhost\r\nSTARTTLS\r\n");

    let mut s = Script::new("454 TLS not available\r\n");
    assert_eq!(Smtp::resume(&mut s).starttls().unwrap_err(), ProtoError::Rejected("454 TLS not available".into()));
}

#[test]
fn text_slipped_in_before_tls_is_refused() {
    // Both lines arrive in one read, as an attacker in the middle would send them.
    let mut s = Script::whole("220 go ahead\r\n250 injected\r\n");
    assert!(matches!(Smtp::resume(&mut s).starttls(), Err(ProtoError::Protocol(_))));
}

#[test]
fn data_is_normalised_and_dot_stuffed() {
    let message = b"Subject: dots\n\n.leading\r\n..two\rbare CR\n.\nno dot. here\n\nlast line without an ending";
    let mut s = Script::new("250 ok\r\n250 ok\r\n354 go\r\n250 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.send("ada@example.org", &["bob@example.org"], message).unwrap();
    let sent = s.sent();
    let data = sent.split_once("DATA\r\n").unwrap().1;
    assert_eq!(data, "Subject: dots\r\n\r\n..leading\r\n...two\r\nbare CR\r\n..\r\nno dot. here\r\n\r\nlast line without an ending\r\n.\r\n");
    // No SIZE or BODY parameter for a server that didn't offer them.
    assert!(sent.starts_with("MAIL FROM:<ada@example.org>\r\n"));
}

#[test]
fn data_blocks() {
    assert_eq!(data_block(b""), (b".\r\n".to_vec(), 0));
    assert_eq!(data_block(b"a"), (b"a\r\n.\r\n".to_vec(), 3));
    assert_eq!(data_block(b"."), (b"..\r\n.\r\n".to_vec(), 3));
    assert_eq!(data_block(b"\r\r\n\n"), (b"\r\n\r\n\r\n.\r\n".to_vec(), 6));
    assert_eq!(data_block(b"a\r"), (b"a\r\n.\r\n".to_vec(), 3));
}

#[test]
fn eight_bit_mail_says_so_when_the_server_takes_it() {
    let mut s = Script::new("250-x\r\n250 8BITMIME\r\n250 ok\r\n250 ok\r\n354 go\r\n250 ok\r\n250 ok\r\n250 ok\r\n354 go\r\n250 ok\r\n");
    let mut smtp = Smtp::resume(&mut s);
    smtp.hello("localhost").unwrap();
    smtp.send("ada@example.org", &["bob@example.org"], "Subject: é\r\n\r\n".as_bytes()).unwrap();
    smtp.send("", &["bob@example.org"], b"Subject: e\r\n\r\n").unwrap();
    let sent = s.sent();
    assert!(sent.contains("MAIL FROM:<ada@example.org> BODY=8BITMIME\r\n"));
    // An empty sender is the bounce address.
    assert!(sent.contains("MAIL FROM:<>\r\n"));
}

#[test]
fn a_refused_recipient_stops_the_message_and_resets() {
    let mut s = Script::new("250 ok\r\n250 ok\r\n550 5.1.1 no such user\r\n250 reset\r\n");
    let mut smtp = Smtp::resume(&mut s);
    let text = rejected(smtp.send("ada@example.org", &["bob@example.org", "nobody@example.org", "carol@example.org"], b"x"));
    assert_eq!(text, "nobody@example.org: 550 5.1.1 no such user");
    assert!(s.sent().ends_with("RCPT TO:<nobody@example.org>\r\nRSET\r\n"));
}

#[test]
fn refusals_at_each_step() {
    // MAIL FROM.
    let mut s = Script::new("552 5.3.4 too big\r\n");
    assert_eq!(rejected(Smtp::resume(&mut s).send("a@example.org", &["b@example.org"], b"x")), "552 5.3.4 too big");
    // DATA.
    let mut s = Script::new("250 ok\r\n250 ok\r\n554 no\r\n250 reset\r\n");
    assert_eq!(rejected(Smtp::resume(&mut s).send("a@example.org", &["b@example.org"], b"x")), "554 no");
    assert!(s.sent().ends_with("DATA\r\nRSET\r\n"));
    // After the message.
    let mut s = Script::new("250 ok\r\n250 ok\r\n354 go\r\n451 4.3.0 try later\r\n");
    assert_eq!(rejected(Smtp::resume(&mut s).send("a@example.org", &["b@example.org"], b"x")), "451 4.3.0 try later");
    // The greeting.
    let mut s = Script::new("554 no service for you\r\n");
    assert_eq!(Smtp::connect(&mut s).unwrap_err(), ProtoError::Rejected("554 no service for you".into()));
}

#[test]
fn unsafe_addresses_are_never_sent() {
    let cases: [(&str, &[&str]); 6] = [
        ("ada@example.org", &[]),
        ("ada@example.org", &["bob@example.org>\r\nRCPT TO:<eve@example.org"]),
        ("ada@example.org", &["bob smith@example.org"]),
        ("ada@example.org", &[""]),
        ("ada@example.org\r\nDATA", &["bob@example.org"]),
        ("ada@example.org", &["bøb@example.org"]),
    ];
    for (from, to) in cases {
        let mut s = Script::new("250 ok\r\n250 ok\r\n354 go\r\n250 ok\r\n");
        assert!(matches!(Smtp::resume(&mut s).send(from, to, b"x"), Err(ProtoError::Protocol(_))), "{from:?} {to:?}");
        assert_eq!(s.sent(), "", "{from:?} {to:?}");
    }
}

#[test]
fn junk_replies_are_protocol_errors() {
    for junk in ["hello there\r\n", "25 ok\r\n", "250ok\r\n", "250-first\r\n251 second\r\n", "\r\n", "2éé ok\r\n"] {
        let mut s = Script::new(junk);
        assert!(matches!(Smtp::resume(&mut s).hello("localhost"), Err(ProtoError::Protocol(_))), "{junk:?}");
    }
    // A reply that never ends.
    let mut s = Script::whole("250-more\r\n".repeat(MAX_REPLY_LINES + 1));
    assert_eq!(Smtp::resume(&mut s).hello("localhost"), Err(ProtoError::Protocol(said!("the server's reply has too many lines"))));
    // A greeting that is a success code but not 220.
    let mut s = Script::new("250 hello\r\n");
    assert!(matches!(Smtp::connect(&mut s), Err(ProtoError::Protocol(_))));
}

#[test]
fn a_dead_connection_is_an_io_error() {
    let mut s = Script::new("250-half a rep");
    assert_eq!(Smtp::resume(&mut s).hello("localhost"), Err(ProtoError::Io(said!("the server closed the connection"))));
    assert!(matches!(Smtp::connect(Broken(ErrorKind::TimedOut)), Err(ProtoError::Io(_))));
    assert!(matches!(Smtp::resume(Broken(ErrorKind::ConnectionReset)).send("a@example.org", &["b@example.org"], b"x"), Err(ProtoError::Io(_))));
}

#[test]
fn quitting() {
    let mut s = Script::new("221 bye\r\n");
    assert_eq!(Smtp::resume(&mut s).quit(), Ok(()));
    // A server that just hangs up has quit too.
    let mut s = Script::new("");
    assert_eq!(Smtp::resume(&mut s).quit(), Ok(()));
    assert_eq!(s.sent(), "QUIT\r\n");
    let mut s = Script::new("500 what\r\n");
    assert!(matches!(Smtp::resume(&mut s).quit(), Err(ProtoError::Rejected(_))));
}

/// Everything a client does, against whatever the "server" says.
fn drive(server: &[u8]) {
    let mut s = Script::new(server);
    if let Ok(mut smtp) = Smtp::connect(&mut s) {
        let _ = smtp.hello("localhost");
        let _ = smtp.auth("ada@example.org", "hunter2");
        let _ = smtp.send("ada@example.org", &["bob@example.org"], b"Subject: x\r\n\r\n.\r\n");
        let _ = smtp.quit();
    }
    let mut s = Script::new(server);
    let mut smtp = Smtp::resume(&mut s);
    smtp.ext = Extensions { auth_login: true, ..Default::default() };
    let _ = smtp.auth("ada", "hunter2");
    let _ = smtp.starttls();
    // With a token, on one line and on its own.
    for token in ["tok.en-123".to_string(), "t".repeat(700)] {
        let mut s = Script::new(server);
        let mut smtp = Smtp::resume(&mut s);
        smtp.ext = Extensions { auth_xoauth2: true, ..Default::default() };
        let _ = smtp.auth_xoauth2("ada@example.org", &token);
        let _ = smtp.send("ada@example.org", &["bob@example.org"], b"x");
    }
}

proptest! {
    #[test]
    fn arbitrary_server_bytes_never_panic_or_hang(server in proptest::collection::vec(any::<u8>(), 0..600)) {
        drive(&server);
    }

    #[test]
    fn reply_shaped_noise_never_panics(lines in proptest::collection::vec(("[0-9]{3}", "[- ]?", "[ -~]{0,30}"), 0..40)) {
        let server: String = lines.iter().map(|(code, sep, text)| format!("{code}{sep}{text}\r\n")).collect();
        drive(server.as_bytes());
    }

    #[test]
    fn data_blocks_never_end_early(message in proptest::collection::vec(prop_oneof![Just(b'.'), Just(b'\r'), Just(b'\n'), any::<u8>()], 0..300)) {
        let (data, size) = data_block(&message);
        prop_assert!(data.ends_with(b".\r\n"));
        let body = &data[..data.len() - 3];
        prop_assert!(body.is_empty() || body.ends_with(b"\r\n"));
        // No bare CR or LF, and no line that is just a dot before the end.
        let text = String::from_utf8_lossy(body);
        prop_assert_eq!(body.iter().filter(|b| **b == b'\r').count(), body.iter().filter(|b| **b == b'\n').count());
        prop_assert!(!text.contains("\r\n.\r\n") && !text.starts_with(".\r\n"));
        prop_assert_eq!(body.windows(2).filter(|w| w[0] == b'\r' && w[1] == b'\n').count(), body.iter().filter(|b| **b == b'\n').count());
        // Taking the stuffing out gives the size that was announced.
        let unstuffed: usize = body.split_inclusive(|b| *b == b'\n').map(|l| l.len() - usize::from(l.starts_with(b"."))).sum();
        prop_assert_eq!(unstuffed, size);
    }
}
