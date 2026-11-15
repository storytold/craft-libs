// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Gmail's IMAP extensions (#911): `X-GM-MSGID` and `X-GM-RAW`.

use super::*;
use crate::script::Script;

#[test]
fn gmail_ids_come_with_the_flags() {
    let mut server = String::new();
    server += "* 1 FETCH (UID 4 FLAGS (\\Seen) X-GM-MSGID 1278455344230334865)\r\n";
    server += "* 2 FETCH (X-GM-MSGID 18446744073709551615 UID 7 FLAGS ())\r\n";
    // An id that is no number, or 0, is none; a later word about a message without one keeps it.
    server += "* 3 FETCH (UID 9 FLAGS () X-GM-MSGID 12ab)\r\n";
    server += "* 4 FETCH (UID 10 FLAGS () X-GM-MSGID 0)\r\n";
    server += "* 2 FETCH (UID 7 FLAGS (\\Flagged))\r\n";
    server += "A0001 OK done\r\n";
    let mut s = Script::new(server);
    let got = Imap::resume(&mut s).uid_fetch_flags_gmail("1:*").unwrap();
    let ids: Vec<(u32, Option<u64>)> = got.iter().map(|f| (f.uid, f.gmail_id)).collect();
    assert_eq!(ids, [(4, Some(1_278_455_344_230_334_865)), (7, Some(u64::MAX)), (9, None), (10, None)]);
    assert_eq!(got.get(1).map(|f| f.flags.clone()), Some(vec!["\\Flagged".to_string()]));
    assert_eq!(s.sent(), "A0001 UID FETCH 1:* (UID FLAGS X-GM-MSGID X-GM-LABELS)\r\n");
    // Without asking for them, an id the server gives anyway is not taken.
    let mut s = Script::new("* 1 FETCH (UID 4 FLAGS () X-GM-MSGID 5)\r\nA0001 OK done\r\n");
    assert_eq!(Imap::resume(&mut s).uid_fetch_flags("1:*").unwrap().first().and_then(|f| f.gmail_id), None);
}

#[test]
fn gmail_search_is_quoted_and_kept_to_its_set() {
    let mut s = Script::new("* SEARCH 9 4 0 x 4 12 600\r\nA0001 OK done\r\nA0002 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    // In UID order, once each, and only what the set names.
    assert_eq!(imap.uid_search_gmail("4:12", "category:social"), Ok(vec![4, 9, 12]));
    // Quotes are quoted; CR, LF and NUL never go out.
    assert_eq!(imap.uid_search_gmail("1:*", "category:\"x\"\r\nA9 DELETE INBOX\0"), Ok(vec![]));
    assert_eq!(
        s.sent(),
        "A0001 UID SEARCH UID 4:12 UNDELETED X-GM-RAW \"category:social\"\r\n\
         A0002 UID SEARCH UID 1:* UNDELETED X-GM-RAW \"category:\\\"x\\\"A9 DELETE INBOX\"\r\n"
    );
    // A refusal is said.
    let mut s = Script::new("A0001 BAD no such search\r\n");
    assert!(matches!(Imap::resume(&mut s).uid_search_gmail("1:*", "category:forums"), Err(ProtoError::Rejected(_))));
    // What can't be sent is refused unsent.
    for (set, query) in [("1:*", ""), ("1:*", "\r\n"), ("1:*", "kategorie:äöü"), ("1 2", "category:social"), ("", "category:social")] {
        let mut s = Script::new("A0001 OK done\r\n");
        assert!(matches!(Imap::resume(&mut s).uid_search_gmail(set, query), Err(ProtoError::Protocol(_))), "{set:?} {query:?}");
        assert_eq!(s.sent(), "", "{set:?} {query:?}");
    }
}

#[test]
fn gmail_search_takes_no_more_than_its_cap() {
    let many: String = (1..=6000).map(|n| format!(" {n}")).collect();
    let mut s = Script::new(format!("* SEARCH{many}\r\nA0001 OK done\r\n"));
    assert_eq!(Imap::resume(&mut s).uid_search_gmail("1:*", "category:promotions").map(|uids| uids.len()), Ok(MAX_GMAIL_FOUND));
    // As many as the cap, at the longest a UID is written, fit in one line.
    let longest: String = (0..MAX_GMAIL_FOUND).map(|_| " 4294967295").collect();
    assert!(format!("* SEARCH{longest}").len() < crate::MAX_LINE);
}

#[test]
fn a_gmail_answer_too_long_is_refused_and_the_session_goes_on() {
    let long: String = (0..20_000).map(|n| format!(" {}", 1_000_000_000u32 + n)).collect();
    assert!(long.len() > crate::MAX_LINE);
    let server = format!("* SEARCH{long}\r\n* SEARCH 3\r\nA0001 OK done\r\n* SEARCH 5\r\nA0002 OK done\r\n");
    let mut s = Script::new(server);
    let mut imap = Imap::resume(&mut s);
    assert!(matches!(imap.uid_search_gmail("1:*", "category:social"), Err(ProtoError::Rejected(_))));
    // The next command is answered as if nothing had happened.
    assert_eq!(imap.uid_search_gmail("1:*", "category:social"), Ok(vec![5]));
    // Elsewhere a line that long still ends the session, and a tagged one is never read past.
    let mut s = Script::new(format!("* SEARCH{long}\r\nA0001 OK done\r\n"));
    assert!(matches!(Imap::resume(&mut s).uid_search_header("Message-ID", "x"), Err(ProtoError::Protocol(_))));
    let mut s = Script::new(format!("A0001 OK{long}\r\n"));
    assert!(matches!(Imap::resume(&mut s).uid_search_gmail("1:*", "category:social"), Err(ProtoError::Protocol(_))));
}
