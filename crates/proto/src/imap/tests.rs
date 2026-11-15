// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use std::io::ErrorKind;

use proptest::prelude::*;

use super::*;
use crate::script::{Broken, Script};

fn mailbox(name: &str, raw: &str, delimiter: Option<char>, flags: &[&str]) -> Mailbox {
    Mailbox { name: name.into(), raw: raw.into(), delimiter, flags: flags.iter().map(|f| f.to_string()).collect() }
}

fn fetched(uid: u32, flags: &[&str]) -> Fetched {
    Fetched { uid, flags: flags.iter().map(|f| f.to_string()).collect(), internal_date: None, body: None, gmail_id: None, gmail_labels: None }
}

/// A FETCH response carrying `body` as a literal, between `before` and `after`.
fn with_literal(before: &str, body: &[u8], after: &str) -> Vec<u8> {
    let mut out = format!("{before}{{{}}}\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out.extend_from_slice(after.as_bytes());
    out
}

#[test]
fn greetings() {
    let mut s = Script::new("* OK [CAPABILITY IMAP4rev1 STARTTLS] mail.example.org ready\r\n");
    let imap = Imap::connect(&mut s).unwrap();
    assert!(!imap.is_authenticated());
    assert_eq!(imap.capabilities, Some(vec!["IMAP4REV1".to_string(), "STARTTLS".to_string()]));

    let mut s = Script::new("* PREAUTH welcome back\r\n");
    assert!(Imap::connect(&mut s).unwrap().is_authenticated());

    let mut s = Script::new("* BYE overloaded, try later\r\n");
    assert_eq!(Imap::connect(&mut s).unwrap_err(), ProtoError::Rejected("overloaded, try later".into()));

    for junk in ["hello\r\n", "+ go on\r\n", "* 3 EXISTS\r\n", "A0001 OK early\r\n", "* NO\r\n", "\r\n", "*\r\n", "220 smtp.example.org ESMTP\r\n"] {
        let mut s = Script::new(junk);
        assert!(matches!(Imap::connect(&mut s), Err(ProtoError::Protocol(_))), "{junk:?}");
    }
    let mut s = Script::new("");
    assert!(matches!(Imap::connect(&mut s), Err(ProtoError::Io(_))));
}

#[test]
fn capabilities_are_upper_cased() {
    let mut s = Script::new(
        "* CAPABILITY IMAP4rev1 StartTLS special-use AUTH=PLAIN\r\nA0001 OK done\r\nA0002 OK [CAPABILITY IMAP4rev1 IDLE] done\r\nA0003 OK none\r\n",
    );
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.capabilities().unwrap(), ["IMAP4REV1", "STARTTLS", "SPECIAL-USE", "AUTH=PLAIN"]);
    // Some servers answer in the tagged line.
    assert_eq!(imap.capabilities().unwrap(), ["IMAP4REV1", "IDLE"]);
    assert_eq!(imap.capabilities().unwrap(), Vec::<String>::new());
    assert_eq!(s.sent(), "A0001 CAPABILITY\r\nA0002 CAPABILITY\r\nA0003 CAPABILITY\r\n");
}

#[test]
fn login_quotes_its_strings() {
    let mut s = Script::new("A0001 OK [CAPABILITY IMAP4rev1] welcome\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.login(r#"ada "the" \ lovelace"#, r#"p"w\d {5}"#).unwrap();
    assert!(imap.is_authenticated());
    assert_eq!(s.sent(), "A0001 LOGIN \"ada \\\"the\\\" \\\\ lovelace\" \"p\\\"w\\\\d {5}\"\r\n");
}

#[test]
fn login_sends_what_quotes_cannot_carry_as_literals() {
    let mut s = Script::new("+ go on\r\nA0001 OK welcome\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.login("ada", "pässwörd").unwrap();
    assert_eq!(s.sent(), "A0001 LOGIN \"ada\" {10}\r\npässwörd\r\n");

    // Both as literals, each after its own go-ahead; a line break can't end the command early.
    let mut s = Script::new("+ go on\r\n+ go on\r\nA0001 OK welcome\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.login("ädä", "two\r\nA0002 LOGOUT").unwrap();
    assert_eq!(s.sent(), "A0001 LOGIN {5}\r\nädä {17}\r\ntwo\r\nA0002 LOGOUT\r\n");

    // A very long password too.
    let long = "x".repeat(MAX_QUOTED + 1);
    let mut s = Script::new("+ go on\r\nA0001 OK welcome\r\n");
    Imap::resume(&mut s).login("ada", &long).unwrap();
    assert!(s.sent().starts_with("A0001 LOGIN \"ada\" {1001}\r\nxxx"));
}

#[test]
fn a_wrong_password_is_an_auth_error_that_never_shows_it() {
    // A careless server repeats what it was sent.
    let mut s = Script::new("A0001 NO [AUTHENTICATIONFAILED] no user ada with password hunter2\r\n");
    let mut imap = Imap::resume(&mut s);
    let err = imap.login("ada", "hunter2").unwrap_err();
    assert_eq!(err, ProtoError::Auth("[AUTHENTICATIONFAILED] no user ada with password ***".into()));
    assert!(!imap.is_authenticated());
    assert!(!format!("{err} {err:?} {imap:?}").contains("hunter2"));

    // BAD is not about the password.
    let mut s = Script::new("A0001 BAD what\r\n");
    assert_eq!(Imap::resume(&mut s).login("ada", "hunter2"), Err(ProtoError::Rejected("what".into())));

    // Refused before the literal went out: the password is never sent.
    let mut s = Script::new("A0001 NO no literals here\r\n");
    assert!(matches!(Imap::resume(&mut s).login("ada", "pässwörd"), Err(ProtoError::Auth(_))));
    assert_eq!(s.sent(), "A0001 LOGIN \"ada\" {10}\r\n");

    // Nothing at all is sent when it can't be.
    let mut s = Script::new("A0001 OK welcome\r\n");
    assert!(matches!(Imap::resume(&mut s).login("ada", "nul\0"), Err(ProtoError::Auth(_))));
    assert_eq!(s.sent(), "");
}

#[test]
fn no_password_for_a_server_that_disabled_login() {
    let mut s = Script::new("* OK [CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED] ready\r\nA0001 OK welcome\r\n");
    let mut imap = Imap::connect(&mut s).unwrap();
    assert!(matches!(imap.login("ada", "hunter2"), Err(ProtoError::Auth(_))));
    assert_eq!(s.sent(), "");
}

const TOKEN: &str = "tok.en-123";

/// What goes out for `ada@example.org` with [`TOKEN`].
fn bearer() -> String {
    let payload = xoauth2("ada@example.org", TOKEN).unwrap();
    assert_eq!(payload, "dXNlcj1hZGFAZXhhbXBsZS5vcmcBYXV0aD1CZWFyZXIgdG9rLmVuLTEyMwEB");
    payload
}

#[test]
fn a_token_goes_with_the_command_when_the_server_takes_it_that_way() {
    let mut s = Script::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN AUTH=XOAUTH2] ready\r\nA0001 OK [CAPABILITY IMAP4rev1 IDLE] welcome\r\n");
    let mut imap = Imap::connect(&mut s).unwrap();
    imap.authenticate_xoauth2("ada@example.org", TOKEN).unwrap();
    assert!(imap.is_authenticated());
    // What the server can do once signed in is noted.
    assert_eq!(imap.capabilities, Some(vec!["IMAP4REV1".to_string(), "IDLE".to_string()]));
    assert_eq!(s.sent(), format!("A0001 AUTHENTICATE XOAUTH2 {}\r\n", bearer()));
    assert!(s.finished());
}

#[test]
fn a_token_waits_for_the_go_ahead_otherwise() {
    // A server that hasn't said what it can do, one without SASL-IR, and one that has SASL-IR
    // but didn't list the mechanism: each is asked first.
    for greeting in ["", "* OK [CAPABILITY IMAP4rev1 AUTH=XOAUTH2] ready\r\n", "* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] ready\r\n"] {
        let mut s = Script::new(format!("{greeting}+ \r\n* CAPABILITY IMAP4rev1 IDLE\r\nA0001 OK welcome\r\n"));
        let mut imap = if greeting.is_empty() { Imap::resume(&mut s) } else { Imap::connect(&mut s).unwrap() };
        imap.authenticate_xoauth2("ada@example.org", TOKEN).unwrap();
        assert!(imap.is_authenticated(), "{greeting:?}");
        assert_eq!(s.sent(), format!("A0001 AUTHENTICATE XOAUTH2\r\n{}\r\n", bearer()), "{greeting:?}");
    }
}

#[test]
fn a_refused_token_is_an_auth_error_that_never_shows_it() {
    let payload = bearer();
    let caps = "* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=XOAUTH2] ready\r\n";
    // The reasons come first (JSON in base64) and the verdict after an empty line; a careless
    // server repeats what it was sent.
    let reasons = "+ eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiYmVhcmVyIiwic2NvcGUiOiJodHRwczovL21haWwuZXhhbXBsZS5vcmcvIn0=\r\n";
    let mut s = Script::new(format!("{caps}{reasons}A0001 NO [AUTHENTICATIONFAILED] no good: {TOKEN} ({payload})\r\n"));
    let mut imap = Imap::connect(&mut s).unwrap();
    let err = imap.authenticate_xoauth2("ada@example.org", TOKEN).unwrap_err();
    assert_eq!(err, ProtoError::Auth("[AUTHENTICATIONFAILED] no good: *** (***)".into()));
    assert!(!imap.is_authenticated());
    assert!(!format!("{err} {err:?} {imap:?}").contains(TOKEN));
    assert_eq!(s.sent(), format!("A0001 AUTHENTICATE XOAUTH2 {payload}\r\n\r\n"));
    assert!(s.finished());

    // The same after a go-ahead.
    let mut s = Script::new(format!("+ \r\n{reasons}A0001 NO AUTHENTICATE failed.\r\n"));
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.authenticate_xoauth2("ada@example.org", TOKEN), Err(ProtoError::Auth("AUTHENTICATE failed.".into())));
    assert_eq!(s.sent(), format!("A0001 AUTHENTICATE XOAUTH2\r\n{payload}\r\n\r\n"));

    // Refused in one word, without reasons.
    let mut s = Script::new("+ \r\nA0001 NO AUTHENTICATE failed.\r\n");
    assert_eq!(Imap::resume(&mut s).authenticate_xoauth2("ada@example.org", TOKEN), Err(ProtoError::Auth("AUTHENTICATE failed.".into())));
    assert_eq!(s.sent(), format!("A0001 AUTHENTICATE XOAUTH2\r\n{payload}\r\n"));

    // A token echoed in a long answer is hidden before the answer is shortened.
    let long = format!("head.{}.tail", "x".repeat(2_000));
    let mut s = Script::new(format!("+ \r\nA0001 NO {} bad token {long}\r\n", "no ".repeat(80)));
    let err = Imap::resume(&mut s).authenticate_xoauth2("ada@example.org", &long).unwrap_err();
    let shown = format!("{err} {err:?}");
    assert!(matches!(err, ProtoError::Auth(_)));
    assert!(!shown.contains("head.") && !shown.contains("xxxx"), "{shown}");
}

#[test]
fn a_server_without_the_mechanism_never_sees_the_token() {
    for (server, expected) in [
        ("A0001 NO [CANNOT] unsupported authentication mechanism\r\n", ProtoError::Auth("[CANNOT] unsupported authentication mechanism".into())),
        ("A0001 BAD what\r\n", ProtoError::Rejected("what".into())),
        // "Signed in" without having been shown anything can't be true.
        ("A0001 OK welcome\r\n", ProtoError::Protocol(said!("the server ended the command early"))),
    ] {
        let mut s = Script::new(format!("* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] ready\r\n{server}"));
        let mut imap = Imap::connect(&mut s).unwrap();
        assert_eq!(imap.authenticate_xoauth2("ada@example.org", TOKEN), Err(expected), "{server:?}");
        assert!(!imap.is_authenticated());
        assert_eq!(s.sent(), "A0001 AUTHENTICATE XOAUTH2\r\n", "{server:?}");
    }
}

#[test]
fn questions_a_token_has_no_answer_to_end_the_sign_in() {
    // After the reasons, another question: the exchange is cancelled.
    let mut s = Script::new("+ \r\n+ e30=\r\n+ more?\r\nA0001 BAD cancelled\r\n");
    let mut imap = Imap::resume(&mut s);
    assert!(matches!(imap.authenticate_xoauth2("ada@example.org", TOKEN), Err(ProtoError::Auth(_))));
    assert!(s.sent().ends_with("\r\n\r\n*\r\n"));
    // A server that calls a cancelled exchange a success is not believed.
    let mut s = Script::new("+ \r\n+ e30=\r\n+ more?\r\nA0001 OK welcome\r\n");
    let mut imap = Imap::resume(&mut s);
    assert!(matches!(imap.authenticate_xoauth2("ada@example.org", TOKEN), Err(ProtoError::Auth(_))));
    assert!(!imap.is_authenticated());
    // One that never stops asking.
    let mut s = Script::new("+ \r\n".repeat(50));
    assert!(matches!(Imap::resume(&mut s).authenticate_xoauth2("ada@example.org", TOKEN), Err(ProtoError::Protocol(_))));
    assert_eq!(s.sent().matches("\r\n").count(), 4);
}

#[test]
fn a_token_or_name_that_cannot_be_sent_is_refused() {
    let mut s = Script::new("+ \r\nA0001 OK welcome\r\n");
    let mut imap = Imap::resume(&mut s);
    for (user, token) in
        [("ada", ""), ("ada", "tok\r\nA0002 LOGOUT"), ("ada", "tok\x01"), ("ada\x01auth=Bearer x", "tok"), ("a\0da", "tok"), ("ada\n", "tok")]
    {
        assert!(matches!(imap.authenticate_xoauth2(user, token), Err(ProtoError::Auth(_))), "{user:?} {token:?}");
    }
    assert!(!imap.is_authenticated());
    assert_eq!(s.sent(), "");
}

#[test]
fn mailboxes_are_listed_with_names_in_every_form() {
    let mut server = String::new();
    server += "* LIST (\\HasNoChildren) \"/\" INBOX\r\n";
    server += "* LIST (\\HasNoChildren \\Sent) \"/\" \"Sent Items\"\r\n";
    server += "* LIST (\\Noselect \\HasChildren) \"/\" \"[Gmail]\"\r\n";
    server += "* LIST (\\HasNoChildren \\Trash) \"/\" [Gmail]/Trash\r\n";
    server += "* LIST () NIL \"Entw&APw-rfe\"\r\n";
    server += "* LIST (\\HasNoChildren) \".\" {12}\r\nINBOX.&Jjo-!\r\n";
    server += "* LIST () \"/\" \"quoted \\\"name\\\" \\\\ here\"\r\n";
    server += "* LSUB () \"/\" subscribed\r\n";
    server += "* 5 EXISTS\r\n";
    server += "* LIST (\\HasNoChildren) \"/\" INBOX\r\n";
    server += "* LIST broken\r\n";
    server += "A0001 OK done\r\n";
    let mut s = Script::new(server);
    let list = Imap::resume(&mut s).list().unwrap();
    assert_eq!(s.sent(), "A0001 LIST \"\" \"*\"\r\n");
    assert_eq!(
        list,
        [
            mailbox("INBOX", "INBOX", Some('/'), &["\\HasNoChildren"]),
            mailbox("Sent Items", "Sent Items", Some('/'), &["\\HasNoChildren", "\\Sent"]),
            mailbox("[Gmail]", "[Gmail]", Some('/'), &["\\Noselect", "\\HasChildren"]),
            mailbox("[Gmail]/Trash", "[Gmail]/Trash", Some('/'), &["\\HasNoChildren", "\\Trash"]),
            mailbox("Entwürfe", "Entw&APw-rfe", None, &[]),
            mailbox("INBOX.☺!", "INBOX.&Jjo-!", Some('.'), &["\\HasNoChildren"]),
            mailbox("quoted \"name\" \\ here", "quoted \"name\" \\ here", Some('/'), &[]),
        ]
    );
    assert!(list[2].has_flag("\\NOSELECT") && !list[0].has_flag("\\Noselect"));
}

#[test]
fn opening_a_mailbox() {
    let mut server = String::new();
    server += "* 172 EXISTS\r\n* 1 RECENT\r\n* OK [UNSEEN 12] Message 12 is first unseen\r\n";
    server += "* OK [UIDVALIDITY 3857529045] UIDs valid\r\n* OK [UIDNEXT 4392] Predicted next UID\r\n";
    server += "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK [PERMANENTFLAGS ()] No permanent flags permitted\r\n";
    server += "A0001 OK [READ-ONLY] EXAMINE completed\r\n";
    // The next one is empty and says little.
    server += "* 0 EXISTS\r\n* OK [UIDVALIDITY 7]\r\nA0002 OK [READ-WRITE] SELECT completed\r\n";
    server += "A0003 NO [NONEXISTENT] no such mailbox\r\n";
    server += "+ go on\r\n* 3 EXISTS\r\n* OK [UIDVALIDITY 99999999999] too large to be one\r\nA0004 OK done\r\n";
    let mut s = Script::new(server);
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.select("Sent Items", true).unwrap(), Selected { exists: 172, uid_validity: 3_857_529_045, uid_next: 4392 });
    assert_eq!(imap.select("INBOX", false).unwrap(), Selected { exists: 0, uid_validity: 7, uid_next: 0 });
    assert_eq!(imap.select("Nowhere", true), Err(ProtoError::Rejected("[NONEXISTENT] no such mailbox".into())));
    // A name the server wrote in UTF-8 goes back as a literal.
    assert_eq!(imap.select("Entwürfe", true).unwrap(), Selected { exists: 3, uid_validity: 0, uid_next: 0 });
    assert_eq!(s.sent(), "A0001 EXAMINE \"Sent Items\"\r\nA0002 SELECT \"INBOX\"\r\nA0003 EXAMINE \"Nowhere\"\r\nA0004 EXAMINE {9}\r\nEntwürfe\r\n");
}

#[test]
fn flags_of_every_message_with_other_news_in_between() {
    let mut server = String::new();
    server += "* 1 FETCH (UID 4 FLAGS (\\Seen \\Answered))\r\n";
    server += "* 2 FETCH (FLAGS (\\Flagged $Forwarded) UID 7)\r\n";
    server += "* 3 EXPUNGE\r\n* 9 EXISTS\r\n* 1 RECENT\r\n* FLAGS (\\Seen)\r\n";
    // Text for people may end like a literal without being one.
    server += "* OK [ALERT] still here {5}\r\n";
    // Without a UID there is no telling which message this is.
    server += "* 4 FETCH (FLAGS (\\Deleted))\r\n";
    server += "* 3 FETCH (UID 9 FLAGS ())\r\n";
    // The later word about a message counts.
    server += "* 2 FETCH (UID 7 FLAGS (\\Seen))\r\n";
    // A message named without its flags is there all the same, and one named again keeps the flags it had.
    server += "* 5 FETCH (UID 2 RFC822.SIZE 100)\r\n";
    server += "* 1 FETCH (UID 4)\r\n";
    server += "A0001 OK done\r\n";
    let mut s = Script::new(server);
    let got = Imap::resume(&mut s).uid_fetch_flags("1:*").unwrap();
    assert_eq!(got, [fetched(2, &[]), fetched(4, &["\\Seen", "\\Answered"]), fetched(7, &["\\Seen"]), fetched(9, &[])]);
    assert_eq!(s.sent(), "A0001 UID FETCH 1:* (UID FLAGS)\r\n");

    let mut s = Script::new("A0001 OK nothing there\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_flags("5,9,12:20").unwrap(), []);
    assert_eq!(s.sent(), "A0001 UID FETCH 5,9,12:20 (UID FLAGS)\r\n");
}

#[test]
fn sizes() {
    let mut s =
        Script::new("* 2 FETCH (RFC822.SIZE 99999999999 UID 9)\r\n* 1 FETCH (UID 4 RFC822.SIZE 2048)\r\n* 3 FETCH (UID 5)\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_sizes("4:9").unwrap(), [(4, 2048), (9, 99_999_999_999)]);
    assert_eq!(s.sent(), "A0001 UID FETCH 4:9 (UID RFC822.SIZE)\r\n");
}

#[test]
fn sets_that_are_not_sets_are_never_sent() {
    for bad in ["", "1:* (BODY[])", "1 2", "a", "1:*\r\nA0002 LOGOUT"] {
        let mut s = Script::new("A0001 OK done\r\n");
        let mut imap = Imap::resume(&mut s);
        assert!(matches!(imap.uid_fetch_flags(bad), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert!(matches!(imap.uid_fetch_sizes(bad), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert_eq!(s.sent(), "", "{bad:?}");
    }
}

#[test]
fn a_message_arrives_whole_whatever_is_in_it() {
    // A body full of things that look like protocol.
    let body = b"Subject: Hi\r\n\r\nA0001 OK fake\r\n* 1 FETCH (UID 2)\r\n{5}\r\n\"(\\\r\n)";
    let mut server = with_literal("* 5 FETCH (UID 12 FLAGS (\\Seen $Forwarded) INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" BODY[] ", body, ")\r\n");
    server.extend_from_slice(b"A0001 OK done\r\n");
    let mut s = Script::new(server);
    let got = Imap::resume(&mut s).uid_fetch_message(12).unwrap().unwrap();
    assert_eq!(
        got,
        Fetched {
            uid: 12,
            flags: vec!["\\Seen".into(), "$Forwarded".into()],
            internal_date: Some("17-Jul-1996 02:44:25 -0700".into()),
            body: Some(body.to_vec()),
            gmail_id: None,
            gmail_labels: None
        }
    );
    assert_eq!(s.sent(), "A0001 UID FETCH 12 (UID FLAGS INTERNALDATE BODY.PEEK[])\r\n");
    assert!(s.finished());
}

#[test]
fn fetch_items_come_in_any_order() {
    let body = b"From: ada@example.org\r\n\r\nBody\r\n";
    let mut server = with_literal(
        "* 5 FETCH (BODY[] ",
        body,
        " INTERNALDATE \" 7-Jul-1996 02:44:25 +0000\" RFC822.SIZE 31 FLAGS () UID 12 X-UNKNOWN (a (b)))\r\n",
    );
    server.extend_from_slice(b"A0001 OK done\r\n");
    let mut s = Script::new(server);
    let got = Imap::resume(&mut s).uid_fetch_message(12).unwrap().unwrap();
    assert_eq!(
        (got.uid, got.flags.len(), got.internal_date.as_deref(), got.body.as_deref()),
        (12, 0, Some(" 7-Jul-1996 02:44:25 +0000"), Some(&body[..]))
    );
}

#[test]
fn bodies_at_the_edges() {
    // Empty, as a literal of no bytes.
    let mut s = Script::new("* 1 FETCH (UID 3 BODY[] {0}\r\n)\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3).unwrap().unwrap().body, Some(Vec::new()));
    // Short, in quotes.
    let mut s = Script::new("* 1 FETCH (UID 3 BODY[] \"tiny \\\"body\\\"\")\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3).unwrap().unwrap().body, Some(b"tiny \"body\"".to_vec()));
    // Under its old name.
    let mut s = Script::new("* 1 FETCH (UID 3 RFC822 {2}\r\nok)\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3).unwrap().unwrap().body, Some(b"ok".to_vec()));
    // Flags in one response, the body in another.
    let mut s = Script::new("* 1 FETCH (UID 3 FLAGS (\\Seen))\r\n* 1 FETCH (UID 3 BODY[] {2}\r\nok)\r\nA0001 OK done\r\n");
    let got = Imap::resume(&mut s).uid_fetch_message(3).unwrap().unwrap();
    assert_eq!((got.flags, got.body), (vec!["\\Seen".to_string()], Some(b"ok".to_vec())));
    // A megabyte in one piece.
    let big = vec![b'x'; 1 << 20];
    let mut server = with_literal("* 1 FETCH (UID 3 BODY[] ", &big, ")\r\n");
    server.extend_from_slice(b"A0001 OK done\r\n");
    let mut s = Script::whole(server);
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3).unwrap().unwrap().body, Some(big));
}

#[test]
fn a_message_that_is_gone_is_none() {
    for server in [
        "A0001 OK nothing\r\n",
        "* 1 FETCH (UID 3 BODY[] NIL)\r\nA0001 OK done\r\n",
        "* 1 FETCH (UID 3 FLAGS (\\Seen))\r\nA0001 OK done\r\n",
        "* 1 FETCH (UID 4 BODY[] {2}\r\nno)\r\n* 2 EXPUNGE\r\nA0001 OK done\r\n",
        "* 1 FETCH (BODY[] {2}\r\nno)\r\nA0001 OK done\r\n",
    ] {
        let mut s = Script::new(server);
        assert_eq!(Imap::resume(&mut s).uid_fetch_message(3), Ok(None), "{server:?}");
    }
    // There is no message 0 to ask for.
    let mut s = Script::new("");
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(0), Ok(None));
    assert_eq!(s.sent(), "");
    let mut s = Script::new("A0001 NO [SERVERBUG] can't read it\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3), Err(ProtoError::Rejected("[SERVERBUG] can't read it".into())));
}

#[test]
fn a_header_arrives_alone() {
    let header = b"From: ada@example.org\r\nSubject: Sixty megabytes\r\n\r\n";
    let mut server = with_literal("* 5 FETCH (UID 12 FLAGS (\\Seen) INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" BODY[HEADER] ", header, ")\r\n");
    server.extend_from_slice(b"A0001 OK done\r\n");
    let mut s = Script::new(server);
    let got = Imap::resume(&mut s).uid_fetch_header(12).unwrap().unwrap();
    assert_eq!(
        got,
        Fetched {
            uid: 12,
            flags: vec!["\\Seen".into()],
            internal_date: Some("17-Jul-1996 02:44:25 -0700".into()),
            body: Some(header.to_vec()),
            gmail_id: None,
            gmail_labels: None
        }
    );
    assert_eq!(s.sent(), "A0001 UID FETCH 12 (UID FLAGS INTERNALDATE BODY.PEEK[HEADER])\r\n");
    assert!(s.finished());
    // Under its old name, and empty.
    let mut s = Script::new("* 1 FETCH (UID 3 RFC822.HEADER {2}\r\n\r\n)\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_header(3).unwrap().unwrap().body, Some(b"\r\n".to_vec()));
    // A server that sends the whole message when asked for its header is not taken at its word,
    // nor is a header taken for a message.
    let mut s = Script::new("* 1 FETCH (UID 3 BODY[] {2}\r\nok)\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_header(3), Ok(None));
    let mut s = Script::new("* 1 FETCH (UID 3 BODY[HEADER] {2}\r\nok)\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3), Ok(None));
    // Gone, never there, refused.
    let mut s = Script::new("A0001 OK nothing\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_header(3), Ok(None));
    let mut s = Script::new("");
    assert_eq!(Imap::resume(&mut s).uid_fetch_header(0), Ok(None));
    assert_eq!(s.sent(), "");
    let mut s = Script::new("A0001 NO [SERVERBUG] can't read it\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_header(3), Err(ProtoError::Rejected("[SERVERBUG] can't read it".into())));
    // A header as large as no header is: refused unread, like a message.
    let mut s = Script::new(format!("* 1 FETCH (UID 3 BODY[HEADER] {{{}}}\r\nxxxx", MAX_LITERAL + 1));
    assert!(matches!(Imap::resume(&mut s).uid_fetch_header(3), Err(ProtoError::Protocol(_))));
}

#[test]
fn the_stream_can_be_reached_between_commands() {
    let mut s = Script::new("A0001 OK done\r\nA0002 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.select("INBOX", true).unwrap();
    // Looked at, not read from: the session goes on where it was.
    assert_eq!(imap.get_mut().sent(), "A0001 EXAMINE \"INBOX\"\r\n");
    imap.select("Sent", true).unwrap();
    assert!(s.finished());
}

#[test]
fn oversized_literals_are_refused_unread() {
    for size in [(MAX_LITERAL + 1).to_string(), "99999999999999999999999999".to_string(), u64::MAX.to_string()] {
        let mut s = Script::new(format!("* 1 FETCH (UID 3 BODY[] {{{size}}}\r\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"));
        let err = Imap::resume(&mut s).uid_fetch_message(3).unwrap_err();
        assert!(matches!(&err, ProtoError::Protocol(text) if text.to_string().contains("64 MB")), "{err:?}");
        // The x's were not read as if they were the message.
        assert!(!s.finished());
    }
    // Several literals that are only too much together.
    let mut s = Script::new(format!("* 1 FETCH (A {{2}}\r\nab B {{{}}}\r\n", MAX_LITERAL - 1));
    assert!(matches!(Imap::resume(&mut s).uid_fetch_message(3), Err(ProtoError::Protocol(_))));
}

#[test]
fn responses_without_end_are_refused() {
    // More literals than any response has.
    let mut s = Script::whole(format!("* 1 FETCH ({}", "X {0}\r\n ".repeat(MAX_LITERALS + 2)));
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3), Err(ProtoError::Protocol(said!("the server's response has too many parts"))));
    // More text than any response has.
    let line = format!("{} {{0}}\r\n", "x".repeat(MAX_LINE - 100));
    let mut s = Script::whole(format!("* 1 FETCH ({}", line.repeat(6)));
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3), Err(ProtoError::Protocol(said!("the server's response is too long"))));
    // A line that never ends.
    let mut s = Script::whole(format!("* 1 FETCH ({}", "x".repeat(MAX_LINE * 2)));
    assert!(matches!(Imap::resume(&mut s).uid_fetch_message(3), Err(ProtoError::Protocol(_))));
    // Lists inside lists inside lists.
    let mut s = Script::new(format!("* 1 FETCH {}\r\nA0001 OK done\r\n", "(".repeat(200)));
    assert_eq!(Imap::resume(&mut s).uid_fetch_message(3), Err(ProtoError::Protocol(said!("the server's response is nested too deeply"))));
}

#[test]
fn storing_flags() {
    let mut s = Script::new("A0001 OK done\r\nA0002 OK done\r\nA0003 NO [READ-ONLY] mailbox is read-only\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.uid_store(7, true, &["\\Seen", "\\Flagged"]).unwrap();
    imap.uid_store(7, false, &["$Forwarded"]).unwrap();
    assert_eq!(imap.uid_store(8, true, &["\\Deleted"]), Err(ProtoError::Rejected("[READ-ONLY] mailbox is read-only".into())));
    assert_eq!(
        s.sent(),
        "A0001 UID STORE 7 +FLAGS.SILENT (\\Seen \\Flagged)\r\nA0002 UID STORE 7 -FLAGS.SILENT ($Forwarded)\r\nA0003 UID STORE 8 +FLAGS.SILENT (\\Deleted)\r\n"
    );

    // Nothing to store, or nothing to store it on: the server isn't asked.
    let mut s = Script::new("A0001 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.uid_store(7, true, &[]), Ok(()));
    assert!(matches!(imap.uid_store(0, true, &["\\Seen"]), Err(ProtoError::Protocol(_))));
    assert_eq!(s.sent(), "");

    for bad in ["", "\\", "two words", "\\Seen) (\\Deleted", "\\\\Seen", "a\r\nb", "sée"] {
        let mut s = Script::new("A0001 OK done\r\n");
        let mut imap = Imap::resume(&mut s);
        assert!(matches!(imap.uid_store(7, true, &[bad]), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert!(matches!(imap.append("Sent", &["\\Seen", bad], b"x"), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert_eq!(s.sent(), "", "{bad:?}");
    }
}

#[test]
fn flags_are_stored_for_many_messages_at_once() {
    let mut s = Script::new("A0001 OK done\r\nA0002 OK done\r\nA0003 NO [READ-ONLY] mailbox is read-only\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.uid_store_set("4,7:9,12", true, &["\\Seen"]).unwrap();
    imap.uid_store_set("5", false, &["\\Flagged", "\\Answered"]).unwrap();
    assert_eq!(imap.uid_store_set("6", true, &["\\Deleted"]), Err(ProtoError::Rejected("[READ-ONLY] mailbox is read-only".into())));
    assert_eq!(
        s.sent(),
        "A0001 UID STORE 4,7:9,12 +FLAGS.SILENT (\\Seen)\r\nA0002 UID STORE 5 -FLAGS.SILENT (\\Flagged \\Answered)\r\nA0003 UID STORE 6 +FLAGS.SILENT (\\Deleted)\r\n"
    );
}

#[test]
fn a_set_for_a_change_names_its_messages() {
    // Nothing that reaches for messages nobody looked at, and nothing that isn't a set at all.
    let long = format!("1:{}", MAX_SET_UIDS + 1);
    for bad in ["1:*", "*", "", "0", "5:3", "1,,2", "1:2:3", "4 5", "+4", "4,", "4\r\nA0002 EXPUNGE", "4294967296", long.as_str()] {
        let mut s = Script::new("A0001 OK done\r\n");
        let mut imap = Imap::resume(&mut s);
        imap.capabilities = Some(vec!["UIDPLUS".into(), "MOVE".into()]);
        assert!(matches!(imap.uid_store_set(bad, true, &["\\Deleted"]), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert!(matches!(imap.uid_move(bad, "Archive"), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert!(matches!(imap.uid_copy(bad, "Archive"), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert!(matches!(imap.uid_expunge(bad), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert_eq!(s.sent(), "", "{bad:?}");
    }
    // Reading may ask for everything.
    let mut s = Script::new("A0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_flags("1:*").unwrap(), []);
}

#[test]
fn moving_and_copying_say_where_the_messages_went() {
    // A copy answers in its last line, a move before the messages leave (RFC 6851).
    let mut server = String::new();
    server += "A0001 OK [COPYUID 38505 304,319:320 3956:3958] Done\r\n";
    server += "* OK [COPYUID 432432 42:43 1202:1203] moved\r\n* 2 EXPUNGE\r\n* 1 EXPUNGE\r\nA0002 OK Done\r\n";
    // Nothing matched, or a server without UIDPLUS: done, and nothing known.
    server += "A0003 OK No messages\r\nA0004 OK done\r\n";
    server += "A0005 NO [TRYCREATE] no such mailbox\r\n";
    let mut s = Script::new(server);
    let mut imap = Imap::resume(&mut s);
    assert_eq!(
        imap.uid_copy("304,319:320", "Archive").unwrap(),
        Some(Copied { uid_validity: 38_505, uids: vec![(304, 3956), (319, 3957), (320, 3958)] })
    );
    assert_eq!(imap.uid_move("42:43", "Entw&APw-rfe").unwrap(), Some(Copied { uid_validity: 432_432, uids: vec![(42, 1202), (43, 1203)] }));
    assert_eq!(imap.uid_move("99", "Archive"), Ok(None));
    assert_eq!(imap.uid_copy("7", "Sent \"old\" mail"), Ok(None));
    assert_eq!(imap.uid_copy("7", "Nowhere"), Err(ProtoError::Rejected("[TRYCREATE] no such mailbox".into())));
    assert_eq!(
        s.sent(),
        "A0001 UID COPY 304,319:320 \"Archive\"\r\nA0002 UID MOVE 42:43 \"Entw&APw-rfe\"\r\nA0003 UID MOVE 99 \"Archive\"\r\n\
         A0004 UID COPY 7 \"Sent \\\"old\\\" mail\"\r\nA0005 UID COPY 7 \"Nowhere\"\r\n"
    );
}

#[test]
fn a_doubtful_copyuid_is_no_answer() {
    // A wrong pairing would tie a message here to another one there: anything short of certain
    // counts as not said.
    for doubtful in [
        "[COPYUID 7 4 9,10]",
        "[COPYUID 7 4,5 9]",
        "[COPYUID 7 5,4 9,10]",
        "[COPYUID 7 4:5 10:9]",
        "[COPYUID 7 5:4 9:10]",
        "[COPYUID 7 4,4 9,10]",
        "[COPYUID 7 4,5 9,9]",
        "[COPYUID 0 4 9]",
        "[COPYUID 7 0 9]",
        "[COPYUID 7 4 0]",
        "[COPYUID 7 4 *]",
        "[COPYUID 7 1:* 1:*]",
        "[COPYUID 7 4 9 12]",
        "[COPYUID 7 4]",
        "[COPYUID 7]",
        "[COPYUID]",
        "[COPYUID x 4 9]",
        "[COPYUID 7 +4 9]",
        "[COPYUID -7 4 9]",
        "[COPYUID 99999999999 4 9]",
        "[COPYUID 7 4 99999999999]",
        "[COPYUID 7 1:4000000000 1:4000000000]",
        "[COPYUID 7 4, 9]",
        "COPYUID 7 4 9",
    ] {
        let mut s = Script::new(format!("A0001 OK {doubtful} done\r\n"));
        assert_eq!(Imap::resume(&mut s).uid_copy("4:5", "Archive"), Ok(None), "{doubtful}");
    }
    // Said twice is said in two ways, even when one can be read; and what was said during
    // another command says nothing about this one.
    let mut s = Script::new("* OK [COPYUID 7 4 9] moved\r\nA0001 OK [COPYUID 7 4 10] done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_move("4", "Archive"), Ok(None));
    let mut s = Script::new("* OK [COPYUID 7 4 9] moved\r\n* OK [COPYUID 7 nonsense] moved\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_move("4", "Archive"), Ok(None));
    let mut s = Script::new("* OK [COPYUID 7 4 9] out of the blue\r\nA0001 OK done\r\nA0002 OK done\r\nA0003 OK [COPYUID 7 4 11] done\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.select("INBOX", false).unwrap();
    assert_eq!(imap.uid_copy("4", "Archive"), Ok(None));
    assert_eq!(imap.uid_copy("4", "Archive"), Ok(Some(Copied { uid_validity: 7, uids: vec![(4, 11)] })));
    // A refused move knows nothing either.
    let mut s = Script::new("* OK [COPYUID 7 4 9] moved\r\nA0001 NO [OVERQUOTA] full\r\n");
    assert_eq!(Imap::resume(&mut s).uid_move("4", "Archive"), Err(ProtoError::Rejected("[OVERQUOTA] full".into())));
    // As many as one set names are taken; more is more than was asked about.
    let all = format!("1:{MAX_SET_UIDS}");
    let mut s = Script::new(format!("A0001 OK [COPYUID 7 {all} {all}] done\r\n"));
    assert_eq!(Imap::resume(&mut s).uid_copy(&all, "Archive").unwrap().map(|c| c.uids.len()), Some(MAX_SET_UIDS));
}

#[test]
fn single_messages_are_removed_only_by_a_server_that_can() {
    // A server that didn't say UIDPLUS only knows the EXPUNGE that takes every marked message:
    // it is never asked.
    for capabilities in [None, Some(vec!["IMAP4REV1".to_string(), "MOVE".to_string()])] {
        let mut s = Script::new("A0001 OK done\r\n");
        let mut imap = Imap::resume(&mut s);
        imap.capabilities = capabilities;
        assert!(matches!(imap.uid_expunge("4"), Err(ProtoError::Protocol(_))));
        assert_eq!(s.sent(), "");
    }
    let mut s = Script::new(
        "* CAPABILITY IMAP4rev1 UIDPLUS\r\nA0001 OK done\r\n* 3 EXPUNGE\r\n* 3 EXPUNGE\r\nA0002 OK gone\r\nA0003 NO [NOPERM] not yours\r\n",
    );
    let mut imap = Imap::resume(&mut s);
    imap.capabilities().unwrap();
    imap.uid_expunge("4,7:8").unwrap();
    assert_eq!(imap.uid_expunge("9"), Err(ProtoError::Rejected("[NOPERM] not yours".into())));
    assert_eq!(s.sent(), "A0001 CAPABILITY\r\nA0002 UID EXPUNGE 4,7:8\r\nA0003 UID EXPUNGE 9\r\n");
}

#[test]
fn mailboxes_are_made_and_subscribed_to() {
    let mut s = Script::new("A0001 OK made\r\nA0002 OK done\r\nA0003 NO [ALREADYEXISTS] there is one\r\n+ go on\r\nA0004 OK made\r\n");
    let mut imap = Imap::resume(&mut s);
    let name = format!("Projects/{}", encode_utf7("Entwürfe & Pläne"));
    imap.create(&name).unwrap();
    imap.subscribe(&name).unwrap();
    assert_eq!(imap.create("INBOX"), Err(ProtoError::Rejected("[ALREADYEXISTS] there is one".into())));
    // A name that isn't in the mailbox encoding still can't break the command.
    imap.create("Pläne\r\nA0005 LOGOUT").unwrap();
    assert!(matches!(imap.create("nul\0here"), Err(ProtoError::Protocol(_))));
    assert!(matches!(imap.subscribe("nul\0here"), Err(ProtoError::Protocol(_))));
    assert_eq!(
        s.sent(),
        "A0001 CREATE \"Projects/Entw&APw-rfe &- Pl&AOQ-ne\"\r\nA0002 SUBSCRIBE \"Projects/Entw&APw-rfe &- Pl&AOQ-ne\"\r\nA0003 CREATE \"INBOX\"\r\n\
         A0004 CREATE {20}\r\nPläne\r\nA0005 LOGOUT\r\n"
    );
}

#[test]
fn uids_are_written_as_short_exact_sets() {
    let texts = |uids: &[u32]| uid_sets(uids).into_iter().map(|(text, _)| text).collect::<Vec<_>>();
    assert_eq!(texts(&[]), Vec::<String>::new());
    assert_eq!(texts(&[0]), Vec::<String>::new());
    assert_eq!(texts(&[7]), ["7"]);
    assert_eq!(texts(&[9, 4, 8, 7, 12, 4, 0]), ["4,7:9,12"]);
    assert_eq!(texts(&[u32::MAX, u32::MAX - 1, 1]), ["1,4294967294:4294967295"]);
    assert_eq!(uid_sets(&[9, 4, 8]), [("4,8:9".to_string(), vec![4, 8, 9])]);
    // A long run is cut where a set is full, not stretched over it.
    let run: Vec<u32> = (10..10 + 2 * MAX_SET_UIDS as u32 + 3).collect();
    assert_eq!(texts(&run), ["10:509", "510:1009", "1010:1012"]);
    // Scattered numbers fill a set by its length first.
    let scattered: Vec<u32> = (0..400).map(|n| 4_000_000_000 + 2 * n).collect();
    let sets = uid_sets(&scattered);
    assert!(sets.len() > 1 && sets.iter().all(|(text, _)| text.len() <= MAX_SET_TEXT));
    assert_eq!(sets.iter().map(|(_, uids)| uids.len()).sum::<usize>(), 400);
}

#[test]
fn appending_waits_for_the_go_ahead() {
    let message = b"Subject: Sent\r\n\r\nKept.\r\n";
    let mut s = Script::new("+ Ready for literal data\r\nA0001 OK [APPENDUID 38505 3955] done\r\n+ go\r\nA0002 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.append("Sent Items", &["\\Seen"], message).unwrap();
    imap.append("Entw&APw-rfe", &[], b"").unwrap();
    assert_eq!(s.sent(), "A0001 APPEND \"Sent Items\" (\\Seen) {24}\r\nSubject: Sent\r\n\r\nKept.\r\n\r\nA0002 APPEND \"Entw&APw-rfe\" {0}\r\n\r\n");

    // Refused before the message was sent: it stays here.
    let mut s = Script::new("A0001 NO [TRYCREATE] no such mailbox\r\n");
    assert_eq!(Imap::resume(&mut s).append("Nowhere", &[], message), Err(ProtoError::Rejected("[TRYCREATE] no such mailbox".into())));
    assert_eq!(s.sent(), "A0001 APPEND \"Nowhere\" {24}\r\n");

    // Refused after.
    let mut s = Script::new("+ go\r\nA0001 NO [OVERQUOTA] mailbox full\r\n");
    assert_eq!(Imap::resume(&mut s).append("Sent", &[], message), Err(ProtoError::Rejected("[OVERQUOTA] mailbox full".into())));

    // "Done" before the message was sent can't be true.
    let mut s = Script::new("A0001 OK done already\r\n");
    assert!(matches!(Imap::resume(&mut s).append("Sent", &[], message), Err(ProtoError::Protocol(_))));
}

#[test]
fn appending_with_a_date_says_where_the_message_went() {
    let message = b"Subject: Moved\r\n\r\nKept.\r\n";
    let date = "01-Jul-2003 09:52:37 +0000";
    let mut s = Script::new("+ go\r\nA0001 OK [APPENDUID 38505 3955] done\r\n+ go\r\nA0002 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.append_at("INBOX", &["\\Seen"], Some(date), message), Ok(Some(Appended { uid_validity: 38505, uid: 3955 })));
    // A server that doesn't say: nothing is known of where it went.
    assert_eq!(imap.append_at("INBOX", &[], Some(" 1-Jul-2003 09:52:37 -0700"), message), Ok(None));
    assert_eq!(
        s.sent(),
        "A0001 APPEND \"INBOX\" (\\Seen) \"01-Jul-2003 09:52:37 +0000\" {25}\r\nSubject: Moved\r\n\r\nKept.\r\n\r\n\
         A0002 APPEND \"INBOX\" \" 1-Jul-2003 09:52:37 -0700\" {25}\r\nSubject: Moved\r\n\r\nKept.\r\n\r\n"
    );
    // An answer that doesn't pair one message with one UID beyond doubt is no answer.
    for code in ["[APPENDUID 0 5]", "[APPENDUID 38505 0]", "[APPENDUID 38505 1:3]", "[APPENDUID 38505]", "[APPENDUID 38505 7 8]", "[APPENDUID x 7]"] {
        let mut s = Script::new(format!("+ go\r\nA0001 OK {code} done\r\n"));
        assert_eq!(Imap::resume(&mut s).append_at("INBOX", &[], None, message), Ok(None), "{code}");
    }
    // Two answers to one message might disagree: neither is taken.
    let mut s = Script::new("+ go\r\n* OK [APPENDUID 1 2] one\r\nA0001 OK [APPENDUID 1 3] two\r\n");
    assert_eq!(Imap::resume(&mut s).append_at("INBOX", &[], None, message), Ok(None));
    // A date that isn't one is never sent, nor is what it carries.
    for bad in
        ["1-Jul-2003 09:52:37 +0000", "01-Jul-2003 09:52:37", "01-Jul-2003 09:52:37 +0000\r\nA2 DELETE INBOX", "01-Jul-2003 09:52:37 +00\"0", ""]
    {
        let mut s = Script::new("+ go\r\nA0001 OK done\r\n");
        assert!(matches!(Imap::resume(&mut s).append_at("INBOX", &[], Some(bad), message), Err(ProtoError::Protocol(_))), "{bad:?}");
        assert_eq!(s.sent(), "", "{bad:?}");
    }
}

#[test]
fn searching_a_header_answers_the_uids_it_names() {
    let mut s = Script::new("* SEARCH 9 4 0 x 4 4294967295\r\n* 3 EXISTS\r\nA0001 OK done\r\nA0002 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    // In UID order and once each; what isn't a UID is passed over.
    assert_eq!(imap.uid_search_header("Message-ID", "<a.1@example.org>"), Ok(vec![4, 9, u32::MAX]));
    // Quotes in the text are quoted.
    assert_eq!(imap.uid_search_header("Message-ID", "<say\"hi\"@example.org>"), Ok(vec![]));
    assert_eq!(
        s.sent(),
        "A0001 UID SEARCH UNDELETED HEADER Message-ID \"<a.1@example.org>\"\r\n\
         A0002 UID SEARCH UNDELETED HEADER Message-ID \"<say\\\"hi\\\"@example.org>\"\r\n"
    );
    // A refusal is said.
    let mut s = Script::new("A0001 NO no searching here\r\n");
    assert!(matches!(Imap::resume(&mut s).uid_search_header("Message-ID", "<a@example.org>"), Err(ProtoError::Rejected(_))));
    // What could end the command early, or isn't text, is never sent.
    for (field, text) in [
        ("Message-ID", "<a@example.org>\r\nA2 DELETE INBOX"),
        ("Message-ID", "<ä@example.org>"),
        ("Message-ID", ""),
        ("Message ID", "<a@example.org>"),
        ("", "<a>"),
    ] {
        let mut s = Script::new("A0001 OK done\r\n");
        assert!(matches!(Imap::resume(&mut s).uid_search_header(field, text), Err(ProtoError::Protocol(_))), "{field:?} {text:?}");
        assert_eq!(s.sent(), "", "{field:?} {text:?}");
    }
    // A server that names more messages than are ever looked for is taken at its first word.
    let many: String = (1..=5000).map(|n| format!(" {n}")).collect();
    let mut s = Script::new(format!("* SEARCH{many}\r\nA0001 OK done\r\n"));
    assert_eq!(Imap::resume(&mut s).uid_search_header("Message-ID", "<a@example.org>").map(|uids| uids.len()), Ok(MAX_FOUND));
}

#[test]
fn starttls_hands_the_stream_back() {
    let mut s = Script::new("* OK ready\r\n* CAPABILITY IMAP4rev1 STARTTLS\r\nA0001 OK done\r\nA0002 OK begin TLS now\r\n");
    let mut imap = Imap::connect(&mut s).unwrap();
    assert!(imap.capabilities().unwrap().contains(&"STARTTLS".to_string()));
    let stream = imap.starttls().unwrap();
    assert_eq!(stream.sent(), "A0001 CAPABILITY\r\nA0002 STARTTLS\r\n");
    // The session starts afresh inside TLS.
    let mut s = Script::new("A0001 OK done\r\n");
    Imap::resume(&mut s).capabilities().unwrap();

    let mut s = Script::new("A0001 BAD no TLS here\r\n");
    assert_eq!(Imap::resume(&mut s).starttls().unwrap_err(), ProtoError::Rejected("no TLS here".into()));
}

#[test]
fn text_slipped_in_before_tls_is_refused() {
    // Both lines arrive in one read, as an attacker in the middle would send them.
    let mut s = Script::whole("A0001 OK begin TLS now\r\nA0002 OK [CAPABILITY IMAP4rev1] injected\r\n");
    assert!(matches!(Imap::resume(&mut s).starttls(), Err(ProtoError::Protocol(_))));
}

#[test]
fn logging_out() {
    let mut s = Script::new("* BYE logging out\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).logout(), Ok(()));
    assert_eq!(s.sent(), "A0001 LOGOUT\r\n");
    // A server that hangs up without the last word has logged us out too.
    for server in ["* BYE\r\n", ""] {
        let mut s = Script::new(server);
        assert_eq!(Imap::resume(&mut s).logout(), Ok(()), "{server:?}");
    }
    let mut s = Script::new("A0001 BAD what\r\n");
    assert!(matches!(Imap::resume(&mut s).logout(), Err(ProtoError::Rejected(_))));
}

#[test]
fn a_server_that_says_bye_has_ended_the_session() {
    let mut s = Script::new("* 3 EXISTS\r\n* BYE [UNAVAILABLE] shutting down\r\n");
    assert_eq!(
        Imap::resume(&mut s).select("INBOX", true),
        Err(ProtoError::Io(said!("the server ended the session: {reason}", reason = Arg::said("[UNAVAILABLE] shutting down".into()))))
    );
}

#[test]
fn tags_count_up_and_must_match() {
    let mut s = Script::new("* OK ready\r\nA0001 OK one\r\nA0002 OK two\r\nA0003 OK three\r\nA0009 OK not ours\r\n");
    let mut imap = Imap::connect(&mut s).unwrap();
    imap.capabilities().unwrap();
    imap.list().unwrap();
    imap.select("INBOX", true).unwrap();
    assert_eq!(imap.uid_fetch_flags("1:*"), Err(ProtoError::Protocol(said!("the server answered a command that wasn't sent"))));
    assert_eq!(s.sent(), "A0001 CAPABILITY\r\nA0002 LIST \"\" \"*\"\r\nA0003 EXAMINE \"INBOX\"\r\nA0004 UID FETCH 1:* (UID FLAGS)\r\n");
}

#[test]
fn junk_in_a_session_is_a_protocol_error() {
    for junk in ["+ send more\r\n", "garbage\r\n", "A0001 FETCH x\r\n", "A0001 BYE\r\n", "A0001 PREAUTH hi\r\n", "\r\n", "A0001\r\n", " OK\r\n"] {
        let mut s = Script::new(junk);
        assert!(matches!(Imap::resume(&mut s).capabilities(), Err(ProtoError::Protocol(_))), "{junk:?}");
    }
    // Refusals keep the server's words; one without words still says something.
    let mut s = Script::new("A0001 NO\r\n");
    assert_eq!(Imap::resume(&mut s).capabilities(), Err(ProtoError::Rejected(said!("the command was refused"))));
}

#[test]
fn a_dead_connection_is_an_io_error() {
    // In the middle of a line, and in the middle of a literal.
    for server in ["* 1 FETCH (UID 3 BO", "* 1 FETCH (UID 3 BODY[] {50}\r\nonly this much", "* 1 FETCH (UID 3 BODY[] {5}\r\nabcde"] {
        let mut s = Script::new(server);
        assert_eq!(Imap::resume(&mut s).uid_fetch_message(3), Err(ProtoError::Io(said!("the server closed the connection"))), "{server:?}");
    }
    assert!(matches!(Imap::connect(Broken(ErrorKind::TimedOut)), Err(ProtoError::Io(_))));
    assert!(matches!(Imap::resume(Broken(ErrorKind::ConnectionReset)).list(), Err(ProtoError::Io(_))));
}

#[test]
fn text_that_cannot_be_sent_is_refused() {
    let mut s = Script::new("A0001 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    assert!(matches!(imap.select("nul\0here", true), Err(ProtoError::Protocol(_))));
    assert!(matches!(imap.append("nul\0here", &[], b"x"), Err(ProtoError::Protocol(_))));
    assert_eq!(s.sent(), "");
}

/// Everything a client does, against whatever the "server" says.
fn drive(server: &[u8]) {
    let mut s = Script::new(server);
    if let Ok(mut imap) = Imap::connect(&mut s) {
        let _ = imap.capabilities();
        let _ = imap.login("ada", "hunter2");
        let _ = imap.login("ädä", "pässword");
        let _ = imap.list();
        let _ = imap.select("INBOX", true);
        let _ = imap.uid_fetch_flags("1:*");
        let _ = imap.uid_fetch_sizes("1:*");
        let _ = imap.uid_fetch_message(1);
        let _ = imap.uid_fetch_header(1);
        let _ = imap.uid_store(1, true, &["\\Seen"]);
        let _ = imap.uid_store_set("1:3,7", false, &["\\Flagged"]);
        let _ = imap.uid_move("1:3", "Archive");
        let _ = imap.uid_copy("7", "Entw&APw-rfe");
        let _ = imap.uid_expunge("7");
        let _ = imap.create("Projects/New");
        let _ = imap.subscribe("Projects/New");
        let _ = imap.append("Sent", &["\\Seen"], b"Subject: x\r\n\r\n");
        let _ = imap.uid_search_header("Message-ID", "<a.1@example.org>");
        let _ = imap.uid_fetch_flags_gmail("1:*");
        let _ = imap.uid_search_gmail("1:500", "category:social");
        let _ = imap.logout();
    }
    // Changes, by a session that was told it may make them.
    let mut s = Script::new(server);
    let mut imap = Imap::resume(&mut s);
    imap.capabilities = Some(vec!["MOVE".into(), "UIDPLUS".into()]);
    let _ = imap.select("INBOX", false);
    let _ = imap.uid_move("1,3:5", "Archive");
    let _ = imap.uid_copy("1", "Archive");
    let _ = imap.uid_expunge("1,3:5");
    let _ = imap.uid_store_set("2", true, &["\\Deleted"]);
    let mut s = Script::new(server);
    let mut imap = Imap::resume(&mut s);
    let _ = imap.uid_fetch_message(1);
    let _ = imap.uid_fetch_header(1);
    let _ = imap.list();
    let _ = imap.uid_fetch_flags("1:*");
    let _ = imap.starttls();
    // With a token: after a go-ahead, and with the command.
    let mut s = Script::new(server);
    let mut imap = Imap::resume(&mut s);
    let _ = imap.authenticate_xoauth2("ada@example.org", "tok.en-123");
    let _ = imap.list();
    let mut s = Script::new(server);
    let mut imap = Imap::resume(&mut s);
    imap.capabilities = Some(vec!["SASL-IR".into(), "AUTH=XOAUTH2".into()]);
    let _ = imap.authenticate_xoauth2("ada@example.org", "tok.en-123");
    let _ = imap.select("INBOX", true);
}

const WORDS: [&str; 43] = [
    "SEARCH ",
    "4294967295 ",
    "BODY[HEADER] ",
    "[COPYUID 7 1 9] ",
    "[COPYUID 7 1,3:5 9:12] ",
    "[COPYUID ",
    "1:4 ",
    "9,10 ",
    "EXPUNGE",
    "* ",
    "A0001 ",
    "A0002 ",
    "A0003 ",
    "OK ",
    "NO ",
    "BAD ",
    "BYE ",
    "PREAUTH ",
    "+ ",
    "1 ",
    "FETCH ",
    "LIST ",
    "EXISTS",
    "CAPABILITY ",
    "(",
    ")",
    "UID 1 ",
    "FLAGS ",
    "BODY[] ",
    "INTERNALDATE ",
    "RFC822.SIZE 9 ",
    "{3}\r\n",
    "{0}\r\n",
    "{99999999999}\r\n",
    "abc",
    "\"",
    "\\",
    "NIL ",
    "\r\n",
    "[UIDNEXT 5] ",
    "[",
    "]",
    "é",
];

proptest! {
    #[test]
    fn arbitrary_server_bytes_never_panic_or_hang(server in proptest::collection::vec(any::<u8>(), 0..600)) {
        drive(&server);
        // And after a proper greeting, so the session gets going.
        let mut greeted = b"* OK ready\r\n".to_vec();
        greeted.extend_from_slice(&server);
        drive(&greeted);
    }

    #[test]
    fn response_shaped_noise_never_panics(words in proptest::collection::vec(proptest::sample::select(&WORDS[..]), 0..80), greeted in any::<bool>()) {
        let server = format!("{}{}", if greeted { "* OK ready\r\n" } else { "" }, words.concat());
        drive(server.as_bytes());
    }

    #[test]
    fn any_body_arrives_as_it_was_sent(body in proptest::collection::vec(any::<u8>(), 0..2000), flags_first in any::<bool>()) {
        let mut server = if flags_first {
            with_literal("* 1 FETCH (UID 9 FLAGS (\\Seen) BODY[] ", &body, ")\r\n")
        } else {
            with_literal("* 1 FETCH (BODY[] ", &body, " FLAGS (\\Seen) UID 9)\r\n")
        };
        server.extend_from_slice(b"A0001 OK done\r\n");
        let mut s = Script::new(server);
        let got = Imap::resume(&mut s).uid_fetch_message(9).unwrap().unwrap();
        prop_assert_eq!(got.body, Some(body));
        prop_assert_eq!(got.flags, vec!["\\Seen".to_string()]);
    }

    #[test]
    fn any_uids_are_named_once_each_by_sets_that_read_back(uids in proptest::collection::vec(prop_oneof![1u32..40, 1u32..3000, any::<u32>()], 0..1500)) {
        let mut expected: Vec<u32> = uids.iter().copied().filter(|uid| *uid != 0).collect();
        expected.sort_unstable();
        expected.dedup();
        let mut named_in_all = Vec::new();
        for (text, named_here) in uid_sets(&uids) {
            // A set says what it is said to say, and is one a changing command takes.
            prop_assert_eq!(uids_of(&text), Some(named_here.clone()), "{}", text);
            prop_assert!(named(&text).is_ok() && is_set(&text), "{}", text);
            prop_assert!(text.len() <= MAX_SET_TEXT && !named_here.is_empty() && named_here.len() <= MAX_SET_UIDS);
            named_in_all.extend(named_here);
        }
        prop_assert_eq!(named_in_all, expected);
    }

    #[test]
    fn any_copyuid_text_is_read_or_not(text in "[0-9,:*+ -]{0,40}") {
        if let Some(copied) = copied_from(&text) {
            prop_assert!(copied.uid_validity != 0 && copied.uids.len() <= MAX_SET_UIDS);
            prop_assert!(copied.uids.iter().all(|(from, to)| *from != 0 && *to != 0));
            prop_assert!(copied.uids.iter().zip(copied.uids.iter().skip(1)).all(|(a, b)| a.0 < b.0 && a.1 != b.1));
        }
    }

    #[test]
    fn any_mailbox_name_survives_quoting(name in "[ -~]{1,40}") {
        // What the client would send as a quoted string, a server sends back in LIST.
        let mut s = Script::new("A0001 OK done\r\n");
        Imap::resume(&mut s).select(&name, true).unwrap();
        let sent = s.sent();
        let quoted = sent.strip_prefix("A0001 EXAMINE ").unwrap().strip_suffix("\r\n").unwrap();
        let mut s = Script::new(format!("* LIST () \"/\" {quoted}\r\nA0001 OK done\r\n"));
        let list = Imap::resume(&mut s).list().unwrap();
        prop_assert_eq!(list.len(), 1);
        prop_assert_eq!(&list[0].raw, &name);
    }
}

#[test]
fn namespace_reads_personal_prefix_and_delimiter() {
    let mut s = Script::new("* NAMESPACE ((\"INBOX.\" \".\")) NIL NIL\r\nA0001 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.capabilities = Some(vec!["IMAP4REV1".into(), "NAMESPACE".into()]);
    assert_eq!(imap.namespace().unwrap(), Some(Namespace { prefix: "INBOX.".into(), delimiter: Some('.') }));
    assert_eq!(s.sent(), "A0001 NAMESPACE\r\n");

    let mut s = Script::new("* NAMESPACE ((\"\" \"/\")) NIL NIL\r\nA0001 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.capabilities = Some(vec!["IMAP4REV1".into(), "NAMESPACE".into()]);
    assert_eq!(imap.namespace().unwrap(), Some(Namespace { prefix: String::new(), delimiter: Some('/') }));

    let mut s = Script::new("");
    let mut imap = Imap::resume(&mut s);
    imap.capabilities = Some(vec!["IMAP4REV1".into()]);
    assert_eq!(imap.namespace().unwrap(), None);
    assert_eq!(s.sent(), "");
}
