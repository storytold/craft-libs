# craft-epub

EPUB packages, written and read. No `unsafe`.

An EPUB is an OCF ZIP package: an uncompressed `mimetype` entry first, `META-INF/container.xml`
naming the package document (OPF), which lists every resource in its manifest and the reading
order in its spine, and (EPUB 3) a navigation document.

| Document | Used for |
|---|---|
| EPUB 3.3 (W3C Recommendation) | package document, navigation document, fixed-layout metadata |
| EPUB OCF 3.3 | the ZIP container, `mimetype`, `container.xml` |
| OPF 2.0.1 | EPUB 2 packages: `spine toc`, `<meta name="cover">`, `dc-metadata` |

The crate packages content documents and finds them again. It does not turn a document model
into XHTML or XHTML into a document model: that is the application's side.

## Writing

```rust
use craft_epub::{Book, Metadata, Nav, NavItem, Resource};

let chapter = craft_epub::xhtml("en", "A Book", "", "", "<h1 id=\"c1\">One</h1>\n");
let book = Book {
    metadata: Metadata {
        identifier: craft_epub::urn_uuid("A Book\n1760000000"),
        title: "A Book".into(),
        language: "en".into(),
        creators: vec!["An Author".into()],
        modified: "2026-10-13T09:30:00Z".into(),
        ..Metadata::default()
    },
    resources: vec![Resource::new("content", "content.xhtml", "application/xhtml+xml", chapter.into_bytes())],
    spine: vec!["content".into()],
    nav: Nav {
        heading: craft_epub::contents_title("en").into(),
        items: vec![NavItem { href: "content.xhtml#c1".into(), label: "One".into(), level: 1 }],
        ..Nav::default()
    },
    ..Book::default()
};
let bytes = craft_epub::write(&book)?;
assert!(craft_epub::is_epub(&bytes));
# Ok::<(), craft_epub::Error>(())
```

- The archive holds `mimetype` (stored), `META-INF/container.xml`, then under `OEBPS/` the package
  document `content.opf`, the navigation document `nav.xhtml`, and the resources in their order.
- `Layout::PrePaginated` writes the fixed-layout metadata; `NavKind::PageList` makes the
  navigation a page list.
- `Book::ncx` also writes `toc.ncx`, the EPUB 2 table of contents, for older reading systems.
- Navigation entries nest by `level`. With no entries the navigation has one, to the first spine
  item, because an empty list is not valid.
- Resource paths become percent-encoded hrefs. A resource marked `stored()` is not compressed
  (images, fonts that are compressed already).
- Refused with `Error::Invalid`: a blank identifier or title, a modification date `w3cdtf` does
  not accept, an empty spine or one naming no resource, an id that is not an XML name or is used
  twice, a path that is absolute, has `.`/`..`/empty segments, a backslash or a control
  character, or is used twice (compared without case), and more than `MAX_RESOURCES` resources.
- A language that is not a BCP 47 tag is written as `und`; the title is cut to `MAX_TITLE_CHARS`.

## Reading

```rust
# fn main() -> Result<(), craft_epub::Error> {
# let bytes: Vec<u8> = Vec::new();
# if bytes.is_empty() { return Ok(()); }
let mut epub = craft_epub::Epub::open(&bytes)?;
let title = epub.package.title.clone();
let spine: Vec<String> = epub.package.spine_paths().map(str::to_string).collect();
for path in &spine {
    if let Some(xhtml) = epub.read(path)? {
        // An image the chapter refers to, resolved against the chapter's own path:
        let _image = epub.read_href(path, "../images/figure.png")?;
    }
}
# Ok(()) }
```

- EPUB 2 and 3 packages. `Package` has the version, title, creators, language, identifier,
  modification date, fixed-layout flag, manifest, spine (with `linear`), the EPUB 2 NCX path, and
  `nav()` and `cover_image()`.
- `Epub::toc()` gives the table of contents as entries with a label, the archive entry and
  fragment they point at, and a level: from the EPUB 3 navigation document, else the EPUB 2 NCX
  (`parse_nav`, `parse_ncx`). At most `MAX_TOC_ENTRIES` (100,000) entries.
- `Epub::encryption()` lists what `META-INF/encryption.xml` says is encrypted, and tells font
  obfuscation from DRM. This crate does not decrypt or de-obfuscate.
- `container.xml` and the package document are read strictly: one that is cut off or whose tags
  don't match is an error, not half a book.
- A missing entry is `Ok(None)`: a spine item whose file is gone is the caller's to skip.
- `resolve` applies `.` and `..`, decodes percent escapes and drops fragments. Its result only
  names an archive entry, so `../../etc/passwd` cannot leave the archive.

### Limits

| `Limits` field | Default | Past it |
|---|---|---|
| `max_entries` | 100,000 | `Error::TooManyEntries` |
| `max_entry_bytes` | 64 MiB | `Error::TooLarge` |
| `max_total_bytes` (all entries read from one package) | 256 MiB | `Error::TooLarge` |
| `max_manifest`, `max_spine` | 10,000 each | further items are ignored |

The ZIP layer is craft-zip: the declared entry count is checked before the archive is indexed, and
sizes are checked against what an entry really expands to, not what the archive declares. The two
XML documents are read with craft-xml's limits (depth 256, 2,000,000 nodes) and `max_entry_bytes`.

## Where it came from

The writer is DesignCraft's EPUB export packaging (`crates/epub/src/package.rs`: container,
package document, nested navigation, fixed layout, identifier), with the metadata, date and
language checks of WordCraft's EPUB export. The reader is WordCraft's EPUB import container layer
(container, package document, href resolution, entry budgets), moved onto craft-xml and extended
to EPUB 2 packages.
