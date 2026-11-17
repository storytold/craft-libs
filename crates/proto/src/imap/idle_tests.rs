// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use proptest::prelude::*;

use super::*;
use crate::script::Script;

// Polling now yields after each underlying read, including partial lines.
fn next_event(imap: &mut Imap<&mut Script>) -> Result<Option<IdleEvent>, ProtoError> {
    for _ in 0..crate::MAX_LINE + 2 {
        match imap.idle_poll() {
            Ok(None) => continue,
            result => return result,
        }
    }
    panic!("script failed to produce an event within its bounded input");
}

#[test]
fn idle_goes_ahead_hears_news_and_ends() {
    // The go-ahead is the first line; the rest is heard by polling.
    let mut s2 = Script::new("+ idling\r\n* 4 EXISTS\r\n* 1 RECENT\r\n* 2 FETCH (FLAGS (\\Seen))\r\n* 3 EXPUNGE\r\n");
    let mut imap2 = Imap::resume(&mut s2);
    imap2.idle_begin().unwrap();
    assert_eq!(next_event(&mut imap2).unwrap(), Some(IdleEvent::Exists(4)));
    assert_eq!(next_event(&mut imap2).unwrap(), Some(IdleEvent::Recent(1)));
    assert_eq!(next_event(&mut imap2).unwrap(), Some(IdleEvent::Other));
    assert_eq!(next_event(&mut imap2).unwrap(), Some(IdleEvent::Expunge(3)));
    assert_eq!(s2.sent(), "A0001 IDLE\r\n");

    let mut s3 = Script::new("+ idling\r\n* 5 EXISTS\r\nA0001 OK IDLE terminated\r\n");
    let mut imap3 = Imap::resume(&mut s3);
    imap3.idle_begin().unwrap();
    assert_eq!(imap3.idle_end().unwrap(), vec![IdleEvent::Exists(5)]);
    assert_eq!(s3.sent(), "A0001 IDLE\r\nDONE\r\n");
}

#[test]
fn a_refusal_or_an_early_end_or_a_goodbye_is_said() {
    let mut s = Script::new("A0001 NO [CANNOT] not now\r\n");
    assert!(matches!(Imap::resume(&mut s).idle_begin(), Err(ProtoError::Rejected(_))));
    let mut s = Script::new("+ idling\r\nA0001 OK the server gave up waiting\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.idle_begin().unwrap();
    assert_eq!(next_event(&mut imap).unwrap(), Some(IdleEvent::Ended));
    // Not idling any more: nothing to poll or to end.
    assert!(matches!(next_event(&mut imap), Err(ProtoError::Protocol(_))));
    assert!(matches!(imap.idle_end(), Err(ProtoError::Protocol(_))));
    let mut s = Script::new("+ idling\r\n* BYE going away\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.idle_begin().unwrap();
    assert!(matches!(next_event(&mut imap), Err(ProtoError::Io(_))));
}

#[test]
fn what_a_server_has_no_reason_to_send_while_idle_ends_the_session() {
    for hostile in ["* 1 FETCH (BODY[] {5}\r\n", "A0099 OK that was never asked\r\n", "garbage\r\n", "* 1 EXISTS\r\n".repeat(0).as_str()] {
        let mut s = Script::new(format!("+ idling\r\n{hostile}"));
        let mut imap = Imap::resume(&mut s);
        imap.idle_begin().unwrap();
        let got = next_event(&mut imap);
        assert!(got.is_err() || hostile.is_empty(), "{hostile:?}: {got:?}");
    }
    // A line longer than any a server is allowed.
    let mut long = b"+ idling\r\n* ".to_vec();
    long.extend(std::iter::repeat_n(b'x', crate::MAX_LINE + 10));
    long.extend_from_slice(b"\r\n");
    let mut s = Script::whole(long);
    let mut imap = Imap::resume(&mut s);
    imap.idle_begin().unwrap();
    assert!(matches!(next_event(&mut imap), Err(ProtoError::Protocol(_))));
}

proptest! {
    #[test]
    fn arbitrary_server_bytes_never_panic_while_idle(server in proptest::collection::vec(any::<u8>(), 0..800), greeted in any::<bool>()) {
        let mut bytes = if greeted { b"+ idling\r\n".to_vec() } else { Vec::new() };
        bytes.extend_from_slice(&server);
        let mut s = Script::new(&bytes);
        let mut imap = Imap::resume(&mut s);
        if imap.idle_begin().is_ok() {
            // However much is said, polling ends (the script runs out) and nothing is kept without bound.
            for _ in 0..2_000 {
                match next_event(&mut imap) {
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => break,
                }
            }
            let _ = imap.idle_end();
        }
    }

    #[test]
    fn idle_shaped_noise_never_panics(words in proptest::collection::vec(proptest::sample::select(&["* ", "1 ", "EXISTS", "EXPUNGE", "RECENT", " ", "\r\n", "A0001 OK ", "BYE ", "{9}", "FETCH (", ")"][..]), 0..60)) {
        let mut s = Script::new(format!("+ go\r\n{}", words.concat()));
        let mut imap = Imap::resume(&mut s);
        if imap.idle_begin().is_ok() {
            for _ in 0..200 {
                if !matches!(next_event(&mut imap), Ok(Some(_))) {
                    break;
                }
            }
            let _ = imap.idle_end();
        }
    }
}

#[test]
fn notifications_before_continuation_are_delivered() {
    let mut s = Script::new("* 4 EXISTS\r\n* 2 EXPUNGE\r\n+ idling\r\nA0001 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    imap.idle_begin().unwrap();
    assert_eq!(imap.idle_poll().unwrap(), Some(IdleEvent::Exists(4)));
    assert_eq!(imap.idle_end().unwrap(), vec![IdleEvent::Expunge(2)]);
}

#[test]
fn idle_bye_errors_hide_registered_secrets() {
    for ending in [false, true] {
        let mut s = Script::new("+ idling\r\n* BYE test-token\r\n");
        let mut imap = Imap::resume(&mut s);
        imap.secrets.add("test-token");
        imap.idle_begin().unwrap();
        let error = if ending { imap.idle_end().unwrap_err() } else { next_event(&mut imap).unwrap_err() };
        assert!(!error.to_string().contains("test-token"));
    }
}

#[test]
fn idle_completion_requires_a_valid_success_status() {
    for (status, rejected) in [("NO", true), ("BAD", true), ("nonsense", false)] {
        let mut s = Script::new(format!("+ idling\r\nA0001 {status} failed\r\n"));
        let mut imap = Imap::resume(&mut s);
        imap.idle_begin().unwrap();
        let error = next_event(&mut imap).unwrap_err();
        assert!(if rejected { matches!(error, ProtoError::Rejected(_)) } else { matches!(error, ProtoError::Protocol(_)) });
    }
}
