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
