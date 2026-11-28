#![cfg(feature = "lenient")]
// From Craft Anything crates/formats/tests/zip_decoded_size.rs.

use super::common;
use craft_zip::lenient::{self as zip, ZipEntry, ZipError};

fn deflated(body: &[u8]) -> (Vec<u8>, ZipEntry) {
    let compressed = miniz_oxide::deflate::compress_to_vec(body, 6);
    let mut data = vec![0; 30];
    data[..4].copy_from_slice(b"PK\x03\x04");
    data.extend_from_slice(&compressed);
    let entry = ZipEntry {
        name: "payload".into(),
        method: 8,
        flags: 0,
        crc32: zip::crc32(body),
        compressed_size: compressed.len() as u64,
        uncompressed_size: body.len() as u64,
        local_header_offset: 0,
        is_dir: false,
        encrypted: false,
        unix_mode: None,
    };
    (data, entry)
}

#[test]
fn decoded_length_must_match_the_directory_even_when_the_crc_matches() {
    let (data, good) = deflated(b"complete body");
    for declared in [1, good.uncompressed_size + 1] {
        let entry = ZipEntry { uncompressed_size: declared, ..good.clone() };
        assert!(matches!(zip::read_entry(&data, &entry, 100), Err(ZipError::Corrupt(_))));
        assert!(matches!(zip::read_entry_unverified(&data, &entry, 100), Err(ZipError::Corrupt(_))));
    }
    assert_eq!(zip::read_entry(&data, &good, 100).unwrap(), b"complete body");
}

#[test]
fn stored_length_must_also_match_the_directory() {
    let data = common::zip_stored(&[("payload", b"hello")]);
    let mut entry = zip::parse(&data, 10).unwrap().entries.remove(0);
    entry.uncompressed_size = 4;
    assert!(matches!(zip::read_entry(&data, &entry, 100), Err(ZipError::Corrupt(_))));
}
