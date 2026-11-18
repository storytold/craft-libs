use crate::armour::{Kind, blocks, cleartext};

#[test]
fn a_block_is_decoded_without_its_headers_or_checksum() {
    let text =
        b"before\r\n-----BEGIN PGP SIGNATURE-----\r\nComment: made by hand\r\n\r\nSGVs\r\nbG8=\r\n=AAAA\r\n-----END PGP SIGNATURE-----\r\nafter";
    assert_eq!(blocks(text, Kind::Signature, 4, 100), vec![b"Hello".to_vec()]);
    // Armour with no headers and no empty line after the first line.
    let text = b"-----BEGIN PGP SIGNATURE-----\nSGVsbG8=\n-----END PGP SIGNATURE-----\n";
    assert_eq!(blocks(text, Kind::Signature, 4, 100), vec![b"Hello".to_vec()]);
}

#[test]
fn blocks_are_capped_in_size_and_number() {
    let block = "-----BEGIN PGP PUBLIC KEY BLOCK-----\n\nSGVsbG8=\n-----END PGP PUBLIC KEY BLOCK-----\n";
    let many = block.repeat(10);
    assert_eq!(blocks(many.as_bytes(), Kind::PublicKey, 3, 100).len(), 3);
    assert!(blocks(block.as_bytes(), Kind::PublicKey, 3, 4).is_empty());
    // Another kind of block, an unclosed one, and an empty one are not blocks.
    assert!(blocks(block.as_bytes(), Kind::Signature, 3, 100).is_empty());
    assert!(blocks(b"-----BEGIN PGP SIGNATURE-----\n\nSGVsbG8=\n", Kind::Signature, 3, 100).is_empty());
    assert!(blocks(b"-----BEGIN PGP SIGNATURE-----\n\n-----END PGP SIGNATURE-----\n", Kind::Signature, 3, 100).is_empty());
}

#[test]
fn cleartext_undoes_dash_escaping_and_trailing_white_space() {
    let text = b"Intro, not signed\r\n-----BEGIN PGP SIGNED MESSAGE-----\r\nHash: SHA256\r\n\r\nline one  \r\n- -dashes\r\n\r\nlast\t\r\n-----BEGIN PGP SIGNATURE-----\r\n\r\nSGVsbG8=\r\n-----END PGP SIGNATURE-----\r\ntrailer";
    let clear = cleartext(text, 100).unwrap();
    assert_eq!(clear.signed, b"line one\r\n-dashes\r\n\r\nlast");
    assert_eq!(clear.signature, b"Hello");
}

#[test]
fn an_incomplete_cleartext_is_none() {
    assert!(cleartext(b"-----BEGIN PGP SIGNED MESSAGE-----\n\ntext\n", 100).is_none());
    assert!(cleartext(b"-----BEGIN PGP SIGNED MESSAGE-----\n\ntext\n-----BEGIN PGP SIGNATURE-----\n\nSGVsbG8=\n", 100).is_none());
    assert!(cleartext(b"plain text", 100).is_none());
}
