//! Writing packages, and reading them back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;

use craft_epub::{Book, Epub, Error, Layout, MAX_TITLE_CHARS, Metadata, Nav, NavItem, NavKind, Resource, TocEntry, is_epub, write, xhtml};

fn metadata() -> Metadata {
    Metadata {
        identifier: "urn:uuid:1".into(),
        title: "Fish & <Chips>".into(),
        language: "en-GB".into(),
        creators: vec!["A & B".into(), "  ".into(), "C".into()],
        subjects: vec!["food".into()],
        description: Some("About \"fish\"".into()),
        modified: "2026-10-13T09:30:00Z".into(),
        generator: Some("Test \"1\"".into()),
    }
}

fn book() -> Book<'static> {
    let chapter =
        |n: u32| xhtml("en-GB", "Fish", "<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>", "", &format!("<p id=\"p{n}\">{n}</p>\n"));
    Book {
        metadata: metadata(),
        layout: Layout::Reflowable,
        resources: vec![
            Resource::new("css", "style.css", "text/css", b"p{margin:0}".to_vec()),
            Resource::new("content", "content.xhtml", "application/xhtml+xml", chapter(1).into_bytes()),
            Resource::new("content2", "content2.xhtml", "application/xhtml+xml", chapter(2).into_bytes()).with_properties("mathml"),
            Resource::new("img0", "images/a b#1.png", "image/png", vec![0x89, b'P', b'N', b'G']).with_properties("cover-image").stored(),
        ],
        spine: vec!["content".into(), "content2".into()],
        nav: Nav { kind: NavKind::Toc, heading: "Contents".into(), items: Vec::new() },
        ncx: false,
    }
}

fn entries(bytes: &[u8]) -> Vec<(String, zip::CompressionMethod, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let name = z.name_for_index(i).unwrap().unwrap().into_owned();
            let mut f = z.by_index(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            (name, f.compression(), data)
        })
        .collect()
}

fn text(bytes: &[u8], name: &str) -> String {
    String::from_utf8(entries(bytes).into_iter().find(|e| e.0 == name).unwrap().2).unwrap()
}

#[test]
fn the_package_starts_with_a_stored_mimetype_and_keeps_the_resource_order() {
    let bytes = write(&book()).unwrap();
    assert!(is_epub(&bytes));
    let e = entries(&bytes);
    let names: Vec<&str> = e.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "mimetype",
            "META-INF/container.xml",
            "OEBPS/content.opf",
            "OEBPS/nav.xhtml",
            "OEBPS/style.css",
            "OEBPS/content.xhtml",
            "OEBPS/content2.xhtml",
            "OEBPS/images/a b#1.png"
        ]
    );
    assert_eq!((e[0].1, e[0].2.as_slice()), (zip::CompressionMethod::Stored, &b"application/epub+zip"[..]));
    assert_eq!(e[2].1, zip::CompressionMethod::Deflated);
    assert_eq!(e[7].1, zip::CompressionMethod::Stored, "a resource marked stored is not compressed");
}

#[test]
fn the_package_document_carries_the_metadata_manifest_and_spine() {
    let bytes = write(&book()).unwrap();
    let opf = text(&bytes, "OEBPS/content.opf");
    for want in [
        "<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"en-GB\">",
        "<dc:identifier id=\"bookid\">urn:uuid:1</dc:identifier>",
        "<dc:title>Fish &amp; &lt;Chips&gt;</dc:title>",
        "<dc:language>en-GB</dc:language>",
        "<dc:creator>A &amp; B</dc:creator>\n<dc:creator>C</dc:creator>",
        "<dc:subject>food</dc:subject>",
        "<dc:description>About \"fish\"</dc:description>",
        "<meta property=\"dcterms:modified\">2026-10-13T09:30:00Z</meta>",
        "<meta name=\"generator\" content=\"Test &quot;1&quot;\"/>",
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>",
        "<item id=\"content2\" href=\"content2.xhtml\" media-type=\"application/xhtml+xml\" properties=\"mathml\"/>",
        "<item id=\"img0\" href=\"images/a%20b%231.png\" media-type=\"image/png\" properties=\"cover-image\"/>",
        "<spine>\n<itemref idref=\"content\"/>\n<itemref idref=\"content2\"/>\n</spine>",
    ] {
        assert!(opf.contains(want), "missing {want}\n{opf}");
    }
    assert!(!opf.contains("rendition:"), "a reflowable book says nothing about rendition");
    craft_xml::Options::default().strict().parse(opf.as_bytes()).expect("well formed");
}

#[test]
fn what_is_written_reads_back() {
    let bytes = write(&book()).unwrap();
    let mut epub = Epub::open(&bytes).unwrap();
    let p = epub.package.clone();
    assert_eq!((p.version.as_str(), p.title.as_str(), p.language.as_deref()), ("3.0", "Fish & <Chips>", Some("en-GB")));
    assert_eq!(p.creators, ["A & B", "C"]);
    assert_eq!(p.identifier.as_deref(), Some("urn:uuid:1"));
    assert_eq!(p.modified.as_deref(), Some("2026-10-13T09:30:00Z"));
    assert!(!p.pre_paginated);
    assert_eq!(p.spine_paths().collect::<Vec<_>>(), ["OEBPS/content.xhtml", "OEBPS/content2.xhtml"]);
    assert_eq!(p.nav().map(|i| i.path.as_str()), Some("OEBPS/nav.xhtml"));
    let cover = p.cover_image().unwrap();
    assert_eq!(cover.path, "OEBPS/images/a b#1.png", "the percent-encoded href names the entry");
    assert_eq!(epub.read(&cover.path).unwrap().unwrap(), [0x89, b'P', b'N', b'G']);
}

#[test]
fn an_empty_navigation_points_at_the_first_spine_item() {
    let nav = text(&write(&book()).unwrap(), "OEBPS/nav.xhtml");
    assert!(
        nav.contains(
            "<nav epub:type=\"toc\" id=\"toc\"><h1>Contents</h1><ol><li><a href=\"content.xhtml\">Fish &amp; &lt;Chips&gt;</a></li></ol></nav>"
        ),
        "{nav}"
    );
    craft_xml::Options::default().strict().parse(nav.as_bytes()).expect("well formed");
}

#[test]
fn navigation_nests_by_level() {
    let item = |href: &str, label: &str, level| NavItem { href: href.into(), label: label.into(), level };
    let mut b = book();
    b.nav.items = vec![
        item("content.xhtml#a", "One", 1),
        item("content.xhtml#b", "One.one", 2),
        item("content.xhtml#c", "One.one.one <x>", 3),
        item("content2.xhtml#d", "Two", 1),
        // A skipped level counts as one.
        item("content2.xhtml#e", "Two.deep", 3),
        item("content2.xhtml#f", "Two.next", 2),
    ];
    let nav = text(&write(&b).unwrap(), "OEBPS/nav.xhtml");
    let want = "<ol><li><a href=\"content.xhtml#a\">One</a><ol><li><a href=\"content.xhtml#b\">One.one</a><ol><li><a href=\"content.xhtml#c\">One.one.one &lt;x&gt;</a></li></ol></li></ol></li><li><a href=\"content2.xhtml#d\">Two</a><ol><li><a href=\"content2.xhtml#e\">Two.deep</a></li><li><a href=\"content2.xhtml#f\">Two.next</a></li></ol></li></ol>";
    assert!(nav.contains(want), "{nav}");
    craft_xml::Options::default().strict().parse(nav.as_bytes()).expect("well formed");
}

#[test]
fn a_fixed_layout_book_declares_its_rendition_and_lists_pages() {
    let mut b = book();
    b.layout = Layout::PrePaginated { width: 612, height: 792 };
    b.nav =
        Nav { kind: NavKind::PageList, heading: "Pages".into(), items: vec![NavItem { href: "content.xhtml".into(), label: "1".into(), level: 1 }] };
    let bytes = write(&b).unwrap();
    let opf = text(&bytes, "OEBPS/content.opf");
    for want in [
        " prefix=\"rendition: http://www.idpf.org/vocab/rendition/#\">",
        "<meta property=\"rendition:layout\">pre-paginated</meta>",
        "<meta property=\"rendition:spread\">auto</meta>",
        "<meta name=\"original-resolution\" content=\"612x792\"/>",
    ] {
        assert!(opf.contains(want), "missing {want}\n{opf}");
    }
    assert!(
        text(&bytes, "OEBPS/nav.xhtml")
            .contains("<nav epub:type=\"page-list\" id=\"toc\"><h1>Pages</h1><ol><li><a href=\"content.xhtml\">1</a></li></ol></nav>")
    );
    assert!(Epub::open(&bytes).unwrap().package.pre_paginated);
}

#[test]
fn a_huge_title_is_capped() {
    let mut b = book();
    b.metadata.title = "é".repeat(MAX_TITLE_CHARS + 500);
    let bytes = write(&b).unwrap();
    assert_eq!(Epub::open(&bytes).unwrap().package.title.chars().count(), MAX_TITLE_CHARS);
    assert!(!text(&bytes, "OEBPS/nav.xhtml").contains(&"é".repeat(MAX_TITLE_CHARS + 1)));
}

#[test]
fn a_language_that_is_not_a_tag_is_written_as_undetermined() {
    let mut b = book();
    b.metadata.language = "en\"><script>".into();
    let bytes = write(&b).unwrap();
    assert!(text(&bytes, "OEBPS/content.opf").contains("xml:lang=\"und\""));
    assert!(text(&bytes, "OEBPS/nav.xhtml").contains("xml:lang=\"und\" lang=\"und\""));
    assert!(
        xhtml("x y", "T", "", " epub:type=\"cover\"", "")
            .contains("lang=\"und\">\n<head><meta charset=\"utf-8\"/><title>T</title></head>\n<body epub:type=\"cover\">\n</body>")
    );
}

#[test]
fn books_that_cannot_be_packages_are_refused() {
    let refused = |change: &dyn Fn(&mut Book<'static>), why: &str| {
        let mut b = book();
        change(&mut b);
        match write(&b) {
            Err(Error::Invalid(msg)) => assert!(msg.contains(why), "{msg} (wanted {why})"),
            other => panic!("{why}: {:?}", other.map(|b| b.len())),
        }
    };
    refused(&|b| b.metadata.identifier = " ".into(), "identifier");
    refused(&|b| b.metadata.title = String::new(), "title");
    refused(&|b| b.metadata.modified = "yesterday".into(), "modification date");
    refused(&|b| b.spine.clear(), "spine is empty");
    refused(&|b| b.spine.push("missing".into()), "`missing`");
    refused(&|b| b.spine.push("nav".into()), "`nav`");
    refused(&|b| b.resources[0].id = "nav".into(), "used twice");
    refused(&|b| b.resources[0].id = "content".into(), "used twice");
    refused(&|b| b.resources[0].id = "a b".into(), "not an XML name");
    refused(&|b| b.resources[0].id = "\"><x".into(), "not an XML name");
    refused(&|b| b.resources[0].path = "CONTENT.xhtml".into(), "used twice");
    refused(&|b| b.resources[0].path = "nav.xhtml".into(), "used twice");
    refused(&|b| b.resources[0].path = "Content.OPF".into(), "used twice");
    for path in ["", "/abs.css", "../mimetype", "a/../b.css", "a//b.css", "a\\b.css", "./a.css", "a/", "a\nb"] {
        refused(&|b| b.resources[0].path = path.into(), "not a relative path");
    }
    refused(&|b| b.resources[0].media_type = " ".into(), "media type");
}

#[test]
fn more_resources_than_the_limit_are_refused() {
    let mut b = book();
    b.resources = (0..=craft_epub::MAX_RESOURCES).map(|i| Resource::new(format!("r{i}"), format!("r{i}"), "text/plain", Vec::new())).collect();
    assert!(matches!(write(&b), Err(Error::Invalid(msg)) if msg.contains("at most")));
}

fn nested_items() -> Vec<NavItem> {
    let item = |href: &str, label: &str, level| NavItem { href: href.into(), label: label.into(), level };
    vec![
        item("content.xhtml#a", "One", 1),
        item("content.xhtml#b", "One.one", 2),
        item("content.xhtml#c", "One.one.one <x>", 3),
        item("content2.xhtml", "Two", 1),
        item("images/a%20b%231.png", "Two.picture", 2),
    ]
}

fn toc_entry(label: &str, path: &str, fragment: Option<&str>, level: u8) -> TocEntry {
    TocEntry { label: label.into(), path: Some(path.into()), fragment: fragment.map(str::to_string), level }
}

fn nested_toc() -> Vec<TocEntry> {
    vec![
        toc_entry("One", "OEBPS/content.xhtml", Some("a"), 1),
        toc_entry("One.one", "OEBPS/content.xhtml", Some("b"), 2),
        toc_entry("One.one.one <x>", "OEBPS/content.xhtml", Some("c"), 3),
        toc_entry("Two", "OEBPS/content2.xhtml", None, 1),
        toc_entry("Two.picture", "OEBPS/images/a b#1.png", None, 2),
    ]
}

#[test]
fn the_navigation_reads_back_as_a_table_of_contents() {
    let mut b = book();
    b.nav.items = nested_items();
    let bytes = write(&b).unwrap();
    assert!(!entries(&bytes).iter().any(|e| e.0.ends_with(".ncx")), "no NCX unless asked for");
    assert_eq!(Epub::open(&bytes).unwrap().toc().unwrap(), nested_toc());
}

#[test]
fn an_ncx_is_written_on_request_and_reads_back() {
    let mut b = book();
    b.nav.items = nested_items();
    b.ncx = true;
    let bytes = write(&b).unwrap();
    let opf = text(&bytes, "OEBPS/content.opf");
    assert!(opf.contains("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>"), "{opf}");
    assert!(opf.contains("<spine toc=\"ncx\">"), "{opf}");
    let ncx = text(&bytes, "OEBPS/toc.ncx");
    for want in [
        "<meta name=\"dtb:uid\" content=\"urn:uuid:1\"/><meta name=\"dtb:depth\" content=\"3\"/>",
        "<docTitle><text>Fish &amp; &lt;Chips&gt;</text></docTitle>",
        "<navMap><navPoint id=\"np1\" playOrder=\"1\"><navLabel><text>One</text></navLabel><content src=\"content.xhtml#a\"/><navPoint id=\"np2\" playOrder=\"2\">",
        "</navPoint></navPoint></navPoint><navPoint id=\"np4\" playOrder=\"4\">",
    ] {
        assert!(ncx.contains(want), "missing {want}\n{ncx}");
    }
    craft_xml::Options::default().strict().parse(ncx.as_bytes()).expect("well formed");
    let epub = Epub::open(&bytes).unwrap();
    assert_eq!(epub.package.ncx.as_deref(), Some("OEBPS/toc.ncx"));
    assert_eq!(craft_epub::parse_ncx(ncx.as_bytes(), "OEBPS/toc.ncx").unwrap(), nested_toc());
    // `ncx` and `toc.ncx` are the writer's own.
    let mut taken = book();
    taken.resources[0].id = "ncx".into();
    assert!(matches!(write(&taken), Err(Error::Invalid(m)) if m.contains("used twice")));
    let mut taken = book();
    taken.resources[0].path = "TOC.ncx".into();
    assert!(matches!(write(&taken), Err(Error::Invalid(m)) if m.contains("used twice")));
}
