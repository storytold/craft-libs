use crate::autocrypt::MAX_HEADER;
use crate::{Error, parse_autocrypt};

#[test]
fn a_header_without_addr_or_keydata_is_unreadable() {
    assert_eq!(parse_autocrypt("keydata=AAAA").unwrap_err(), Error::Unreadable);
    assert_eq!(parse_autocrypt("addr=a@example.org").unwrap_err(), Error::Unreadable);
    assert_eq!(parse_autocrypt("").unwrap_err(), Error::Unreadable);
}

#[test]
fn an_unknown_critical_attribute_makes_the_header_unreadable() {
    assert_eq!(parse_autocrypt("addr=a@example.org; colour=blue; keydata=AAAA").unwrap_err(), Error::Unreadable);
    assert_eq!(parse_autocrypt("addr=a@example.org; novalue; keydata=AAAA").unwrap_err(), Error::Unreadable);
    // One starting with `_` is skipped; the keydata here is no key, so it still doesn't read.
    assert_eq!(parse_autocrypt("addr=a@example.org; _colour=blue; keydata=AAAA").unwrap_err(), Error::Unreadable);
}

#[test]
fn a_header_too_long_is_refused_before_it_is_read() {
    let long = format!("addr=a@example.org; keydata={}", "A".repeat(MAX_HEADER));
    assert_eq!(parse_autocrypt(&long).unwrap_err(), Error::TooLarge);
}
