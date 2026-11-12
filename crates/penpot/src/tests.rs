//! The archive, its manifest, members and budgets, on archives built in code.
#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::json;
use std::io::{Cursor, Write};

use zip::ZipArchive;
use zip::write::SimpleFileOptions;

use super::*;

const FILE: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
const SHAPE: &str = "cccccccc-cccc-cccc-cccc-cccccccccccc";
const MEDIA: &str = "dddddddd-dddd-dddd-dddd-dddddddddddd";
const OBJECT: &str = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";

fn zip_of(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in members {
        z.start_file(name.as_str(), SimpleFileOptions::default()).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn good_manifest() -> (String, Vec<u8>) {
    ("manifest.json".into(), json!({"type": MANIFEST_TYPE, "version": 1, "files": [{"id": FILE, "name": "Advanced"}]}).to_string().into_bytes())
}

fn member(path: String, value: Value) -> (String, Vec<u8>) {
    (path, value.to_string().into_bytes())
}

/// One file with one page: the root frame, one rectangle, a component, a colour, an image.
fn members() -> Vec<(String, Vec<u8>)> {
    vec![
        good_manifest(),
        member(format!("files/{FILE}.json"), json!({"features": ["components/v2"], "version": 67})),
        member(format!("files/{FILE}/pages/{PAGE}.json"), json!({"id": PAGE, "name": "Canvas", "index": 0})),
        member(format!("files/{FILE}/pages/{PAGE}/{ROOT_ID}.json"), json!({"id": ROOT_ID, "type": "frame", "shapes": [SHAPE]})),
        member(format!("files/{FILE}/pages/{PAGE}/{SHAPE}.json"), json!({"id": SHAPE, "type": "rect", "parentId": ROOT_ID})),
        member(format!("files/{FILE}/components/{SHAPE}.json"), json!({"id": SHAPE, "name": "Button"})),
        member(format!("files/{FILE}/colors/{SHAPE}.json"), json!({"id": SHAPE, "color": "#ff0000"})),
        member(format!("files/{FILE}/tokens.json"), json!({"$themes": []})),
        member(format!("files/{FILE}/media/{MEDIA}.json"), json!({"id": MEDIA, "mediaId": OBJECT, "mtype": "image/png"})),
        member(format!("objects/{OBJECT}.json"), json!({"id": OBJECT, "contentType": "image/png"})),
        (format!("objects/{OBJECT}.png"), b"\x89PNG-bytes".to_vec()),
    ]
}

fn open_err(members: &[(String, Vec<u8>)]) -> String {
    Archive::open(&zip_of(members)).err().map(|e| e.to_string()).unwrap_or_default()
}

#[test]
fn an_archive_opens_and_hands_out_its_members() {
    let bytes = zip_of(&members());
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(a.files, [FileEntry { id: FILE.into(), name: Some("Advanced".into()) }]);
    assert_eq!(a.manifest["type"], MANIFEST_TYPE);
    assert_eq!(a.json(&format!("files/{FILE}.json")).unwrap()["version"], 67);
    let pages = a.pages(FILE).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["name"], "Canvas");
    let shapes = a.page_shapes(FILE, PAGE).unwrap();
    assert_eq!(shapes.keys().map(String::as_str).collect::<Vec<_>>(), [ROOT_ID, SHAPE]);
    assert_eq!(shapes[ROOT_ID]["shapes"][0], SHAPE);
    assert_eq!(a.uuid_members(&format!("files/{FILE}/components"), 10).unwrap(), [format!("files/{FILE}/components/{SHAPE}.json")]);
    assert!(a.has(&format!("files/{FILE}/tokens.json")) && !a.has("files/tokens.json"));
    assert_eq!(a.media(FILE, MEDIA, 1 << 20).as_deref(), Some(&b"\x89PNG-bytes"[..]));
    assert_eq!(a.names().len(), 11);
    assert_eq!(a.embedded_fonts(), 0);
    assert!(a.check().is_ok());
    assert!(a.bytes_left() < MAX_DECOMPRESSED);
}

#[test]
fn a_written_archive_reads_back() {
    let all = members();
    let manifest = serde_json::to_vec(&manifest("craft-penpot/test", &[(FILE, "Advanced", json!(["components/v2"]))])).unwrap();
    let mut list: Vec<(&str, &[u8])> = vec![("manifest.json", &manifest)];
    list.extend(all.iter().skip(1).map(|(n, b)| (n.as_str(), b.as_slice())));
    let bytes = write_archive(list).unwrap();
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(
        a.manifest,
        json!({"type": "penpot/export-files", "version": 1, "generatedBy": "craft-penpot/test", "refer": "penpot",
        "files": [{"id": FILE, "name": "Advanced", "features": ["components/v2"]}], "relations": []})
    );
    assert_eq!(a.page_shapes(FILE, PAGE).unwrap().len(), 2);
    assert_eq!(a.names().first().map(String::as_str), Some("manifest.json"));
    // What the reader would refuse is not written.
    for bad in ["../x", "/abs", "a/./b", "a\\b", "", "a\u{1}b"] {
        assert!(write_archive([(bad, &b"x"[..])]).is_err(), "{bad:?}");
    }
    assert!(write_archive([("a", &b"x"[..]), ("a", &b"y"[..])]).is_err());
}

#[test]
fn the_manifest_is_checked() {
    let with = |manifest: Value| open_err(&[member("manifest.json".into(), manifest)]);
    assert!(open_err(&[("other.json".into(), b"{}".to_vec())]).contains("manifest.json"));
    assert!(with(json!({"type": "something/else", "version": 1, "files": [{"id": FILE}]})).contains("not a Penpot export-files archive"));
    assert!(with(json!({"type": MANIFEST_TYPE, "version": 2, "files": [{"id": FILE}]})).contains("unsupported Penpot export version"));
    assert!(with(json!({"type": MANIFEST_TYPE, "version": 1})).contains("no files in manifest"));
    assert!(with(json!({"type": MANIFEST_TYPE, "version": 1, "files": []})).contains("invalid number of Penpot files"));
    let many: Vec<Value> = (0..=MAX_FILES).map(|_| json!({"id": FILE})).collect();
    assert!(with(json!({"type": MANIFEST_TYPE, "version": 1, "files": many})).contains("invalid number of Penpot files"));
    assert!(with(json!({"type": MANIFEST_TYPE, "version": 1, "files": [{"id": "../../etc"}]})).contains("invalid file UUID"));
    assert!(with(json!({"type": MANIFEST_TYPE, "version": 1, "files": [{"name": "No id"}]})).contains("invalid file UUID"));
    // Not a ZIP at all, and an empty one.
    assert!(Archive::open(b"not a zip").is_err());
    assert!(Archive::open(b"").is_err());
}

#[test]
fn ids_are_uuids() {
    assert!(is_uuid(ROOT_ID) && is_uuid(FILE) && is_uuid("0123abcd-0123-ABCD-0123-0123456789ab"));
    for bad in [
        "",
        "aaaaaaaa",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaaa",
        "aaaaaaaa/aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        "gaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    ] {
        assert!(!is_uuid(bad), "{bad}");
    }
}

#[test]
fn unsafe_and_duplicate_member_names_are_refused() {
    for bad in ["../evil.json", "files/../x.json", "a/./b.json", "back\\slash.json", "ctrl\u{7}.json"] {
        let e = open_err(&[good_manifest(), (bad.into(), b"{}".to_vec())]);
        assert!(e.contains("unsafe ZIP member path"), "{bad}: {e}");
    }
    // Two members of one name: written under two names of the same length, then renamed in the
    // local header and in the directory.
    let mut bytes = zip_of(&[good_manifest(), ("a.json".into(), b"{}".to_vec()), ("b.json".into(), b"[]".to_vec())]);
    let mut renamed = 0;
    for i in 0..bytes.len().saturating_sub(6) {
        if &bytes[i..i + 6] == b"b.json" {
            bytes[i] = b'a';
            renamed += 1;
        }
    }
    assert_eq!(renamed, 2);
    assert_eq!(Archive::open(&bytes).err(), Some(Error::Corrupt("duplicate ZIP member names".into())));
}

#[test]
fn an_archive_declaring_more_than_it_may_unpack_is_refused_before_reading() {
    let mut bytes = zip_of(&[good_manifest(), ("big.bin".into(), vec![0; 64])]);
    // The directory record of big.bin: its uncompressed size, at offset 24.
    let start = {
        let mut z = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        z.by_name("big.bin").unwrap().central_header_start() as usize
    };
    bytes[start + 24..start + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    let e = Archive::open(&bytes).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("ZIP contents exceed 1 GiB") || e.contains("invalid ZIP"), "{e}");
}

#[test]
fn too_many_members_are_refused() {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..=MAX_ENTRIES {
        z.start_file(format!("{i}"), stored).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    assert!(bytes.len() < MAX_ARCHIVE);
    assert_eq!(Archive::open(&bytes).err(), Some(Error::Corrupt("too many ZIP members".into())));
}

#[test]
fn an_archive_with_a_zip64_directory_opens() {
    // More members than the classic end record can count: the ZIP64 record's count is compared.
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let (name, data) = good_manifest();
    z.start_file(name, stored).unwrap();
    z.write_all(&data).unwrap();
    for i in 0..70_000 {
        z.start_file(format!("{i}"), stored).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    assert_eq!(Archive::open(&bytes).unwrap().names().len(), 70_001);
}

#[test]
fn the_byte_budget_counts_bytes_read_even_when_headers_claim_less() {
    // b.json's headers say one byte; it holds five. The read stops past the claim and is charged it.
    let mut bytes = zip_of(&[good_manifest(), ("a.json".into(), b"[0,1]".to_vec()), ("b.json".into(), b"[2,3]".to_vec())]);
    let start = {
        let mut z = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        z.by_name("b.json").unwrap().central_header_start() as usize
    };
    bytes[start + 24..start + 28].copy_from_slice(&1u32.to_le_bytes());
    let mut a = Archive::open(&bytes).unwrap();
    let before = a.bytes_left();
    assert!(a.json("b.json").is_err());
    assert_eq!(before - a.bytes_left(), 1);
    assert!(a.check().is_ok(), "a lying member is an error of its own, not the budget's");
    // Two members share one limit; ending exactly at it is not an overrun.
    let left = a.bytes_left();
    a.zip.budget_mut().charge(left - 5).unwrap();
    assert_eq!(a.json("a.json").unwrap(), json!([0, 1]));
    assert!(a.check().is_ok());
    assert!(a.json("a.json").is_err(), "bytes beyond the combined budget must be rejected");
    assert!(a.check().is_err());
}

#[test]
fn an_exhausted_byte_budget_fails_the_archive_even_for_optional_members() {
    let bytes = zip_of(&[good_manifest(), ("a.json".into(), b"[0,0]".to_vec())]);
    let mut a = Archive::open(&bytes).unwrap();
    let left = a.bytes_left();
    a.zip.budget_mut().charge(left - 3).unwrap();
    assert!(a.json("a.json").is_err());
    assert_eq!(a.check().err(), Some(Error::Corrupt("actual decompressed ZIP contents exceed 1 GiB".into())));
}

#[test]
fn the_json_value_budget_is_shared_by_shape_and_other_members() {
    let bytes = zip_of(&[good_manifest(), ("tokens.json".into(), b"[0,0]".to_vec()), ("shape.json".into(), b"[0,0]".to_vec())]);
    let mut a = Archive::open(&bytes).unwrap();
    a.budget.json_values = 6;
    let mut page_bytes = MAX_PAGE_SHAPES_BYTES;
    assert_eq!(a.json("tokens.json").unwrap(), json!([0, 0]));
    assert_eq!(a.shape_json("shape.json", &mut page_bytes).unwrap(), json!([0, 0]));
    assert_eq!(a.budget.json_values, 0);
    assert!(a.check().is_ok(), "an exact-budget read is allowed");
    assert!(a.json("tokens.json").is_err());
    assert!(a.check().is_err(), "optional-member callers cannot swallow exhaustion");
}

fn budget_of(json_values: usize) -> Budget {
    Budget { exceeded: false, json_values, json_exceeded: false }
}

#[test]
fn the_json_value_budget_rejects_dense_input_before_deserialization() {
    // The malformed tail would cause a serde syntax error after allocating the array.
    // Admission must reject it for structure first, while the input is still only bytes.
    let dense = "[".to_owned() + &"0,".repeat(10_000);
    let mut budget = budget_of(100);
    let err = budget.parse_json(dense.as_bytes(), "shape.json").unwrap_err();
    assert!(err.to_string().contains("value budget"), "{err}");
    assert!(budget.check().is_err());
}

#[test]
fn the_json_value_budget_handles_strings_escapes_and_empty_input() {
    for text in ["[{,:", "\\", "\\\"[{,:", "é"] {
        let value = Value::String(text.into());
        let mut budget = budget_of(1);
        assert_eq!(budget.parse_json(&serde_json::to_vec(&value).unwrap(), "shape.json").unwrap(), value);
        assert!(budget.check().is_ok());
    }
    // An escaped backslash before a closing quote must not hide later containers.
    let mut budget = budget_of(3);
    assert!(budget.parse_json(br#"["\\",[0,0]]"#, "shape.json").is_err());
    assert!(budget.check().is_err());
    let mut budget = budget_of(0);
    assert!(budget.parse_json(b"", "shape.json").is_err());
    assert!(budget.check().is_err());
}

#[test]
fn a_dense_shape_within_the_zip_byte_limits_is_refused() {
    let dense = format!(r#"{{"id":"{SHAPE}","type":"rect","unused":[{}0]}}"#, "0,".repeat(MAX_JSON_VALUES));
    assert!(dense.len() < MAX_JSON as usize);
    let mut all = members();
    all[4].1 = dense.into_bytes();
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in &all {
        z.start_file(name.as_str(), SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        z.write_all(data).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    assert!(bytes.len() < 1 << 20);
    let mut a = Archive::open(&bytes).unwrap();
    let err = a.page_shapes(FILE, PAGE).unwrap_err();
    assert!(err.to_string().contains("value budget"), "{err}");
}

#[test]
fn the_page_shape_budget_rejects_shapes_past_its_bytes() {
    let bytes = zip_of(&[good_manifest(), ("shape.json".into(), br#"{"id":"00000000-0000-0000-0000-000000000001","type":"rect"}"#.to_vec())]);
    let mut a = Archive::open(&bytes).unwrap();
    let mut page_budget = 10u64; // Too small for the shape JSON (about 60 bytes).
    assert_eq!(a.shape_json("shape.json", &mut page_budget).err(), Some(Error::Corrupt("page shapes exceed byte budget".into())));
    let mut page_budget = 1000u64;
    assert!(a.shape_json("shape.json", &mut page_budget).is_ok());
    assert!(page_budget < 1000);
}

#[test]
fn a_json_member_past_its_size_is_refused() {
    let big = format!("\"{}\"", "x".repeat(MAX_JSON as usize));
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in
        [good_manifest(), ("big.json".into(), big.into_bytes()), ("fits.json".into(), format!("\"{}\"", "x".repeat(1000)).into_bytes())]
    {
        z.start_file(name.as_str(), deflated).unwrap();
        z.write_all(&data).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    let mut a = Archive::open(&bytes).unwrap();
    let left = a.bytes_left();
    assert_eq!(a.json("big.json").err(), Some(Error::Corrupt("JSON member too large: big.json".into())));
    assert_eq!(a.bytes_left(), left, "refused by its header, before unpacking");
    let mut page_budget = MAX_PAGE_SHAPES_BYTES;
    assert!(a.shape_json("big.json", &mut page_budget).is_err());
    assert!(a.json("fits.json").is_ok());
    assert!(a.json("missing.json").is_err());
}

#[test]
fn pages_and_shapes_past_their_counts_are_refused() {
    let uuid = |i: usize| format!("{i:08x}-0000-0000-0000-000000000000");
    let mut all = vec![good_manifest()];
    all.extend((0..=MAX_PAGES).map(|i| member(format!("files/{FILE}/pages/{}.json", uuid(i)), json!({"id": uuid(i), "index": MAX_PAGES - i}))));
    let bytes = zip_of(&all);
    assert_eq!(Archive::open(&bytes).unwrap().pages(FILE).err(), Some(Error::Corrupt("too many pages".into())));
    all.pop();
    let bytes = zip_of(&all);
    let pages = Archive::open(&bytes).unwrap().pages(FILE).unwrap();
    assert_eq!(pages.len(), MAX_PAGES);
    // Sorted by index, whatever the archive order.
    assert_eq!(pages[0]["index"], 1);
    assert_eq!(pages[0]["id"], uuid(MAX_PAGES - 1));

    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let (name, data) = good_manifest();
    z.start_file(name, stored).unwrap();
    z.write_all(&data).unwrap();
    for i in 0..=MAX_PAGE_SHAPES {
        z.start_file(format!("files/{FILE}/pages/{PAGE}/{}.json", uuid(i)), stored).unwrap();
        z.write_all(b"{}").unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(a.page_shapes(FILE, PAGE).err(), Some(Error::Corrupt("too many layers on a page".into())));
    // A caller that truncates instead gets the first `max`.
    assert_eq!(a.uuid_members(&format!("files/{FILE}/pages/{PAGE}"), 3).unwrap_err().len(), 3);
}

#[test]
fn only_uuid_json_members_of_the_directory_are_listed() {
    let all = vec![
        good_manifest(),
        member(format!("files/{FILE}/colors/{SHAPE}.json"), json!({})),
        member(format!("files/{FILE}/colors/not-a-uuid.json"), json!({})),
        member(format!("files/{FILE}/colors/{SHAPE}.txt"), json!({})),
        member(format!("files/{FILE}/colors/{SHAPE}/{SHAPE}.json"), json!({})),
        member(format!("files/{FILE}/colorsX/{SHAPE}.json"), json!({})),
    ];
    let bytes = zip_of(&all);
    let a = Archive::open(&bytes).unwrap();
    assert_eq!(a.uuid_members(&format!("files/{FILE}/colors"), 10).unwrap(), [format!("files/{FILE}/colors/{SHAPE}.json")]);
}

#[test]
fn media_is_read_through_its_record_within_the_callers_budget() {
    let bytes = zip_of(&members());
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(a.media(FILE, MEDIA, 10).map(|b| b.len()), Some(10));
    assert_eq!(a.media(FILE, MEDIA, 9), None, "larger than the caller allows");
    assert_eq!(a.media(FILE, SHAPE, 100), None, "no such media record");
    assert_eq!(a.media("../..", MEDIA, 100), None);
    assert_eq!(a.media(FILE, "../../manifest", 100), None);
    // A record naming an object that isn't a UUID, or a type whose file isn't there.
    let mut all = members();
    all[8] = member(format!("files/{FILE}/media/{MEDIA}.json"), json!({"mediaId": "../manifest", "mtype": "image/png"}));
    assert_eq!(Archive::open(&zip_of(&all)).unwrap().media(FILE, MEDIA, 100), None);
    all[8] = member(format!("files/{FILE}/media/{MEDIA}.json"), json!({"mediaId": OBJECT, "mtype": "image/jpeg"}));
    assert_eq!(Archive::open(&zip_of(&all)).unwrap().media(FILE, MEDIA, 100), None);
}

#[test]
fn fonts_are_counted_by_signature_without_reading_their_payload() {
    let mut font = b"wOF2".to_vec();
    font.resize(100_000, 0);
    let bytes = zip_of(&[
        good_manifest(),
        (format!("objects/{OBJECT}"), font),
        ("fonts/a.otf".into(), b"OTTOrest".to_vec()),
        ("assets/fonts/b.ttf".into(), vec![0, 1, 0, 0, 9, 9]),
        ("objects/short".into(), b"wO".to_vec()),
        (format!("objects/{OBJECT}.png"), b"\x89PNG".to_vec()),
        ("elsewhere/c.ttf".into(), b"true".to_vec()),
    ]);
    let mut a = Archive::open(&bytes).unwrap();
    let before = a.bytes_left();
    assert_eq!(a.embedded_fonts(), 3);
    // Four bytes of each of the five candidates (two of the short one).
    assert_eq!(before - a.bytes_left(), 4 * 4 + 2);
}

#[test]
fn mangled_archives_never_panic() {
    let bytes = zip_of(&members());
    let read = |bytes: &[u8]| {
        if let Ok(mut a) = Archive::open(bytes) {
            let _ = a.pages(FILE);
            let _ = a.page_shapes(FILE, PAGE);
            let _ = a.media(FILE, MEDIA, 1 << 20);
            let _ = a.embedded_fonts();
            let _ = a.check();
        }
    };
    for cut in (0..bytes.len()).step_by(5) {
        read(&bytes[..cut]);
    }
    let mut seed = 0x2545_f491_u32;
    for _ in 0..400 {
        let mut b = bytes.clone();
        for _ in 0..6 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let i = seed as usize % b.len();
            b[i] = (seed >> 8) as u8;
        }
        read(&b);
    }
}

/// A ZIP64 end record can declare millions of members in a small file: refused on the raw bytes
/// before the archive is indexed.
#[test]
fn an_archive_declaring_too_many_members_is_refused_before_it_is_indexed() {
    let bomb = craft_zip::zip64_entry_count_sample(MAX_ENTRIES as u64 + 1);
    assert_eq!(Archive::open(&bomb).err(), Some(Error::Corrupt("too many ZIP members".into())));
}
