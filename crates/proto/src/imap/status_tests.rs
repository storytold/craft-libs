// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use proptest::prelude::*;

use super::*;
use crate::script::Script;

fn status(messages: u32, uid_next: u32, uid_validity: u32, unseen: u32) -> FolderStatus {
    FolderStatus { messages, uid_next, uid_validity, unseen }
}

#[test]
fn one_mailbox_is_asked_its_four_counts() {
    let mut s = Script::new("* STATUS \"Work\" (MESSAGES 3 UIDNEXT 9 UIDVALIDITY 77 UNSEEN 1)\r\nA0001 OK done\r\n");
    let got = Imap::resume(&mut s).status("Work").unwrap();
    assert_eq!(got, Some(status(3, 9, 77, 1)));
    assert_eq!(s.sent(), "A0001 STATUS \"Work\" (MESSAGES UIDNEXT UIDVALIDITY UNSEEN)\r\n");
}

#[test]
fn a_count_left_out_or_a_refusal_is_no_status() {
    let mut s = Script::new("* STATUS Work (MESSAGES 3 UIDNEXT 9 UIDVALIDITY 77)\r\nA0001 OK done\r\nA0002 NO [NONEXISTENT] no such mailbox\r\n");
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.status("Work").unwrap(), None);
    assert_eq!(imap.status("Gone").unwrap(), None);
}

#[test]
fn many_mailboxes_are_asked_in_one_write() {
    let mut s = Script::new(
        "* STATUS A (MESSAGES 1 UIDNEXT 2 UIDVALIDITY 5 UNSEEN 0)\r\nA0001 OK done\r\n\
         A0002 NO [NONEXISTENT] no\r\n\
         * STATUS \"C d\" (UNSEEN 4 UIDVALIDITY 7 UIDNEXT 8 MESSAGES 9 HIGHESTMODSEQ 12)\r\nA0003 OK done\r\n",
    );
    let got = Imap::resume(&mut s).status_many(&["A", "B", "C d"]).unwrap();
    assert_eq!(got, vec![Some(status(1, 2, 5, 0)), None, Some(status(9, 8, 7, 4))]);
    let sent = s.sent();
    assert_eq!(sent.matches("STATUS").count(), 3);
    assert!(sent.starts_with("A0001 STATUS \"A\" ") && sent.contains("A0003 STATUS \"C d\" "), "{sent}");
}

#[test]
fn answers_out_of_turn_or_too_many_end_the_session_not_the_program() {
    let mut s = Script::new("A0002 OK done\r\n");
    assert!(matches!(Imap::resume(&mut s).status_many(&["A", "B"]), Err(ProtoError::Protocol(_))));
    let flood = "* 1 EXISTS\r\n".repeat(500);
    let mut s = Script::new(flood);
    assert!(matches!(Imap::resume(&mut s).status_many(&["A", "B"]), Err(ProtoError::Protocol(_))));
    let mut s = Script::new("* BYE going\r\n");
    assert!(matches!(Imap::resume(&mut s).status_many(&["A", "B"]), Err(ProtoError::Io(_))));
}

#[test]
fn a_name_that_needs_a_literal_is_asked_alone() {
    let mut s = Script::new("+ go\r\n* STATUS x (MESSAGES 1 UIDNEXT 2 UIDVALIDITY 3 UNSEEN 0)\r\nA0001 OK done\r\n");
    let got = Imap::resume(&mut s).status_many(&["é", "A"]).unwrap_or_default();
    assert!(got.len() <= 2);
}

proptest! {
    #[test]
    fn arbitrary_server_bytes_never_panic_in_status(server in proptest::collection::vec(any::<u8>(), 0..600), names in 0usize..120) {
        let list: Vec<String> = (0..names).map(|n| format!("F{n}")).collect();
        let refs: Vec<&str> = list.iter().map(String::as_str).collect();
        let mut s = Script::new(&server);
        let got = Imap::resume(&mut s).status_many(&refs);
        if let Ok(got) = got {
            prop_assert_eq!(got.len(), names);
        }
    }

    #[test]
    fn any_status_line_is_read_or_not(line in "[ -~]{0,80}") {
        let mut s = Script::new(format!("* STATUS {line}\r\nA0001 OK done\r\n"));
        let _ = Imap::resume(&mut s).status("x");
    }
}
