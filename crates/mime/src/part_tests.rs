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
fn a_plain_message() {
    let m = read(&eml(&[
        "Return-Path: <ada@example.org>",
        "Received: from mail.example.org (mail.example.org [192.0.2.1])",
        "\tby mx.example.org with ESMTP id 4F2A1; Tue, 1 Jul 2003 10:52:40 +0200",
        "Date: Tue, 1 Jul 2003 10:52:37 +0200",
        "From: Ada Lovelace <ada@example.org>",
        "Subject: =?UTF-8?Q?Caf=C3=A9_=E2=82=AC5?=",
        " =?UTF-8?Q?_tomorrow?=",
        "MIME-Version: 1.0",
        "Content-Type: text/plain; charset=us-ascii",
        "",
        "Hello Bob,",
        "",
        "Ada",
        "",
    ]));
    assert!(!m.truncated);
    let names: Vec<&str> = m.root.headers.0.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Return-Path", "Received", "Date", "From", "Subject", "MIME-Version", "Content-Type"]);
    assert_eq!(
        m.root.headers.get("received"),
        Some("from mail.example.org (mail.example.org [192.0.2.1])\tby mx.example.org with ESMTP id 4F2A1; Tue, 1 Jul 2003 10:52:40 +0200")
    );
    assert_eq!(m.root.headers.get("subject").map(crate::decode_words).as_deref(), Some("Café €5 tomorrow"));
    assert_eq!(m.root.headers.get("date").and_then(crate::parse_date).map(|d| (d.day, d.offset_minutes)), Some((1, 120)));
    assert_eq!(crate::parse_address_list(m.root.headers.get("FROM").unwrap_or("")).first().map(|a| a.email.as_str()), Some("ada@example.org"));
    assert_eq!((m.root.media_type(), m.root.declared_media_type(), m.root.is_multipart()), ("text/plain", Some("text/plain"), false));
    assert_eq!(m.root.text(), "Hello Bob,\r\n\r\nAda\r\n");
    assert!(m.root.parts().is_empty() && m.root.file_name().is_none() && m.root.content_id().is_none());
    assert_eq!(m.root.leaves().len(), 1);
}

#[test]
fn bare_lf_line_ends_and_no_content_type() {
    let m = read(b"From: ada@example.org\nSubject: Unix\n\nLine one\nLine two\n");
    assert_eq!(m.root.headers.get("subject"), Some("Unix"));
    assert_eq!((m.root.declared_media_type(), m.root.media_type()), (None, "text/plain"));
    assert_eq!(m.root.data(), b"Line one\nLine two\n");
    // A type without a subtype is none.
    assert_eq!(read(b"Content-Type: garbage\n\nx").root.declared_media_type(), None);
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
fn quoted_printable_latin1_and_base64_utf8() {
    let m = read(&eml(&[
        "Content-Type: text/plain; charset=ISO-8859-1",
        "Content-Transfer-Encoding: quoted-printable",
        "",
        "La r=E9union est =E0 10h. Co=FBt: 5=80 (=93environ=94).=20",
        "Une ligne tr=E8s longue qui continue =",
        "sur la suivante.",
    ]));
    assert_eq!(m.root.text(), "La réunion est à 10h. Coût: 5€ (“environ”). \r\nUne ligne très longue qui continue sur la suivante.");
    let m = read(&eml(&[
        "Content-Type: text/plain; charset=\"UTF-8\"",
        "Content-Transfer-Encoding: BASE64",
        "",
        "R3LDvMOfZSBhdXMg",
        "TcO8bmNoZW4g8J+Ziw0KWndlaXRlIFplaWxl",
        "",
    ]));
    assert_eq!(crate::charset::unix_lines(&m.root.text()), "Grüße aus München 🙋\nZweite Zeile");
}

#[test]
fn an_attached_message_is_not_opened() {
    let m = read(&eml(&[
        "Subject: Fwd",
        "Content-Type: multipart/mixed; boundary=fw",
        "",
        "--fw",
        "",
        "See below.",
        "--fw",
        "Content-Type: message/rfc822",
        "",
        "Subject: Inner",
        "Content-Type: multipart/mixed; boundary=inner",
        "",
        "--inner",
        "",
        "Inner text",
        "--inner--",
        "--fw--",
    ]));
    let parts = m.root.parts();
    assert_eq!(parts.len(), 2);
    assert_eq!((parts[0].media_type(), parts[0].data()), ("text/plain", &b"See below."[..]));
    assert_eq!(parts[1].media_type(), "message/rfc822");
    assert!(parts[1].data().starts_with(b"Subject: Inner\r\n"));
    let inner = parse_entity(parts[1].data(), &Limits::default());
    assert_eq!((inner.root.headers.get("subject"), leaves(&inner)), (Some("Inner"), vec![pair("text/plain", "Inner text")]));
}

#[test]
fn nested_multiparts() {
    let m = read(&eml(&[
        "Content-Type: multipart/mixed; boundary=outer",
        "",
        "--outer",
        "Content-Type: multipart/alternative; boundary=alt",
        "",
        "--alt",
        "Content-Type: text/plain",
        "",
        "Plain",
        "--alt",
        "Content-Type: multipart/related; boundary=rel",
        "",
        "--rel",
        "Content-Type: text/html",
        "",
        "<p>Rich</p>",
        "--rel",
        "Content-Type: image/gif",
        "Content-ID: <dot@example.org>",
        "Content-Transfer-Encoding: base64",
        "",
        "R0lGODlh",
        "--rel--",
        "--alt--",
        "--outer",
        "Content-Type: application/zip; name=files.zip",
        "Content-Disposition: attachment",
        "Content-Transfer-Encoding: base64",
        "",
        "UEsDBA==",
        "--outer",
        "Content-Type: text/plain",
        "",
        "-- ",
        "Sent via the example list",
        "--outer--",
    ]));
    let types: Vec<&str> = m.root.leaves().iter().map(|p| p.media_type()).collect();
    assert_eq!(types, ["text/plain", "text/html", "image/gif", "application/zip", "text/plain"]);
    let outer = m.root.parts();
    assert_eq!(outer.iter().map(Part::media_type).collect::<Vec<_>>(), ["multipart/alternative", "application/zip", "text/plain"]);
    assert_eq!(outer[0].parts()[1].parts()[1].data(), b"GIF89a");
    assert_eq!(outer[2].text(), "-- \r\nSent via the example list");
    assert!(!m.truncated);
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
fn garbage_gives_a_mostly_empty_message() {
    assert_eq!(read(b""), Message::default());
    let m = read(b"\x00\x01\x02\xff\xfe binary \x80 junk");
    assert!(m.root.headers.0.is_empty());
    assert_eq!(m.root.data(), b"\x00\x01\x02\xff\xfe binary \x80 junk");
    assert_eq!(read(b"no headers at all, just text\r\nsecond line\r\n").root.data(), b"no headers at all, just text\r\nsecond line\r\n");
    assert_eq!(read(b"\r\n\r\nstarts with blank lines").root.data(), b"\r\nstarts with blank lines");
    let _ = read(&[b'\n'; 10_000]);
    let _ = read(&[b':'; 10_000]);
    let _ = read(b"Content-Type: multipart/mixed; boundary=\r\n\r\n--\r\n----\r\n");
    let _ = read(b"Content-Type: ;;;=;\"\r\nContent-Disposition: \"\r\nContent-Transfer-Encoding: base64\r\n\r\n====");
}

#[test]
fn a_byte_order_mark_and_raw_8bit_text() {
    assert_eq!(read(b"Content-Type: text/plain; charset=utf-8\n\n\xEF\xBB\xBFHello").root.text(), "Hello");
    // No charset: UTF-8 when it is valid, else Windows-1252.
    assert_eq!(read("Subject: x\n\ncafé".as_bytes()).root.text(), "café");
    assert_eq!(read(b"Subject: x\n\ncaf\xE9 \x80").root.text(), "café €");
    assert_eq!(read(b"Content-Type: text/plain; charset=x-unknown\n\ncaf\xE9").root.text(), "caf\u{fffd}");
    assert_eq!(
        read(b"Content-Type: text/html; charset=windows-1250\nContent-Transfer-Encoding: quoted-printable\n\n<p>=E8lanak</p>").root.text(),
        "<p>članak</p>"
    );
}

#[test]
fn an_mbox_from_line_is_skipped_at_the_top_only() {
    let bytes = b"From ada@example.org Sat Jan  3 01:05:34 1996\r\nSubject: x\r\n\r\nB";
    assert_eq!(read(bytes).root.headers.get("subject"), Some("x"));
    let entity = parse_entity(bytes, &Limits::default());
    assert!(entity.root.headers.0.is_empty());
    assert_eq!(entity.root.data(), bytes);
}

// WordCraft's MHT reader.

const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn mht(html_part: &str) -> String {
    format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"----=_B\"\r\n\r\n\
         ------=_B\r\n{html_part}\r\n\
         ------=_B\r\nContent-Location: file:///C:/page_files/dot.png\r\nContent-Transfer-Encoding: base64\r\nContent-Type: image/png\r\n\r\n{PNG_1X1}\r\n\
         ------=_B--\r\n"
    )
}

#[test]
fn an_mht_page_and_its_pictures() {
    let page = "Content-Location: file:///C:/page.htm\r\nContent-Transfer-Encoding: quoted-printable\r\nContent-Type: text/html; charset=\"utf-8\"\r\n\r\n\
                <html><body><p class=3D\"x\">Dobar dan, gospo=C4=91o, ovo je =\r\njedan red.</p><p><img src=3D\"page_files/dot.png\"></p></body></html>";
    let m = read(mht(page).as_bytes());
    let html = m.root.leaves().into_iter().find(|p| p.media_type() == "text/html").map(Part::text).unwrap_or_default();
    assert_eq!(html, "<html><body><p class=\"x\">Dobar dan, gospođo, ovo je jedan red.</p><p><img src=\"page_files/dot.png\"></p></body></html>");
    let resources = crate::Resources::of(&m.root);
    let picture = resources.find("page_files/dot.png", "file:///C:/page.htm");
    assert_eq!(picture.map(|p| (p.media_type(), p.data().len())), Some(("image/png", 70)));
}

#[test]
fn quotes_and_comments_do_not_supply_a_false_boundary() {
    let page = "Content-Type: text/html\r\n\r\n<p>Kept content</p>";
    for name in [r#"name="notes; boundary=wrong""#, r#"name="notes\"; boundary=wrong""#, r#"name="notes (10\" screen; boundary=wrong""#] {
        let m = read(mht(page).replace("multipart/related; boundary=", &format!("multipart/related; {name}; boundary=")).as_bytes());
        assert_eq!(m.root.parts().first().map(Part::text).as_deref(), Some("<p>Kept content</p>"), "{name}");
    }
    for comment in [r#"(10" screen)"#, r#"(outer (10" screen); boundary=wrong)"#, r#"(10\)" screen; boundary=wrong)"#] {
        let m = read(mht(page).replace("multipart/related; boundary=", &format!("multipart/related {comment}; boundary=")).as_bytes());
        assert_eq!(m.root.parts().first().map(Part::text).as_deref(), Some("<p>Kept content</p>"), "{comment}");
        assert_eq!(m.root.parts().len(), 2);
    }
}

#[test]
fn hostile_mht_never_panics() {
    for m in [
        &b""[..],
        b"MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"\"\r\n\r\n",
        b"MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\n--x--",
        b"MIME-Version: 1.0\nContent-Type: multipart/related; boundary=x\n\n--x\nContent-Transfer-Encoding: quoted-printable\nContent-Type: text/html\n\n=\n=Z=4",
        b"Content-Type: text/html\r\nContent-Transfer-Encoding: base64\r\n\r\n!!!!",
    ] {
        let m = read(m);
        let _ = (crate::Resources::of(&m.root), crate::start_part(&m.root));
    }
}

// Each limit.

#[test]
fn the_input_size_is_capped() {
    let limits = Limits { max_input: 20, ..Limits::default() };
    let m = parse(b"Subject: x\r\n\r\n0123456789", &limits);
    assert_eq!((m.root.data(), m.truncated), (&b"012345"[..], true));
    let m = parse(b"Subject: x\r\n\r\n012345", &limits);
    assert_eq!((m.root.data(), m.truncated), (&b"012345"[..], false));
}

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

#[test]
fn header_limits_reach_the_message() {
    let limits = Limits { max_header_fields: 2, ..Limits::default() };
    let m = parse(b"A: 1\nB: 2\nContent-Type: text/html\n\nx", &limits);
    assert_eq!((m.root.headers.0.len(), m.root.media_type(), m.truncated), (2, "text/plain", true));
    // In a part's header block too.
    let m = parse(b"Content-Type: multipart/mixed; boundary=b\n\n--b\nA: 1\nB: 2\nC: 3\n\nx\n--b--\n", &limits);
    assert_eq!((m.root.parts()[0].headers.0.len(), m.truncated), (2, true));
}

proptest::proptest! {
    #[test]
    fn any_bytes_parse(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..600)) {
        let m = read(&data);
        let total: usize = m.root.leaves().iter().map(|p| p.data().len()).sum();
        proptest::prop_assert!(total <= data.len());
        proptest::prop_assert!(!m.truncated);
    }

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
