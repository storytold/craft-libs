#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde::{Deserialize, Serialize};

use super::*;

/// A stand-in for an app's presentation: plain numbers, an optional number and a tagged enum
/// (whose `null`s serde reports only at the end of the enclosing object).
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Deck {
    #[serde(default)]
    width: f64,
    #[serde(default)]
    zoom: Option<f64>,
    #[serde(default)]
    shapes: Vec<Shape>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum Shape {
    Box { x: f64, y: f64 },
    Line { len: f64 },
}

fn package(members: &[Member<'_>]) -> Vec<u8> {
    write(&Deck { width: 960.0, ..Default::default() }, "Test 1", members).unwrap()
}

#[test]
fn a_package_holds_mimetype_then_manifest_then_members_in_order() {
    let png = [0x89, b'P', b'N', b'G'];
    let svg = b"<svg/>".repeat(20);
    let bytes = package(&[
        Member { name: &media_entry(7), content_type: "image/png", bytes: &png },
        Member { name: &media_entry(8), content_type: "image/svg+xml", bytes: &svg },
        Member { name: &chart_part_entry(0), content_type: "application/zip", bytes: b"PK" },
    ]);
    assert!(sniff(&bytes));
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    assert_eq!(z.len(), 5);
    let methods: Vec<_> = (0..z.len())
        .map(|i| {
            let f = z.by_index(i).unwrap();
            (f.name().unwrap().into_owned(), f.compression())
        })
        .collect();
    use zip::CompressionMethod::{Deflated, Stored};
    assert_eq!(
        methods,
        [("mimetype", Stored), ("presentation.json", Deflated), ("media/7", Stored), ("media/8", Deflated), ("chart-parts/0", Stored)]
            .map(|(n, m)| (n.to_string(), m))
    );
    // The mimetype is the first, stored member, readable in place.
    assert_eq!(craft_zip::stored_mimetype(&bytes), Some(MIMETYPE.as_bytes()));
    let (m, mut p) = Package::open_manifest::<Deck>(&bytes, MAX_MANIFEST).unwrap();
    assert_eq!((m.format.as_str(), m.version, m.generator.as_str(), m.presentation.width), (FORMAT, VERSION, "Test 1", 960.0));
    assert_eq!(p.read(&media_entry(8)).unwrap(), svg);
    assert_eq!(p.declared_size(&media_entry(7)), Some(4));
    assert_eq!(p.declared_size("media/99"), None);
}

#[test]
fn the_manifest_is_compact_json_with_its_fields_in_order() {
    let bytes = package(&[]);
    let mut p = Package::open(&bytes).unwrap();
    let json = String::from_utf8(p.read(MANIFEST_ENTRY).unwrap()).unwrap();
    assert_eq!(json, r#"{"format":"deckcraft","version":1,"generator":"Test 1","presentation":{"width":960.0,"zoom":null,"shapes":[]}}"#);
}

#[test]
fn non_finite_numbers_are_written_as_zero_and_read_back() {
    let deck = Deck { width: f64::NAN, zoom: Some(f64::INFINITY), shapes: vec![Shape::Box { x: f64::NEG_INFINITY, y: 2.0 }] };
    let bytes = write(&deck, "t", &[]).unwrap();
    let (m, _) = Package::open_manifest::<Deck>(&bytes, MAX_MANIFEST).unwrap();
    assert_eq!(m.presentation, Deck { width: 0.0, zoom: Some(0.0), shapes: vec![Shape::Box { x: 0.0, y: 2.0 }] });
}

/// Files written before [`Finite`] (plain serde_json: NaN as `null`) open too, including a
/// `null` inside a tagged enum, where serde reports only the enclosing object.
#[test]
fn older_manifests_with_null_numbers_parse() {
    let cases: [(&str, Deck); 3] = [
        (r#"{"format":"deckcraft","version":1,"generator":"old","presentation":{"width":null}}"#, Deck::default()),
        (
            r#"{"format":"deckcraft","version":1,"generator":"old","presentation":{"width":3,"shapes":[{"kind":"Box","x":null,"y":4}]}}"#,
            Deck { width: 3.0, shapes: vec![Shape::Box { x: 0.0, y: 4.0 }], ..Default::default() },
        ),
        (
            "{\n  \"format\": \"deckcraft\",\n  \"version\": 1,\n  \"generator\": \"t\",\n  \"presentation\": {\"shapes\": [{\"kind\": \"Line\", \"len\": null}]}\n}",
            Deck { shapes: vec![Shape::Line { len: 0.0 }], ..Default::default() },
        ),
    ];
    for (json, want) in cases {
        let m = parse_manifest::<Deck>(json.as_bytes()).unwrap_or_else(|e| panic!("{json}: {e}"));
        assert_eq!(m.presentation, want, "{json}");
    }
}

/// Only `null`s where numbers belong are patched: other damage still fails.
#[test]
fn other_manifest_damage_is_still_reported() {
    let ok = br#"{"format": "deckcraft", "version": 1, "generator": "t", "presentation": {}}"#;
    assert!(parse_manifest::<Deck>(ok).is_ok());
    let bad = br#"{"format": null, "version": 1, "generator": "t", "presentation": {}}"#;
    assert!(matches!(parse_manifest::<Deck>(bad), Err(FormatError::Damaged(_))));
    assert!(parse_manifest::<Deck>(&ok[..ok.len() - 3]).is_err());
}

#[test]
fn a_manifest_full_of_nulls_is_refused_within_bounds() {
    let shapes = (0..2000).map(|_| r#"{"kind":"Box","x":1,"y":"not a number"}"#).collect::<Vec<_>>().join(",");
    let json = format!(r#"{{"format":"deckcraft","version":1,"generator":"t","presentation":{{"width":null,"shapes":[{shapes}]}}}}"#);
    assert!(matches!(parse_manifest::<Deck>(json.as_bytes()), Err(FormatError::Damaged(_))));
}

#[test]
fn what_is_not_a_package_is_not_ours() {
    for bytes in [&b""[..], b"PK\x03\x04garbage", &[0u8; 1000]] {
        assert!(matches!(Package::open_manifest::<Deck>(bytes, MAX_MANIFEST), Err(FormatError::NotOurs(_))));
    }
    // A ZIP without the manifest, and one from another app.
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/zip", craft_zip::Compression::Stored).unwrap();
    let no_manifest = z.finish().unwrap();
    let err = Package::open_manifest::<Deck>(&no_manifest, MAX_MANIFEST).err().unwrap();
    assert_eq!(err.to_string(), "not a DeckCraft file: no presentation.json");
    let mut z = craft_zip::Writer::new();
    z.add(MANIFEST_ENTRY, br#"{"format":"other","version":1,"generator":"x","presentation":{}}"#, craft_zip::Compression::Deflated).unwrap();
    let other = z.finish().unwrap();
    let err = Package::open_manifest::<Deck>(&other, MAX_MANIFEST).err().unwrap();
    assert_eq!(err.to_string(), "not a DeckCraft file: other");
    // A truncated package is an error, not a panic.
    let mut b = package(&[]);
    b.truncate(b.len() / 2);
    assert!(Package::open_manifest::<Deck>(&b, MAX_MANIFEST).is_err());
}

#[test]
fn legacy_slidecraft_packages_are_recognised() {
    let mut z = craft_zip::Writer::new();
    z.add(MIMETYPE_ENTRY, LEGACY_MIMETYPE.as_bytes(), craft_zip::Compression::Stored).unwrap();
    assert!(sniff(&z.finish().unwrap()));
    assert!(!sniff(b"PK\x03\x04 application/zip"));
    assert!(!sniff(MIMETYPE.as_bytes()));
}

#[test]
fn an_oversized_manifest_is_refused_before_inflating() {
    let bytes = package(&[]);
    let size = Package::open(&bytes).unwrap().declared_size(MANIFEST_ENTRY).unwrap();
    assert!(Package::open_manifest::<Deck>(&bytes, size).is_ok());
    let err = Package::open_manifest::<Deck>(&bytes, size - 1).err().unwrap();
    assert_eq!(err.to_string(), "damaged file: presentation.json is too large");
}

#[test]
fn a_member_that_inflates_past_its_declared_size_stops_at_the_limit() {
    // Declared sizes are the file's word: a stored member whose header lies is still capped.
    let data = vec![7u8; 100];
    let bytes = package(&[Member { name: &media_entry(1), content_type: "image/png", bytes: &data }]);
    let mut p = Package::open(&bytes).unwrap();
    assert_eq!(p.read_max(&media_entry(1), 100).unwrap().len(), 100);
    assert!(p.read_max(&media_entry(1), 99).is_err());
    let err = p.read("media/2").unwrap_err();
    assert!(matches!(err, FormatError::Damaged(ref m) if m.starts_with("media/2: ")), "{err}");
}

#[test]
fn corrupt_members_are_charged_for_what_they_inflated() {
    let a = b"PICTURE-AAAAAAAAAAAA";
    let b = b"PICTURE-BBBBBBBBBBBB";
    let mut bytes = package(&[
        Member { name: &media_entry(1), content_type: "image/png", bytes: a },
        Member { name: &media_entry(2), content_type: "image/png", bytes: b },
    ]);
    // Corrupt both stored members, so each fails its CRC only after inflating all its bytes.
    for tag in [b"PICTURE-AAAA", b"PICTURE-BBBB"] {
        let at = bytes.windows(tag.len()).position(|w| w == tag).unwrap();
        bytes[at] ^= 0xff;
    }
    let mut p = Package::open(&bytes).unwrap();
    let mut left = 30;
    assert!(p.read_charged(&media_entry(1), &mut left).is_err());
    assert_eq!(left, 10, "a failed read still spends what it inflated");
    // The second fits a fresh budget on its own, but not what is left: refused before inflating.
    let err = p.read_charged(&media_entry(2), &mut left).unwrap_err();
    assert_eq!(err.to_string(), "damaged file: media/2 is too large");
    assert_eq!(left, 10);
}

#[test]
fn chart_part_names_are_exact() {
    assert_eq!(chart_part_entry(3), "chart-parts/3");
    assert!(is_chart_part_entry("chart-parts/0"));
    assert!(is_chart_part_entry("chart-parts/12"));
    for not in ["media/1", "chart-parts/../0", "chart-parts/00", "chart-parts/-1", "chart-parts/", "chart-parts/+1", "chart-parts/1 "] {
        assert!(!is_chart_part_entry(not), "{not}");
    }
}

#[test]
fn compressed_content_types_are_stored() {
    for stored in [
        "image/jpeg",
        "image/png",
        "video/mp4",
        "audio/mpeg",
        "audio/mp4",
        "application/zip",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet+zip",
    ] {
        assert!(is_compressed(stored), "{stored}");
    }
    for deflated in ["image/svg+xml", "image/gif", "audio/wav", "application/octet-stream", "text/plain"] {
        assert!(!is_compressed(deflated), "{deflated}");
    }
}

#[test]
fn duplicate_members_are_a_write_error() {
    let err = write(
        &Deck::default(),
        "t",
        &[Member { name: "media/1", content_type: "x", bytes: b"a" }, Member { name: "media/1", content_type: "x", bytes: b"b" }],
    )
    .unwrap_err();
    assert!(matches!(err, FormatError::Write(_)), "{err}");
}

#[test]
fn the_default_budget_is_the_formats_limits() {
    assert_eq!(Budget::default(), Budget { total: 4 << 30, items: 100_000, manifest: 256 << 20 });
    assert_eq!(Budget::media(10, 2), Budget { total: 10, items: 2, manifest: MAX_MANIFEST });
}
