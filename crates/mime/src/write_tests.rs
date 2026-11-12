//! The writer: MailCraft's writer tests, for the parts of it that are format.

use super::*;
use crate::{Limits, parse};

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_default()
}

fn field<'a>(entity: &'a Entity, name: &str) -> Option<&'a str> {
    entity.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

#[test]
fn a_plain_message_byte_for_byte() {
    let subject = encode_unstructured("Subject", "Lunch on Friday?");
    let from = crate::format_address_list(&[crate::Mailbox { name: "Ada Lovelace".into(), email: "ada@example.org".into() }]);
    let headers = [
        ("Date", "Tue, 01 Jul 2003 08:52:37 +0000"),
        ("From", from.as_str()),
        ("Subject", subject.as_str()),
        ("Message-ID", "<abc.123@example.org>"),
    ];
    let want = "Date: Tue, 01 Jul 2003 08:52:37 +0000\r\n\
                From: Ada Lovelace <ada@example.org>\r\n\
                Subject: Lunch on Friday?\r\n\
                Message-ID: <abc.123@example.org>\r\n\
                MIME-Version: 1.0\r\n\
                Content-Type: text/plain; charset=utf-8\r\n\
                Content-Transfer-Encoding: 7bit\r\n\
                \r\n\
                Hello Bob,\r\n\r\nLunch?\r\n\r\n";
    assert_eq!(text(write_message(&headers, &Entity::text("plain", "Hello Bob,\n\nLunch?\n"))), want);
}

#[test]
fn the_body_says_what_it_is_not_the_headers() {
    let headers =
        [("X-Note", "kept"), ("Content-Type", "text/evil"), ("mime-version", "9"), ("Bad Name", "x"), ("Content-Transfer-Encoding", "base64")];
    let out = text(write_message(&headers, &Entity::text("plain", "x")));
    assert_eq!(out, "X-Note: kept\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nx\r\n");
}

#[test]
fn long_and_non_ascii_subjects_fold() {
    let written = |subject: &str| {
        let mut out = Vec::new();
        assert!(write_field(&mut out, "Subject", &encode_unstructured("Subject", subject)));
        text(out)
    };
    let out = written(&"Ünïcödé subject ".repeat(12));
    assert!(out.starts_with("Subject: =?UTF-8?B?") && out.split("\r\n").all(|l| l.len() <= 78), "{out}");
    assert_eq!(
        crate::decode_words(crate::split_headers(out.as_bytes(), true, &Limits::default()).0.get("subject").unwrap_or("")),
        "Ünïcödé subject ".repeat(12)
    );
    let out = written("plain words ".repeat(30).trim());
    assert!(out.starts_with("Subject: plain words plain words") && out.split("\r\n").all(|l| l.len() <= 78));
    // A subject that could not fold is encoded rather than written as one endless line.
    assert!(written(&"x".repeat(3000)).split("\r\n").all(|l| l.len() <= 78));
    // Readers trim values: text with a space at an end is encoded, to keep it.
    assert!(encode_unstructured("Subject", " x").starts_with("=?UTF-8?B?"));
    assert_eq!(crate::decode_words(&encode_unstructured("Subject", "x ")), "x ");
    assert_eq!(encode_unstructured("Subject", "a\r\nb\0"), "ab");
}

#[test]
fn kept_values_stay_seven_bit() {
    assert_eq!(seven_bit_value("Received", "from a\tby b; =?x?="), "from a\tby b; =?x?=");
    assert!(seven_bit_value("X-Note", "ñ").starts_with("=?UTF-8?B?"));
    assert!(seven_bit_value("X-Note", "\tx").starts_with("=?UTF-8?B?"));
    assert!(seven_bit_value("X-Note", &"x".repeat(3000)).split(' ').all(|w| w.len() <= 75));
    assert_eq!(seven_bit_value("X-Note", "a\r\nb"), "ab");
}

#[test]
fn text_encodings() {
    let part = |t: &str| {
        let p = Entity::text("plain", t);
        (field(&p, "Content-Transfer-Encoding").unwrap_or("").to_string(), text(p.content))
    };
    assert_eq!(part("plain\ntext\r\nmixed\rends"), ("7bit".into(), "plain\r\ntext\r\nmixed\r\nends".into()));
    assert_eq!(part("café"), ("quoted-printable".into(), "caf=C3=A9".into()));
    assert_eq!(part("nul\0"), ("quoted-printable".into(), "nul=00".into()));
    let (encoding, content) = part(&"x".repeat(1200));
    assert_eq!(encoding, "quoted-printable");
    assert!(content.split("\r\n").all(|l| l.len() <= 76));
    assert_eq!(part(&"x".repeat(998)).0, "7bit");
    assert_eq!(part("").0, "7bit");
    assert_eq!(field(&Entity::text("HTML", "x"), "Content-Type"), Some("text/html; charset=utf-8"));
    assert_eq!(field(&Entity::text("html\r\nX-Evil: 1", "x"), "Content-Type"), Some("text/plain; charset=utf-8"));
}

#[test]
fn alternative_related_and_mixed_nest_as_they_should() {
    let mut boundaries = Boundaries::new("=_mc_");
    let logo = Entity::file(&File {
        media_type: "image/png",
        name: "logo.png",
        inline: true,
        content_id: "logo@example.org",
        data: &[0x89, b'P', b'N', b'G'],
    });
    let report = Entity::file(&File { media_type: "application/pdf", name: "report.pdf", data: b"%PDF-1.4\n", ..File::default() });
    let rich = Entity::multipart(
        "related",
        &[("type", "text/html")],
        &[Entity::text("html", "<p>Hello <img src=\"cid:logo@example.org\"></p>"), logo],
        &mut boundaries,
    );
    let forms = Entity::multipart("alternative", &[], &[Entity::text("plain", "Hello Bob,\n\nLunch?\n"), rich], &mut boundaries);
    let root = Entity::multipart("mixed", &[], &[forms, report], &mut boundaries);
    let bytes = write_message(&[("Subject", "Files")], &root);
    let out = text(bytes.clone());
    let types: Vec<&str> = out.split("\r\n").filter(|l| l.starts_with("Content-Type:")).collect();
    assert_eq!(types.len(), 7, "{out}");
    assert!(types[0].starts_with("Content-Type: multipart/mixed; boundary=\"=_mc_"));
    assert!(types[1].starts_with("Content-Type: multipart/alternative; boundary=\"=_mc_"));
    assert_eq!(types[2], "Content-Type: text/plain; charset=utf-8");
    assert!(types[3].starts_with("Content-Type: multipart/related; type=\"text/html\";"));
    assert_eq!(types[4], "Content-Type: text/html; charset=utf-8");
    assert_eq!(types[5], "Content-Type: image/png; name=\"logo.png\"");
    assert_eq!(types[6], "Content-Type: application/pdf; name=\"report.pdf\"");
    assert!(out.contains("Content-Transfer-Encoding: base64\r\nContent-Disposition: inline; filename=\"logo.png\"\r\nContent-ID: <logo@example.org>\r\n\r\niVBORw==\r\n--=_mc_"));
    assert!(out.contains("Content-Disposition: attachment; filename=\"report.pdf\"\r\n\r\nJVBERi0xLjQK\r\n--=_mc_"));
    assert!(out.ends_with("--\r\n"));
    // Three multiparts, three different boundaries.
    let mut found: Vec<&str> = out.split("boundary=\"").skip(1).filter_map(|s| s.split('"').next()).collect();
    found.sort_unstable();
    found.dedup();
    assert_eq!(found.len(), 3);

    // And the reader finds it all again.
    let m = parse(&bytes, &Limits::default());
    let leaves = m.root.leaves();
    let got: Vec<(&str, Vec<u8>)> = leaves.iter().map(|p| (p.media_type(), p.data().to_vec())).collect();
    assert_eq!(
        got,
        [
            ("text/plain", b"Hello Bob,\r\n\r\nLunch?\r\n".to_vec()),
            ("text/html", b"<p>Hello <img src=\"cid:logo@example.org\"></p>".to_vec()),
            ("image/png", vec![0x89, b'P', b'N', b'G']),
            ("application/pdf", b"%PDF-1.4\n".to_vec()),
        ]
    );
    assert_eq!(leaves[2].content_id(), Some("logo@example.org"));
    assert_eq!((leaves[2].disposition().value.as_str(), leaves[3].disposition().value.as_str()), ("inline", "attachment"));
    assert_eq!(crate::Resources::of(&m.root).find("cid:logo@example.org", "").map(|p| p.media_type()), Some("image/png"));
}

#[test]
fn file_names() {
    assert_eq!(file_name_params("report.pdf"), "; filename=\"report.pdf\"");
    assert_eq!(file_name_params("my \"best\" file\\x.txt"), r#"; filename="my \"best\" file\\x.txt""#);
    assert_eq!(file_name_params("résumé €.txt"), "; filename=\"r_sum___.txt\"; filename*=utf-8''r%C3%A9sum%C3%A9%20%E2%82%AC.txt");
    // Something that looks like an encoded-word must not be read as one.
    assert!(file_name_params("=?UTF-8?Q?x?=.txt").contains("filename*=utf-8''%3D%3FUTF-8%3FQ%3Fx%3F%3D.txt"));
    let long = file_name_params(&"ü".repeat(40));
    assert!(long.contains("; filename*0*=utf-8''%C3%BC") && long.contains("; filename*4*=%") && !long.contains("filename*5"), "{long}");
    // Each piece fits a folded line: a space before it, a semicolon after.
    for piece in long.split("; ") {
        assert!(piece.len() + 2 <= 78, "{piece}");
    }
    let file = Entity::file(&File { media_type: "image/png", name: &"ü".repeat(200), ..File::default() });
    assert!(text(file.to_bytes()).split("\r\n").all(|l| l.len() <= 78));
    // Every name reads back, from either place.
    for name in ["report.pdf", "my \"best\" file\\x.txt", "résumé €.txt", "=?UTF-8?Q?x?=.txt", &"ü".repeat(200), "a;b=c.txt", "tab\there"] {
        let bytes = write_message(&[], &Entity::file(&File { media_type: "application/pdf", name, ..File::default() }));
        let m = parse(&bytes, &Limits::default());
        assert_eq!(m.root.file_name().as_deref(), Some(name));
        assert_eq!(m.root.content_type.text("name").as_deref(), Some(name));
    }
}

#[test]
fn unsafe_media_types_and_nameless_files() {
    let part = |media_type: &str, name: &str| Entity::file(&File { media_type, name, ..File::default() });
    assert_eq!(field(&part("multipart/mixed; boundary=x", "a.bin"), "Content-Type"), Some("application/octet-stream; name=\"a.bin\""));
    assert_eq!(field(&part("", ""), "Content-Type"), Some("application/octet-stream"));
    assert_eq!(field(&part("application/pdf", ""), "Content-Disposition"), Some("attachment"));
    assert_eq!(field(&part("image/png", "ünï.png"), "Content-Type"), Some("image/png; name=\"=?UTF-8?B?w7xuw68ucG5n?=\""));
    assert_eq!(field(&part("image/png", ""), "Content-ID"), None);
    assert_eq!(safe_media_type(" Application/PDF "), "application/pdf");
    assert_eq!(safe_media_type("image/svg+xml"), "image/svg+xml");
    for bad in ["", "pdf", "a/b/c", "multipart/mixed", "MULTIPART/related", "text/plain; charset=x", "a b/c", "text/pl\r\nain", "/x", "x/"] {
        assert_eq!(safe_media_type(bad), "application/octet-stream", "{bad}");
    }
}

#[test]
fn attached_messages_travel_as_they_are_when_they_can() {
    let part = |data: &[u8]| {
        let p = Entity::file(&File { media_type: "message/rfc822", name: "m.eml", data, ..File::default() });
        (field(&p, "Content-Transfer-Encoding").unwrap_or("").to_string(), p.content)
    };
    assert_eq!(part(b"Subject: x\r\n\r\nbody\r\n"), ("7bit".into(), b"Subject: x\r\n\r\nbody\r\n".to_vec()));
    assert_eq!(part(b"").0, "7bit");
    assert_eq!(part(&[b'x'; 998]).0, "7bit");
    // Anything that could not travel as it is: base64, which loses nothing.
    assert_eq!(part("Subject: x\r\n\r\ncafé".as_bytes()), ("base64".into(), b"U3ViamVjdDogeA0KDQpjYWbDqQ==".to_vec()));
    assert_eq!(part(b"Subject: x\n\nbare lf").0, "base64");
    assert_eq!(part(b"bare\rcr").0, "base64");
    assert_eq!(part(b"nul\0").0, "base64");
    assert_eq!(part(&[b'x'; 999]).0, "base64");
    // Other files are always base64, however plain they are.
    let p = Entity::file(&File { media_type: "text/plain", name: "a.txt", data: b"plain", ..File::default() });
    assert_eq!(field(&p, "Content-Transfer-Encoding"), Some("base64"));
}

#[test]
fn everything_written_is_seven_bit() {
    let mut boundaries = Boundaries::default();
    let parts = [
        Entity::text("plain", "Grüße\n"),
        Entity::text("html", "<p>Grüße</p>"),
        Entity::file(&File { media_type: "message/rfc822", name: "ü.eml", data: "Subject: é\r\n\r\nü".as_bytes(), ..File::default() }),
        Entity::file(&File { media_type: "application/octet-stream", name: "b.bin", data: &[0, 128, 255], content_id: "ïd", ..File::default() }),
    ];
    let root = Entity::multipart("mixed", &[("type", "tëxt")], &parts, &mut boundaries);
    let subject = encode_unstructured("Subject", "Grüße 🙋");
    let from = crate::format_address_list(&[crate::Mailbox { name: "Ünï Cödé".into(), email: "ada@example.org".into() }]);
    let note = seven_bit_value("X-Note", "ñ");
    let out = write_message(&[("From", &from), ("Subject", &subject), ("X-Note", &note)], &root);
    // What a caller writes into a parameter or an id as it is, is the caller's to encode.
    let out = text(out).replace("tëxt", "").replace("ïd", "");
    assert!(out.is_ascii(), "{out}");
}

#[test]
fn boundaries_are_deterministic_and_avoid_the_content() {
    let write = |body: &str| {
        let mut boundaries = Boundaries::default();
        let root = Entity::multipart("alternative", &[], &[Entity::text("plain", body), Entity::text("html", "<p>x</p>")], &mut boundaries);
        text(write_message(&[], &root))
    };
    assert_eq!(write("x"), write("x"));
    let first = write("x");
    let boundary = first.split("boundary=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or_default().to_string();
    assert!(boundary.starts_with("=_craft_") && boundary.len() <= 70);
    // Put the boundary that text will get in the text: the writer must pick another.
    let mut body = "before\n--x\nafter".to_string();
    for _ in 0..3 {
        let out = write(&body);
        let boundary = out.split("boundary=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or_default().to_string();
        assert_eq!(out.matches(&format!("--{boundary}")).count(), 3, "{out}");
        body = format!("before\n--{boundary}\nafter");
    }
}

#[test]
fn a_boundary_found_in_a_part_is_passed_over() {
    let mut boundaries = Boundaries::new("=_mc_");
    assert_eq!(boundaries.free(42, &[b"nothing here".to_vec()]), "=_mc_000000000000002a_0");
    assert_eq!(boundaries.counter, 1);
    // The parts hold the first two candidates, one of them in the second part.
    let parts = [b"text --=_mc_000000000000002a_1 text".to_vec(), b"=_mc_000000000000002a_2".to_vec()];
    assert_eq!(boundaries.free(42, &parts), "=_mc_000000000000002a_3");
    assert_eq!(boundaries.counter, 4);
    // The counter sets apart two multiparts with the same content.
    let same = [b"same".to_vec()];
    assert_ne!(boundaries.next(&same), boundaries.next(&same));
    // A prefix can't carry what would break the parameter or the delimiter line.
    let hostile = Boundaries::new("a\"\r\n b;c=d\0é--").next(&[]);
    assert!(hostile.starts_with("ab") && hostile.bytes().all(|b| b.is_ascii_alphanumeric() || b"=_-.".contains(&b)), "{hostile}");
    assert!(Boundaries::new(&"x".repeat(500)).next(&[]).len() <= 70);
}

#[test]
fn substring_search() {
    assert!(contains(b"hello world", b"lo w"));
    assert!(contains(b"aaab", b"aab"));
    assert!(contains(b"abc", b""));
    assert!(contains(b"abc", b"abc"));
    assert!(!contains(b"abc", b"abcd"));
    assert!(!contains(b"", b"a"));
    assert!(!contains(b"ababab", b"abb"));
}

#[test]
fn ids_lose_what_could_end_them() {
    assert_eq!(clean_id("id@example.org>\r\nX-Evil: 1\r\n<x"), "id@example.orgX-Evil:1x");
    assert_eq!(clean_id(" a\t@\u{0}b "), "a@b");
    assert_eq!(quoted("a\"b\\c\r\nd"), r#""a\"b\\cd""#);
}
