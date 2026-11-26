//! Tests of `craft_testkit::fonts` on faces assembled here from bytes.
//!
//! The helpers below read the output by the offsets the OpenType specification gives, without the
//! crate's own parser, so a test fails when a field is wrong even if the crate could read it back.

use craft_testkit::fonts::{FontError, collection, rename_family};

const FROM: &str = "Source Sans 3";
const TO: &str = "Sysfont Sans3";
const CHECKSUM_TARGET: u32 = 0xB1B0_AFBA;

fn be16(data: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([data[at], data[at + 1]])
}

fn be32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn set16(data: &mut [u8], at: usize, value: u16) {
    data[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn set32(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

/// The table checksum: big-endian 32-bit words added with wrapping, the last padded with zeros.
fn sum(data: &[u8]) -> u32 {
    let mut total = 0u32;
    let mut at = 0;
    while at < data.len() {
        let mut word = [0u8; 4];
        for (k, byte) in word.iter_mut().enumerate() {
            if let Some(&b) = data.get(at + k) {
                *byte = b;
            }
        }
        total = total.wrapping_add(u32::from_be_bytes(word));
        at += 4;
    }
    total
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// A name record: platform, encoding, name ID, and where its string is in storage.
struct Rec {
    platform: u16,
    encoding: u16,
    name_id: u16,
    offset: u16,
    length: u16,
}

/// A `name` table of `version` with the records as given, language tags as (length, offset), then storage.
fn name_table(version: u16, records: &[Rec], lang_tags: &[(u16, u16)], storage: &[u8]) -> Vec<u8> {
    let mut header = 6 + 12 * records.len();
    if version == 1 {
        header += 2 + 4 * lang_tags.len();
    }
    let mut table = Vec::new();
    table.extend_from_slice(&version.to_be_bytes());
    table.extend_from_slice(&(records.len() as u16).to_be_bytes());
    table.extend_from_slice(&(header as u16).to_be_bytes());
    for rec in records {
        for value in [rec.platform, rec.encoding, 0, rec.name_id, rec.length, rec.offset] {
            table.extend_from_slice(&value.to_be_bytes());
        }
    }
    if version == 1 {
        table.extend_from_slice(&(lang_tags.len() as u16).to_be_bytes());
        for &(length, offset) in lang_tags {
            table.extend_from_slice(&length.to_be_bytes());
            table.extend_from_slice(&offset.to_be_bytes());
        }
    }
    table.extend_from_slice(storage);
    table
}

/// Name records with their strings stored one after another: (platform, encoding, name ID, bytes).
fn names(list: &[(u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut storage = Vec::new();
    let mut records = Vec::new();
    for (platform, encoding, name_id, bytes) in list {
        records.push(Rec { platform: *platform, encoding: *encoding, name_id: *name_id, offset: storage.len() as u16, length: bytes.len() as u16 });
        storage.extend_from_slice(bytes);
    }
    name_table(0, &records, &[], &storage)
}

/// A 54-byte `head` table whose `checkSumAdjustment` holds a stale value.
fn head() -> Vec<u8> {
    let mut table = vec![0u8; 54];
    set32(&mut table, 0, 0x0001_0000);
    set32(&mut table, 8, 0x1234_5678);
    set32(&mut table, 12, 0x5F0F_3CF5);
    set16(&mut table, 18, 1000);
    table
}

/// A standalone TrueType face holding `tables` in the order given, each on a four-byte boundary.
fn face(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    font.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    font.extend_from_slice(&[0; 6]);
    font.resize(12 + 16 * tables.len(), 0);
    for (index, (tag, data)) in tables.iter().enumerate() {
        let offset = font.len();
        font.extend_from_slice(data);
        font.resize(font.len().div_ceil(4) * 4, 0);
        let record = 12 + 16 * index;
        font[record..record + 4].copy_from_slice(&tag[..]);
        set32(&mut font, record + 4, sum(data));
        set32(&mut font, record + 8, offset as u32);
        set32(&mut font, record + 12, data.len() as u32);
    }
    font
}

fn family_face(list: &[(u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
    face(&[(b"head", head()), (b"name", names(list))])
}

/// A table of the face whose directory starts at `directory`: (record offset, table offset, length).
fn table_at(font: &[u8], directory: usize, tag: &[u8; 4]) -> (usize, usize, usize) {
    let count = usize::from(be16(font, directory + 4));
    (0..count)
        .map(|i| directory + 12 + 16 * i)
        .find(|&record| &font[record..record + 4] == tag)
        .map(|record| (record, be32(font, record + 8) as usize, be32(font, record + 12) as usize))
        .unwrap_or_else(|| panic!("no {} table", String::from_utf8_lossy(tag)))
}

fn table(font: &[u8], tag: &[u8; 4]) -> (usize, usize, usize) {
    table_at(font, 0, tag)
}

/// Every name record of a standalone face: (platform, encoding, name ID, bytes).
fn strings(font: &[u8]) -> Vec<(u16, u16, u16, Vec<u8>)> {
    let (_, at, _) = table(font, b"name");
    let count = usize::from(be16(font, at + 2));
    let storage = at + usize::from(be16(font, at + 4));
    (0..count)
        .map(|i| {
            let rec = at + 6 + 12 * i;
            let start = storage + usize::from(be16(font, rec + 10));
            let end = start + usize::from(be16(font, rec + 8));
            (be16(font, rec), be16(font, rec + 2), be16(font, rec + 6), font[start..end].to_vec())
        })
        .collect()
}

fn win(name_id: u16, text: &str) -> (u16, u16, u16, Vec<u8>) {
    (3, 1, name_id, utf16(text))
}

fn mac(name_id: u16, bytes: &[u8]) -> (u16, u16, u16, Vec<u8>) {
    (1, 0, name_id, bytes.to_vec())
}

#[test]
fn rename_handles_utf16_and_ascii() {
    let font = family_face(&[win(1, FROM), (0, 3, 1, utf16(FROM)), mac(1, FROM.as_bytes()), win(16, FROM), win(21, FROM)]);
    let out = rename_family(&font, FROM, TO).unwrap();
    assert_eq!(out.len(), font.len());
    assert_eq!(strings(&out), vec![win(1, TO), (0, 3, 1, utf16(TO)), mac(1, TO.as_bytes()), win(16, TO), win(21, TO)]);
}

#[test]
fn rename_only_family_records() {
    let list = [
        win(0, "Copyright 2026 the Source Sans 3 authors"),
        win(1, FROM),
        win(2, "Regular"),
        win(6, "SourceSans3-Regular"),
        win(13, "Source Sans 3 is licensed under the OFL"),
        win(16, "Source Sans 3 Extra"),
    ];
    let other = utf16(FROM);
    let font = face(&[(b"head", head()), (b"name", names(&list)), (b"zzzz", other.clone())]);
    let out = rename_family(&font, FROM, TO).unwrap();
    let mut want = list.to_vec();
    want[1] = win(1, TO);
    assert_eq!(strings(&out), want);
    let (_, at, length) = table(&out, b"zzzz");
    assert_eq!(&out[at..at + length], &other[..], "a table other than name keeps its bytes");
}

#[test]
fn rename_full_name_prefix_only() {
    let list =
        [win(1, FROM), win(4, "Source Sans 3 Bold"), (0, 3, 4, utf16("Source Sans 3Bold")), mac(4, b"My Source Sans 3"), mac(4, FROM.as_bytes())];
    let out = rename_family(&family_face(&list), FROM, TO).unwrap();
    assert_eq!(strings(&out), vec![win(1, TO), win(4, "Sysfont Sans3 Bold"), list[2].clone(), list[3].clone(), mac(4, TO.as_bytes())]);
}

#[test]
fn rename_rejects_non_ascii_mac_roman() {
    let font = family_face(&[win(1, FROM), mac(1, FROM.as_bytes())]);
    assert_eq!(rename_family(&font, FROM, "Sourcé Sans 3"), Err(FontError::UnsupportedEncoding));
    // "Café Sans 3" in Mac Roman, where é is 0x8E.
    let font = family_face(&[win(1, "Café Sans 3"), mac(1, b"Caf\x8E Sans 3")]);
    assert_eq!(rename_family(&font, "Café Sans 3", "Cafe Sans 3"), Err(FontError::UnsupportedEncoding));
    let font = family_face(&[win(1, "Café Sans 3"), mac(4, b"Caf\x8E Sans 3 Bold")]);
    assert_eq!(rename_family(&font, "Café Sans 3", "Cafe Sans 3"), Err(FontError::UnsupportedEncoding));
}

#[test]
fn rename_keeps_mac_roman_names_with_other_letters() {
    // Mac Roman 0x80 is "Ä" and 0x8F is "è", so neither Mac record holds "Café Sans 3".
    let list = [win(1, "Café Sans 3"), mac(1, b"Caf\x80 Sans 3"), mac(4, b"Caf\x8F Sans 3 Bold")];
    let out = rename_family(&family_face(&list), "Café Sans 3", "Cafe Sans 3").unwrap();
    assert_eq!(strings(&out), vec![win(1, "Cafe Sans 3"), list[1].clone(), list[2].clone()]);
}

#[test]
fn rename_keeps_unrelated_mac_roman_names() {
    // Mac Roman 0x8E is "é". Neither Mac record can hold "Café Sans 3": one is shorter, and the other
    // differs from it in an ASCII position.
    let list = [win(1, "Café Sans 3"), mac(4, b"Autre \x8E"), mac(16, b"Caf\x8E Sans 4")];
    let out = rename_family(&family_face(&list), "Café Sans 3", "Cafe Sans 3").unwrap();
    assert_eq!(strings(&out), vec![win(1, "Cafe Sans 3"), list[1].clone(), list[2].clone()]);
}

#[test]
fn rename_keeps_mac_roman_full_name_suffix() {
    let list = [win(1, FROM), mac(4, b"Source Sans 3 Cr\x8Ame")];
    let out = rename_family(&family_face(&list), FROM, TO).unwrap();
    assert_eq!(strings(&out), vec![win(1, TO), mac(4, b"Sysfont Sans3 Cr\x8Ame")]);
}

#[test]
fn rename_requires_a_family_record() {
    let font = family_face(&[win(1, "Other Family"), win(4, "Source Sans 3 Bold")]);
    assert_eq!(rename_family(&font, FROM, TO), Err(FontError::NameNotFound));
}

#[test]
fn rename_preserves_other_encodings() {
    let list = [win(1, FROM), (3, 3, 1, utf16(FROM)), (1, 1, 1, FROM.as_bytes().to_vec()), (2, 1, 1, utf16(FROM))];
    let out = rename_family(&family_face(&list), FROM, TO).unwrap();
    let mut want = list.to_vec();
    want[0] = win(1, TO);
    assert_eq!(strings(&out), want);
}

#[test]
fn rename_rejects_odd_utf16() {
    let mut odd = utf16(FROM);
    odd.push(0);
    let font = family_face(&[(3, 1, 1, odd)]);
    assert_eq!(rename_family(&font, FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_rejects_bad_surrogate() {
    let font = family_face(&[(3, 1, 1, vec![0xD8, 0x00, 0x00, b'A']), win(16, FROM)]);
    assert_eq!(rename_family(&font, FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_handles_shared_family_storage() {
    let storage = utf16("Source Sans 3 Bold");
    let records = [
        Rec { platform: 3, encoding: 1, name_id: 1, offset: 0, length: 26 },
        Rec { platform: 3, encoding: 1, name_id: 16, offset: 0, length: 26 },
        Rec { platform: 3, encoding: 1, name_id: 4, offset: 0, length: 36 },
    ];
    let font = face(&[(b"head", head()), (b"name", name_table(0, &records, &[], &storage))]);
    let out = rename_family(&font, FROM, TO).unwrap();
    assert_eq!(strings(&out), vec![win(1, TO), win(16, TO), win(4, "Sysfont Sans3 Bold")]);
}

#[test]
fn rename_rejects_shared_nonfamily_storage() {
    let records =
        [Rec { platform: 3, encoding: 1, name_id: 1, offset: 0, length: 26 }, Rec { platform: 3, encoding: 1, name_id: 6, offset: 2, length: 10 }];
    let font = face(&[(b"head", head()), (b"name", name_table(0, &records, &[], &utf16(FROM)))]);
    assert_eq!(rename_family(&font, FROM, TO), Err(FontError::ConflictingNameStorage));
}

#[test]
fn rename_keeps_a_style_sharing_the_full_name_suffix() {
    // The style name (ID 2) "Bold" is the last 8 bytes of the full name, which the rename does not change.
    let storage = [utf16(FROM), utf16("Source Sans 3 Bold")].concat();
    let records = [
        Rec { platform: 3, encoding: 1, name_id: 1, offset: 0, length: 26 },
        Rec { platform: 3, encoding: 1, name_id: 2, offset: 54, length: 8 },
        Rec { platform: 3, encoding: 1, name_id: 4, offset: 26, length: 36 },
    ];
    let font = face(&[(b"head", head()), (b"name", name_table(0, &records, &[], &storage))]);
    let out = rename_family(&font, FROM, TO).unwrap();
    assert_eq!(strings(&out), vec![win(1, TO), win(2, "Bold"), win(4, "Sysfont Sans3 Bold")]);
}

#[test]
fn rename_rejects_a_family_name_in_the_full_name_suffix() {
    // The full name "A A" is stored once, and the family name (ID 1) is its last "A". Renaming the
    // family would change the full name's suffix " A".
    let records =
        [Rec { platform: 3, encoding: 1, name_id: 4, offset: 0, length: 6 }, Rec { platform: 3, encoding: 1, name_id: 1, offset: 4, length: 2 }];
    let font = face(&[(b"head", head()), (b"name", name_table(0, &records, &[], &utf16("A A")))]);
    assert_eq!(rename_family(&font, "A", "B"), Err(FontError::ConflictingNameStorage));
}

#[test]
fn rename_updates_name_checksum() {
    let font = family_face(&[win(1, FROM), win(2, "Regular")]);
    let out = rename_family(&font, FROM, TO).unwrap();
    let (record, at, length) = table(&out, b"name");
    assert_eq!(be32(&out, record + 4), sum(&out[at..at + length]));
    assert_ne!(be32(&out, record + 4), be32(&font, record + 4), "the renamed table has a new checksum");
}

#[test]
fn rename_updates_head_adjustment() {
    let font = family_face(&[win(1, FROM)]);
    let out = rename_family(&font, FROM, TO).unwrap();
    let (_, at, _) = table(&out, b"head");
    let mut zeroed = out.clone();
    set32(&mut zeroed, at + 8, 0);
    assert_eq!(be32(&out, at + 8), CHECKSUM_TARGET.wrapping_sub(sum(&zeroed)));
    assert_eq!(sum(&out), CHECKSUM_TARGET);
}

#[test]
fn rename_format_one_preserves_language_tags() {
    let mut storage = utf16(FROM);
    storage.extend(utf16("en-US"));
    let records = [Rec { platform: 3, encoding: 1, name_id: 1, offset: 0, length: 26 }];
    let font = face(&[(b"head", head()), (b"name", name_table(1, &records, &[(10, 26)], &storage))]);
    let out = rename_family(&font, FROM, TO).unwrap();
    assert_eq!(strings(&out), vec![win(1, TO)]);
    let (_, at, length) = table(&out, b"name");
    assert_eq!(&out[at + length - 10..at + length], &utf16("en-US")[..]);
    assert_eq!(be16(&out, at + 18), 1, "the language-tag count stays");
}

#[test]
fn rename_keeps_language_tag_storage() {
    let records = [Rec { platform: 3, encoding: 1, name_id: 1, offset: 0, length: 26 }];
    let font = face(&[(b"head", head()), (b"name", name_table(1, &records, &[(26, 0)], &utf16(FROM)))]);
    assert_eq!(rename_family(&font, FROM, TO), Err(FontError::ConflictingNameStorage));
}

#[test]
fn rename_rejects_short_sfnt() {
    assert_eq!(rename_family(&[], FROM, TO), Err(FontError::MalformedFont));
    assert_eq!(rename_family(&[0, 1, 0, 0, 0], FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_rejects_truncated_directory() {
    let font = family_face(&[win(1, FROM)]);
    assert_eq!(rename_family(&font[..12 + 16 + 8], FROM, TO), Err(FontError::MalformedFont));
    let mut many = font.clone();
    set16(&mut many, 4, 4000);
    assert_eq!(rename_family(&many, FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_rejects_collection() {
    let font = family_face(&[win(1, FROM)]);
    let ttc = collection(&[&font]).unwrap();
    assert_eq!(rename_family(&ttc, FROM, TO), Err(FontError::UnsupportedFace));
}

#[test]
fn rename_rejects_missing_name_table() {
    let font = face(&[(b"head", head())]);
    assert_eq!(rename_family(&font, FROM, TO), Err(FontError::MissingNameTable));
}

#[test]
fn rename_rejects_short_head() {
    let font = face(&[(b"head", head()[..8].to_vec()), (b"name", names(&[win(1, FROM)]))]);
    assert_eq!(rename_family(&font, FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_rejects_bad_name_storage() {
    let font = family_face(&[win(1, FROM)]);
    let (_, at, length) = table(&font, b"name");
    let mut beyond = font.clone();
    set16(&mut beyond, at + 4, length as u16 + 1);
    assert_eq!(rename_family(&beyond, FROM, TO), Err(FontError::MalformedFont));
    let mut inside_header = font.clone();
    set16(&mut inside_header, at + 4, 6);
    assert_eq!(rename_family(&inside_header, FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_rejects_bad_name_record_range() {
    let font = family_face(&[win(1, FROM)]);
    let (_, at, _) = table(&font, b"name");
    let mut long = font.clone();
    set16(&mut long, at + 6 + 8, 28);
    assert_eq!(rename_family(&long, FROM, TO), Err(FontError::MalformedFont));
    let mut late = font;
    set16(&mut late, at + 6 + 10, 2);
    assert_eq!(rename_family(&late, FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_rejects_name_table_sharing_bytes() {
    let font = face(&[(b"head", head()), (b"name", names(&[win(1, FROM)])), (b"zzzz", vec![7; 8])]);
    let (_, at, _) = table(&font, b"name");
    let (record, _, _) = table(&font, b"zzzz");
    let mut shared = font.clone();
    set32(&mut shared, record + 8, (at + 18) as u32);
    assert_eq!(rename_family(&shared, FROM, TO), Err(FontError::MalformedFont));
    let (_, head_at, _) = table(&font, b"head");
    let mut under_head = font;
    set32(&mut under_head, record + 8, (head_at + 8) as u32);
    assert_eq!(rename_family(&under_head, FROM, TO), Err(FontError::MalformedFont), "checkSumAdjustment would land in zzzz");
}

#[test]
fn rename_rejects_unknown_name_format() {
    let font = family_face(&[win(1, FROM)]);
    let (_, at, _) = table(&font, b"name");
    let mut two = font;
    set16(&mut two, at, 2);
    assert_eq!(rename_family(&two, FROM, TO), Err(FontError::MalformedFont));
}

#[test]
fn rename_rejects_unequal_lengths() {
    let font = family_face(&[win(1, FROM)]);
    assert_eq!(rename_family(&font, FROM, "Other"), Err(FontError::UnequalLength));
}

#[test]
fn rename_rejects_unequal_utf16_lengths() {
    // Both names are four bytes of UTF-8, but "𝐀" is two UTF-16 code units and "abcd" is four.
    let font = family_face(&[win(1, "𝐀")]);
    assert_eq!(rename_family(&font, "𝐀", "abcd"), Err(FontError::UnequalLength));
}

#[test]
fn rename_rejects_missing_family() {
    let font = family_face(&[win(1, FROM), win(2, "Regular")]);
    assert_eq!(rename_family(&font, "Source Sans 4", TO), Err(FontError::NameNotFound));
}

#[test]
fn rename_rejects_empty_names() {
    let font = family_face(&[win(1, FROM)]);
    assert_eq!(rename_family(&font, "", ""), Err(FontError::InvalidName));
    assert_eq!(rename_family(&font, FROM, ""), Err(FontError::InvalidName));
}

/// Two faces that differ in every table, with table lengths that are not multiples of four.
fn two_faces() -> (Vec<u8>, Vec<u8>) {
    let a = face(&[(b"head", head()), (b"name", names(&[mac(1, b"Alpha")])), (b"zzzz", vec![1, 2, 3, 4, 5])]);
    let mut other_head = head();
    set32(&mut other_head, 4, 0x0002_0000);
    let b = face(&[(b"head", other_head), (b"name", names(&[mac(1, b"Beta")])), (b"zzzz", vec![9; 7])]);
    (a, b)
}

/// Each face's directory offset, read from the TTC header.
fn ttc_faces(ttc: &[u8]) -> Vec<usize> {
    assert_eq!(&ttc[..4], b"ttcf");
    assert_eq!(be32(ttc, 4), 0x0001_0000);
    (0..be32(ttc, 8) as usize).map(|i| be32(ttc, 12 + 4 * i) as usize).collect()
}

/// Every table of a face in a collection: (tag, checksum, offset, length).
fn ttc_tables(ttc: &[u8], directory: usize) -> Vec<([u8; 4], u32, usize, usize)> {
    (0..usize::from(be16(ttc, directory + 4)))
        .map(|i| {
            let r = directory + 12 + 16 * i;
            (ttc[r..r + 4].try_into().unwrap(), be32(ttc, r + 4), be32(ttc, r + 8) as usize, be32(ttc, r + 12) as usize)
        })
        .collect()
}

#[test]
fn collection_offsets_are_file_relative() {
    let (a, b) = two_faces();
    let ttc = collection(&[&a, &b]).unwrap();
    let faces = ttc_faces(&ttc);
    assert_eq!(faces.len(), 2);
    for (directory, input) in faces.into_iter().zip([&a, &b]) {
        assert_eq!(&ttc[directory..directory + 12], &input[..12], "the sfnt header is copied");
        for (tag, _, offset, length) in ttc_tables(&ttc, directory) {
            let (_, from, from_length) = table(input, &tag);
            assert_eq!(length, from_length);
            let mut want = input[from..from + from_length].to_vec();
            if &tag == b"head" {
                set32(&mut want, 8, 0);
            }
            assert_eq!(&ttc[offset..offset + length], &want[..], "{} sits at its file offset", String::from_utf8_lossy(&tag));
        }
    }
}

#[test]
fn collection_pads_odd_faces() {
    let (a, b) = two_faces();
    let ttc = collection(&[&a, &b]).unwrap();
    assert_eq!(ttc.len() % 4, 0);
    for directory in ttc_faces(&ttc) {
        assert_eq!(directory % 4, 0);
        for (_, _, offset, length) in ttc_tables(&ttc, directory) {
            assert_eq!(offset % 4, 0, "tables start on four-byte boundaries");
            let padded = length.div_ceil(4) * 4;
            assert!(ttc[offset + length..offset + padded].iter().all(|&b| b == 0), "padding is zero");
        }
    }
    let lengths: Vec<usize> = ttc_faces(&ttc).into_iter().flat_map(|d| ttc_tables(&ttc, d)).map(|t| t.3).collect();
    assert!(lengths.iter().any(|length| length % 4 != 0), "the fixture has unpadded lengths");
}

#[test]
fn collection_checksums_match_tables() {
    let (a, b) = two_faces();
    let ttc = collection(&[&a, &b]).unwrap();
    for directory in ttc_faces(&ttc) {
        for (tag, checksum, offset, length) in ttc_tables(&ttc, directory) {
            assert_eq!(checksum, sum(&ttc[offset..offset + length]));
            if &tag == b"head" {
                assert_eq!(be32(&ttc, offset + 8), 0, "checkSumAdjustment is zero in a collection");
            }
        }
    }
}

#[test]
fn collection_copies_tables() {
    let (a, _) = two_faces();
    let ttc = collection(&[&a, &a]).unwrap();
    let mut ranges: Vec<(usize, usize)> = ttc_faces(&ttc).into_iter().flat_map(|d| ttc_tables(&ttc, d)).map(|t| (t.2, t.2 + t.3)).collect();
    assert_eq!(ranges.len(), 6);
    ranges.sort();
    assert!(ranges.windows(2).all(|pair| pair[0].1 <= pair[1].0), "each face has its own tables");
}

#[test]
fn collection_rejects_empty() {
    assert_eq!(collection(&[]), Err(FontError::EmptyCollection));
}

#[test]
fn collection_rejects_nested_ttc() {
    let (a, _) = two_faces();
    let ttc = collection(&[&a]).unwrap();
    assert_eq!(collection(&[&a, &ttc]), Err(FontError::UnsupportedFace));
}

#[test]
fn collection_rejects_short_directory() {
    let (a, _) = two_faces();
    assert_eq!(collection(&[&a[..3]]), Err(FontError::MalformedFont));
    assert_eq!(collection(&[&a[..12 + 16 * 2]]), Err(FontError::MalformedFont));
    let mut many = a.clone();
    set16(&mut many, 4, 400);
    assert_eq!(collection(&[&a, &many]), Err(FontError::MalformedFont));
}

#[test]
fn collection_rejects_out_of_range_table() {
    let (a, _) = two_faces();
    let (record, _, _) = table(&a, b"zzzz");
    let mut late = a.clone();
    set32(&mut late, record + 8, a.len() as u32);
    assert_eq!(collection(&[&late]), Err(FontError::MalformedFont));
    let mut long = a.clone();
    set32(&mut long, record + 12, u32::MAX);
    assert_eq!(collection(&[&long]), Err(FontError::MalformedFont));
}

#[test]
fn collection_rejects_short_head() {
    let font = face(&[(b"head", head()[..8].to_vec())]);
    assert_eq!(collection(&[&font]), Err(FontError::MalformedFont));
}

#[test]
fn collection_rejects_offset_overflow() {
    // 4,096 references to one face with a 1 MiB table add up to more than 4 GiB of output, which
    // 32-bit offsets cannot address. The function refuses before allocating any of it.
    let big = face(&[(b"zzzz", vec![0; 1 << 20])]);
    let faces = vec![&big[..]; 4096];
    assert_eq!(collection(&faces), Err(FontError::OutputTooLarge));
}

#[test]
fn collection_rejects_directories_past_the_offset_limit() {
    // 65,536 references to one face with 4,096 empty tables: the table directories alone take more
    // than 4 GiB. The function refuses without keeping a directory for every face.
    let tables: Vec<(&[u8; 4], Vec<u8>)> = (0..4096).map(|_| (b"zzzz", Vec::new())).collect();
    let many = face(&tables);
    let faces = vec![&many[..]; 65536];
    assert_eq!(collection(&faces), Err(FontError::OutputTooLarge));
}

#[test]
fn collection_counts_directories_before_reading_records() {
    // The same number of directories, each with every table record out of range. The size limit is
    // found from the faces' table counts before any table record is read.
    let mut directory = vec![0u8; 12 + 16 * 4096];
    set32(&mut directory, 0, 0x0001_0000);
    set16(&mut directory, 4, 4096);
    for index in 0..4096 {
        set32(&mut directory, 12 + 16 * index + 8, u32::MAX);
    }
    let faces = vec![&directory[..]; 65536];
    assert_eq!(collection(&faces), Err(FontError::OutputTooLarge));
}
