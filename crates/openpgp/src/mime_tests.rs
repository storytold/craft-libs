use crate::mime::{Params, address, base64, crlf, decode, parts, split};

#[test]
fn headers_unfold_and_parameters_unquote() {
    let (headers, body) =
        split(b"Content-Type: multipart/signed;\r\n\tprotocol=\"application/pgp-signature\";\r\n micalg=pgp-sha256; boundary=\"a;b\"\r\n\r\nbody");
    assert_eq!(body, b"body");
    let ct = headers.content_type();
    assert_eq!(ct.value, "multipart/signed");
    assert_eq!(ct.get("protocol"), Some("application/pgp-signature"));
    assert_eq!(ct.get("boundary"), Some("a;b"));
    assert_eq!(ct.get("micalg"), Some("pgp-sha256"));
}

#[test]
fn a_part_without_fields_or_type_is_plain_text() {
    let (headers, body) = split(b"\r\nonly a body");
    assert_eq!(body, b"only a body");
    assert_eq!(headers.content_type().value, "text/plain");
    let (_, body) = split(b"Subject: no body at all");
    assert!(body.is_empty());
}

#[test]
fn parts_are_cut_exactly_with_the_line_break_before_a_delimiter_belonging_to_it() {
    let body = b"preamble\r\n--b\r\nA: 1\r\n\r\nfirst\r\n\r\n--b  \r\nsecond\r\n--b--\r\nepilogue";
    assert_eq!(parts(body, "b", 10), vec![&b"A: 1\r\n\r\nfirst\r\n"[..], &b"second"[..]]);
    // Bare line feeds, and a delimiter that only starts like the boundary isn't one.
    let body = b"--b\nx\n--bc\ny\n--b--\n";
    assert_eq!(parts(body, "b", 10), vec![&b"x\n--bc\ny"[..]]);
    // A part that is never closed still counts; an empty one too.
    assert_eq!(parts(b"--b\r\n--b\r\nrest", "b", 10), vec![&b""[..], &b"rest"[..]]);
    assert_eq!(parts(b"--b\r\na\r\n--b\r\nb\r\n--b--", "b", 1), vec![&b"a"[..]]);
    assert!(parts(b"no delimiters", "b", 10).is_empty());
    assert!(parts(b"--\r\n", "", 10).is_empty());
}

#[test]
fn canonical_form_adds_carriage_returns_only_where_missing() {
    assert_eq!(crlf(b"a\nb\r\nc\n"), b"a\r\nb\r\nc\r\n");
    assert_eq!(crlf(b""), b"");
}

#[test]
fn transfer_encodings_are_undone() {
    assert_eq!(base64(b"SGVs\r\nbG8=\r\n"), b"Hello");
    assert_eq!(decode("base64", b"!!QUJD"), b"ABC");
    assert_eq!(decode("quoted-printable", b"a=3Db=\r\nc= \r\nd=ZZ="), b"a=bcd=ZZ");
    assert_eq!(decode("7bit", b"same"), b"same");
}

#[test]
fn addresses_are_taken_from_angle_brackets_and_lower_cased() {
    assert_eq!(address("Alice <Alice@Example.org>"), "alice@example.org");
    assert_eq!(address(" bob@example.org "), "bob@example.org");
    assert_eq!(address("\"a <x>\" <c@example.org>"), "c@example.org");
    assert_eq!(address("no address"), "");
    assert_eq!(address("two words@example.org"), "");
    // An address as the display name, as many writers send it.
    assert_eq!(address("bob@example.org <Bob@example.org>"), "bob@example.org");
    assert_eq!(address("\"Bob, the builder\" <bob@example.org>"), "bob@example.org");
    // Two mailboxes: no one address.
    assert_eq!(address("alice@example.org, Mallory <mallory@example.org>"), "");
    assert_eq!(address("Alice <alice@example.org> <mallory@example.org>"), "");
    assert_eq!(address("alice@example.org mallory@example.org"), "");
    // What is in a comment or a quoted string is never the address, wherever it is.
    assert_eq!(address("Mallory <mallory@evil.example> (<alice@example.org>)"), "mallory@evil.example");
    assert_eq!(address("Mallory <mallory@evil.example> \"<alice@example.org>\""), "mallory@evil.example");
    assert_eq!(address("mallory@evil.example (alice@example.org)"), "mallory@evil.example");
    assert_eq!(address("Alice <alice@example.org"), "");
    // Characters that don't show (or show as nothing) make no address.
    for hidden in ["\u{200b}", "\0", "\u{202e}", "\u{7f}", "\u{feff}"] {
        assert_eq!(address(&format!("Mallory <alice@example.org{hidden}>")), "", "{hidden:?}");
        assert_eq!(address(&format!("ali{hidden}ce@example.org")), "", "{hidden:?}");
    }
}

#[test]
fn an_autocrypt_header_is_matched_to_the_address_readers_show() {
    use crate::fixtures::{Make, base64, secret};
    let alice = secret("Alice <alice@example.org>", Make::default());
    let keydata = base64(&pgp::ser::Serialize::to_bytes(&pgp::composed::SignedPublicKey::from(alice)).unwrap()).replace("\r\n", "");
    for from in ["Mallory <mallory@evil.example> (<alice@example.org>)", "Mallory <mallory@evil.example> \"<alice@example.org>\""] {
        let message = format!("From: {from}\r\nAutocrypt: addr=alice@example.org; keydata={keydata}\r\n\r\nHi\r\n");
        assert!(crate::keys_in_message(message.as_bytes()).autocrypt.is_none(), "{from}");
    }
}

#[test]
fn params_with_no_value_and_odd_quoting_dont_break() {
    let p = Params::parse("text/plain; ; =x; a=\"unterminated; b=\\\"");
    assert_eq!(p.value, "text/plain");
    assert_eq!(p.get("a"), Some("unterminated; b=\""));
}

#[test]
fn the_format_characters_are_unicode_cf() {
    // General category Cf in Unicode 17.0 has 170 characters.
    assert_eq!((0..=0x10ffffu32).filter_map(char::from_u32).filter(|c| crate::mime::is_format(*c)).count(), 170);
    assert!(address("ada\u{200b}@example.org").is_empty());
}
