use proptest::prelude::*;

use crate::fake::Canned;
use crate::line::{Line, Wire, escape_data, parse, unescape};
use crate::{Error, MAX_LINE, ServerError};

#[test]
fn each_kind_of_line() {
    assert_eq!(parse(b"OK Pleased to meet you").unwrap(), Line::Ok(b"Pleased to meet you".to_vec()));
    assert_eq!(parse(b"OK").unwrap(), Line::Ok(Vec::new()));
    assert_eq!(parse(b"D (3:abc)").unwrap(), Line::Data(b"(3:abc)".to_vec()));
    assert_eq!(parse(b"D 100%25%0A%0d").unwrap(), Line::Data(b"100%\n\r".to_vec()));
    assert_eq!(parse(b"# a comment").unwrap(), Line::Comment);
    assert_eq!(parse(b"#").unwrap(), Line::Comment);
    assert_eq!(parse(b"S PADDING 0").unwrap(), Line::Status(b"PADDING".to_vec(), b"0".to_vec()));
    assert_eq!(parse(b"S PROGRESS").unwrap(), Line::Status(b"PROGRESS".to_vec(), Vec::new()));
    assert_eq!(parse(b"INQUIRE CIPHERTEXT").unwrap(), Line::Inquire(b"CIPHERTEXT".to_vec(), Vec::new()));
    assert_eq!(
        parse(b"ERR 67108881 No secret key <GPG Agent>").unwrap(),
        Line::Err(ServerError { code: 67_108_881, text: "No secret key <GPG Agent>".into() })
    );
}

#[test]
fn junk_lines_are_errors_not_panics() {
    for junk in [&b""[..], b"HELLO", b"ok", b"ERR", b"ERR x", b"ERR 99999999999", b"S", b"S \x01", b"INQUIRE ", b"\xff\xfe"] {
        assert!(matches!(parse(junk), Err(Error::Protocol(_))), "{junk:?}");
    }
}

#[test]
fn an_error_text_is_tidied() {
    let Line::Err(e) = parse(b"ERR 99 bad%0Aline\x07 here").unwrap() else { panic!() };
    assert_eq!(e.text, "bad line  here");
    assert_eq!(e.kind(), crate::ErrorKind::Canceled);
    let long = format!("ERR 1 {}", "x".repeat(900));
    let Line::Err(e) = parse(long.as_bytes()).unwrap() else { panic!() };
    assert_eq!(e.text.len(), 200);
}

#[test]
fn unescaping_keeps_a_stray_percent() {
    assert_eq!(unescape(b"50% off %zz %4"), b"50% off %zz %4".to_vec());
    assert_eq!(unescape(b"%41%42c"), b"ABc".to_vec());
}

#[test]
fn a_line_of_the_longest_size_is_read_and_one_byte_more_is_not() {
    let longest = format!("D {}\n", "a".repeat(MAX_LINE - 3));
    assert_eq!(longest.len(), MAX_LINE);
    let mut wire = Wire::new(Canned::new(&longest));
    assert_eq!(wire.read_line().unwrap().len(), MAX_LINE - 1);

    let over = format!("D {}\n", "a".repeat(MAX_LINE - 2));
    let mut wire = Wire::new(Canned::new(&over));
    assert_eq!(wire.read_line(), Err(Error::LineTooLong));

    // One that never ends is stopped too, without reading it all.
    let endless = "a".repeat(10 * MAX_LINE);
    let mut wire = Wire::new(Canned::new(&endless));
    assert_eq!(wire.read_line(), Err(Error::LineTooLong));
}

#[test]
fn the_end_of_the_stream_is_closed() {
    let mut wire = Wire::new(Canned::new("OK\nhalf"));
    assert_eq!(wire.read_line().unwrap(), b"OK".to_vec());
    assert_eq!(wire.read_line(), Err(Error::Closed));
}

#[test]
fn data_goes_in_lines_of_at_most_the_longest_size() {
    let data: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
    let mut wire = Wire::new(Canned::new(""));
    wire.write_data(&data).unwrap();
    let written = wire.into_inner().written;
    let mut back = Vec::new();
    let lines: Vec<&[u8]> = written.split(|b| *b == b'\n').collect();
    assert_eq!(lines.last(), Some(&&b""[..]));
    for line in &lines[..lines.len() - 2] {
        assert!(line.len() < MAX_LINE, "{}", line.len());
        assert!(line.starts_with(b"D "));
        assert!(!line.contains(&b'\r'));
        back.extend(unescape(&line[2..]));
    }
    assert_eq!(lines[lines.len() - 2], b"END");
    assert_eq!(back, data);
}

#[test]
fn escaping_data() {
    let mut out = Vec::new();
    for b in b"a%\r\nb" {
        escape_data(&mut out, *b);
    }
    assert_eq!(out, b"a%25%0D%0Ab".to_vec());
}

proptest! {
    #[test]
    fn any_bytes_read_as_lines_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let mut wire = Wire::new(Canned::new(&bytes));
        for _ in 0..64 {
            match wire.read_line() {
                Ok(line) => {
                    prop_assert!(line.len() < MAX_LINE);
                    let _ = parse(&line);
                }
                Err(_) => break,
            }
        }
    }

    #[test]
    fn any_data_comes_back_through_escaping(data in proptest::collection::vec(any::<u8>(), 0..3000)) {
        let mut wire = Wire::new(Canned::new(""));
        wire.write_data(&data).unwrap();
        let written = wire.into_inner().written;
        let mut reader = Wire::new(Canned::new(&written));
        let mut back = Vec::new();
        loop {
            let line = reader.read_line().unwrap();
            if line == b"END" {
                break;
            }
            match parse(&line).unwrap() {
                Line::Data(d) => back.extend(d),
                other => panic!("{other:?}"),
            }
        }
        prop_assert_eq!(back, data);
    }
}
