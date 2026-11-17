//! The reader's tests, from Craft Anything's `zip.rs`.

use super::*;

/// Minimal stored-only ZIP writer for tests.
pub(crate) fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, body) in files {
        let off = out.len() as u32;
        let crc = crc32(body);
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        cd.extend_from_slice(b"PK\x01\x02");
        cd.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&(body.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(body.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&[0; 12]);
        cd.extend_from_slice(&off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(all(feature = "lenient-bzip2", feature = "lenient-lzma"))]
fn build_codec_entry(method: u16, compressed: &[u8], plain: &[u8]) -> Vec<u8> {
    let mut file = build(&[("entry", compressed)]);
    let central = 30 + "entry".len() + compressed.len();
    let version: u16 = if method == 12 { 46 } else { 63 };
    file[4..6].copy_from_slice(&version.to_le_bytes());
    file[8..10].copy_from_slice(&method.to_le_bytes());
    file[14..18].copy_from_slice(&crc32(plain).to_le_bytes());
    file[22..26].copy_from_slice(&(plain.len() as u32).to_le_bytes());
    file[central + 6..central + 8].copy_from_slice(&version.to_le_bytes());
    file[central + 10..central + 12].copy_from_slice(&method.to_le_bytes());
    file[central + 16..central + 20].copy_from_slice(&crc32(plain).to_le_bytes());
    file[central + 24..central + 28].copy_from_slice(&(plain.len() as u32).to_le_bytes());
    file
}

#[cfg(all(feature = "lenient-bzip2", feature = "lenient-lzma"))]
#[test]
fn bzip2_and_xz_zip_entries_keep_size_crc_and_output_checks() {
    use crate::lenient::testkit::{BZIP2_FIXTURE, BZIP2_PLAIN, XZ_FIXTURE, XZ_PLAIN};
    for (method, compressed, plain) in [(12, BZIP2_FIXTURE, BZIP2_PLAIN), (95, XZ_FIXTURE, XZ_PLAIN)] {
        let file = build_codec_entry(method, compressed, plain);
        let original = parse(&file, 1).unwrap().entries.remove(0);
        assert_eq!(read_entry(&file, &original, plain.len() as u64).unwrap(), plain);
        let mut entry = original.clone();
        entry.crc32 ^= 1;
        assert!(matches!(read_entry(&file, &entry, 100), Err(ZipError::CrcMismatch { .. })));
        assert_eq!(read_entry_unverified(&file, &entry, 100).unwrap(), plain);
        entry = original.clone();
        entry.uncompressed_size = 0;
        assert!(matches!(read_entry(&file, &entry, 100), Err(ZipError::Corrupt(_))));
        assert!(matches!(read_entry(&file, &entry, plain.len() as u64 - 1), Err(ZipError::TooLarge(_))));
        let mut damaged = file.clone();
        damaged[30 + "entry".len()] ^= 0xff;
        assert!(matches!(read_entry(&damaged, &original, 100), Err(ZipError::Corrupt(_))));
        entry = original;
        entry.encrypted = true;
        assert!(matches!(read_entry(&file, &entry, 100), Err(ZipError::Unsupported(_))));
    }
}

#[cfg(feature = "lenient-lzma")]
fn zip_lzma_member(plain: &[u8], end_marker: bool) -> Vec<u8> {
    let mut encoded = Vec::new();
    let size = if end_marker { None } else { Some(plain.len() as u64) };
    let options = lzma_rs::compress::Options { unpacked_size: lzma_rs::compress::UnpackedSize::WriteToHeader(size) };
    lzma_rs::lzma_compress_with_options(&mut std::io::Cursor::new(plain), &mut encoded, &options).unwrap();
    let mut compressed = vec![9, 4, 5, 0];
    compressed.extend_from_slice(&encoded[..5]);
    compressed.extend_from_slice(&encoded[13..]);
    let mut file = build(&[("entry", &compressed)]);
    let central = 30 + "entry".len() + compressed.len();
    let flags: u16 = if end_marker { 2 } else { 0 };
    for (version_at, flags_at, method_at, crc_at, size_at) in
        [(4, 6, 8, 14, 22), (central + 6, central + 8, central + 10, central + 16, central + 24)]
    {
        file[version_at..version_at + 2].copy_from_slice(&63u16.to_le_bytes());
        file[flags_at..flags_at + 2].copy_from_slice(&flags.to_le_bytes());
        file[method_at..method_at + 2].copy_from_slice(&14u16.to_le_bytes());
        file[crc_at..crc_at + 4].copy_from_slice(&crc32(plain).to_le_bytes());
        file[size_at..size_at + 4].copy_from_slice(&(plain.len() as u32).to_le_bytes());
    }
    file
}

#[cfg(feature = "lenient-lzma")]
#[test]
fn zip_lzma_reads_size_delimited_and_end_marked_streams() {
    for end_marker in [false, true] {
        for plain in [&b"ZIP LZMA contents"[..], &[0; 23][..]] {
            let file = zip_lzma_member(plain, end_marker);
            let original = parse(&file, 1).unwrap().entries.remove(0);
            assert_eq!(read_entry(&file, &original, plain.len() as u64).unwrap(), plain);
            assert!(matches!(read_entry(&file, &original, plain.len() as u64 - 1), Err(ZipError::TooLarge(_))));
            let mut entry = original.clone();
            entry.crc32 ^= 1;
            assert!(matches!(read_entry(&file, &entry, 100), Err(ZipError::CrcMismatch { .. })));
            entry = original.clone();
            entry.uncompressed_size -= 1;
            assert!(read_entry(&file, &entry, 100).is_err());
            entry = original.clone();
            entry.compressed_size -= 1;
            assert!(read_entry(&file, &entry, 100).is_err());
            entry = original;
            entry.compressed_size += 1;
            assert!(read_entry(&file, &entry, 100).is_err());
        }
    }
}

#[cfg(feature = "lenient-lzma")]
#[test]
fn zip_lzma_rejects_bad_properties_without_unbounded_dictionaries() {
    let mut file = zip_lzma_member(b"small", true);
    let entry = parse(&file, 1).unwrap().entries.remove(0);
    let body = 30 + "entry".len();
    file[body + 5..body + 9].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(read_entry(&file, &entry, 5).unwrap(), b"small");
    file[body + 4] = 255;
    assert!(read_entry(&file, &entry, 5).is_err());
    file[body + 2..body + 4].copy_from_slice(&6u16.to_le_bytes());
    assert!(matches!(read_entry(&file, &entry, 5), Err(ZipError::Unsupported(_))));
}

fn build_named_entry(name: &[u8], flags: u16) -> Vec<u8> {
    let mut zip = build(&[(&"x".repeat(name.len()), b"")]);
    let central = 30 + name.len();
    zip[6..8].copy_from_slice(&flags.to_le_bytes());
    zip[30..central].copy_from_slice(name);
    zip[central + 8..central + 10].copy_from_slice(&flags.to_le_bytes());
    zip[central + 46..central + 46 + name.len()].copy_from_slice(name);
    zip
}

#[test]
fn verified_unicode_path_overrides_a_legacy_zip_name() {
    let raw = b"notes.txt";
    let unicode = "笔记.txt";
    for (flags, version, checksum, expected) in
        [(0, 1, crc32(raw), unicode), (0, 1, crc32(b"old-name"), "notes.txt"), (0, 2, crc32(raw), "notes.txt"), (1 << 11, 1, crc32(raw), "notes.txt")]
    {
        let mut zip = build_named_entry(raw, flags);
        let central = 30 + raw.len();
        let end = central + 46 + raw.len();
        let mut extra = 0x7075_u16.to_le_bytes().to_vec();
        extra.extend_from_slice(&((5 + unicode.len()) as u16).to_le_bytes());
        extra.push(version);
        extra.extend_from_slice(&checksum.to_le_bytes());
        extra.extend_from_slice(unicode.as_bytes());
        zip[central + 30..central + 32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
        zip[end + 12..end + 16].copy_from_slice(&((end - central + extra.len()) as u32).to_le_bytes());
        zip.splice(end..end, extra);
        assert_eq!(parse(&zip, 1).unwrap().entries[0].name, expected);
    }
}

#[test]
fn directory_markers_do_not_require_a_trailing_slash() {
    let sample = build(&[("folder", b"")]);
    let central = 30 + "folder".len();
    for (host, attrs, directory, special) in [
        (0, 0x10u32, true, None),
        (10, 0x10, true, None),
        (3, 0o040755 << 16, true, None),
        (3, 0o120777 << 16, false, Some("symlink")),
        (0, 0x20, false, None),
        (14, 0x10, true, None),
        (14, 0x20, false, None),
        (19, 0o040755 << 16, true, None),
        (19, 0o100644 << 16, false, None),
        (19, 0o120777 << 16, false, Some("symlink")),
        (42, 0x10, false, None),
    ] {
        let mut file = sample.clone();
        file[central + 5] = host;
        file[central + 38..central + 42].copy_from_slice(&attrs.to_le_bytes());
        let archive = parse(&file, 1).unwrap();
        let entry = &archive.entries[0];
        assert_eq!((entry.is_dir, entry.special_kind()), (directory, special), "host {host}, attributes {attrs:#x}");
    }
}

#[test]
fn a_cp437_name_without_the_utf8_flag_is_decoded() {
    let zip = build_named_entry(b"caf\x82.txt", 0);
    assert_eq!(parse(&zip, 10).unwrap().entries[0].name, "café.txt");
}

#[test]
fn a_cp437_box_drawing_name_without_the_utf8_flag_is_decoded() {
    let zip = build_named_entry(b"\xc9.txt", 0);
    assert_eq!(parse(&zip, 10).unwrap().entries[0].name, "╔.txt");
}

#[test]
fn a_utf8_name_with_the_utf8_flag_is_preserved() {
    let zip = build_named_entry("한글/é.txt".as_bytes(), 0x0800);
    assert_eq!(parse(&zip, 10).unwrap().entries[0].name, "한글/é.txt");
}

#[test]
fn a_valid_utf8_name_without_the_utf8_flag_is_preserved() {
    let zip = build_named_entry("é.txt".as_bytes(), 0);
    assert_eq!(parse(&zip, 10).unwrap().entries[0].name, "é.txt");
}

#[test]
fn an_ascii_name_is_preserved() {
    let zip = build_named_entry(b"dir/file.txt", 0);
    assert_eq!(parse(&zip, 10).unwrap().entries[0].name, "dir/file.txt");
}

#[test]
fn valid_entry_survives_eocd_signature_in_comment() {
    let mut zip = build(&[("a.txt", b"hello")]);
    let comment = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
    let eocd = zip.len() - 22;
    zip[eocd + 20..eocd + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    zip.extend_from_slice(comment);

    assert_eq!(parse(&zip, 10).unwrap().entries.len(), 1);
}

#[test]
fn valid_entry_survives_bytes_after_eocd_comment() {
    let mut zip = build(&[("a.txt", b"hello")]);
    zip.extend_from_slice(b"tail");

    assert_eq!(parse(&zip, 10).unwrap().entries.len(), 1);
}

#[test]
fn checks_the_zip64_record_extent_including_extension_bytes() {
    let mut record = vec![0; 56];
    record[..4].copy_from_slice(b"PK\x06\x06");
    record[4..12].copy_from_slice(&46u64.to_le_bytes());
    record.extend([1, 2]); // extensible data
    record.extend(b"PK\x06\x07");
    record.extend([0; 12]);
    record.extend(1u32.to_le_bytes());
    record.extend(b"PK\x05\x06");
    record.extend([0; 4]);
    record.extend([0xff; 4]);
    record.extend([0; 10]);
    assert!(parse(&record, 1).unwrap().zip64);
    for len in [44u64, 47] {
        record[4..12].copy_from_slice(&len.to_le_bytes());
        let archive = parse(&record, 1).unwrap();
        assert!(archive.zip64);
        assert_eq!(archive.warnings.len(), 1);
    }
    for len in [43u64, u64::MAX] {
        record[4..12].copy_from_slice(&len.to_le_bytes());
        assert!(parse(&record, 1).is_err(), "{len}");
    }
}

#[test]
fn lists_and_reads_stored_entries() {
    let z = build(&[("a.txt", b"hello"), ("dir/b.js", b"console.log(1)")]);
    let a = parse(&z, 100).unwrap();
    assert_eq!(a.entries.len(), 2);
    assert_eq!(a.entries[1].name, "dir/b.js");
    assert_eq!(read_entry(&z, &a.entries[0], 100).unwrap(), b"hello");
    assert!(matches!(read_entry(&z, &a.entries[1], 3), Err(ZipError::TooLarge(3))));
}

#[cfg(feature = "lenient-zstd")]
#[test]
fn zstandard_members_are_read_with_their_limits_and_checks() {
    // A raw Zstandard frame holding "hello zip" (single segment, one raw block).
    let body = b"hello zip";
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, body.len() as u8];
    let block_header = ((body.len() as u32) << 3) | 1;
    frame.extend(&block_header.to_le_bytes()[..3]);
    frame.extend(body);
    let mut file = vec![0u8; 30];
    file[..4].copy_from_slice(b"PK\x03\x04");
    file.extend(&frame);
    let mut e = ZipEntry {
        name: "x".into(),
        method: 93,
        flags: 0,
        crc32: crc32(body),
        compressed_size: frame.len() as u64,
        uncompressed_size: body.len() as u64,
        local_header_offset: 0,
        is_dir: false,
        encrypted: false,
        unix_mode: None,
    };
    assert_eq!(read_entry(&file, &e, 100).unwrap(), body);
    assert!(matches!(read_entry(&file, &e, 3), Err(ZipError::TooLarge(3))));
    e.crc32 ^= 1;
    assert!(matches!(read_entry(&file, &e, 100), Err(ZipError::CrcMismatch { .. })));
}

#[test]
fn deflate_round_trip_and_crc_check() {
    let body = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".repeat(20);
    let comp = miniz_oxide::deflate::compress_to_vec(&body, 6);
    let e = ZipEntry {
        name: "x".into(),
        method: 8,
        flags: 0,
        crc32: crc32(&body),
        compressed_size: comp.len() as u64,
        uncompressed_size: body.len() as u64,
        local_header_offset: 0,
        is_dir: false,
        encrypted: false,
        unix_mode: None,
    };
    let mut z = b"PK\x03\x04".to_vec();
    z.extend_from_slice(&[0; 26]);
    z.extend_from_slice(&comp);
    assert_eq!(read_entry(&z, &e, 1 << 20).unwrap(), body);
    // A lying uncompressed size can't make us exceed the cap.
    let liar = ZipEntry { uncompressed_size: 10, ..e.clone() };
    assert!(matches!(read_entry(&z, &liar, 10), Err(ZipError::TooLarge(10))));
    let bad = ZipEntry { crc32: 1, ..e };
    assert!(matches!(read_entry(&z, &bad, 1 << 20), Err(ZipError::CrcMismatch { .. })));
}

#[test]
fn hostile_inputs_do_not_panic() {
    let z = build(&[("a", b"1"), ("b", b"22")]);
    for cut in 0..z.len() {
        if let Ok(a) = parse(&z[..cut], 10) {
            for e in &a.entries {
                let _ = read_entry(&z[..cut], e, 100);
            }
        }
    }
    for i in 0..z.len() {
        let mut m = z.clone();
        m[i] ^= 0xff;
        if let Ok(a) = parse(&m, 10) {
            for e in &a.entries {
                let _ = read_entry(&m, e, 100);
            }
        }
    }
    assert_eq!(parse(b"", 1), Err(ZipError::NotZip));
}

#[test]
fn crc_known_value() {
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

#[cfg(feature = "serde")]
#[test]
fn listings_serialize_without_empty_warnings_or_unix_mode() {
    let data = crate::lenient::testkit::zip_stored(&[("a.txt", b"hi")]);
    let archive = parse(&data, 10).unwrap();
    let value = serde_json::to_value(&archive).unwrap();
    assert!(value.get("warnings").is_none());
    assert_eq!(value["entries"][0]["name"], "a.txt");
    assert!(value["entries"][0].get("unix_mode").is_none());
}

#[cfg(not(feature = "lenient-zstd"))]
#[test]
fn a_method_left_out_of_the_build_is_unsupported() {
    let mut data = crate::lenient::testkit::zip_stored(&[("a", b"abc")]);
    let archive = parse(&data, 10).unwrap();
    let mut entry = archive.entries[0].clone();
    entry.method = 93;
    data[8..10].copy_from_slice(&93u16.to_le_bytes());
    assert!(matches!(read_entry_unverified(&data, &entry, 100), Err(ZipError::Unsupported(m)) if m.contains("not built in")));
}

// RRFZ, Craft Anything 29badae7: deprecated ZIP method 20 is Zstandard.
#[cfg(feature = "lenient-zstd")]
mod legacy_zstandard {
    use crate::lenient as zip;
    use crate::lenient::testkit::zip_stored;

    const BODY: &[u8] = b"zstandard payload";

    /// Build a ZIP with one single-segment Zstandard frame containing a raw final block.
    fn sample(method: u16) -> Vec<u8> {
        let size = BODY.len() as u8;
        let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, size, (size << 3) | 1, 0, 0];
        frame.extend_from_slice(BODY);
        let mut data = zip_stored(&[("a.txt", &frame)]);
        let cd = 30 + "a.txt".len() + frame.len();
        data[8..10].copy_from_slice(&method.to_le_bytes());
        data[14..18].copy_from_slice(&zip::crc32(BODY).to_le_bytes());
        data[22..26].copy_from_slice(&u32::from(size).to_le_bytes());
        data[cd + 10..cd + 12].copy_from_slice(&method.to_le_bytes());
        data[cd + 16..cd + 20].copy_from_slice(&zip::crc32(BODY).to_le_bytes());
        data[cd + 24..cd + 28].copy_from_slice(&u32::from(size).to_le_bytes());
        data
    }

    /// The deprecated and current method IDs both decode through the bounded ZIP reader.
    #[test]
    fn reads_zstandard_members_under_both_method_ids() {
        for method in [93, 20] {
            let data = sample(method);
            let entry = zip::parse(&data, 1).unwrap().entries.remove(0);
            assert_eq!(entry.method_name(), "zstd", "method {method}");
            assert_eq!(zip::read_entry(&data, &entry, 100).unwrap(), BODY);
            assert!(matches!(zip::read_entry(&data, &entry, 2), Err(zip::ZipError::TooLarge(2))));
        }
    }
}
