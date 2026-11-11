//! Base64: the RFC 4648 test vectors, the strict refusals, and that only the one spelling of a
//! value decodes, on every config.

use super::*;
use proptest::prelude::*;

#[test]
fn the_rfc_vectors_round_trip_in_every_config() {
    let vectors = [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")];
    for (plain, coded) in vectors {
        assert_eq!(STANDARD.encode(plain), coded);
        assert_eq!(URL_SAFE.encode(plain), coded);
        assert_eq!(STANDARD_NO_PAD.encode(plain), coded.trim_end_matches('='));
        assert_eq!(URL_SAFE_NO_PAD.encode(plain), coded.trim_end_matches('='));
        assert_eq!(STANDARD.decode(coded).unwrap(), plain.as_bytes());
        assert_eq!(URL_SAFE_NO_PAD.decode(coded.trim_end_matches('=')).unwrap(), plain.as_bytes());
    }
}

#[test]
fn the_two_alphabets_differ_in_the_last_two_symbols() {
    assert_eq!(STANDARD.encode([0xfb, 0xff]), "+/8=");
    assert_eq!(URL_SAFE.encode([0xfb, 0xff]), "-_8=");
    assert!(URL_SAFE.decode("+/8=").is_err());
    assert!(STANDARD.decode("-_8=").is_err());
}

#[test]
fn what_is_not_exactly_base64_is_refused() {
    for bad in ["Zg", "Zg=", "Zg===", "Z", "Z===", "====", "Zm9v\n", " Zm9v", "Zm 9v", "Zh==", "Zm9=", "Zm=v", "Zm9v=", "é", "Zm9\u{0}"] {
        assert!(STANDARD.decode(bad).is_err(), "{bad:?}");
    }
    for bad in ["Zg==", "Z", "Zh", "Zm9v\n", "Zm 9v", "Zg=", "Zm9="] {
        assert!(STANDARD_NO_PAD.decode(bad).is_err(), "{bad:?}");
    }
    assert!(STANDARD_NO_PAD.decode("Zm8").is_ok());
}

#[test]
fn hostile_inputs_do_not_panic() {
    let big = vec![b'A'; 1 << 20];
    for cfg in [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD] {
        assert_eq!(cfg.decode(&big).map(|b| b.len()).unwrap_or(0), 786_432);
        for bytes in [&b""[..], b"=", b"==", b"===", b"\xff\xfe", b"=A=A", b"AA=A"] {
            let _ = cfg.decode(bytes);
        }
    }
}

const CONFIGS: [Config; 4] = [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD];

proptest! {
    #[test]
    fn what_is_encoded_decodes_to_itself(data in proptest::collection::vec(any::<u8>(), 0..400)) {
        for config in CONFIGS {
            prop_assert_eq!(config.decode(config.encode(&data)).ok(), Some(data.clone()));
        }
    }

    /// Strict: a text is accepted only when it is the one spelling of its bytes.
    #[test]
    fn what_decodes_is_the_one_spelling_of_its_bytes(
        data in proptest::collection::vec(any::<u8>(), 0..400),
        junk in proptest::collection::vec(any::<u8>(), 0..64),
        text in "[A-Za-z0-9+/=_ -]{0,40}",
    ) {
        for config in CONFIGS {
            let mut cut = config.encode(&data).into_bytes();
            cut.pop();
            let mut pad = cut.clone();
            if let Some(first) = pad.first_mut() {
                *first = b'=';
            }
            for candidate in [junk.as_slice(), text.as_bytes(), cut.as_slice(), pad.as_slice()] {
                if let Ok(bytes) = config.decode(candidate) {
                    prop_assert_eq!(config.encode(&bytes).into_bytes(), candidate.to_vec());
                }
            }
        }
    }
}
