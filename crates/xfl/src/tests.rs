//! The zip reader against the layouts writers produce and the archives an attacker would
//! (ADR 0054). Archives are made by the independent writer in `tests/support/fla.rs`.

use super::pack::{self, Raw, write, write_at};
use super::*;

fn zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    write(&files.iter().map(|(n, b, d)| Raw::new(n, b, *d)).collect::<Vec<_>>(), false)
}

/// The error opening `z`, or reading every file in it.
fn error(z: &[u8]) -> ZipError {
    match Archive::open(z) {
        Err(e) => e,
        Ok(a) => {
            let names: Vec<String> = a.names().map(str::to_string).collect();
            names.iter().find_map(|n| a.read(n).err()).expect("an error")
        }
    }
}

#[test]
fn reads_stored_and_deflated_entries() {
    let z = zip(&[("DOMDocument.xml", b"<DOMDocument/>", true), ("LIBRARY\\Ball.xml", b"<DOMSymbolItem/>", false)]);
    let a = Archive::open(&z).unwrap();
    assert_eq!(a.read("DOMDocument.xml").unwrap().unwrap(), b"<DOMDocument/>");
    assert_eq!(a.read("LIBRARY/Ball.xml").unwrap().unwrap(), b"<DOMSymbolItem/>", "backslashes normalised");
    assert_eq!(a.read("missing").unwrap(), None);
}

#[test]
fn the_checksum_matches_the_published_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
    let bytes: Vec<u8> = (0..5000u32).map(|i| (i * 7 + i / 13) as u8).collect();
    assert_eq!(crc32(&bytes), pack::crc32(&bytes), "the table matches the bitwise form");
}

#[test]
fn every_layout_a_writer_uses_reads_the_same() {
    let files: std::collections::BTreeMap<String, Vec<u8>> =
        [("DOMDocument.xml", "<DOMDocument/>"), ("LIBRARY/Café/Ünïcödé ★.xml", "<DOMSymbolItem/>"), ("bin/M 1 1.dat", "data")]
            .iter()
            .map(|(p, b)| (p.to_string(), b.as_bytes().to_vec()))
            .collect();
    for v in pack::VARIANTS {
        let z = pack::pack(&files, v);
        let a = Archive::open(&z).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert_eq!(a.counts().0, 3 + usize::from(v.mimetype), "{}", v.name);
        for (path, body) in &files {
            assert_eq!(a.read(path).unwrap_or_else(|e| panic!("{}: {e}", v.name)).as_deref(), Some(body.as_slice()), "{}: {path}", v.name);
        }
    }
}

#[test]
fn names_are_cp437_unless_flagged_or_utf8() {
    // "Café.xml" in CP437 (é is 0x82), not valid UTF-8, without the flag.
    let mut r = Raw::new("", b"x", false);
    (r.name, r.utf8) = (b"LIBRARY/Caf\x82.xml".to_vec(), false);
    let z = write(&[r.clone()], false);
    assert_eq!(Archive::open(&z).unwrap().names().collect::<Vec<_>>(), ["LIBRARY/Café.xml"]);
    assert_eq!(pack::cp437("LIBRARY/Café.xml").unwrap(), b"LIBRARY/Caf\x82.xml");
    // Flagged as UTF-8 but isn't: refused, not guessed.
    r.utf8 = true;
    assert!(matches!(Archive::open(&write(&[r], false)).err(), Some(ZipError::Damaged(_))));
    // A Unicode path field whose checksum doesn't match the name it stands in for is ignored.
    let mut r = Raw::new("plain.xml", b"x", false);
    r.utf8 = false;
    r.extra = [0x75, 0x70, 9, 0, 1, 0, 0, 0, 0, b'o', b't', b'h', b'r'].to_vec();
    assert_eq!(Archive::open(&write(&[r], false)).unwrap().names().collect::<Vec<_>>(), ["plain.xml"]);
}

#[test]
fn damaged_archives_are_errors() {
    assert_eq!(Archive::open(b"not a zip").err(), Some(ZipError::NotZip));
    assert_eq!(Archive::open(b"").err(), Some(ZipError::NotZip));
    let mut z = zip(&[("a", b"hello", true)]);
    let n = z.len();
    // Point the directory past the end.
    z[n - 6] = 0xff;
    assert!(Archive::open(&z).is_err());
}

#[test]
fn a_truncated_central_directory_is_damaged() {
    let z = zip(&[("DOMDocument.xml", b"<DOMDocument/>", true), ("b.xml", b"<b/>", true), ("c.xml", b"<c/>", false)]);
    let dir_at = u32_at(&z, z.len() - 6).unwrap() as usize;
    let end = z.len() - 22;
    // Every cut inside the directory, keeping the end record (which still claims 3 entries).
    for cut in dir_at..end {
        let mut t = z[..cut].to_vec();
        t.extend_from_slice(&z[end..]);
        let e = Archive::open(&t).err().unwrap_or_else(|| panic!("cut at {cut} opened"));
        assert!(!e.to_string().is_empty());
    }
    // The end record says more entries than there are.
    let mut t = z.clone();
    t[end + 8] = 9;
    t[end + 10] = 9;
    assert_eq!(Archive::open(&t).err(), Some(ZipError::Damaged("the central directory is cut short")));
    // Cut short anywhere at all: an error, never a panic.
    for cut in 0..z.len() {
        let _ = Archive::open(&z[..cut]).map(|a| a.read("DOMDocument.xml"));
    }
}

#[test]
fn overlapping_entries_are_refused() {
    // Two names for one local header (the overlapping-file zip bomb).
    let mut b = Raw::new("b.xml", b"<a/>", true);
    b.share = Some(0);
    let z = write(&[Raw::new("a.xml", b"<a/>", true), b], false);
    assert!(matches!(Archive::open(&z).err(), Some(ZipError::Overlap(..))), "{:?}", Archive::open(&z).err());
    // An entry whose stated compressed size runs into the next one.
    let mut z = zip(&[("a.xml", b"aaaaaaaaaa", false), ("b.xml", b"bbbbbbbbbb", false)]);
    let dir_at = u32_at(&z, z.len() - 6).unwrap() as usize;
    z[dir_at + 20] = 20; // a's compressed size, in its central record
    z[dir_at + 24] = 20;
    assert!(matches!(Archive::open(&z).err(), Some(ZipError::Overlap(..))));
}

#[test]
fn entries_never_reach_past_their_data() {
    // An entry's data running into the central directory, or past the end.
    for size in [64u32, 0xffff_fff0] {
        let mut z = zip(&[("a.xml", b"aaaaaaaaaa", false)]);
        let dir_at = u32_at(&z, z.len() - 6).unwrap() as usize;
        z[dir_at + 20..dir_at + 24].copy_from_slice(&size.to_le_bytes());
        assert_eq!(Archive::open(&z).err(), Some(ZipError::DamagedEntry("a.xml".into(), "is outside the archive")));
    }
    // A local header offset past the end.
    let mut z = zip(&[("a.xml", b"aaaaaaaaaa", false)]);
    let dir_at = u32_at(&z, z.len() - 6).unwrap() as usize;
    z[dir_at + 42..dir_at + 46].copy_from_slice(&0x7fff_0000u32.to_le_bytes());
    assert_eq!(Archive::open(&z).err(), Some(ZipError::DamagedEntry("a.xml".into(), "has no local header")));
}

#[test]
fn lying_sizes_and_checksums_are_damaged() {
    // Says 100 bytes, inflates to 1000.
    let body = vec![b'x'; 1000];
    let mut r = Raw::new("a.xml", &body, true);
    r.size = Some(100);
    assert_eq!(error(&write(&[r], false)), ZipError::DamagedEntry("a.xml".into(), "expands past the size it gives"));
    // Says 2000, inflates to 1000.
    let mut r = Raw::new("a.xml", &body, true);
    r.size = Some(2000);
    assert_eq!(error(&write(&[r], false)), ZipError::DamagedEntry("a.xml".into(), "is not the size it gives"));
    // A stored entry with two sizes.
    let mut r = Raw::new("a.xml", &body, false);
    r.size = Some(10);
    assert_eq!(error(&write(&[r], false)), ZipError::DamagedEntry("a.xml".into(), "is stored with two different sizes"));
    // The wrong checksum.
    let mut r = Raw::new("a.xml", &body, true);
    r.crc = Some(12345);
    assert_eq!(error(&write(&[r], false)), ZipError::DamagedEntry("a.xml".into(), "fails its checksum"));
    // Garbage instead of deflate data.
    let mut r = Raw::new("a.xml", &body, true);
    r.compressed = Some(vec![0xff; 40]);
    assert_eq!(error(&write(&[r], false)), ZipError::DamagedEntry("a.xml".into(), "has damaged compressed data"));
}

#[test]
fn compression_bombs_are_refused() {
    // A real 64 MB of zeros deflates past the ratio cap: refused before inflating.
    let zeros = vec![0u8; 64 << 20];
    let z = zip(&[("bomb.xml", &zeros, true)]);
    assert!(z.len() < 100_000, "{}", z.len());
    assert_eq!(Archive::open(&z).err(), Some(ZipError::Bomb("bomb.xml".into())));
    // A size claim no deflate stream of that length could meet.
    let mut r = Raw::new("a.xml", &[7; 100], true);
    r.size = Some(200 << 20);
    assert_eq!(Archive::open(&write(&[r], false)).err(), Some(ZipError::Bomb("a.xml".into())));
    // One entry larger than an entry may be, at a believable ratio: refused before inflating.
    let mut r = Raw::new("big.dat", b"", true);
    r.compressed = Some(vec![0; 300 << 10]);
    r.size = Some(MAX_ENTRY as u64 + 1);
    let z = write(&[r], false);
    assert_eq!(error(&z), ZipError::TooLarge("big.dat".into()));
}

#[test]
fn the_archive_hands_out_at_most_its_budget() {
    let body = vec![b'a'; 4096];
    let z = zip(&[("a.xml", &body, true)]);
    let a = Archive::open(&z).unwrap();
    a.budget.set(10_000);
    assert!(a.read("a.xml").is_ok() && a.read("a.xml").is_ok());
    assert_eq!(a.read("a.xml").err(), Some(ZipError::TooMuch), "a third read passes 10,000 bytes");
}

#[test]
fn unsafe_paths_are_refused() {
    for name in [
        "../escape.xml",
        "LIBRARY/../../escape.xml",
        "/etc/passwd",
        "\\Windows\\win.ini",
        "C:\\Windows\\win.ini",
        "C:escape.xml",
        "LIBRARY\\..\\..\\x",
        "a\0b",
    ] {
        let z = zip(&[("DOMDocument.xml", b"<DOMDocument/>", true), (name, b"x", false)]);
        assert!(matches!(Archive::open(&z).err(), Some(ZipError::Unsafe(_))), "{name:?}: {:?}", Archive::open(&z).err());
    }
    // Harmless odd names are tidied, not refused.
    let z = zip(&[("./LIBRARY//a.xml", b"x", false)]);
    assert_eq!(Archive::open(&z).unwrap().names().collect::<Vec<_>>(), ["LIBRARY/a.xml"]);
}

#[test]
fn duplicate_names_are_refused() {
    let z = zip(&[("LIBRARY/a.xml", b"one", false), ("LIBRARY\\a.xml", b"two", false)]);
    assert_eq!(Archive::open(&z).err(), Some(ZipError::Duplicate("LIBRARY\\a.xml".into())));
    // Folders listed twice are harmless.
    let z = zip(&[("LIBRARY/", b"", false), ("LIBRARY\\", b"", false), ("LIBRARY/a.xml", b"one", false)]);
    assert_eq!(Archive::open(&z).unwrap().counts(), (1, 2));
}

#[test]
fn drive_prefix_after_normalized_dot_segments_is_refused() {
    for name in ["./C:/LIBRARY/a.xml", "./C:\\LIBRARY\\a.xml"] {
        assert!(matches!(Archive::open(&zip(&[(name, b"x", false)])).err(), Some(ZipError::Unsafe(_))));
    }
    assert!(Archive::open(&zip(&[("./LIBRARY/a.xml", b"x", false)])).is_ok());
}

#[test]
fn invalid_eocd_comment_candidate_does_not_mask_archive_end() {
    let mut z = zip(&[("a.xml", b"x", false)]);
    let end = z.len() - 22;
    z[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    let fake = z.len();
    z.extend_from_slice(&END.to_le_bytes());
    z.extend_from_slice(&[0; 16]);
    z.extend_from_slice(&u16::MAX.to_le_bytes());
    assert!(Archive::open(&z).is_ok());
    assert_eq!(fake + 22, z.len());
}

#[test]
fn eocd_signature_inside_comment_is_not_an_empty_archive() {
    let mut z = zip(&[("a.xml", b"x", false)]);
    let end = z.len() - 22;
    z[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    z.extend_from_slice(&END.to_le_bytes());
    z.extend_from_slice(&[0; 18]);
    let archive = Archive::open(&z).unwrap();
    assert_eq!(archive.names().collect::<Vec<_>>(), ["a.xml"]);
}

#[test]
fn central_directory_must_be_consumed_exactly() {
    let mut z = zip(&[("a.xml", b"x", false)]);
    let end = z.len() - 22;
    let size = u32::from_le_bytes(z[end + 12..end + 16].try_into().unwrap());
    z.insert(end, 0);
    z[end + 12 + 1..end + 16 + 1].copy_from_slice(&(size + 1).to_le_bytes());
    assert_eq!(Archive::open(&z).err(), Some(ZipError::Damaged("the central directory has trailing data")));
}

#[test]
fn nonempty_comment_eocd_candidate_does_not_mask_valid_archive() {
    let mut bytes = zip(&[("a.xml", b"x", false)]);
    let end = bytes.len() - 22;
    // The fake EOCD sits in the real EOCD's comment and claims the real directory plus the
    // real EOCD as a shifted directory. Candidate selection must reject its trailing bytes.
    bytes[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    bytes.extend_from_slice(&END.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&73u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    let archive = Archive::open(&bytes).unwrap();
    assert_eq!(archive.read("a.xml").unwrap().unwrap(), b"x");
}

#[test]
fn repeated_eocd_directory_candidates_reuse_entry_validation() {
    let entries = (0..1024).map(|index| Raw::new(&format!("entry-{index:04}.xml"), b"", false)).collect::<Vec<_>>();
    let mut bytes = write(&entries, false);
    let eocd = bytes.len() - 22;
    bytes[eocd + 20..eocd + 22].copy_from_slice(&1000u16.to_le_bytes());
    bytes.extend_from_slice(&vec![0; 1000]);
    let directory_start = u32_at(&bytes, eocd + 16).unwrap() as usize;
    let directory_size = u64::from(u32_at(&bytes, eocd + 12).unwrap());
    let count = u64::from(u16_at(&bytes, eocd + 10).unwrap());
    let mut jumps = std::collections::HashMap::new();

    assert!(directory_is_exact(&bytes, directory_start, directory_size, count, &mut jumps));
    let indexed_records = jumps.len();
    // Each size models a fake shifted EOCD in the archive comment that starts at the same
    // central-directory record but claims additional bytes through the real EOCD.
    for extra in 1..=1000 {
        assert!(!directory_is_exact(&bytes, directory_start, directory_size + extra, count, &mut jumps));
    }
    assert_eq!(jumps.len(), indexed_records, "repeated candidates must reuse the indexed entry jumps");
}

#[test]
fn deeply_nested_folders_are_refused() {
    let ok = format!("{}a.xml", "f/".repeat(MAX_DEPTH));
    assert!(Archive::open(&zip(&[(&ok, b"x", false)])).is_ok());
    let deep = format!("{}a.xml", "f/".repeat(MAX_DEPTH + 1));
    assert!(matches!(Archive::open(&zip(&[(&deep, b"x", false)])).err(), Some(ZipError::TooDeep(_))));
    // As deep as a name can be: refused quickly, not recursed into.
    let deepest = "f/".repeat(30_000);
    assert!(matches!(Archive::open(&zip(&[(&deepest, b"", false)])).err(), Some(ZipError::TooDeep(_))));
}

#[test]
fn unsupported_features_say_what_they_are() {
    assert_eq!(
        ZipError::UnsupportedArchive("split across several files").to_string(),
        "the archive uses a zip feature this XFL reader can't read (split across several files)"
    );
    assert_eq!(ZipError::Unsupported("a.xml".into(), "encryption").to_string(), "'a.xml' uses a zip feature this XFL reader can't read (encryption)");
    let mut r = Raw::new("a.xml", b"secret", false);
    r.flags = 1;
    assert_eq!(error(&write(&[r], false)), ZipError::Unsupported("a.xml".into(), "encryption"));
    let mut r = Raw::new("a.xml", b"x", false);
    r.method = Some(14);
    assert_eq!(error(&write(&[r], false)), ZipError::Unsupported("a.xml".into(), "LZMA compression"));
    // Split archives.
    let mut z = zip(&[("a.xml", b"x", false)]);
    let end = z.len() - 22;
    z[end + 4] = 1;
    assert_eq!(Archive::open(&z).err(), Some(ZipError::UnsupportedArchive("split across several files")));
    // Too many entries for the directory to hold, and more than the cap.
    let mut z = zip(&[("a.xml", b"x", false)]);
    let end = z.len() - 22;
    z[end + 8..end + 12].copy_from_slice(&[0xfe, 0xff, 0xfe, 0xff]);
    assert!(Archive::open(&z).is_err());
}

#[test]
fn zip64_archives_open_and_their_damage_is_named() {
    let mut a = Raw::new("a.xml", b"<a/>", true);
    a.zip64 = true;
    let mut b = Raw::new("b.xml", b"<b/>", false);
    (b.zip64, b.descriptor) = (true, true);
    let z = write(&[a.clone(), b], true);
    let arch = Archive::open(&z).unwrap();
    assert_eq!(arch.read("b.xml").unwrap().unwrap(), b"<b/>");
    // The locator gone.
    let mut t = z.clone();
    let loc = t.len() - 22 - 20;
    t[loc] = 0;
    assert_eq!(Archive::open(&t).err(), Some(ZipError::Damaged("the ZIP64 end record is missing")));
    // An entry that says ZIP64 without the extra field.
    a.zip64 = false;
    let mut t = write(&[a], false);
    let dir_at = u32_at(&t, t.len() - 6).unwrap() as usize;
    t[dir_at + 24..dir_at + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(Archive::open(&t).err(), Some(ZipError::DamagedEntry("a.xml".into(), "has no ZIP64 sizes")));
}

#[test]
fn zip64_count_sentinel_keeps_a_comment_eocd_from_masking_the_archive() {
    let mut bytes = write(&[Raw::new("a.xml", b"x", false)], true);
    let end = bytes.len() - 22;
    let locator = end - 20;
    let zip64_end = locator - 56;
    let directory_size = u64_at(&bytes, zip64_end + 40).unwrap() as u32;
    let directory_offset = u64_at(&bytes, zip64_end + 48).unwrap() as u32;
    // Count requires ZIP64, while directory size and offset still fit in the classic fields.
    bytes[end + 12..end + 16].copy_from_slice(&directory_size.to_le_bytes());
    bytes[end + 16..end + 20].copy_from_slice(&directory_offset.to_le_bytes());
    bytes[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    // This valid empty EOCD-shaped signature is inside the real end record's comment.
    bytes.extend_from_slice(&END.to_le_bytes());
    bytes.extend_from_slice(&[0; 18]);

    let archive = Archive::open(&bytes).unwrap();
    assert_eq!(archive.read("a.xml").unwrap().unwrap(), b"x");
}

#[test]
fn empty_relative_zip64_directory_opens_after_sfx_stub() {
    let mut bytes = b"MZ self-extracting stub".to_vec();
    bytes.extend_from_slice(&write_at(&[], true, 0));
    let archive = Archive::open(&bytes).unwrap();
    assert_eq!(archive.names().count(), 0);
}

#[test]
fn self_extracting_archives_support_relative_and_adjusted_offsets() {
    let entries = [Raw::new("DOMDocument.xml", b"<DOMDocument/>", true), Raw::new("LIBRARY/a.xml", b"<DOMSymbolItem/>", false)];
    let stub = b"MZ\x90\0 self-extracting program with no ZIP signatures ".repeat(31);
    for zip64 in [false, true] {
        for adjusted in [false, true] {
            let base = if adjusted { stub.len() as u64 } else { 0 };
            let mut bytes = stub.clone();
            bytes.extend_from_slice(&write_at(&entries, zip64, base));
            let archive = Archive::open(&bytes).unwrap_or_else(|error| panic!("ZIP64 {zip64}, adjusted {adjusted}: {error}"));
            assert_eq!(archive.read("DOMDocument.xml").unwrap().unwrap(), b"<DOMDocument/>");
            assert_eq!(archive.read("LIBRARY/a.xml").unwrap().unwrap(), b"<DOMSymbolItem/>");
            if !zip64 && !adjusted {
                let eocd = bytes.len() - 22;
                let stated = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
                assert_eq!(&bytes[stated + stub.len()..stated + stub.len() + 4], b"PK\x01\x02");
            }
        }
    }

    // A gap between the directory and end record cannot be guessed as a prefix shift.
    let mut bytes = stub;
    bytes.extend_from_slice(&write_at(&entries, false, 0));
    let eocd = bytes.len() - 22;
    bytes.splice(eocd..eocd, *b"junk");
    assert!(Archive::open(&bytes).is_err());
}

#[test]
fn errors_read_as_sentences() {
    let all = [
        ZipError::NotZip,
        ZipError::Damaged("the central directory is cut short"),
        ZipError::DamagedEntry("a.xml".into(), "fails its checksum"),
        ZipError::UnsupportedArchive("split across several files"),
        ZipError::Unsupported("a.xml".into(), "encryption"),
        ZipError::TooLarge("a.xml".into()),
        ZipError::TooMuch,
        ZipError::TooMany,
        ZipError::Bomb("a.xml".into()),
        ZipError::Overlap("a.xml".into(), "b.xml".into()),
        ZipError::Unsafe("../a.xml".into()),
        ZipError::Duplicate("a.xml".into()),
        ZipError::TooDeep("f/f/a.xml".into()),
    ];
    for e in all {
        let s = e.to_string();
        assert!(s.len() > 12 && !s.contains("{") && s.chars().next().is_some_and(|c| !c.is_uppercase() || c == 'A'), "{s}");
    }
    assert_eq!(ZipError::TooMuch.to_string(), "the archive asks for more than 1024 MB in all");
}
