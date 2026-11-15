use super::*;

// RFC 8620 §1.2: 1 to 255 octets of A-Z a-z 0-9 - _.
#[test]
fn accepts_valid_ids() {
    for text in ["a", "A13824", "msg-1023_x", "0", "-", "_", &"x".repeat(255)] {
        assert_eq!(Id::new(text).unwrap().as_str(), text);
    }
}

// RFC 8620 §1.2: padding, other base64 characters, separators and controls are refused.
#[test]
fn refuses_invalid_ids() {
    assert_eq!(Id::new(""), Err(IdError::Empty));
    assert_eq!(Id::new(&"x".repeat(256)), Err(IdError::TooLong));
    for text in ["abc=", "a+b", "a/b", "a b", "a.b", "a\u{0}", "é", "a#b", "../x", "\"", "a\nb"] {
        assert_eq!(Id::new(text), Err(IdError::InvalidCharacter), "{text:?}");
    }
}

// RFC 8620 §1.2: ids compare as exact strings (case matters).
#[test]
fn case_sensitive() {
    assert_ne!(Id::new("A").unwrap(), Id::new("a").unwrap());
    assert_eq!("M1".parse::<Id>().unwrap(), Id::try_from("M1").unwrap());
}
