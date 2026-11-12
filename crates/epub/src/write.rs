//! A [`Book`] as an EPUB 3 package.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::Write as _;

use craft_xml::{esc, esc_attr, is_name};

use crate::text::{valid_language_tag, w3cdtf};
use crate::{Error, MIMETYPE};

/// Characters of the title the package keeps. A title is usually repeated in every content
/// document, so a huge one would multiply by the number of chapters or pages.
pub const MAX_TITLE_CHARS: usize = 1024;

/// Most resources one book may hold.
pub const MAX_RESOURCES: usize = 100_000;

/// Where the package keeps its own files, and every resource under it.
const ROOT: &str = "OEBPS";
const OPF: &str = "content.opf";
const NAV: &str = "nav.xhtml";
const NCX: &str = "toc.ncx";

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>\n";

/// The package metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    /// The unique identifier (`dc:identifier`), usually a `urn:uuid:` ([`crate::urn_uuid`]). Must
    /// not be blank.
    pub identifier: String,
    /// `dc:title`, cut to [`MAX_TITLE_CHARS`]. Must not be blank.
    pub title: String,
    /// A BCP 47 tag; one that is not ([`crate::valid_language_tag`]) is written as `und`.
    pub language: String,
    /// `dc:creator`, one element each; blank ones are left out.
    pub creators: Vec<String>,
    /// `dc:subject`, one element each; blank ones are left out.
    pub subjects: Vec<String>,
    /// `dc:description`, left out when blank.
    pub description: Option<String>,
    /// `dcterms:modified`: must be a date and time [`crate::w3cdtf`] accepts.
    pub modified: String,
    /// The writing application, as `<meta name="generator">`.
    pub generator: Option<String>,
}

/// Reflowable text, or one fixed page per spine item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Reflowable,
    /// `rendition:layout` `pre-paginated`; the size is the first page's, in CSS pixels.
    PrePaginated { width: u32, height: u32 },
}

/// A file of the book: a content document, style sheet, image or font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource<'a> {
    /// The manifest id: an XML name, unique in the book, not `nav`.
    pub id: String,
    /// Where the file goes, relative to the package document: `chapter1.xhtml`,
    /// `images/cover.png`. No `.` or `..` segments, no leading `/`, no backslash.
    pub path: String,
    pub media_type: String,
    pub data: Cow<'a, [u8]>,
    /// The manifest `properties` (`cover-image`, `mathml`, `svg`, `scripted`, …), space separated.
    pub properties: String,
    /// Store the file uncompressed (images and other data that is compressed already).
    pub stored: bool,
}

impl<'a> Resource<'a> {
    pub fn new(id: impl Into<String>, path: impl Into<String>, media_type: impl Into<String>, data: impl Into<Cow<'a, [u8]>>) -> Self {
        Resource { id: id.into(), path: path.into(), media_type: media_type.into(), data: data.into(), properties: String::new(), stored: false }
    }

    pub fn with_properties(mut self, properties: &str) -> Self {
        self.properties = properties.to_string();
        self
    }

    pub fn stored(mut self) -> Self {
        self.stored = true;
        self
    }
}

/// What the navigation document's list is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavKind {
    /// A table of contents (`epub:type="toc"`).
    #[default]
    Toc,
    /// A list of pages (`epub:type="page-list"`), for a fixed-layout book.
    PageList,
}

/// One navigation entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavItem {
    /// The target, relative to the package document, with its fragment: `chapter2.xhtml#s3`.
    /// Written as given.
    pub href: String,
    pub label: String,
    /// Nesting: an entry deeper than the one before it goes into that entry's own list; skipped
    /// levels count as one.
    pub level: u8,
}

/// The navigation document (`nav.xhtml`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nav {
    pub kind: NavKind,
    /// The list's heading ([`crate::contents_title`] for a table of contents).
    pub heading: String,
    /// Empty: one entry to the first spine item, labelled with the book's title.
    pub items: Vec<NavItem>,
}

/// A book to package.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book<'a> {
    pub metadata: Metadata,
    pub layout: Layout,
    /// The files, in the order they are listed in the manifest and stored in the archive.
    pub resources: Vec<Resource<'a>>,
    /// The reading order: resource ids.
    pub spine: Vec<String>,
    pub nav: Nav,
    /// Also write `toc.ncx`, the EPUB 2 table of contents, with the navigation's entries, and
    /// name it in the spine: reading systems from before EPUB 3 have no other one.
    pub ncx: bool,
}

/// A content document's shell around `body`: XML declaration, `html` with the XHTML and `epub`
/// namespaces and the language, a `head` with the character set and title, then `head`, and a
/// `body` element with `body_attrs`.
///
/// `lang` and `title` are escaped here. `head`, `body_attrs` (with its leading space) and `body`
/// are markup, written as given.
pub fn xhtml(lang: &str, title: &str, head: &str, body_attrs: &str, body: &str) -> String {
    let lang = language(lang);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{lang}\" lang=\"{lang}\">\n<head><meta charset=\"utf-8\"/><title>{}</title>{head}</head>\n<body{body_attrs}>\n{body}</body>\n</html>\n",
        esc(title)
    )
}

fn language(tag: &str) -> &str {
    if valid_language_tag(tag) { tag } else { "und" }
}

fn capped_title(title: &str) -> &str {
    match title.char_indices().nth(MAX_TITLE_CHARS) {
        Some((at, _)) => title.get(..at).unwrap_or(title),
        None => title,
    }
}

/// A resource path as a relative URL: everything but unreserved characters and `/` is
/// percent-encoded.
fn href(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn invalid<T>(msg: String) -> Result<T, Error> {
    Err(Error::Invalid(msg))
}

fn check(book: &Book) -> Result<(), Error> {
    if book.metadata.identifier.trim().is_empty() {
        return invalid("the identifier is blank".into());
    }
    if book.metadata.title.trim().is_empty() {
        return invalid("the title is blank".into());
    }
    if book.resources.len() > MAX_RESOURCES {
        return invalid(format!("{} resources (at most {MAX_RESOURCES})", book.resources.len()));
    }
    let mut ids = HashSet::from(["nav", "ncx"]);
    // Archive names compare without case on some systems, so two paths that differ only in case
    // are refused as well.
    let mut paths = HashSet::from([OPF.to_string(), NAV.to_string(), NCX.to_string()]);
    for r in &book.resources {
        if !is_name(&r.id) || r.id.contains(':') {
            return invalid(format!("resource id `{}` is not an XML name", r.id));
        }
        if !ids.insert(r.id.as_str()) {
            return invalid(format!("resource id `{}` is used twice", r.id));
        }
        let bad_path = r.path.is_empty() || r.path.contains('\\') || r.path.split('/').any(|s| matches!(s, "" | "." | ".."));
        if bad_path || r.path.chars().any(char::is_control) {
            return invalid(format!("resource path `{}` is not a relative path inside the package", r.path));
        }
        if !paths.insert(r.path.to_lowercase()) {
            return invalid(format!("resource path `{}` is used twice", r.path));
        }
        if r.media_type.trim().is_empty() {
            return invalid(format!("resource `{}` has no media type", r.id));
        }
    }
    if book.spine.is_empty() {
        return invalid("the spine is empty".into());
    }
    if let Some(missing) = book.spine.iter().find(|id| !ids.contains(id.as_str()) || matches!(id.as_str(), "nav" | "ncx")) {
        return invalid(format!("spine item `{missing}` is not a resource"));
    }
    Ok(())
}

fn package_document(book: &Book, modified: &str) -> String {
    let m = &book.metadata;
    let lang = language(&m.language);
    let fixed = matches!(book.layout, Layout::PrePaginated { .. });
    let prefix = if fixed { " prefix=\"rendition: http://www.idpf.org/vocab/rendition/#\"" } else { "" };
    let mut o = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"{lang}\"{prefix}>\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"bookid\">{}</dc:identifier>\n<dc:title>{}</dc:title>\n<dc:language>{lang}</dc:language>\n",
        esc(m.identifier.trim()),
        esc(capped_title(&m.title))
    );
    let mut dc = |name: &str, value: &str| {
        if !value.trim().is_empty() {
            let _ = writeln!(o, "<dc:{name}>{}</dc:{name}>", esc(value.trim()));
        }
    };
    m.creators.iter().for_each(|c| dc("creator", c));
    m.subjects.iter().for_each(|s| dc("subject", s));
    dc("description", m.description.as_deref().unwrap_or(""));
    let _ = writeln!(o, "<meta property=\"dcterms:modified\">{modified}</meta>");
    if let Layout::PrePaginated { width, height } = book.layout {
        let _ = writeln!(
            o,
            "<meta property=\"rendition:layout\">pre-paginated</meta>\n<meta property=\"rendition:spread\">auto</meta>\n<meta name=\"original-resolution\" content=\"{width}x{height}\"/>"
        );
    }
    if let Some(g) = m.generator.as_deref().filter(|g| !g.trim().is_empty()) {
        let _ = writeln!(o, "<meta name=\"generator\" content=\"{}\"/>", esc_attr(g.trim()));
    }
    o.push_str("</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    if book.ncx {
        o.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");
    }
    for r in &book.resources {
        let _ = write!(o, "<item id=\"{}\" href=\"{}\" media-type=\"{}\"", esc_attr(&r.id), href(&r.path), esc_attr(r.media_type.trim()));
        if !r.properties.trim().is_empty() {
            let _ = write!(o, " properties=\"{}\"", esc_attr(r.properties.trim()));
        }
        o.push_str("/>\n");
    }
    o.push_str(if book.ncx { "</manifest>\n<spine toc=\"ncx\">\n" } else { "</manifest>\n<spine>\n" });
    for id in &book.spine {
        let _ = writeln!(o, "<itemref idref=\"{}\"/>", esc_attr(id));
    }
    o.push_str("</spine>\n</package>\n");
    o
}

/// How nested entries are written: an entry's start, its end, and what goes around the entries
/// nested in it.
struct Nesting<'a> {
    start: &'a dyn Fn(usize, &NavItem) -> String,
    end: &'a str,
    sub_start: &'a str,
    sub_end: &'a str,
}

/// The entries nested by level: a deeper entry goes into the one before it.
fn nested(items: &[NavItem], n: &Nesting) -> String {
    let mut out = String::new();
    let mut open: Vec<u8> = Vec::new();
    for (k, item) in items.iter().enumerate() {
        let level = item.level;
        match open.last().copied() {
            None => open.push(level),
            Some(top) if level > top => {
                out.push_str(n.sub_start);
                open.push(level);
            }
            Some(_) => {
                // Close the levels whose parent is as deep as this entry or deeper.
                while open.len() > 1 && open.len().checked_sub(2).and_then(|i| open.get(i)).is_some_and(|parent| *parent >= level) {
                    open.pop();
                    out.push_str(n.end);
                    out.push_str(n.sub_end);
                }
                out.push_str(n.end);
                if let Some(t) = open.last_mut() {
                    *t = (*t).min(level);
                }
            }
        }
        out.push_str(&(n.start)(k, item));
    }
    for i in (0..open.len()).rev() {
        out.push_str(n.end);
        if i > 0 {
            out.push_str(n.sub_end);
        }
    }
    out
}

/// The navigation's entries; with none, one to the first spine item, labelled with the title.
fn nav_items<'b>(book: &'b Book) -> Cow<'b, [NavItem]> {
    if !book.nav.items.is_empty() {
        return Cow::Borrowed(&book.nav.items);
    }
    let path = book.spine.first().and_then(|id| book.resources.iter().find(|r| &r.id == id)).map(|r| href(&r.path)).unwrap_or_default();
    Cow::Owned(vec![NavItem { href: path, label: capped_title(&book.metadata.title).to_string(), level: 1 }])
}

fn nav_document(book: &Book) -> String {
    let kind = match book.nav.kind {
        NavKind::Toc => "toc",
        NavKind::PageList => "page-list",
    };
    let start = |_: usize, i: &NavItem| format!("<li><a href=\"{}\">{}</a>", esc_attr(&i.href), esc(&i.label));
    let list = nested(&nav_items(book), &Nesting { start: &start, end: "</li>", sub_start: "<ol>", sub_end: "</ol>" });
    let body = format!("<nav epub:type=\"{kind}\" id=\"toc\"><h1>{}</h1><ol>{list}</ol></nav>", esc(&book.nav.heading));
    xhtml(&book.metadata.language, capped_title(&book.metadata.title), "", "", &body)
}

/// The EPUB 2 table of contents (NCX): the same entries as nested `navPoint`s in reading order.
fn ncx_document(book: &Book) -> String {
    let start = |k: usize, i: &NavItem| {
        let n = k + 1;
        format!(
            "<navPoint id=\"np{n}\" playOrder=\"{n}\"><navLabel><text>{}</text></navLabel><content src=\"{}\"/>",
            esc(&i.label),
            esc_attr(&i.href)
        )
    };
    let items = nav_items(book);
    let depth = {
        // Nesting depth, as `nested` nests: the most levels open at once.
        let (mut open, mut deepest): (Vec<u8>, usize) = (Vec::new(), 1);
        for i in items.iter() {
            while open.last().is_some_and(|top| *top >= i.level) {
                open.pop();
            }
            open.push(i.level);
            deepest = deepest.max(open.len());
        }
        deepest
    };
    let points = nested(&items, &Nesting { start: &start, end: "</navPoint>", sub_start: "", sub_end: "" });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\" xml:lang=\"{}\">\n<head><meta name=\"dtb:uid\" content=\"{}\"/><meta name=\"dtb:depth\" content=\"{depth}\"/><meta name=\"dtb:totalPageCount\" content=\"0\"/><meta name=\"dtb:maxPageNumber\" content=\"0\"/></head>\n<docTitle><text>{}</text></docTitle>\n<navMap>{points}</navMap>\n</ncx>\n",
        language(&book.metadata.language),
        esc_attr(book.metadata.identifier.trim()),
        esc(capped_title(&book.metadata.title))
    )
}

/// `book` as an EPUB 3 package: the stored `mimetype`, `META-INF/container.xml`, then under
/// `OEBPS/` the package document, the navigation document, `toc.ncx` when [`Book::ncx`] asks for
/// it, and the resources in their order.
pub fn write(book: &Book) -> Result<Vec<u8>, Error> {
    check(book)?;
    let modified =
        w3cdtf(&book.metadata.modified).ok_or_else(|| Error::Invalid(format!("`{}` is not a modification date", book.metadata.modified)))?;
    let opf = package_document(book, &modified);
    let nav = nav_document(book);
    let ncx = book.ncx.then(|| ncx_document(book));
    let fail = |e: craft_zip::Error| Error::Invalid(e.to_string());
    let mut z = craft_zip::Writer::new();
    let mut own = vec![
        ("mimetype".to_string(), MIMETYPE.as_bytes(), true),
        ("META-INF/container.xml".to_string(), CONTAINER.as_bytes(), false),
        (format!("{ROOT}/{OPF}"), opf.as_bytes(), false),
        (format!("{ROOT}/{NAV}"), nav.as_bytes(), false),
    ];
    if let Some(ncx) = &ncx {
        own.push((format!("{ROOT}/{NCX}"), ncx.as_bytes(), false));
    }
    let resources = book.resources.iter().map(|r| (format!("{ROOT}/{}", r.path), r.data.as_ref(), r.stored));
    for (name, data, store) in own.into_iter().chain(resources) {
        z.add(&name, data, if store { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated }).map_err(fail)?;
    }
    z.finish().map_err(fail)
}
