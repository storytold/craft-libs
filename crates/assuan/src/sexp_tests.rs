use proptest::prelude::*;

use crate::sexp::{MAX_DEPTH, MAX_INPUT, MAX_NODES, Sexp, SexpError, parse};

fn atom(b: &[u8]) -> Sexp {
    Sexp::Atom(b.to_vec())
}

#[test]
fn a_signature_is_read() {
    let s = parse(b"(7:sig-val(5:ecdsa(1:r3:\x01\x02\x03)(1:s2:\x04\x05)))").unwrap();
    assert_eq!(s.name(), Some("sig-val"));
    let ecdsa = s.find("ecdsa").unwrap();
    assert_eq!(ecdsa.value("r"), Some(&b"\x01\x02\x03"[..]));
    assert_eq!(ecdsa.value("s"), Some(&b"\x04\x05"[..]));
    assert_eq!(ecdsa.value("t"), None);
}

#[test]
fn the_canonical_form_comes_back_the_same() {
    let text = b"(10:public-key(3:rsa(1:n3:abc)(1:e1:\x01)))";
    let s = parse(text).unwrap();
    assert_eq!(s.to_canonical(), text.to_vec());
    assert_eq!(Sexp::List(vec![Sexp::token("value"), atom(b"")]).to_canonical(), b"(5:value0:)".to_vec());
}

#[test]
fn a_display_hint_is_read_past() {
    let s = parse(b"(3:foo[10:text/plain]5:hello)").unwrap();
    assert_eq!(s, Sexp::List(vec![atom(b"foo"), atom(b"hello")]));
}

#[test]
fn malformed_input_is_refused() {
    let cases: &[(&[u8], SexpError)] = &[
        (b"", SexpError::Truncated),
        (b"(", SexpError::Truncated),
        (b"(3:ab", SexpError::Truncated),
        (b"(3:abc", SexpError::Truncated),
        (b"5:abc", SexpError::Truncated),
        (b"(3:abc))", SexpError::Trailing),
        (b"3:abc3:def", SexpError::Trailing),
        (b"(abc)", SexpError::Malformed(1)),
        (b":", SexpError::Malformed(0)),
        (b"03:abc", SexpError::Malformed(1)),
        (b"9999999999:a", SexpError::TooBig),
        (b"999999999:a", SexpError::Truncated),
        (b"([1:a]", SexpError::Truncated),
        (b"([1:a]1:b", SexpError::Truncated),
        (b"[1:a][1:b]1:c", SexpError::Malformed(5)),
        (b"[1:a1:b", SexpError::Malformed(4)),
    ];
    for (input, want) in cases {
        assert_eq!(parse(input).as_ref().err(), Some(want), "{:?}", String::from_utf8_lossy(input));
    }
}

#[test]
fn depth_nodes_and_size_are_capped() {
    let deep = |n: usize| format!("{}{}", "(".repeat(n), ")".repeat(n));
    assert!(parse(deep(MAX_DEPTH).as_bytes()).is_ok());
    assert_eq!(parse(deep(MAX_DEPTH + 1).as_bytes()), Err(SexpError::TooDeep));
    assert_eq!(parse("(".repeat(100_000).as_bytes()), Err(SexpError::TooDeep));

    let many = |n: usize| format!("({})", "0:".repeat(n));
    assert!(parse(many(MAX_NODES - 1).as_bytes()).is_ok());
    assert_eq!(parse(many(MAX_NODES).as_bytes()), Err(SexpError::TooBig));

    let big = vec![b'('; MAX_INPUT + 1];
    assert_eq!(parse(&big), Err(SexpError::TooBig));
}

#[test]
fn debug_shows_tokens_but_not_secrets() {
    let s = parse(b"(5:value4:\x00key)").unwrap();
    assert_eq!(format!("{s:?}"), "[value, <4 bytes>]");
}

fn arb_sexp() -> impl Strategy<Value = Sexp> {
    let leaf = proptest::collection::vec(any::<u8>(), 0..40).prop_map(Sexp::Atom);
    leaf.prop_recursive(6, 64, 6, |inner| proptest::collection::vec(inner, 0..6).prop_map(Sexp::List))
}

proptest! {
    #[test]
    fn any_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        if let Ok(s) = parse(&bytes) {
            // What is read is canonical once more, apart from display hints, which are dropped.
            let again = s.to_canonical();
            prop_assert_eq!(parse(&again).unwrap(), s);
        }
    }

    #[test]
    fn written_s_expressions_read_back(s in arb_sexp()) {
        prop_assert_eq!(parse(&s.to_canonical()).unwrap(), s);
    }

    #[test]
    fn bracket_soup_never_panics(bytes in proptest::collection::vec(prop_oneof![Just(b'('), Just(b')'), Just(b'['), Just(b']'), Just(b':'), Just(b'1'), Just(b'0'), Just(b'a')], 0..512)) {
        let _ = parse(&bytes);
    }
}
