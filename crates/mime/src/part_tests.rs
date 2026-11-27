//! The reader: MailCraft's message tests and WordCraft's MHT tests, at the level of the tree.

use super::*;

/// A message from lines, joined with CRLF.
fn eml(lines: &[&str]) -> Vec<u8> {
    lines.join("\r\n").into_bytes()
}

fn read(bytes: &[u8]) -> Message {
    parse(bytes, &Limits::default())
}

/// The media type and text of every leaf.
fn leaves(m: &Message) -> Vec<(String, String)> {
    m.root.leaves().iter().map(|p| (p.media_type().to_string(), p.text())).collect()
}

fn pair(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

#[test]
fn an_alternative_without_its_preamble_and_epilogue() {
    let m = read(&eml(&[
        "Content-Type: multipart/alternative; boundary=\"alt-1\"",
        "",
        "This is the preamble, for readers that don't know MIME.",
        "--alt-1",
        "Content-Type: text/plain; charset=utf-8",
        "",
        "Hello *world*",
        "--alt-1",
        "Content-Type: text/html; charset=utf-8",
        "",
        "<p>Hello <b>world</b></p>",
        "--alt-1--",
        "The epilogue.",
    ]));
    assert!(m.root.is_multipart() && m.root.data().is_empty());
    assert_eq!(leaves(&m), [pair("text/plain", "Hello *world*"), pair("text/html", "<p>Hello <b>world</b></p>")]);
}

#[test]
fn related_parts_with_ids_names_and_encodings() {
    let m = read(&eml(&[
        "Content-Type: multipart/related; boundary=\"rel\"; type=\"text/html\"",
        "",
        "--rel",
        "Content-Type: text/html; charset=utf-8",
        "",
        "<p>Logo: <img src=\"cid:logo@example.org\" alt=\"Our logo\"></p>",
        "--rel",
        "Content-Type: image/png; name=\"logo.png\"",
        "Content-Transfer-Encoding: base64",
        "Content-ID: <logo@example.org>",
        "",
        "iVBORw0KGgo=",
        "--rel",
        "Content-Type: text/plain; charset=utf-8",
        "Content-Disposition: attachment;",
        " filename*0*=utf-8''r%C3%A9sum%C3%A9%20;",
        " filename*1*=%E2%82%AC.txt",
        "Content-Transfer-Encoding: quoted-printable",
        "",
        "Prix: 5 =E2=82=AC=",
        "",
        "--rel",
        "Content-Type: application/octet-stream",
        "Content-Disposition: inline; filename=\"=?UTF-8?B?csOpc3Vtw6kucGRm?=\"",
        "Content-Location: <file:///C:/x/r.pdf>",
        "",
        "raw",
        "--rel--",
    ]));
    let parts = m.root.parts();
    assert_eq!(parts.len(), 4);
    assert_eq!(m.root.content_type.get("type"), Some("text/html"));
    let logo = &parts[1];
    assert_eq!((logo.media_type(), logo.file_name().as_deref(), logo.content_id()), ("image/png", Some("logo.png"), Some("logo@example.org")));
    assert_eq!((logo.transfer_encoding().as_str(), logo.data()), ("base64", &b"\x89PNG\r\n\x1a\n"[..]));
    assert_eq!(logo.disposition().value, "");
    let file = &parts[2];
    assert_eq!((file.disposition().value.as_str(), file.file_name().as_deref()), ("attachment", Some("résumé €.txt")));
    assert_eq!(file.text(), "Prix: 5 €");
    let raw = &parts[3];
    assert_eq!((raw.disposition().value.as_str(), raw.file_name().as_deref()), ("inline", Some("résumé.pdf")));
    assert_eq!((raw.content_location(), raw.data()), (Some("file:///C:/x/r.pdf"), &b"raw"[..]));
    assert_eq!(parts[0].content_location(), None);
}

#[test]
fn a_truncated_multipart_keeps_what_arrived() {
    let m = read(&eml(&[
        "Content-Type: multipart/mixed; boundary=cut",
        "",
        "--cut",
        "Content-Type: text/plain",
        "",
        "The text arrived.",
        "--cut",
        "Content-Type: application/pdf; name=half.pdf",
        "Content-Transfer-Encoding: base64",
        "",
        "JVBERi0xLjQKJ",
    ]));
    assert_eq!(m.root.parts().len(), 2);
    assert_eq!(m.root.parts()[0].data(), b"The text arrived.");
    assert_eq!(m.root.parts()[1].data(), b"%PDF-1.4\n");
    // Cut inside the headers of a part.
    let m = read(&eml(&["Content-Type: multipart/mixed; boundary=cut", "", "--cut", "Content-Type: text/pl"]));
    assert_eq!(m.root.parts().len(), 1);
    assert_eq!((m.root.parts()[0].media_type(), m.root.parts()[0].data()), ("text/pl", &b""[..]));
}

#[test]
fn a_missing_or_wrong_boundary() {
    // No boundary to split on: the raw body is kept, so that a caller can show it.
    let m = read(&eml(&["Content-Type: multipart/mixed", "", "--who-knows", "", "Hidden text", "--who-knows--"]));
    assert!(m.root.is_multipart() && m.root.parts().is_empty());
    assert_eq!(m.root.data(), b"--who-knows\r\n\r\nHidden text\r\n--who-knows--");
    let m = read(&eml(&["Content-Type: multipart/mixed; boundary=\"\"", "Content-Transfer-Encoding: base64", "", "text"]));
    assert_eq!(m.root.data(), b"text");
    // The boundary never appears: there are no parts.
    let m = read(&eml(&["Content-Type: multipart/mixed; boundary=absent", "", "--other", "", "text", "--other--"]));
    assert_eq!(m.root.body, Body::Parts(Vec::new()));
    // A boundary on something that is no multipart means nothing.
    assert_eq!(read(b"Content-Type: text/plain; boundary=b\n\n--b\nx\n--b--").root.data(), b"--b\nx\n--b--");
}

#[test]
fn boundaries_that_look_alike() {
    let m = read(&eml(&[
        "Content-Type: multipart/mixed; boundary=b",
        "",
        "--b",
        "",
        "--bb is not a delimiter",
        "--b-- neither is this",
        "--b  ",
        "",
        "second",
        "--b--  ",
        "--b",
        "",
        "after the end",
    ]));
    assert_eq!(leaves(&m), [pair("text/plain", "--bb is not a delimiter\r\n--b-- neither is this"), pair("text/plain", "second")]);
}

#[test]
fn an_mbox_from_line_is_skipped_at_the_top_only() {
    let bytes = b"From ada@example.org Sat Jan  3 01:05:34 1996\r\nSubject: x\r\n\r\nB";
    assert_eq!(read(bytes).root.headers.get("subject"), Some("x"));
    let entity = parse_entity(bytes, &Limits::default());
    assert!(entity.root.headers.0.is_empty());
    assert_eq!(entity.root.data(), bytes);
}

// Each limit.

#[test]
fn deep_nesting_is_capped() {
    let nested = |levels: usize| {
        let mut deep = Vec::new();
        for i in 0..levels {
            deep.extend_from_slice(format!("Content-Type: multipart/mixed; boundary=b{i}\r\n\r\n--b{i}\r\n").as_bytes());
        }
        deep.extend_from_slice(b"Content-Type: text/plain\r\n\r\ndeep text\r\n");
        deep
    };
    let depth = |m: &Message| {
        let (mut part, mut depth) = (&m.root, 0);
        while let Some(first) = part.parts().first() {
            (part, depth) = (first, depth + 1);
        }
        (depth, part.data().to_vec())
    };
    let m = read(&nested(20));
    assert_eq!((depth(&m), m.truncated), ((20, b"deep text\r\n".to_vec()), false));
    // The multipart at the cap is not opened: it has no parts.
    let m = read(&nested(21));
    assert_eq!((depth(&m), m.truncated), ((20, Vec::new()), true));
    assert!(m.root.leaves().is_empty());
    // Whatever is asked for, the hard cap holds (the reader recurses once per level).
    let m = parse(&nested(5000), &Limits { max_depth: usize::MAX, max_parts: usize::MAX, ..Limits::default() });
    assert_eq!((depth(&m).0, m.truncated), (Limits::HARD_MAX_DEPTH, true));
    let m = parse(&nested(3), &Limits { max_depth: 0, ..Limits::default() });
    assert_eq!((m.root.body.clone(), m.truncated), (Body::Parts(Vec::new()), true));
}

#[test]
fn the_number_of_parts_is_capped() {
    let wide = |n: usize| {
        let mut wide = b"Content-Type: multipart/mixed; boundary=w\r\n\r\n".to_vec();
        for _ in 0..n {
            wide.extend_from_slice(b"--w\r\nContent-Type: application/x-a\r\n\r\nx\r\n");
        }
        wide.extend_from_slice(b"--w--\r\n");
        wide
    };
    let max = Limits::DEFAULT.max_parts;
    // The message itself is a part.
    let m = read(&wide(max + 500));
    assert_eq!((m.root.parts().len(), m.truncated), (max - 1, true));
    let m = read(&wide(max - 1));
    assert_eq!((m.root.parts().len(), m.truncated), (max - 1, false));
    // Parts of nested multiparts count too.
    let nested = b"Content-Type: multipart/mixed; boundary=a\n\n--a\nContent-Type: multipart/mixed; boundary=b\n\n--b\n\n1\n--b\n\n2\n--b\n\n3\n--b--\n--a\n\n4\n--a--\n";
    let m = parse(nested, &Limits { max_parts: 4, ..Limits::default() });
    assert_eq!((m.root.leaves().len(), m.truncated), (2, true));
    assert_eq!(parse(nested, &Limits { max_parts: 6, ..Limits::default() }).root.leaves().len(), 4);
    assert_eq!(split_multipart(b"--b\n1\n--b\n2\n--b\n3\n--b--\n", "b", 2), [&b"1"[..], b"2"]);
    assert!(split_multipart(b"--b\n1\n--b--\n", "b", 0).is_empty());
}

#[test]
fn the_decoded_size_is_capped() {
    let bytes = b"Content-Type: multipart/mixed; boundary=b\n\n--b\n\n0123456789\n--b\nContent-Transfer-Encoding: base64\n\nMDEyMzQ1Njc4OQ==\n--b\n\nabc\n--b--\n";
    let m = parse(bytes, &Limits { max_decoded: 15, ..Limits::default() });
    let data: Vec<&[u8]> = m.root.parts().iter().map(Part::data).collect();
    // The second part would pass the cap and is empty; the third still fits.
    assert_eq!((data, m.truncated), (vec![&b"0123456789"[..], b"", b"abc"], true));
    let m = parse(bytes, &Limits { max_decoded: 23, ..Limits::default() });
    assert_eq!((m.root.parts()[1].data(), m.truncated), (&b"0123456789"[..], false));
    assert!(parse(b"Content-Type: multipart/mixed\n\nraw", &Limits { max_decoded: 2, ..Limits::default() }).truncated);
}

proptest::proptest! {
    #[test]
    fn mime_soup_parses(s in "(Content-Type: |multipart/mixed|; boundary=b|--b|--b--|\n|\r\n|text/html|Content-Transfer-Encoding: |base64|quoted-printable|=|a| |\\(|\\)|\"){0,60}") {
        let small = Limits { max_parts: 5, max_depth: 2, max_decoded: 40, max_header_fields: 3, max_header_value: 30, max_header_block: 60, max_input: 300, ..Limits::default() };
        for limits in [Limits::default(), small] {
            let m = parse(s.as_bytes(), &limits);
            let leaves = m.root.leaves();
            let total: usize = leaves.iter().map(|p| p.data().len()).sum();
            proptest::prop_assert!(total <= limits.max_decoded.min(s.len()));
            for part in leaves {
                let _ = (part.text(), part.file_name(), part.content_id(), part.content_location(), part.disposition());
            }
        }
    }
}
