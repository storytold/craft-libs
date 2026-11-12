//! Reading packages: the container, the package document, and the limits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write};

use craft_epub::{Epub, Error, Limits, dir_of, parse_container, parse_package, resolve};

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        z.start_file(*name, o).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const CONTAINER: &[u8] = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

fn opf(spine_items: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test Book</dc:title>
<dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/">Jane Author</dc:creator>
<dc:language xmlns:dc="http://purl.org/dc/elements/1.1/">en</dc:language></metadata>
<manifest>
<item id="c1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
<item id="img1" href="images/cover.png" media-type="image/png"/>
</manifest>
<spine>{spine_items}</spine>
</package>"#
    )
    .into_bytes()
}

const PNG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

fn sample_package() -> Vec<u8> {
    zip_of(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf(r#"<itemref idref="c1"/><itemref idref="c2"/>"#)),
        ("OEBPS/text/chapter1.xhtml", b"<html><body><h1>Chapter One</h1></body></html>"),
        ("OEBPS/text/chapter2.xhtml", br#"<html><body><img src="../images/cover.png"/></body></html>"#),
        ("OEBPS/images/cover.png", &PNG),
    ])
}

#[test]
fn resolves_relative_paths() {
    assert_eq!(resolve("OEBPS/text", "../images/cover.png"), "OEBPS/images/cover.png");
    assert_eq!(resolve("OEBPS", "text/chapter1.xhtml"), "OEBPS/text/chapter1.xhtml");
    assert_eq!(resolve("OEBPS/text", "chapter1.xhtml#frag"), "OEBPS/text/chapter1.xhtml");
    assert_eq!(resolve("", "cover%20art.png"), "cover art.png");
    assert_eq!(resolve("OEBPS", "/root.xhtml"), "root.xhtml");
    assert_eq!(resolve("a", "%zz%4"), "a/%zz%4", "a malformed escape stays as written");
    assert_eq!(dir_of("a/b/c.xhtml"), "a/b");
    assert_eq!(dir_of("c.xhtml"), "");
}

#[test]
fn path_traversal_href_cannot_escape_the_archive() {
    assert_eq!(resolve("OEBPS/text", "../../../../../../etc/passwd"), "etc/passwd");
}

#[test]
fn reads_container_and_package() {
    assert_eq!(parse_container(CONTAINER).unwrap().as_deref(), Some("OEBPS/content.opf"));
    let pkg =
        parse_package(&opf(r#"<itemref idref="c1"/><itemref idref="c2" linear="no"/><itemref idref="gone"/>"#), "OEBPS", &Limits::DEFAULT).unwrap();
    assert_eq!(pkg.version, "3.0");
    assert_eq!(pkg.title, "Test Book");
    assert_eq!(pkg.creators, ["Jane Author"]);
    assert_eq!(pkg.language.as_deref(), Some("en"));
    assert_eq!(pkg.spine_paths().collect::<Vec<_>>(), ["OEBPS/text/chapter1.xhtml", "OEBPS/text/chapter2.xhtml"]);
    assert_eq!(pkg.spine.iter().map(|s| s.linear).collect::<Vec<_>>(), [true, false, true]);
    assert_eq!(pkg.spine.last().unwrap().path, None, "an idref no item has is kept, without a path");
    assert_eq!(pkg.manifest.len(), 3);
    assert_eq!(pkg.manifest[2].media_type, "image/png");
}

/// Text around an entity reference is one title, and a repeated element (OPF allows more than
/// one `dc:title`) does not replace what the first gave.
#[test]
fn metadata_accumulates_across_entity_references_and_keeps_only_the_first_element() {
    let xml = br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:title>Fish &amp; Chips</dc:title>
<dc:title>A second, ignored title</dc:title>
<dc:creator>Smith &amp; Sons</dc:creator>
<dc:identifier id="other">isbn:1</dc:identifier>
<dc:identifier id="id">urn:uuid:2</dc:identifier>
<meta property="dcterms:modified">2026-01-02T03:04:05Z</meta>
<meta property="rendition:layout">pre-paginated</meta>
</metadata>
<manifest/>
<spine/>
</package>"#;
    let pkg = parse_package(xml, "OEBPS", &Limits::DEFAULT).unwrap();
    assert_eq!(pkg.title, "Fish & Chips");
    assert_eq!(pkg.creators, ["Smith & Sons"]);
    assert_eq!(pkg.identifier.as_deref(), Some("urn:uuid:2"));
    assert_eq!(pkg.modified.as_deref(), Some("2026-01-02T03:04:05Z"));
    assert!(pkg.pre_paginated);
}

#[test]
fn epub2_package_ncx_cover_and_dc_metadata() {
    let xml = br#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
<metadata><dc-metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Old</dc:title><dc:identifier id="BookId">x</dc:identifier></dc-metadata>
<meta name="cover" content="cov"/></metadata>
<manifest>
<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
<item id="cov" href="img/c%20over.jpg" media-type="image/jpeg"/>
<item id="c" href="c.html" media-type="application/xhtml+xml"/>
<item id="c" href="dup.html" media-type="application/xhtml+xml"/>
</manifest>
<spine toc="ncx"><itemref idref="c"/></spine></package>"#;
    let pkg = parse_package(xml, "", &Limits::DEFAULT).unwrap();
    assert_eq!((pkg.version.as_str(), pkg.title.as_str(), pkg.identifier.as_deref()), ("2.0", "Old", Some("x")));
    assert_eq!(pkg.ncx.as_deref(), Some("toc.ncx"));
    assert_eq!(pkg.cover_image().map(|i| i.path.as_str()), Some("img/c over.jpg"));
    assert_eq!(pkg.nav(), None);
    assert_eq!(pkg.spine_paths().collect::<Vec<_>>(), ["c.html"], "the first item with an id wins");
}

#[test]
fn opens_a_package_and_reads_its_files() {
    let bytes = sample_package();
    assert!(!craft_epub::is_epub(&bytes), "this mimetype entry is compressed");
    let mut epub = Epub::open(&bytes).unwrap();
    assert_eq!(epub.opf_path, "OEBPS/content.opf");
    assert_eq!(epub.package.title, "Test Book");
    let spine: Vec<String> = epub.package.spine_paths().map(str::to_string).collect();
    assert!(epub.read(&spine[0]).unwrap().unwrap().starts_with(b"<html>"));
    assert_eq!(epub.read_href(&spine[1], "../images/cover.png").unwrap().as_deref(), Some(&PNG[..]));
    assert_eq!(epub.read_href(&spine[1], "#top").unwrap(), None);
    assert!(epub.contains("mimetype") && !epub.contains("nope"));
}

#[test]
fn a_missing_spine_file_is_none_not_an_error() {
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", &opf(r#"<itemref idref="c1"/>"#))]);
    let mut epub = Epub::open(&bytes).unwrap();
    assert_eq!(epub.read("OEBPS/text/chapter1.xhtml").unwrap(), None);
    assert_eq!(epub.read("OEBPS/").unwrap(), None);
}

#[test]
fn missing_container_or_package_is_an_error() {
    assert!(matches!(Epub::open(&zip_of(&[("mimetype", b"application/epub+zip")])), Err(Error::NotEpub(_))));
    assert!(matches!(Epub::open(&zip_of(&[("META-INF/container.xml", CONTAINER)])), Err(Error::NotEpub(_))));
    let no_rootfile = b"<container><rootfiles/></container>";
    assert!(matches!(Epub::open(&zip_of(&[("META-INF/container.xml", no_rootfile)])), Err(Error::NotEpub(_))));
}

#[test]
fn not_a_zip_is_an_error() {
    assert!(matches!(Epub::open(b"not a zip file"), Err(Error::NotEpub(_))));
    assert!(matches!(Epub::open(b""), Err(Error::NotEpub(_))));
}

/// Cut off mid-element, after the rootfile it names: read leniently this is a complete
/// container, and whatever of the package is still there would open as half a book.
#[test]
fn container_truncated_after_its_useful_content_is_still_an_error() {
    let truncated = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>"#;
    let bytes = zip_of(&[("META-INF/container.xml", &truncated[..]), ("OEBPS/content.opf", &opf(""))]);
    assert!(matches!(Epub::open(&bytes), Err(Error::Part { .. })));
}

/// The same for the package document: cut off after its manifest and before the end of its
/// spine, it would open as a book with fewer chapters and no error.
#[test]
fn package_document_truncated_mid_spine_is_still_an_error() {
    let truncated = br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test Book</dc:title></metadata>
<manifest><item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="c1"/>"#;
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", &truncated[..])]);
    match Epub::open(&bytes) {
        Err(Error::Part { part, .. }) => assert_eq!(part, "OEBPS/content.opf"),
        other => panic!("{:?}", other.map(|e| e.package)),
    }
}

#[test]
fn mismatched_closing_tag_is_an_error() {
    let mismatched = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</wrong>"#;
    let bytes = zip_of(&[("META-INF/container.xml", &mismatched[..]), ("OEBPS/content.opf", &opf(""))]);
    assert!(matches!(Epub::open(&bytes), Err(Error::Part { .. })));
}

// ---------- limits ----------

#[test]
fn an_entry_past_the_entry_limit_is_refused() {
    let bytes = sample_package();
    let limits = Limits { max_entry_bytes: 1024, ..Limits::DEFAULT };
    assert!(Epub::open_with(&bytes, limits).is_ok(), "everything here is under the limit");
    let big = vec![b' '; 2000];
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", &opf("")), ("OEBPS/big.bin", &big)]);
    let mut epub = Epub::open_with(&bytes, limits).unwrap();
    assert_eq!(epub.read("OEBPS/big.bin"), Err(Error::TooLarge { part: "OEBPS/big.bin".into(), limit: 1024 }));
}

#[test]
fn entries_cannot_add_up_past_the_total_budget() {
    let chunk = vec![b'x'; 600];
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", &opf("")), ("a", &chunk), ("b", &chunk), ("c", &chunk)]);
    let used = CONTAINER.len() + opf("").len();
    let limits = Limits { max_total_bytes: (used + 1500) as u64, ..Limits::DEFAULT };
    let mut epub = Epub::open_with(&bytes, limits).unwrap();
    assert_eq!(epub.budget_left(), 1500);
    assert!(epub.read("a").unwrap().is_some());
    assert!(epub.read("b").unwrap().is_some());
    assert_eq!(epub.read("c"), Err(Error::TooLarge { part: "c".into(), limit: 300 }));
    // The container itself is inside the budget too.
    assert!(matches!(Epub::open_with(&bytes, Limits { max_total_bytes: 10, ..Limits::DEFAULT }), Err(Error::TooLarge { .. })));
}

#[test]
fn an_archive_with_too_many_entries_is_refused() {
    let bytes = sample_package();
    assert_eq!(Epub::open_with(&bytes, Limits { max_entries: 5, ..Limits::DEFAULT }).err(), Some(Error::TooManyEntries { found: 6, limit: 5 }));
    assert!(Epub::open_with(&bytes, Limits { max_entries: 6, ..Limits::DEFAULT }).is_ok());
}

#[test]
fn manifest_and_spine_items_past_their_limits_are_ignored() {
    let xml = opf(r#"<itemref idref="c1"/><itemref idref="c2"/><itemref idref="c1"/>"#);
    let pkg = parse_package(&xml, "", &Limits { max_manifest: 2, max_spine: 2, ..Limits::DEFAULT }).unwrap();
    assert_eq!(pkg.manifest.len(), 2);
    assert_eq!(pkg.spine.len(), 2);
}

#[test]
fn a_package_document_past_the_entry_limit_is_refused() {
    let xml = opf("");
    let limits = Limits { max_entry_bytes: (xml.len() - 1) as u64, ..Limits::DEFAULT };
    assert!(matches!(parse_package(&xml, "", &limits), Err(Error::Part { .. })));
}

/// A ZIP64 end record can declare millions of entries in a small file; the count is refused on
/// the raw bytes, before the archive is indexed.
#[test]
fn an_archive_declaring_too_many_entries_is_refused_before_it_is_indexed() {
    let bomb = craft_zip::zip64_entry_count_sample(100_001);
    assert_eq!(Epub::open(&bomb).err(), Some(Error::TooManyEntries { found: 100_001, limit: 100_000 }));
}
