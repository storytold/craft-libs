use super::*;

// RFC 3986 §2.3: unreserved characters are never encoded; everything else is.
#[test]
fn encodes_everything_but_unreserved() {
    assert_eq!(encode_component("AZaz09-._~"), "AZaz09-._~");
    assert_eq!(encode_component("a b/c?d&e=f+g%h#i"), "a%20b%2Fc%3Fd%26e%3Df%2Bg%25h%23i");
    assert_eq!(encode_component("application/pdf"), "application%2Fpdf");
}

// RFC 3986 §2.1 and §2.5: non-ASCII text is encoded as the octets of its UTF-8 form.
#[test]
fn encodes_utf8_octets() {
    assert_eq!(encode_component("é"), "%C3%A9");
    assert_eq!(encode_component("Ünïcode €"), "%C3%9Cn%C3%AFcode%20%E2%82%AC");
}

// RFC 3986 §2.1: decoding is the inverse; hex digits are case-insensitive.
#[test]
fn decodes_round_trip() {
    for text in ["", "plain", "a b/c?d", "é€😀", "%", "100%"] {
        assert_eq!(decode(&encode_component(text)).unwrap(), text);
    }
    assert_eq!(decode("%c3%a9").unwrap(), "é");
    assert_eq!(decode("a+b").unwrap(), "a+b");
}

// RFC 3986 §2.1: a percent sign must start a full triplet.
#[test]
fn refuses_malformed_escapes() {
    assert_eq!(decode("%"), Err(DecodeError::MalformedEscape));
    assert_eq!(decode("%4"), Err(DecodeError::MalformedEscape));
    assert_eq!(decode("%zz"), Err(DecodeError::MalformedEscape));
    assert_eq!(decode("%%41"), Err(DecodeError::MalformedEscape));
}

// Requirement 1.5: decoding never produces invalid UTF-8 silently.
#[test]
fn refuses_invalid_utf8() {
    assert_eq!(decode("%FF"), Err(DecodeError::InvalidUtf8));
    assert_eq!(decode("%C3"), Err(DecodeError::InvalidUtf8));
    assert_eq!(decode_bytes("%FF").unwrap(), vec![0xFF]);
}

// RFC 3986 §6.2.2.1 and §6.2.2.2: upper-case hex, unreserved characters decoded.
#[test]
fn normalises_escapes() {
    assert_eq!(normalise_escapes("%7esmith").unwrap(), "~smith");
    assert_eq!(normalise_escapes("%2f%2E%41").unwrap(), "%2F.A");
    assert_eq!(normalise_escapes("%2"), None);
}
