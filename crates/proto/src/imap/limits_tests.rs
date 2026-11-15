// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What one command may make the client keep or read, however much the server sends.
use proptest::prelude::*;

use super::*;
use crate::script::Script;

/// A session whose open mailbox has `exists` messages and the next UID `uid_next`.
fn opened(s: &mut Script, exists: u32, uid_next: u32) -> Imap<&mut Script> {
    let mut imap = Imap::resume(s);
    imap.selected = Selected { exists, uid_validity: 1, uid_next };
    imap
}

/// `lines` lines made by `line(n)`.
fn flood(lines: usize, line: impl Fn(usize) -> String) -> String {
    (0..lines).map(line).collect()
}

fn padding() -> String {
    "x".repeat(60 * 1024)
}

#[test]
fn a_flood_of_long_lines_ends_the_command() {
    let pad = padding();
    // 600 lines of 60 KB are more than the 32 MB one command may bring.
    let server = flood(600, |n| format!("* {n} FETCH (UID {} FLAGS ({pad}))\r\n", n + 1)) + "A0001 OK done\r\n";
    let mut s = Script::new(server);
    let err = opened(&mut s, 1000, 2000).uid_fetch_flags("1:*").unwrap_err();
    assert!(matches!(err, ProtoError::Protocol(ref m) if m.to_string().contains("too much")), "{err:?}");
    // The same in a list.
    let server = flood(600, |n| format!("* LIST () \"/\" \"{n}{pad}\"\r\n")) + "A0001 OK done\r\n";
    let mut s = Script::new(server);
    assert!(matches!(opened(&mut s, 0, 0).list(), Err(ProtoError::Protocol(_))));
}

#[test]
fn a_flood_of_responses_ends_the_command() {
    let server = "* OK still here\r\n".repeat(MAX_RESPONSES + 1) + "A0001 OK done\r\n";
    let mut s = Script::new(server);
    let err = opened(&mut s, 0, 0).uid_fetch_sizes("1:*").unwrap_err();
    assert!(matches!(err, ProtoError::Protocol(ref m) if m.to_string().contains("too many")), "{err:?}");
}

#[test]
fn a_list_keeps_a_capped_number_of_mailboxes() {
    let server = flood(MAX_MAILBOXES + 500, |n| format!("* LIST () \"/\" f{n}\r\n")) + "A0001 OK done\r\n";
    let mut s = Script::new(server);
    let got = opened(&mut s, 0, 0).list().unwrap();
    assert_eq!(got.len(), MAX_MAILBOXES);
    assert_eq!(got.first().map(|m| m.raw.as_str()), Some("f0"));
    // #991: and how many were dropped is said; none, for a list kept whole.
    let server = flood(MAX_MAILBOXES + 500, |n| format!("* LIST () \"/\" f{n}\r\n")) + "A0001 OK done\r\n";
    let mut s = Script::new(server);
    assert_eq!(opened(&mut s, 0, 0).list_counted().unwrap().1, 500);
    let mut s = Script::new("* LIST () \"/\" a\r\n* LIST () \"/\" a\r\nA0001 OK done\r\n");
    assert_eq!(
        opened(&mut s, 0, 0).list_counted().unwrap(),
        (vec![Mailbox { name: "a".into(), raw: "a".into(), delimiter: Some('/'), flags: Vec::new() }], 0)
    );
}

/// #991: a list keeps a bounded number of bytes too, however many mailboxes that is.
#[test]
fn a_list_keeps_a_capped_number_of_bytes() {
    let flags: Vec<String> = (0..MAX_FLAGS).map(|n| format!("\\F{n}{}", "x".repeat(MAX_FLAG_LEN - 8))).collect();
    let flags = flags.join(" ");
    let server = flood(1_500, |n| format!("* LIST ({flags}) \"/\" f{n}\r\n")) + "A0001 OK done\r\n";
    let mut s = Script::new(server);
    let (got, dropped) = opened(&mut s, 0, 0).list_counted().unwrap();
    assert_eq!(got.len() + dropped, 1_500);
    let kept: usize = got.iter().map(|m| m.raw.len() * 2 + m.name.len() + m.flags.iter().map(String::len).sum::<usize>()).sum();
    assert!(!got.is_empty() && got.len() < 1_500, "{}", got.len());
    // The last one kept may take it over by one mailbox's worth, never more.
    assert!(kept < MAX_LIST_BYTES + MAX_LINE, "{kept}");
}

#[test]
fn names_and_flags_are_kept_short() {
    let long_flag = "é".repeat(400);
    let long_name = "n".repeat(MAX_MAILBOX_NAME + 1);
    let server = format!("* LIST ({long_flag} \\Sent) \"/\" Fine\r\n* LIST () \"/\" {long_name}\r\nA0001 OK done\r\n");
    let mut s = Script::new(server);
    let got = opened(&mut s, 0, 0).list().unwrap();
    // The mailbox with the oversized name is left out, not cut into another one.
    assert_eq!(got.len(), 1);
    assert!(got[0].flags[0].len() <= MAX_FLAG_LEN && got[0].flags[0].chars().all(|c| c == 'é'));
    assert_eq!(got[0].flags[1], "\\Sent");

    let server = format!("* 1 FETCH (UID 1 FLAGS ({long_flag}))\r\nA0001 OK done\r\n");
    let mut s = Script::new(server);
    let got = opened(&mut s, 1, 2).uid_fetch_flags("1:*").unwrap();
    assert!(got[0].flags[0].len() <= MAX_FLAG_LEN);
}

#[test]
fn only_uids_that_were_asked_for_and_exist_are_kept() {
    let server = "* 1 FETCH (UID 3 FLAGS (\\Seen))\r\n* 2 FETCH (UID 7 FLAGS ())\r\n* 3 FETCH (UID 9 FLAGS ())\r\n* 4 FETCH (UID 4000000000 FLAGS ())\r\n\
                  * 5 FETCH (UID 10 FLAGS ())\r\nA0001 OK done\r\n";
    // Not in the set.
    let mut s = Script::new(server);
    let got = opened(&mut s, 5, 11).uid_fetch_flags("3,8:9").unwrap();
    assert_eq!(got.iter().map(|f| f.uid).collect::<Vec<_>>(), [3, 9]);
    // Not below UIDNEXT: 10 isn't, so `*` stops short of it.
    let sizes = server.replace("FLAGS (\\Seen)", "RFC822.SIZE 5").replace("FLAGS ()", "RFC822.SIZE 6");
    let mut s = Script::new(sizes);
    let got = opened(&mut s, 5, 10).uid_fetch_sizes("1:*").unwrap();
    assert_eq!(got, [(3, 5), (7, 6), (9, 6)]);
}

#[test]
fn more_distinct_uids_than_the_mailbox_can_hold_fail_the_command() {
    // 3 messages: room for 2 * 3 + FETCH_SLACK distinct UIDs, and not one more.
    let room = 2 * 3 + FETCH_SLACK;
    let many = |n: usize| flood(n, |i| format!("* {i} FETCH (UID {} FLAGS ())\r\n", i + 1)) + "A0001 OK done\r\n";
    let mut s = Script::new(many(room));
    assert_eq!(opened(&mut s, 3, u32::MAX).uid_fetch_flags("1:*").unwrap().len(), room);
    let mut s = Script::new(many(room + 1));
    assert!(matches!(opened(&mut s, 3, u32::MAX).uid_fetch_flags("1:*"), Err(ProtoError::Protocol(_))));
    let sizes = flood(room + 1, |i| format!("* {i} FETCH (UID {} RFC822.SIZE 1)\r\n", i + 1)) + "A0001 OK done\r\n";
    let mut s = Script::new(sizes);
    assert!(matches!(opened(&mut s, 3, u32::MAX).uid_fetch_sizes("1:*"), Err(ProtoError::Protocol(_))));
    // The same UID again and again is one.
    let same = flood(room * 3, |_| "* 1 FETCH (UID 1 FLAGS ())\r\n".to_string()) + "A0001 OK done\r\n";
    let mut s = Script::new(same);
    assert_eq!(opened(&mut s, 3, 100).uid_fetch_flags("1:*").unwrap().len(), 1);
}

#[test]
fn set_ranges_read_what_is_written() {
    assert_eq!(set_ranges("1:*", 9), Some(vec![(1, 9)]));
    assert_eq!(set_ranges("5,9,20:12", 9), Some(vec![(5, 5), (9, 9), (12, 20)]));
    assert_eq!(set_ranges("1:x", 9), None);
    assert_eq!(set_ranges("", 9), None);
}

proptest! {
    /// Whatever the server sends, the command ends, and what it keeps is bounded by the caps.
    #[test]
    fn arbitrary_responses_are_kept_within_bounds(lines in proptest::collection::vec(("[ -~]{0,200}", any::<u32>(), 0usize..4), 0..400)) {
        let server: String = lines
            .iter()
            .map(|(text, uid, kind)| match kind {
                0 => format!("* LIST (\\{text}) \"/\" {text}\r\n"),
                1 => format!("* 1 FETCH (UID {uid} FLAGS ({text}))\r\n"),
                2 => format!("* 1 FETCH (UID {uid} RFC822.SIZE {uid})\r\n"),
                _ => format!("* {text}\r\n"),
            })
            .collect::<String>()
            + "A0001 OK done\r\n";
        let mut s = Script::new(server.clone());
        if let Ok(list) = opened(&mut s, 10, 50).list() {
            prop_assert!(list.len() <= MAX_MAILBOXES);
            prop_assert!(list.iter().all(|m| m.raw.len() <= MAX_MAILBOX_NAME && m.flags.len() <= MAX_FLAGS && m.flags.iter().all(|f| f.len() <= MAX_FLAG_LEN)));
        }
        let mut s = Script::new(server.clone());
        if let Ok(got) = opened(&mut s, 10, 50).uid_fetch_flags("1:*") {
            prop_assert!(got.len() <= 2 * 10 + FETCH_SLACK);
            prop_assert!(got.iter().all(|f| f.uid >= 1 && f.uid < 50 && f.flags.iter().all(|x| x.len() <= MAX_FLAG_LEN)));
        }
        let mut s = Script::new(server);
        if let Ok(got) = opened(&mut s, 10, 50).uid_fetch_sizes("5:20") {
            prop_assert!(got.iter().all(|(uid, _)| (5..=20).contains(uid)));
        }
    }
}

#[test]
fn a_folder_with_more_messages_than_other_commands_may_answer_still_reads() {
    let exists = 300_000u32;
    let lines = |n: u32| flood(n as usize, |i| format!("* {i} FETCH (UID {} FLAGS ())\r\n", i + 1)) + "A0001 OK done\r\n";
    let mut s = Script::new(lines(exists));
    assert_eq!(opened(&mut s, exists, exists + 1).uid_fetch_flags("1:*").unwrap().len(), exists as usize);
    let sizes = flood(exists as usize, |i| format!("* {i} FETCH (UID {} RFC822.SIZE 1)\r\n", i + 1)) + "A0001 OK done\r\n";
    let mut s = Script::new(sizes);
    assert_eq!(opened(&mut s, exists, exists + 1).uid_fetch_sizes("1:*").unwrap().len(), exists as usize);
    // Responses that are not messages still end at the cap for that folder, and the next command is back to the usual one.
    let noise = "* OK still here\r\n".repeat(2 * exists as usize + FETCH_SLACK + 1) + "A0001 OK done\r\n";
    let mut s = Script::new(noise);
    let mut imap = opened(&mut s, exists, exists + 1);
    assert!(matches!(imap.uid_fetch_flags("1:*"), Err(ProtoError::Protocol(_))));
    assert_eq!(imap.response_cap, MAX_RESPONSES);
}
