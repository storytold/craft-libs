//! Hostile input: arbitrary bytes into every reader never panic, and stay within bounds.

use proptest::prelude::*;

use crate::armour::{Kind, blocks, cleartext};
use crate::fixtures::{ALICE_FOR_PROPS, LATER};
use crate::{KeyRing, PublicKey, keys_in_message, mime, parse_autocrypt, verify};

/// Bytes that look like the pieces of a signed message, shuffled and cut, so the readers go
/// deeper than random bytes would take them.
fn mail_like() -> impl Strategy<Value = Vec<u8>> {
    let pieces = prop::sample::select(vec![
        &b"Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=b\r\n"[..],
        b"Content-Type: multipart/mixed; boundary=b\r\n",
        b"Content-Type: application/pgp-keys\r\n",
        b"Content-Transfer-Encoding: base64\r\n",
        b"Content-Transfer-Encoding: quoted-printable\r\n",
        b"Autocrypt: addr=a@example.org; keydata=xjMEZ\r\n",
        b"From: a@example.org\r\n",
        b"\r\n",
        b"\n",
        b"--b\r\n",
        b"--b--\r\n",
        b"-----BEGIN PGP SIGNED MESSAGE-----\r\n",
        b"-----BEGIN PGP SIGNATURE-----\r\n",
        b"-----END PGP SIGNATURE-----\r\n",
        b"-----BEGIN PGP PUBLIC KEY BLOCK-----\r\n",
        b"-----END PGP PUBLIC KEY BLOCK-----\r\n",
        b"- - dashed\r\n",
        b"wnUEARYKAB0WIQ\r\n",
        b"xjMEZ0AAABYJKwYBBAHaRw8BAQdA\r\n",
        b"=AAAA\r\n",
        b"\xc2\x00\xff\x80\x01",
    ]);
    (prop::collection::vec(pieces, 0..40), prop::collection::vec(any::<u8>(), 0..64)).prop_map(|(pieces, noise)| {
        let mut out: Vec<u8> = pieces.concat();
        out.extend_from_slice(&noise);
        out
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn verify_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let keys: KeyRing = ALICE_FOR_PROPS.iter().cloned().collect();
        let _ = verify(&bytes, "a@example.org", &keys, LATER, |_, _| true);
    }

    #[test]
    fn verify_never_panics_on_mail_like_bytes(bytes in mail_like()) {
        let keys: KeyRing = ALICE_FOR_PROPS.iter().cloned().collect();
        let _ = verify(&bytes, "a@example.org", &keys, LATER, |_, _| true);
        let _ = keys_in_message(&bytes);
    }

    #[test]
    fn packets_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = PublicKey::parse(&bytes);
        let keys: KeyRing = ALICE_FOR_PROPS.iter().cloned().collect();
        let _ = crate::check::check(&bytes, b"data", &crate::check::Context { from: "a@example.org".into(), keys: &keys, now: LATER, verified: &|_, _| true, verifications: Default::default(), data_passes: Default::default() });
    }

    #[test]
    fn armour_stays_within_its_limits(bytes in mail_like(), limit in 0usize..64, max in 0usize..4) {
        let found = blocks(&bytes, Kind::Signature, max, limit);
        prop_assert!(found.len() <= max);
        prop_assert!(found.iter().all(|b| b.len() <= limit));
        if let Some(clear) = cleartext(&bytes, limit) {
            prop_assert!(clear.signature.len() <= limit);
            prop_assert!(clear.signed.len() <= bytes.len().saturating_mul(2));
        }
    }

    #[test]
    fn autocrypt_headers_never_panic(value in "\\PC{0,300}") {
        let _ = parse_autocrypt(&value);
    }

    #[test]
    fn mime_pieces_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..1024), boundary in "[a-z=-]{0,4}") {
        let (headers, body) = mime::split(&bytes);
        let _ = headers.content_type();
        let found = mime::parts(body, &boundary, 8);
        prop_assert!(found.len() <= 8);
        let _ = mime::decode("quoted-printable", &bytes);
        let _ = mime::decode("base64", &bytes);
        prop_assert!(mime::crlf(&bytes).len() <= bytes.len() * 2);
    }
}
