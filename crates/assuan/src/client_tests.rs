use proptest::prelude::*;

use crate::fake::{Canned, FakeAgent};
use crate::{Answer, Client, Error, MAX_DATA, MAX_INQUIRIES, MAX_INQUIRY_DATA, MAX_LINE, MAX_LINES, MAX_STATUS};

#[test]
fn a_greeting_then_a_command_with_data_status_and_comments() {
    let agent = FakeAgent::new().on("GETINFO version", "# here it comes\nS PROGRESS x\nD 2.4%2E7\nD -beta\nOK done\n");
    let mut client = Client::connect(agent).unwrap();
    let r = client.command("GETINFO version").unwrap();
    assert_eq!(r.data, b"2.4.7-beta".to_vec());
    assert_eq!(r.status("PROGRESS"), Some("x"));
    assert_eq!(r.ok, "done");
    assert!(client.into_inner().done());
}

#[test]
fn a_refused_greeting() {
    let agent = FakeAgent::greeting(b"ERR 1 go away\n");
    let e = Client::connect(agent).unwrap_err();
    assert_eq!(e.server().map(|e| e.code), Some(1));
}

#[test]
fn an_err_leaves_the_connection_usable() {
    let agent = FakeAgent::new().on("HAVEKEY X", "ERR 67108881 No secret key\n").on("NOP", "OK\n");
    let mut client = Client::connect(agent).unwrap();
    let e = client.command("HAVEKEY X").unwrap_err();
    assert_eq!(e.server().map(|e| e.kind()), Some(crate::ErrorKind::NoSecretKey));
    client.command("NOP").unwrap();
    assert!(client.into_inner().done());
}

#[test]
fn junk_breaks_the_connection_for_good() {
    let agent = FakeAgent::new().on("NOP", "HELLO\n");
    let mut client = Client::connect(agent).unwrap();
    assert!(matches!(client.command("NOP"), Err(Error::Protocol(_))));
    assert_eq!(client.command("NOP").unwrap_err(), Error::Broken);
}

#[test]
fn an_inquiry_is_answered_with_data_or_refused() {
    let agent = FakeAgent::new()
        .on("PKDECRYPT", "S INQUIRE_MAXLEN 4096\nINQUIRE CIPHERTEXT\n")
        .on("D 1%0A2", "")
        .on("END", "D ok\nOK\n")
        .on("PKSIGN", "INQUIRE SOMETHING odd\n")
        .on("CAN", "ERR 99 Operation cancelled\n");
    let mut client = Client::connect(agent).unwrap();
    let mut asked = Vec::new();
    let r = client
        .transact("PKDECRYPT", &mut |i| {
            asked.push(i.keyword.clone());
            if i.keyword == "CIPHERTEXT" { Answer::Data(b"1\n2".to_vec()) } else { Answer::Cancel }
        })
        .unwrap();
    assert_eq!(r.data, b"ok".to_vec());
    assert_eq!(asked, ["CIPHERTEXT"]);
    let e = client.command("PKSIGN").unwrap_err();
    assert_eq!(e.server().map(|e| e.kind()), Some(crate::ErrorKind::Canceled));
    assert!(client.into_inner().done());
}

#[test]
fn an_answer_too_big_to_send_is_refused_and_said() {
    let agent = FakeAgent::new().on("PKDECRYPT", "INQUIRE CIPHERTEXT\n").on("CAN", "ERR 99 cancelled\n").on("NOP", "OK\n");
    let mut client = Client::connect(agent).unwrap();
    let e = client.transact("PKDECRYPT", &mut |_| Answer::Data(vec![0; MAX_INQUIRY_DATA + 1])).unwrap_err();
    assert!(matches!(e, Error::Refused(_)), "{e:?}");
    client.command("NOP").unwrap();
    assert!(client.into_inner().done());
}

#[test]
fn commands_that_cant_go_on_one_line_are_never_sent() {
    let mut client = Client::connect(FakeAgent::new()).unwrap();
    for bad in ["", "NOP\nBYE", "NOP\r", "NOP\0"] {
        assert!(matches!(client.command(bad), Err(Error::Refused(_))), "{bad:?}");
    }
    assert!(matches!(client.command(&"A".repeat(MAX_LINE)), Err(Error::Refused(_))));
    let agent = client.into_inner();
    assert!(agent.sent.is_empty());
}

#[test]
fn too_many_inquiries_status_lines_lines_or_data_are_stopped() {
    let inquiries = "INQUIRE X\n".repeat(MAX_INQUIRIES + 1);
    let agent = FakeAgent::new().on("NOP", inquiries);
    let mut client = Client::connect(agent).unwrap();
    // Each inquiry is answered with an empty D … END that the script doesn't expect: what matters
    // is that the client stops.
    assert_eq!(client.transact("NOP", &mut |_| Answer::Data(Vec::new())).err(), Some(Error::TooMuch));

    let status = "S X y\n".repeat(MAX_STATUS + 1);
    let mut client = Client::connect(Canned::new(format!("OK\n{status}OK\n"))).unwrap();
    assert_eq!(client.command("NOP").err(), Some(Error::TooMuch));

    let comments = "#\n".repeat(MAX_LINES + 1);
    let mut client = Client::connect(Canned::new(format!("OK\n{comments}OK\n"))).unwrap();
    assert_eq!(client.command("NOP").err(), Some(Error::TooMuch));

    let line = format!("D {}\n", "x".repeat(MAX_LINE - 3));
    let data = line.repeat(MAX_DATA / (MAX_LINE - 3) + 1);
    let mut client = Client::connect(Canned::new(format!("OK\n{data}OK\n"))).unwrap();
    assert_eq!(client.command("NOP").err(), Some(Error::TooMuch));
}

#[test]
fn an_oversized_line_and_a_closed_stream() {
    let agent = FakeAgent::new().on("NOP", format!("D {}\n", "x".repeat(MAX_LINE)));
    let mut client = Client::connect(agent).unwrap();
    assert_eq!(client.command("NOP").err(), Some(Error::LineTooLong));

    let mut client = Client::connect(Canned::new("OK\nD half")).unwrap();
    assert_eq!(client.command("NOP").err(), Some(Error::Closed));
    assert!(Client::connect(Canned::new("")).is_err());
}

#[test]
fn debug_never_shows_data() {
    let agent = FakeAgent::new().on("PKDECRYPT", "D secret\nOK\n");
    let mut client = Client::connect(agent).unwrap();
    let r = client.command("PKDECRYPT").unwrap();
    let shown = format!("{r:?} {:?}", Answer::Data(b"secret".to_vec()));
    assert!(!shown.contains("secret"), "{shown}");
}

proptest! {
    #[test]
    fn any_server_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        if let Ok(mut client) = Client::connect(Canned::new(&bytes)) {
            for _ in 0..8 {
                let _ = client.transact("NOP", &mut |i| if i.keyword.len() % 2 == 0 { Answer::Data(b"x\n%".to_vec()) } else { Answer::Cancel });
            }
        }
    }

    #[test]
    fn any_lines_of_the_protocol_never_panic(lines in proptest::collection::vec(
        prop_oneof![
            Just("OK".to_string()), Just("OK x".to_string()), Just("ERR 99 no".to_string()), Just("ERR".to_string()),
            Just("D %41%".to_string()), Just("S KEYINFO 1 2 3".to_string()), Just("INQUIRE CIPHERTEXT".to_string()),
            Just("INQUIRE PINENTRY_LAUNCHED 1 2".to_string()), Just("# c".to_string()), "[ -~]{0,40}",
        ], 0..64)) {
        let text = lines.join("\n") + "\n";
        if let Ok(mut client) = Client::connect(Canned::new(text)) {
            for _ in 0..8 {
                let _ = client.transact("NOP", &mut |_| Answer::Data(b"abc".to_vec()));
            }
        }
    }
}

#[test]
fn growing_a_buffer_wipes_the_old_allocation() {
    let mut buf = Vec::with_capacity(4);
    buf.extend_from_slice(b"key!");
    let old = crate::grow(&mut buf, 9);
    assert!(old.iter().all(|b| *b == 0), "{old:?}");
    assert_eq!(buf, b"key!");
    crate::extend_wiped(&mut buf, b"more bytes");
    assert_eq!(buf, b"key!more bytes");

    let mut buf = b"0123456789".to_vec();
    crate::discard_front(&mut buf, 4);
    assert_eq!(buf, b"456789");
    crate::discard_front(&mut buf, 99);
    assert!(buf.is_empty());
}

#[test]
fn status_lines_handed_over_as_they_come_are_not_capped_by_max_status() {
    let status = "S KEYINFO x\n".repeat(MAX_STATUS * 3);
    let agent = FakeAgent::new().on("KEYINFO --list", format!("{status}OK\n"));
    let mut client = Client::connect(agent).unwrap();
    let mut seen = 0;
    let r = client.command_with_status("KEYINFO --list", &mut |_| seen += 1).unwrap();
    assert_eq!(seen, MAX_STATUS * 3);
    assert!(r.status.is_empty());
}

/// The text of `source` from `start` up to `end`.
fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let from = source.find(start).unwrap();
    let to = from + source[from..].find(end).unwrap();
    &source[from..to]
}

#[test]
fn what_the_agent_sends_only_grows_through_the_wiping_helpers() {
    // A plain `extend_from_slice` or `drain` on these buffers leaves an unwiped copy of what
    // the agent sent (a session key, perhaps) in memory that is given back: the reading and the
    // gathering of data must go through `extend_wiped` and `discard_front`.
    let fill = section(include_str!("line.rs"), "fn fill(", "fn write_line(");
    assert!(fill.contains("crate::discard_front(") && fill.contains("crate::extend_wiped("), "{fill}");
    assert!(!fill.contains("extend_from_slice") && !fill.contains(".drain(") && !fill.contains(".extend("), "{fill}");
    let data = section(include_str!("client.rs"), "Line::Data(", "Line::Status(");
    assert!(data.contains("crate::extend_wiped(&mut response.data") && data.contains("crate::wipe(&mut data)"), "{data}");
    assert!(!data.contains("extend_from_slice") && !data.contains(".extend("), "{data}");
}
