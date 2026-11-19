//! Detection, the archive, the version cut, pages and images, on files built in code.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::Write;

use serde_json::{Value, json};

use super::*;

/// A `.sketch` zip: `document.json`, `meta.json` saying `app_version` / `version`, one
/// `pages/<id>.json` per page and any other `files` (images, fonts).
fn sketch_zip(app_version: &str, version: u64, document: Value, pages: &[Value], files: &[(&str, &[u8])]) -> Vec<u8> {
    let meta = json!({"app": "com.bohemiancoding.sketch3", "appVersion": app_version, "version": version, "compatibilityVersion": 99, "pagesAndArtboards": {}});
    raw_zip(&document.to_string(), &meta, pages, files)
}

/// A `.sketch` zip with the given `document.json` text and `meta.json` as they are.
fn raw_zip(document: &str, meta: &Value, pages: &[Value], files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("document.json", opts).unwrap();
        z.write_all(document.as_bytes()).unwrap();
        z.start_file("meta.json", opts).unwrap();
        z.write_all(meta.to_string().as_bytes()).unwrap();
        for p in pages {
            let id = p["do_objectID"].as_str().unwrap_or("page");
            z.start_file(format!("pages/{id}.json"), opts).unwrap();
            z.write_all(p.to_string().as_bytes()).unwrap();
        }
        for (name, bytes) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    out
}

/// `document.json` listing `pages` (ids) in order.
fn document(pages: &[&str]) -> Value {
    json!({
        "_class": "document",
        "do_objectID": "DOC",
        "pages": pages.iter().map(|p| json!({"_class": "MSJSONFileReference", "_ref_class": "MSImmutablePage", "_ref": format!("pages/{p}")})).collect::<Vec<_>>(),
    })
}

fn page(id: &str, name: &str, layers: Vec<Value>) -> Value {
    json!({"_class": "page", "do_objectID": id, "name": name, "layers": layers})
}

fn page_names(a: &Archive) -> Vec<String> {
    a.pages.iter().map(|(_, p)| p["name"].as_str().unwrap().to_string()).collect()
}

/// Bytes that barely compress.
fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as u8
        })
        .collect()
}

#[test]
fn detects_sketch_and_not_others() {
    let z = sketch_zip("50.2", 103, document(&["P1"]), &[page("P1", "Page 1", vec![])], &[]);
    assert!(is_sketch(&z));
    assert!(!is_sketch(b"fig-kiwi...."));
    assert!(!is_sketch(b"{}"));
    assert!(!is_sketch(b""));
    // A zip that isn't a Sketch file.
    let mut other = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut other));
        w.start_file("canvas.fig", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
    }
    assert!(!is_sketch(&other));
    assert!(Archive::open(&other).is_err());
}

#[test]
fn the_parts_are_read_as_the_file_holds_them() {
    let layer = json!({"_class": "rectangle", "do_objectID": "R", "frame": {"x": 1, "y": 2, "width": 3, "height": 4}});
    let z = sketch_zip("50.2", 103, document(&["B", "A"]), &[page("A", "Alpha", vec![layer.clone()]), page("B", "Beta", vec![])], &[]);
    let a = Archive::open(&z).unwrap();
    assert_eq!(a.version, 103);
    assert_eq!(app_major(&a.meta), Some(50));
    assert_eq!(a.document["do_objectID"], "DOC");
    // Pages come in the document's order, not the archive's.
    assert_eq!(a.pages.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["B", "A"]);
    assert_eq!(page_names(&a), ["Beta", "Alpha"]);
    assert_eq!(a.pages[1].1["layers"][0], layer);
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
}

#[test]
fn a_page_listed_twice_is_read_once() {
    let z = sketch_zip("50.2", 103, document(&["A", "A", "A"]), &[page("A", "Alpha", vec![])], &[]);
    assert_eq!(page_names(&Archive::open(&z).unwrap()), ["Alpha"]);
}

#[test]
fn older_than_48_is_refused() {
    let z = sketch_zip("47.1", 95, document(&["P"]), &[page("P", "P", vec![])], &[]);
    assert_eq!(Archive::open(&z).err(), Some(Error::TooOld));
    assert_eq!(Error::TooOld.to_string(), TOO_OLD);
    let mut sqlite = b"SQLite format 3\0".to_vec();
    sqlite.resize(512, 0);
    assert!(is_sketch(&sqlite));
    assert_eq!(Archive::open(&sqlite).err(), Some(Error::TooOld));
    // 48 itself, and a meta.json without an app version, open.
    assert!(Archive::open(&sketch_zip("48", 97, document(&["P"]), &[page("P", "P", vec![])], &[])).is_ok());
    let no_version = raw_zip(&document(&["P"]).to_string(), &json!({"app": "com.bohemiancoding.sketch3"}), &[page("P", "P", vec![])], &[]);
    assert_eq!(Archive::open(&no_version).unwrap().version, 0);
}

#[test]
fn newer_than_tested_opens_with_one_warning() {
    let z = sketch_zip("2099.1", NEWEST_TESTED_VERSION + 1, document(&["P"]), &[page("P", "P", vec![])], &[]);
    let a = Archive::open(&z).unwrap();
    assert_eq!(a.warnings.iter().filter(|w| w.contains("newer Sketch")).count(), 1, "{:?}", a.warnings);
    let z = sketch_zip("83", NEWEST_TESTED_VERSION, document(&["P"]), &[page("P", "P", vec![])], &[]);
    assert!(Archive::open(&z).unwrap().warnings.is_empty());
}

#[test]
fn junk_never_panics() {
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "P", vec![json!({"_class": "rectangle"})])], &[("images/a.png", b"png")]);
    let read = |bytes: &[u8]| {
        let _ = is_sketch(bytes);
        if let Ok(a) = Archive::open(bytes) {
            let _ = a.image("images/a.png");
            let _ = embedded_font_families(&a.document);
        }
    };
    for cut in (0..z.len()).step_by(7) {
        read(&z[..cut]);
    }
    for (n, seed) in (0..200u32).enumerate() {
        let mut b = z.clone();
        for (i, v) in noise(16, seed + 1).chunks(2).enumerate() {
            let at = (usize::from(v[0]) * 251 + i * 97 + n * 13) % b.len();
            b[at] = v[1];
        }
        read(&b);
    }
    for doc in [json!([]), json!(null), json!({"pages": 5}), json!({"pages": [{"_ref": 7}, {"_ref": "pages/../../x"}]})] {
        let z = sketch_zip("50.2", 103, doc, &[json!(5), json!({"do_objectID": "Q", "layers": "x"})], &[]);
        let a = Archive::open(&z).unwrap();
        assert!(a.pages.len() <= 1);
    }
    assert!(Archive::open(&raw_zip(&document(&["P"]).to_string(), &json!("x"), &[], &[])).is_err());
}

#[test]
fn page_references_that_are_not_ids_are_skipped() {
    let doc = json!({"pages": [{"_ref": "pages/../meta"}, {"_ref": 7}, {"_ref": "elsewhere/A"}, {"_ref": format!("pages/{}", "a".repeat(129))}, {"_ref": "pages/A"}]});
    let z = sketch_zip("50.2", 103, doc, &[page("A", "Alpha", vec![])], &[]);
    let a = Archive::open(&z).unwrap();
    assert_eq!(page_names(&a), ["Alpha"]);
    assert_eq!(a.warnings.iter().filter(|w| w.contains("without a valid reference")).count(), 4, "{:?}", a.warnings);
    // A page the document lists but the archive lacks; then one that isn't an object.
    let doc = document(&["Gone", "A"]);
    let z = sketch_zip("50.2", 103, doc, &[page("A", "Alpha", vec![])], &[]);
    let a = Archive::open(&z).unwrap();
    assert_eq!(page_names(&a), ["Alpha"]);
    assert!(a.warnings.iter().any(|w| w.starts_with("a page was skipped: Sketch file: pages/Gone.json")), "{:?}", a.warnings);
    let numbers = raw_zip(&document(&["page"]).to_string(), &json!({"appVersion": "50"}), &[json!(5)], &[]);
    let a = Archive::open(&numbers).unwrap();
    assert!(a.pages.is_empty() && a.warnings.iter().any(|w| w.contains("aren't JSON objects")), "{:?}", a.warnings);
}

#[test]
fn deep_nesting_does_not_fail_the_file() {
    let mut deep = json!({"_class": "rectangle"});
    for _ in 0..300 {
        deep = json!({"_class": "group", "layers": [deep]});
    }
    let z = sketch_zip("50.2", 103, document(&["A", "B"]), &[page("A", "Deep", vec![deep]), page("B", "Normal", vec![])], &[]);
    let a = Archive::open(&z).unwrap();
    assert_eq!(page_names(&a), ["Normal"]);
    assert!(a.warnings.iter().any(|w| w.starts_with("a page was skipped")), "{:?}", a.warnings);
}

#[test]
fn a_document_that_cannot_be_read_keeps_the_pages() {
    let pages = [page("B", "Beta", vec![]), page("A", "Alpha", vec![])];
    for broken in ["{not json", "[1, 2]"] {
        let z = raw_zip(broken, &json!({"appVersion": "50.2", "version": 103}), &pages, &[]);
        let a = Archive::open(&z).unwrap();
        // Archive order stands in for the document's.
        assert_eq!(page_names(&a), ["Beta", "Alpha"]);
        assert!(a.warnings.iter().any(|w| w.contains("shared styles and library symbols were skipped")), "{:?}", a.warnings);
    }
}

#[test]
fn pages_over_the_page_limit_are_skipped() {
    let z = sketch_zip("50.2", 103, document(&["A", "B"]), &[page("A", &"Big".repeat(100), vec![]), page("B", "Small", vec![])], &[]);
    let a = Archive::open_with(&z, Limits { max_page: 200, ..Limits::default() }).unwrap();
    assert_eq!(page_names(&a), ["Small"]);
    assert!(a.warnings.iter().any(|w| w.contains("pages/A.json is too large")), "{:?}", a.warnings);
}

#[test]
fn pages_past_the_page_count_are_not_read() {
    let ids: Vec<String> = (0..MAX_PAGES + 5).map(|i| format!("P{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let pages: Vec<Value> = ids.iter().map(|id| page(id, id, vec![])).collect();
    let z = sketch_zip("50.2", 103, document(&refs), &pages, &[]);
    let a = Archive::open(&z).unwrap();
    assert_eq!(a.pages.len(), MAX_PAGES);
    assert!(a.warnings.iter().any(|w| w.contains("more than 1000 pages")), "{:?}", a.warnings);
}

#[test]
fn a_member_that_unpacks_far_past_its_packed_size_is_refused() {
    // 17 MiB of spaces around a page pack into a few KB: past the 16 MiB floor and 100 times.
    let padded = format!("{}{}", page("A", "Padded", vec![]), " ".repeat(17 << 20));
    let mut out = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, text) in [("document.json", document(&["A", "B"]).to_string()), ("meta.json", json!({"appVersion": "50"}).to_string())] {
            z.start_file(name, opts).unwrap();
            z.write_all(text.as_bytes()).unwrap();
        }
        z.start_file("pages/A.json", opts).unwrap();
        z.write_all(padded.as_bytes()).unwrap();
        z.start_file("pages/B.json", opts).unwrap();
        z.write_all(page("B", "Small", vec![]).to_string().as_bytes()).unwrap();
        z.finish().unwrap();
    }
    assert!(out.len() < 1 << 20, "{} bytes", out.len());
    let a = Archive::open(&out).unwrap();
    assert_eq!(page_names(&a), ["Small"]);
    assert!(a.warnings.iter().any(|w| w.contains("pages/A.json is too large")), "{:?}", a.warnings);
}

#[test]
fn images_are_read_by_plain_names_within_their_budget() {
    let (one, two) = (noise(600, 1), noise(600, 2));
    let files: [(&str, &[u8]); 3] = [("images/a1.png", &one), ("images/b2", &two), ("fonts/f", b"font")];
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "P", vec![])], &files);
    let a = Archive::open(&z).unwrap();
    assert_eq!(a.image("images/a1.png"), Some(one.clone()));
    assert_eq!(a.image("a1.png"), Some(one.clone()));
    assert_eq!(a.image("b2"), Some(two.clone()));
    // Never a path, never another folder, never two extensions.
    for bad in ["", "images/../meta.json", "../meta.json", "a1.png.bak", "images/missing.png", "f", "a/b.png"] {
        assert_eq!(a.image(bad), None, "{bad}");
    }
    // The per-image cap, and the total: the second image no longer fits.
    let small = Archive::open_with(&z, Limits { max_image: 599, ..Limits::default() }).unwrap();
    assert_eq!(small.image("a1.png"), None);
    let tight = Archive::open_with(&z, Limits { max_image_total: 1000, ..Limits::default() }).unwrap();
    assert_eq!(tight.image("a1.png"), Some(one));
    assert_eq!(tight.image("b2"), None);
    let exact = Archive::open_with(&z, Limits { max_image_total: 1200, ..Limits::default() }).unwrap();
    assert!(exact.image("a1.png").is_some() && exact.image("b2").is_some());
}

#[test]
fn embedded_fonts_are_listed_by_family() {
    let doc = json!({"fontReferences": [
        {"fontFamilyName": "Zeta", "fontData": {"_ref": "fonts/z"}},
        {"fontFamilyName": "Alpha", "fontData": {"_ref": "fonts/a"}},
        {"fontFamilyName": "Alpha", "fontData": {"_ref": "fonts/a2"}},
        {"fontFamilyName": "Named only"},
        {"fontData": {}},
        {"fontFamilyName": "x".repeat(100), "fontData": {}},
    ]});
    assert_eq!(embedded_font_families(&doc), ["Alpha".to_string(), "Zeta".into(), "x".repeat(64)]);
    assert!(embedded_font_families(&json!({"fontReferences": 5})).is_empty());
    let many: Vec<Value> = (0..50).map(|i| json!({"fontFamilyName": format!("F{i:02}"), "fontData": {}})).collect();
    assert_eq!(embedded_font_families(&json!({"fontReferences": many})).len(), 20);
}

/// A ZIP64 end record can declare millions of entries in a small file: refused on the raw bytes
/// before the archive is indexed, and such a file is not taken for a `.sketch`.
#[test]
fn an_archive_declaring_too_many_entries_is_refused_before_it_is_indexed() {
    let bomb = craft_zip::zip64_entry_count_sample(MAX_ENTRIES + 1);
    let e = Archive::open(&bomb).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains(&format!("declares {} entries", MAX_ENTRIES + 1)), "{e}");
    assert!(!is_sketch(&bomb));
}

/// Two entries of one name: `zip` keeps one, and which one the writer meant is not known.
#[test]
fn two_entries_of_one_name_are_refused() {
    let z = sketch_zip("50.2", 103, document(&["P1"]), &[page("P1", "Page 1", vec![])], &[("images/dupA.png", b"a"), ("images/dupB.png", b"b")]);
    assert!(Archive::open(&z).is_ok());
    let mut renamed = z.clone();
    let mut i = 0;
    while let Some(at) = renamed[i..].windows(4).position(|w| w == b"dupB") {
        renamed[i + at + 3] = b'A';
        i += at + 4;
    }
    let e = Archive::open(&renamed).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("two entries of the archive share a name"), "{e}");
}
