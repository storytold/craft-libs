//! Hostile input and hostile values: every prefix and many mutations of real messages read
//! without a panic and within their bounds, and nothing a caller hands the writer can add a
//! header field or a part.

use craft_mime::{Boundaries, Entity, File, Limits, Mailbox, Message, Part};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

/// A message with every construct the reader knows, written by the writer.
fn sample() -> Vec<u8> {
    let mut boundaries = Boundaries::default();
    let picture = Entity::file(&File { media_type: "image/png", name: "lögo (1).png", inline: true, content_id: "logo@example.org", data: PNG });
    let mut page = Entity::text("html", "<p>Grüße <img src=\"cid:logo@example.org\"></p>\n");
    page.fields.push(("Content-Location".into(), "https://example.test/reports/page.html".into()));
    let rich = Entity::multipart("related", &[("type", "text/html"), ("start", "<page>")], &[page, picture], &mut boundaries);
    let forms = Entity::multipart("alternative", &[], &[Entity::text("plain", "Grüße\n\nFrom here on.\n"), rich], &mut boundaries);
    let inner = craft_mime::write_message(&[("Subject", "Inner")], &Entity::text("plain", "inner"));
    let attached = Entity::file(&File { media_type: "message/rfc822", name: "inner.eml", data: &inner, ..File::default() });
    let root = Entity::multipart("mixed", &[], &[forms, attached], &mut boundaries);
    let to = craft_mime::format_address_list(&[
        Mailbox { name: "Hopper, Grace".into(), email: "grace@example.org".into() },
        Mailbox { name: "Renée".into(), email: "renee@example.org".into() },
    ]);
    let subject = craft_mime::encode_unstructured("Subject", &"Ünïcödé subject ".repeat(8));
    craft_mime::write_message(
        &[("Date", "Tue, 01 Jul 2003 10:52:37 +0200"), ("To", &to), ("Subject", &subject), ("Message-ID", "<abc@example.org>")],
        &root,
    )
}

/// A hand-written MHT archive with LF line ends, a comment in its content type and RFC 2231
/// parameters.
fn archive() -> Vec<u8> {
    b"From nobody Tue Jul  1 08:52:37 2003\n\
MIME-Version: 1.0\n\
Content-Type: multipart/related (a \"comment\"; boundary=wrong);\n\
\ttype=\"text/html\"; start=\"<page>\"; boundary=\"----=_B\"\n\
\n\
preamble\n\
------=_B\n\
Content-ID: <page>\n\
Content-Location: file:///C:/page.htm\n\
Content-Transfer-Encoding: quoted-printable\n\
Content-Type: text/html; charset=\"windows-1250\"\n\
\n\
<p class=3D\"x\">=E8lanak, jedan =\n\
red.</p><img src=3D\"page_files/dot.png\">\n\
------=_B\n\
Content-Location: file:///C:/page_files/dot.png\n\
Content-Transfer-Encoding: base64\n\
Content-Type: image/png\n\
Content-Disposition: inline; filename*0*=utf-8''d%C3%B6;\n\
\x20filename*1=\"t.png\"\n\
\n\
iVBORw0KGgoAAAANSUhEUg==\n\
------=_B--\n\
epilogue\n"
        .to_vec()
}

/// Everything a caller can ask of a message, and what must hold whatever the bytes were.
fn exercise(bytes: &[u8], limits: &Limits) -> Message {
    let message = craft_mime::parse(bytes, limits);
    let mut stack = vec![(&message.root, 0usize)];
    let (mut parts, mut decoded) = (0usize, 0usize);
    while let Some((part, depth)) = stack.pop() {
        parts += 1;
        decoded += part.data().len();
        assert!(depth <= limits.max_depth.min(Limits::HARD_MAX_DEPTH));
        assert!(part.headers.0.len() <= limits.max_header_fields);
        assert!(
            part.headers.0.iter().all(|f| craft_mime::is_field_name(f.name.as_bytes()) && f.value.len() <= limits.max_header_value.saturating_mul(3))
        );
        let _ = (part.text(), part.file_name(), part.content_id(), part.content_location(), part.disposition(), part.transfer_encoding());
        for field in &part.headers.0 {
            let _ = (craft_mime::decode_words(&field.value), craft_mime::parse_address_list(&field.value), craft_mime::parse_date(&field.value));
        }
        stack.extend(part.parts().iter().map(|p| (p, depth + 1)));
    }
    assert!(parts <= limits.max_parts.max(1));
    assert!(decoded <= limits.max_decoded.min(bytes.len()));
    let resources = craft_mime::Resources::of(&message.root);
    let _ = (
        craft_mime::start_part(&message.root),
        resources.find("page_files/dot.png", "file:///C:/page.htm"),
        resources.find("cid:logo@example.org", ""),
    );
    assert_eq!(craft_mime::parse(bytes, limits), message, "reading is deterministic");
    message
}

const SMALL: Limits = Limits {
    max_input: 700,
    max_header_fields: 3,
    max_header_value: 40,
    max_header_block: 120,
    max_depth: 1,
    max_parts: 3,
    max_decoded: 50,
    ..Limits::DEFAULT
};

#[test]
fn the_samples_read_whole() {
    let m = exercise(&sample(), &Limits::default());
    assert!(!m.truncated);
    let types: Vec<&str> = m.root.leaves().iter().map(|p| p.media_type()).collect();
    assert_eq!(types, ["text/plain", "text/html", "image/png", "message/rfc822"]);
    assert_eq!(m.root.leaves()[2].file_name().as_deref(), Some("lögo (1).png"));
    assert_eq!(m.root.leaves()[2].data(), PNG);

    let m = exercise(&archive(), &Limits::default());
    assert!(!m.truncated);
    assert_eq!(
        craft_mime::start_part(&m.root).map(Part::text).as_deref(),
        Some("<p class=\"x\">članak, jedan red.</p><img src=\"page_files/dot.png\">")
    );
    let picture = craft_mime::Resources::of(&m.root).find("page_files/dot.png", "file:///C:/page.htm");
    assert_eq!(picture.and_then(Part::file_name).as_deref(), Some("döt.png"));
    assert!(picture.is_some_and(|p| p.data().starts_with(b"\x89PNG")));
}

#[test]
fn every_truncation_reads() {
    for bytes in [sample(), archive()] {
        for end in 0..=bytes.len() {
            let cut = bytes.get(..end).unwrap_or_default();
            exercise(cut, &Limits::default());
            exercise(cut, &SMALL);
            // An mbox reader takes any prefix too.
            assert!(craft_mime::mbox_messages(cut).map(|m| m.len()).sum::<usize>() <= cut.len());
        }
    }
}

#[test]
fn mutations_read() {
    // A small xorshift generator: the same mutations every run.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    const INTERESTING: &[u8] = b"\0\r\n\t \"\\()<>;:=?-_%*'/@,.\x7f\x80\xff";
    for original in [sample(), archive()] {
        for round in 0..4000u32 {
            let mut bytes = original.clone();
            for _ in 0..=(round % 4) {
                let at = (next() as usize) % bytes.len().max(1);
                let pick = INTERESTING.get((next() as usize) % INTERESTING.len()).copied().unwrap_or(0);
                match next() % 5 {
                    0 => {
                        if let Some(b) = bytes.get_mut(at) {
                            *b = pick;
                        }
                    }
                    1 => bytes.insert(at.min(bytes.len()), pick),
                    2 if !bytes.is_empty() => {
                        bytes.remove(at);
                    }
                    // Repeat a slice of the message somewhere else (boundaries, header lines).
                    3 => {
                        let len = (next() as usize) % 80;
                        let slice = original.get(at..at.saturating_add(len).min(original.len())).unwrap_or_default().to_vec();
                        let to = (next() as usize) % bytes.len().max(1);
                        bytes.splice(to..to, slice);
                    }
                    _ => {
                        if let Some(b) = bytes.get_mut(at) {
                            *b ^= 1 << (next() % 8);
                        }
                    }
                }
            }
            exercise(&bytes, &Limits::default());
            exercise(&bytes, &SMALL);
        }
    }
}

#[test]
fn large_hostile_shapes_stay_linear() {
    // Each of these is a megabyte or so; quadratic work on any would not finish.
    let n = 200_000;
    let shapes: Vec<Vec<u8>> = vec![
        b"X: ".iter().copied().chain(b"=?".repeat(n)).collect(),
        format!("Content-Type: multipart/mixed; boundary=b\n\n{}", "--b\n".repeat(n)).into_bytes(),
        format!("Content-Type: multipart/mixed; boundary=b\n\n{}", "--b--x\n--bb\n".repeat(n / 2)).into_bytes(),
        format!("Content-Type: text/plain{}\n\nx", "; a*0*=''%41".repeat(n / 8)).into_bytes(),
        format!("Content-Type: text/plain; a=\"b\"{}\n\nx", "(".repeat(n)).into_bytes(),
        format!("To: {}\n\nx", "(<\"\\".repeat(n / 2)).into_bytes(),
        format!("A: 1\n{}\nbody", " fold\n".repeat(n)).into_bytes(),
        "A: 1\n".repeat(n).into_bytes(),
        format!("Content-Transfer-Encoding: quoted-printable\n\n{}", "= ".repeat(n)).into_bytes(),
        format!("Content-Transfer-Encoding: base64\n\n{}", "=A".repeat(n)).into_bytes(),
        "From \n\n".repeat(n).into_bytes(),
    ];
    for bytes in &shapes {
        exercise(bytes, &Limits::default());
        let _ = craft_mime::mbox_messages(bytes).count();
    }
}

/// The header block of a written message, as lines; no line of it may start a field the caller
/// didn't write.
fn header_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    assert!(!text.contains('\0'), "{text:?}");
    // Every CR is followed by LF and every LF follows a CR: no bare line break got through.
    assert_eq!(text.matches('\r').count(), text.matches("\r\n").count());
    assert_eq!(text.matches('\n').count(), text.matches("\r\n").count());
    text.split("\r\n\r\n").next().unwrap_or_default().split("\r\n").map(str::to_string).collect()
}

const EVIL: &[&str] = &[
    "x\r\nBcc: eve@example.org",
    "x\nX-Evil: 1",
    "x\rX-Evil: 2",
    "x\r\n\r\n<script>body</script>",
    "x\0\r\n X-Evil: 3",
    "x\"\r\nX-Evil: 4",
    "x>\r\nX-Evil: 5\r\n<y",
    "é\r\nX-Evil: 6",
    "\r\nX-Evil: 7",
    "x\u{2028}X-Evil: 8\u{85}X-Evil: 9",
];

fn assert_no_injection(bytes: &[u8], allowed: &[&str]) {
    for line in header_lines(bytes) {
        let name = line.split(':').next().unwrap_or_default();
        assert!(line.starts_with([' ', '\t']) || allowed.iter().any(|a| a.eq_ignore_ascii_case(name)), "an injected line: {line:?}");
    }
    // And a reader sees only those fields.
    let m = craft_mime::parse(bytes, &Limits::default());
    for field in &m.root.headers.0 {
        assert!(allowed.iter().any(|a| a.eq_ignore_ascii_case(&field.name)), "an injected field: {field:?}");
    }
}

#[test]
fn header_values_cannot_inject_fields() {
    const ALLOWED: &[&str] = &["Subject", "X-Note", "To", "Message-ID", "MIME-Version", "Content-Type", "Content-Transfer-Encoding"];
    for evil in EVIL {
        // Raw, and through each encoder a caller would use.
        let to = craft_mime::format_address_list(&[Mailbox { name: evil.to_string(), email: format!("a@example.org{evil}") }]);
        let id = format!("<{}>", craft_mime::clean_id(evil));
        for subject in [evil.to_string(), craft_mime::encode_unstructured("Subject", evil), craft_mime::encode_words(evil)] {
            let note = craft_mime::seven_bit_value("X-Note", evil);
            let headers = [("Subject", subject.as_str()), ("X-Note", note.as_str()), ("To", to.as_str()), ("Message-ID", id.as_str())];
            let out = craft_mime::write_message(&headers, &Entity::text("plain", "body"));
            assert_no_injection(&out, ALLOWED);
            let m = craft_mime::parse(&out, &Limits::default());
            assert_eq!(m.root.data(), b"body\r\n", "the body starts where the writer put it");
            assert_eq!(craft_mime::parse_address_list(m.root.headers.get("to").unwrap_or_default()).len(), 1, "{to}");
        }
    }
}

#[test]
fn header_names_cannot_inject_fields() {
    for name in ["X-A\r\nBcc", "X-A: 1\r\nBcc", "X A", "", "X-A\0", "Bcc:", "X-É"] {
        let out = craft_mime::write_message(&[(name, "eve@example.org")], &Entity::text("plain", "body"));
        assert_no_injection(&out, &["MIME-Version", "Content-Type", "Content-Transfer-Encoding"]);
        let mut entity = Entity::text("plain", "body");
        entity.fields.push((name.to_string(), "eve@example.org".to_string()));
        assert_no_injection(&craft_mime::write_message(&[], &entity), &["MIME-Version", "Content-Type", "Content-Transfer-Encoding"]);
    }
}

#[test]
fn part_values_cannot_inject_fields_or_parts() {
    const ALLOWED: &[&str] = &["MIME-Version", "Content-Type", "Content-Transfer-Encoding", "Content-Disposition", "Content-ID", "Content-Location"];
    for evil in EVIL {
        let mut file = Entity::file(&File { media_type: evil, name: evil, inline: false, content_id: evil, data: b"data" });
        file.fields.push(("Content-Location".into(), evil.to_string()));
        assert_no_injection(&craft_mime::write_message(&[], &file), ALLOWED);
        assert_no_injection(&craft_mime::write_message(&[], &Entity::text(evil, "x")), ALLOWED);

        // In a multipart: the subtype, a parameter's name and value, the boundary prefix, and
        // a body that holds what looks like a delimiter.
        let mut boundaries = Boundaries::new(evil);
        let text = Entity::text("plain", &format!("--{evil}\n--=_craft_\n{evil}"));
        let root = Entity::multipart(evil, &[("type", evil), (evil, "x"), ("boundary", "mine")], &[text, file], &mut boundaries);
        let out = craft_mime::write_message(&[], &root);
        assert_no_injection(&out, ALLOWED);
        let m = craft_mime::parse(&out, &Limits::default());
        assert_eq!(m.root.parts().len(), 2, "{}", String::from_utf8_lossy(&out));
        assert_eq!(m.root.parts()[1].data(), b"data");
        for part in m.root.parts() {
            assert!(part.headers.0.iter().all(|f| ALLOWED.iter().any(|a| a.eq_ignore_ascii_case(&f.name))), "{:?}", part.headers);
        }
    }
}

#[test]
fn mbox_values_cannot_inject_messages() {
    let date = craft_mime::parse_date("Tue, 1 Jul 2003 08:52:37 +0000").unwrap_or_default();
    for evil in EVIL {
        let mut out = Vec::new();
        let sender = format!("a@example.org{evil}\nFrom eve@example.org Tue Jul  1 08:52:37 2003");
        let message = format!("Subject: x\r\n\r\nFrom the top\r\n\r\nFrom eve@example.org Tue Jul  1 08:52:37 2003\r\n{evil}\r\n");
        craft_mime::write_mbox_message(&mut out, &sender, &date, message.as_bytes());
        let back: Vec<Vec<u8>> = craft_mime::mbox_messages(&out).collect();
        assert_eq!(back, [message.into_bytes()]);
    }
}
