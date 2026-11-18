//! A book's content documents read into the flow model (feature `doc`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write};
use std::sync::Arc;

use craft_epub::doc::{BookFlow, BookOptions, Boundary, Warning, read_book};
use craft_epub::{Book, Error, Layout, Limits, Metadata, Nav, NavItem, Resource};
use craft_flow::{FBlock, Flow, Inline, Kind};

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        z.start_file(*name, o).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const CONTAINER: &[u8] = br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;

/// A package with the package document at `OEBPS/content.opf`: `manifest` and `spine` are its
/// `item`s and `itemref`s, `files` the other entries.
fn package(manifest: &str, spine: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let opf = format!(
        r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>A Book</dc:title></metadata><manifest>{manifest}</manifest><spine{spine}</spine></package>"#
    );
    let mut entries: Vec<(&str, &[u8])> = vec![("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", opf.as_bytes())];
    entries.extend_from_slice(files);
    zip_of(&entries)
}

fn xhtml(id: &str, href: &str) -> String {
    format!(r#"<item id="{id}" href="{href}" media-type="application/xhtml+xml"/>"#)
}

fn chapter(body: &str) -> Vec<u8> {
    craft_epub::xhtml("en", "", "", "", body).into_bytes()
}

fn read(bytes: &[u8]) -> BookFlow {
    read_book(bytes, &BookOptions::default()).unwrap()
}

/// A PNG signature and `tag`: enough for the reader to know it as a picture.
fn png(tag: &str) -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(tag.as_bytes());
    b
}

fn paras(flow: &Flow) -> Vec<&craft_flow::Para> {
    flow.blocks
        .iter()
        .filter_map(|b| match b {
            FBlock::Para(p) => Some(p),
            FBlock::Table(_) => None,
        })
        .collect()
}

fn text(flow: &Flow) -> Vec<String> {
    paras(flow).iter().map(|p| p.text()).filter(|t| !t.is_empty()).collect()
}

fn inlines(flow: &Flow) -> impl Iterator<Item = &Inline> {
    flow.blocks.iter().flat_map(|b| match b {
        FBlock::Para(p) => p.inlines.iter(),
        FBlock::Table(_) => [].iter(),
    })
}

fn anchors(flow: &Flow) -> Vec<&str> {
    inlines(flow).filter_map(|i| if let Inline::Anchor(a) = i { Some(a.as_str()) } else { None }).collect()
}

/// Each link's text and target.
fn links(flow: &Flow) -> Vec<(&str, &str)> {
    inlines(flow).filter_map(|i| if let Inline::Text(t, f) = i { Some((t.as_str(), f.link.as_deref()?)) } else { None }).collect()
}

fn images(flow: &Flow) -> Vec<Arc<Vec<u8>>> {
    inlines(flow).filter_map(|i| if let Inline::Image(img) = i { Some(img.data.clone()) } else { None }).collect()
}

fn paths(book: &BookFlow) -> Vec<&str> {
    book.chapters.iter().map(|c| c.path.as_str()).collect()
}

#[test]
fn a_two_chapter_book_round_trips() {
    let figure = png("figure");
    let one = chapter(r#"<h1>One</h1><p>First words.</p><p><img src="images/figure.png" alt="A figure"/></p>"#);
    let two = chapter("<h1>Two</h1><p>Second words.</p>");
    let book = Book {
        metadata: Metadata {
            identifier: "urn:uuid:1".into(),
            title: "A Book".into(),
            language: "en".into(),
            creators: vec!["An Author".into(), "Another".into()],
            modified: "2026-10-13T09:30:00Z".into(),
            ..Metadata::default()
        },
        resources: vec![
            Resource::new("c1", "one.xhtml", "application/xhtml+xml", one),
            Resource::new("c2", "two.xhtml", "application/xhtml+xml", two),
            Resource::new("fig", "images/figure.png", "image/png", figure.clone()).with_properties("cover-image"),
        ],
        spine: vec!["c1".into(), "c2".into()],
        nav: Nav {
            items: vec![
                NavItem { href: "one.xhtml".into(), label: "Chapter One".into(), level: 1 },
                NavItem { href: "two.xhtml".into(), label: "Chapter Two".into(), level: 1 },
            ],
            ..Nav::default()
        },
        ..Book::default()
    };
    let book = read(&craft_epub::write(&book).unwrap());
    assert_eq!(book.warnings, []);
    assert_eq!((book.package.title.as_str(), book.package.language.as_deref()), ("A Book", Some("en")));
    assert_eq!(book.package.creators, ["An Author", "Another"]);
    assert_eq!(book.package.identifier.as_deref(), Some("urn:uuid:1"));
    assert_eq!(book.cover.as_deref(), Some(&figure));
    assert_eq!(book.toc.len(), 2);
    assert_eq!(paths(&book), ["OEBPS/one.xhtml", "OEBPS/two.xhtml"]);
    let titles: Vec<_> = book.chapters.iter().map(|c| c.title.as_deref()).collect();
    assert_eq!(titles, [Some("Chapter One"), Some("Chapter Two")]);
    let first = &book.chapters[0].flow;
    assert_eq!(text(first), ["One", "First words."]);
    assert_eq!(paras(first)[0].kind, Kind::Heading(1));
    assert_eq!(images(first).len(), 1);
    // The picture is the cover's bytes, read once.
    assert!(Arc::ptr_eq(&images(first)[0], book.cover.as_ref().unwrap()));

    let joined = book.join(Boundary::PageBreak);
    assert_eq!(text(&joined), ["One", "First words.", "Two", "Second words."]);
    assert_eq!((joined.meta.title.as_str(), joined.meta.author.as_str()), ("A Book", "An Author, Another"));
    let breaks: Vec<_> = paras(&joined).iter().filter(|p| p.page_break).map(|p| p.text()).collect();
    assert_eq!(breaks, ["Two"]);
    let flowing = book.join(Boundary::Continuous);
    assert_eq!(text(&flowing), text(&joined));
    assert!(paras(&flowing).iter().all(|p| !p.page_break));
}

#[test]
fn a_document_listed_twice_in_the_spine_is_read_once() {
    let bytes = package(
        &[xhtml("a", "a.xhtml"), xhtml("b", "b.xhtml")].concat(),
        r#"><itemref idref="a"/><itemref idref="b"/><itemref idref="a"/>"#,
        &[("OEBPS/a.xhtml", &chapter("<p>A</p>")), ("OEBPS/b.xhtml", &chapter("<p>B</p>"))],
    );
    let book = read(&bytes);
    assert_eq!(paths(&book), ["OEBPS/a.xhtml", "OEBPS/b.xhtml"]);
    assert_eq!(text(&book.join(Boundary::Continuous)), ["A", "B"]);
}

#[test]
fn a_missing_chapter_is_a_warning() {
    let bytes = package(
        &[xhtml("a", "a.xhtml"), xhtml("gone", "gone.xhtml"), xhtml("c", "c.xhtml")].concat(),
        r#"><itemref idref="a"/><itemref idref="gone"/><itemref idref="nothing"/><itemref idref="c"/>"#,
        &[("OEBPS/a.xhtml", &chapter("<p>A</p>")), ("OEBPS/c.xhtml", &chapter("<p>C</p>"))],
    );
    let book = read(&bytes);
    assert_eq!(paths(&book), ["OEBPS/a.xhtml", "OEBPS/c.xhtml"]);
    assert_eq!(book.warnings, [Warning::MissingChapter { path: "nothing".into() }, Warning::MissingChapter { path: "OEBPS/gone.xhtml".into() }]);
}

#[test]
fn a_non_linear_item_is_kept_in_spine_order_and_flagged() {
    let bytes = package(
        &[xhtml("a", "a.xhtml"), xhtml("notes", "notes.xhtml"), xhtml("c", "c.xhtml")].concat(),
        r#"><itemref idref="a"/><itemref idref="notes" linear="no"/><itemref idref="c"/>"#,
        &[("OEBPS/a.xhtml", &chapter("<p>A</p>")), ("OEBPS/notes.xhtml", &chapter("<p>Notes</p>")), ("OEBPS/c.xhtml", &chapter("<p>C</p>"))],
    );
    let book = read(&bytes);
    let linear: Vec<_> = book.chapters.iter().map(|c| c.linear).collect();
    assert_eq!(linear, [true, false, true]);
    assert_eq!(text(&book.join(Boundary::Continuous)), ["A", "Notes", "C"]);
}

#[test]
fn an_image_shared_by_chapters_is_read_once() {
    // Twice the image would not fit in the budget: each chapter reading it would fail the book.
    let image = png(&"x".repeat(100_000));
    let manifest = [xhtml("a", "text/a.xhtml"), xhtml("b", "text/b.xhtml"), xhtml("c", "c.xhtml")].concat();
    let a = chapter(r#"<p><img src="../images/shared.png"/></p>"#);
    let b = chapter(r#"<p><img src="../images/shared.png"/></p>"#);
    let c = chapter(r#"<p><img src="images/shared.png"/></p>"#);
    let files: [(&str, &[u8]); 4] =
        [("OEBPS/text/a.xhtml", &a), ("OEBPS/text/b.xhtml", &b), ("OEBPS/c.xhtml", &c), ("OEBPS/images/shared.png", &image)];
    let bytes = package(&manifest, r#"><itemref idref="a"/><itemref idref="b"/><itemref idref="c"/>"#, &files);
    let opts = BookOptions { limits: Limits { max_total_bytes: 150_000, ..Limits::DEFAULT }, ..BookOptions::default() };
    let book = read_book(&bytes, &opts).unwrap();
    let found: Vec<_> = book.chapters.iter().flat_map(|c| images(&c.flow)).collect();
    assert_eq!(found.len(), 3);
    assert!(found.iter().all(|i| Arc::ptr_eq(i, &found[0])), "one buffer, read once");
    assert_eq!(*found[0], image);
}

#[test]
fn a_missing_image_is_a_warning() {
    let bytes = package(&xhtml("a", "a.xhtml"), r#"><itemref idref="a"/>"#, &[("OEBPS/a.xhtml", &chapter(r#"<p>A<img src="gone.png"/></p>"#))]);
    let book = read(&bytes);
    assert_eq!(text(&book.join(Boundary::Continuous)), ["A"]);
    assert_eq!(book.warnings, [Warning::MissingImage { path: "OEBPS/gone.png".into(), from: "OEBPS/a.xhtml".into() }]);
}

/// One chapter showing a 5,000-byte image that is also the cover.
fn with_large_image() -> Vec<u8> {
    let image = png(&"x".repeat(5_000));
    package(
        &[xhtml("a", "a.xhtml"), r#"<item id="pic" href="big.png" media-type="image/png" properties="cover-image"/>"#.into()].concat(),
        r#"><itemref idref="a"/>"#,
        &[("OEBPS/a.xhtml", &chapter(r#"<p>A<img src="big.png"/></p>"#)), ("OEBPS/big.png", &image)],
    )
}

#[test]
fn an_image_over_the_entry_limit_is_a_warning() {
    let opts = BookOptions { limits: Limits { max_entry_bytes: 2_000, ..Limits::DEFAULT }, ..BookOptions::default() };
    let book = read_book(&with_large_image(), &opts).unwrap();
    assert_eq!(text(&book.join(Boundary::Continuous)), ["A"]);
    assert_eq!((book.cover.as_ref(), images(&book.chapters[0].flow).len()), (None, 0));
    // The cover is read first; the chapter's use of the same image is not said again.
    assert_eq!(book.warnings, [Warning::ImageTooLarge { path: "OEBPS/big.png".into(), from: "OEBPS/content.opf".into() }]);
}

#[test]
fn an_image_past_the_total_budget_stops_the_book() {
    let opts = BookOptions { limits: Limits { max_total_bytes: 3_000, ..Limits::DEFAULT }, ..BookOptions::default() };
    assert!(matches!(read_book(&with_large_image(), &opts), Err(Error::TooLarge { part, .. }) if part == "OEBPS/big.png"));
}

#[test]
fn a_remote_image_is_said_once() {
    let body = r#"<p>A<img src="https://example.com/a.png"/><img src="https://example.com/a.png"/></p>"#;
    let bytes = package(&xhtml("a", "a.xhtml"), r#"><itemref idref="a"/>"#, &[("OEBPS/a.xhtml", &chapter(body))]);
    assert_eq!(read(&bytes).warnings, [Warning::MissingImage { path: "https://example.com/a.png".into(), from: "OEBPS/a.xhtml".into() }]);
}

#[test]
fn a_spent_budget_stops_the_book() {
    let big = chapter(&"<p>words</p>".repeat(10_000));
    let bytes = package(&xhtml("a", "a.xhtml"), r#"><itemref idref="a"/>"#, &[("OEBPS/a.xhtml", &big)]);
    let opts = BookOptions { limits: Limits { max_total_bytes: 2_000, ..Limits::DEFAULT }, ..BookOptions::default() };
    assert!(matches!(read_book(&bytes, &opts), Err(Error::TooLarge { part, .. }) if part == "OEBPS/a.xhtml"));
}

fn encrypted(algorithm: &str, path: &str) -> Vec<u8> {
    let encryption = format!(
        r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:EncryptedData><enc:EncryptionMethod Algorithm="{algorithm}"/><enc:CipherData><enc:CipherReference URI="{path}"/></enc:CipherData></enc:EncryptedData></encryption>"#
    );
    package(
        &[xhtml("a", "a.xhtml"), r#"<item id="f" href="font.otf" media-type="font/otf"/>"#.into()].concat(),
        r#"><itemref idref="a"/>"#,
        &[
            ("META-INF/encryption.xml", encryption.as_bytes()),
            ("OEBPS/a.xhtml", &chapter("<p>A</p>")),
            ("OEBPS/font.otf", b"font"),
            ("OEBPS/x.otf", b"font"),
        ],
    )
}

const OBFUSCATION: [&str; 2] = ["http://www.idpf.org/2008/embedding", "http://ns.adobe.com/pdf/enc#RC"];

#[test]
fn encrypted_content_is_refused_and_obfuscated_fonts_are_not() {
    let drm = encrypted("http://www.w3.org/2001/04/xmlenc#aes128-cbc", "OEBPS/a.xhtml");
    assert_eq!(read_book(&drm, &BookOptions::default()), Err(Error::Encrypted { path: "OEBPS/a.xhtml".into() }));
    for algorithm in OBFUSCATION {
        let book = read(&encrypted(algorithm, "OEBPS/font.otf"));
        assert_eq!(text(&book.join(Boundary::Continuous)), ["A"], "{algorithm}");
    }
}

#[test]
fn font_obfuscation_exempts_only_what_the_manifest_lists_as_a_font() {
    // A chapter, and a file the manifest does not list, under a font obfuscation algorithm.
    for algorithm in OBFUSCATION {
        for path in ["OEBPS/a.xhtml", "OEBPS/x.otf"] {
            assert_eq!(
                read_book(&encrypted(algorithm, path), &BookOptions::default()),
                Err(Error::Encrypted { path: path.into() }),
                "{algorithm} {path}"
            );
        }
    }
}

#[test]
fn a_fixed_layout_book_is_read_and_flagged() {
    let book = Book {
        metadata: Metadata { identifier: "urn:uuid:2".into(), title: "Pages".into(), modified: "2026-10-13T09:30:00Z".into(), ..Metadata::default() },
        layout: Layout::PrePaginated { width: 600, height: 800 },
        resources: vec![Resource::new("p1", "p1.xhtml", "application/xhtml+xml", chapter("<p>Page</p>"))],
        spine: vec!["p1".into()],
        ..Book::default()
    };
    let book = read(&craft_epub::write(&book).unwrap());
    assert_eq!(book.warnings, [Warning::FixedLayout { path: "OEBPS/content.opf".into() }]);
    assert_eq!(text(&book.chapters[0].flow), ["Page"]);
}

/// Two chapters that both have a bookmark `n1`, and links between and out of them.
fn linked() -> BookFlow {
    let a = chapter(
        r##"<p><a id="n1"/>A <a href="b.xhtml#n1">to b</a> <a href="#n1">here</a> <a href="b.xhtml">b start</a> <a href="b.xhtml#nowhere">lost</a></p><p><a href="https://example.com/b.xhtml#n1">web</a> <a href="mailto:a@example.com">mail</a></p>"##,
    );
    let b = chapter(r##"<p>B</p><p><a name="n1"/>target <a href="../OEBPS/a.xhtml#n1">back</a></p>"##);
    let bytes = package(
        &[xhtml("a", "a.xhtml"), xhtml("b", "b.xhtml")].concat(),
        r#"><itemref idref="a"/><itemref idref="b"/>"#,
        &[("OEBPS/a.xhtml", &a), ("OEBPS/b.xhtml", &b)],
    );
    read(&bytes)
}

#[test]
fn ids_that_collide_across_chapters_stay_distinct() {
    let book = linked();
    let keys: Vec<_> = book.chapters.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["c1", "c2"]);
    let joined = book.join(Boundary::PageBreak);
    assert_eq!(anchors(&joined), ["c1", "c1-n1", "c2", "c2-n1"]);
}

#[test]
fn links_between_chapters_land_on_their_bookmarks() {
    let joined = linked().join(Boundary::Continuous);
    assert_eq!(
        links(&joined),
        [
            ("to b", "#c2-n1"),
            ("here", "#c1-n1"),
            ("b start", "#c2"),
            ("lost", "#c2"),
            ("web", "https://example.com/b.xhtml#n1"),
            ("mail", "mailto:a@example.com"),
            ("back", "#c1-n1"),
        ]
    );
    let names = anchors(&joined);
    for (_, link) in links(&joined) {
        if let Some(name) = link.strip_prefix('#') {
            assert!(names.contains(&name), "{link} has its bookmark");
        }
    }
}

#[test]
fn a_spine_item_that_is_not_xhtml_is_a_warning() {
    let bytes = package(
        &[xhtml("a", "a.xhtml"), r#"<item id="pic" href="pic.svg" media-type="image/svg+xml"/>"#.into()].concat(),
        r#"><itemref idref="pic"/><itemref idref="a"/>"#,
        &[("OEBPS/a.xhtml", &chapter("<p>A</p>")), ("OEBPS/pic.svg", b"<svg/>")],
    );
    let book = read(&bytes);
    assert_eq!(paths(&book), ["OEBPS/a.xhtml"]);
    assert_eq!(book.warnings, [Warning::NotXhtml { path: "OEBPS/pic.svg".into(), media_type: "image/svg+xml".into() }]);
}

#[test]
fn titles_come_from_an_epub_2_ncx() {
    let ncx = br#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
<navPoint><navLabel><text>Opening</text></navLabel><content src="a.xhtml"/></navPoint>
<navPoint><navLabel><text>Section</text></navLabel><content src="a.xhtml#s"/></navPoint>
<navPoint><navLabel><text>Closing</text></navLabel><content src="b.xhtml#top"/></navPoint>
</navMap></ncx>"#;
    let bytes = package(
        &[
            xhtml("a", "a.xhtml"),
            xhtml("b", "b.xhtml"),
            xhtml("c", "c.xhtml"),
            r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#.into(),
        ]
        .concat(),
        r#" toc="ncx"><itemref idref="a"/><itemref idref="b"/><itemref idref="c"/>"#,
        &[
            ("OEBPS/a.xhtml", &chapter("<p>A</p>")),
            ("OEBPS/b.xhtml", &chapter("<p>B</p>")),
            ("OEBPS/c.xhtml", &chapter("<p>C</p>")),
            ("OEBPS/toc.ncx", ncx),
        ],
    );
    let book = read(&bytes);
    let titles: Vec<_> = book.chapters.iter().map(|c| c.title.as_deref()).collect();
    assert_eq!(titles, [Some("Opening"), Some("Closing"), None]);
}

#[test]
fn chapters_past_the_cap_are_left_out() {
    let bytes = package(
        &[xhtml("a", "a.xhtml"), xhtml("b", "b.xhtml")].concat(),
        r#"><itemref idref="a"/><itemref idref="b"/>"#,
        &[("OEBPS/a.xhtml", &chapter("<p>A</p>")), ("OEBPS/b.xhtml", &chapter("<p>B</p>"))],
    );
    let book = read_book(&bytes, &BookOptions { max_chapters: 1, ..BookOptions::default() }).unwrap();
    assert_eq!(paths(&book), ["OEBPS/a.xhtml"]);
    assert_eq!(book.warnings, [Warning::ChaptersLeftOut { path: "OEBPS/b.xhtml".into() }]);
}

/// A chapter linking to a bookmark with a non-ASCII name, as written and percent-encoded, and
/// to a document that is not in the book.
fn escaped() -> BookFlow {
    let a = chapter(r##"<p><a href="b.xhtml#café">plain</a> <a href="b.xhtml#caf%C3%A9">escaped</a> <a href="gone.xhtml#x">gone</a></p>"##);
    let b = chapter(r##"<p>B</p><p><a id="café"/>target</p>"##);
    let nav = r##"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol>
<li><a href="a.xhtml">A</a></li><li><a href="b.xhtml#caf%C3%A9">Café</a></li><li><a href="b.xhtml#elsewhere">Elsewhere</a></li></ol></nav></body></html>"##;
    let bytes = package(
        &[
            xhtml("a", "a.xhtml"),
            xhtml("b", "b.xhtml"),
            r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#.into(),
        ]
        .concat(),
        r#"><itemref idref="a"/><itemref idref="b"/>"#,
        &[("OEBPS/a.xhtml", &a), ("OEBPS/b.xhtml", &b), ("OEBPS/nav.xhtml", nav.as_bytes())],
    );
    read(&bytes)
}

#[test]
fn percent_encoded_fragments_find_their_bookmark() {
    let joined = escaped().join(Boundary::Continuous);
    assert_eq!(links(&joined), [("plain", "#c2-café"), ("escaped", "#c2-café"), ("gone", "gone.xhtml#x")]);
}

#[test]
fn table_of_contents_entries_map_to_the_flows_bookmarks() {
    let book = escaped();
    let names = anchors(&book.join(Boundary::Continuous)).into_iter().map(str::to_string).collect::<Vec<_>>();
    let found: Vec<_> = book.toc.iter().map(|e| book.bookmark(e.path.as_deref().unwrap(), e.fragment.as_deref())).collect();
    assert_eq!(found, [Some("c1".to_string()), Some("c2-café".to_string()), Some("c2".to_string())]);
    for name in found.iter().flatten() {
        assert!(names.contains(name), "{name}");
    }
    assert_eq!(book.bookmark("OEBPS/b.xhtml", Some("café")).as_deref(), Some("c2-café"));
    assert_eq!(book.bookmark("OEBPS/gone.xhtml", None), None);
}

/// The numbering of each list item of `flow` once it is a document, in order.
fn list_numbers(flow: &Flow) -> Vec<u32> {
    craft_flow::to_doc(flow)
        .body
        .iter()
        .filter_map(|b| match &**b {
            craft_wordmodel::Block::Para(p) => p.props.numbering.map(|n| n.num),
            _ => None,
        })
        .collect()
}

#[test]
fn a_list_ending_one_chapter_does_not_continue_into_the_next() {
    let list = chapter("<ol><li>One</li><li>Two</li></ol>");
    let bytes = package(
        &[xhtml("a", "a.xhtml"), xhtml("b", "b.xhtml")].concat(),
        r#"><itemref idref="a"/><itemref idref="b"/>"#,
        &[("OEBPS/a.xhtml", &list), ("OEBPS/b.xhtml", &list)],
    );
    let book = read(&bytes);
    for at in [Boundary::PageBreak, Boundary::Continuous] {
        let numbers = list_numbers(&book.join(at));
        assert_eq!(numbers.len(), 4, "{at:?}");
        assert_eq!((numbers[0], numbers[2]), (numbers[1], numbers[3]), "{at:?}: each chapter's items stay one list");
        assert_ne!(numbers[1], numbers[2], "{at:?}: the second chapter starts its own list");
    }
}

#[test]
fn a_cut_off_encryption_list_refuses_the_book() {
    // 10,000 obfuscated fonts, then the chapter encrypted: a list read only up to its limit would
    // never see the last record.
    let font = r#"<enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/><enc:CipherData><enc:CipherReference URI="OEBPS/font.otf"/></enc:CipherData></enc:EncryptedData>"#;
    let drm = r#"<enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/><enc:CipherData><enc:CipherReference URI="OEBPS/a.xhtml"/></enc:CipherData></enc:EncryptedData>"#;
    let encryption = format!(
        r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">{}{drm}</encryption>"#,
        font.repeat(10_000)
    );
    let bytes = package(
        &[xhtml("a", "a.xhtml"), r#"<item id="f" href="font.otf" media-type="font/otf"/>"#.into()].concat(),
        r#"><itemref idref="a"/>"#,
        &[("META-INF/encryption.xml", encryption.as_bytes()), ("OEBPS/a.xhtml", &chapter("<p>A</p>")), ("OEBPS/font.otf", b"font")],
    );
    assert_eq!(read_book(&bytes, &BookOptions::default()), Err(Error::Encrypted { path: "META-INF/encryption.xml".into() }));
}

#[test]
fn a_navigation_document_in_the_spine_is_read_once() {
    // The navigation document and another chapter fit the budget once each, not the navigation
    // document twice.
    let nav = format!(
        r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="a.xhtml">A</a></li></ol></nav><p>{}</p></body></html>"#,
        "n".repeat(1_200)
    );
    let a = chapter(&format!("<p>{}</p>", "a".repeat(1_000)));
    let bytes = package(
        &[r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#.to_string(), xhtml("a", "a.xhtml")].concat(),
        r#"><itemref idref="nav"/><itemref idref="a"/>"#,
        &[("OEBPS/nav.xhtml", nav.as_bytes()), ("OEBPS/a.xhtml", &a)],
    );
    // What opening the package costs (its container and package document), then each document once.
    let opened = Limits::DEFAULT.max_total_bytes - craft_epub::Epub::open(&bytes).unwrap().budget_left();
    let once = opened + (nav.len() + a.len()) as u64;
    let opts = BookOptions { limits: Limits { max_total_bytes: once + nav.len() as u64 / 2, ..Limits::DEFAULT }, ..BookOptions::default() };
    let book = read_book(&bytes, &opts).unwrap();
    assert_eq!(paths(&book), ["OEBPS/nav.xhtml", "OEBPS/a.xhtml"]);
    assert_eq!(book.toc.len(), 1);
}

#[test]
fn an_image_the_reader_cannot_keep_is_a_warning() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
    let body = r#"<p>A<img src="pic.svg"/><img src="pic.png"/><img src="pic.svg"/></p>"#;
    let bytes = package(
        &[xhtml("a", "a.xhtml"), r#"<item id="cover" href="pic.svg" media-type="image/svg+xml" properties="cover-image"/>"#.into()].concat(),
        r#"><itemref idref="a"/>"#,
        &[("OEBPS/a.xhtml", &chapter(body)), ("OEBPS/pic.svg", svg), ("OEBPS/pic.png", &png("png"))],
    );
    let book = read(&bytes);
    assert_eq!(images(&book.chapters[0].flow).len(), 1, "the PNG is kept");
    assert_eq!(book.cover.as_deref().map(Vec::as_slice), Some(&svg[..]), "the cover's bytes are carried whatever their format");
    assert_eq!(book.warnings, [Warning::UnsupportedImage { path: "OEBPS/pic.svg".into(), from: "OEBPS/a.xhtml".into() }]);
}
