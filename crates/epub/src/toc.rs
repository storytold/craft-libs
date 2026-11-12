//! The table of contents (EPUB 3 navigation document, EPUB 2 NCX) and `encryption.xml`, read.

use craft_xml::{El, Local, Options};

use crate::read::{dir_of, parse_xml, resolve};
use crate::{Error, Limits};

/// Most entries a table of contents gives; further ones are ignored.
pub const MAX_TOC_ENTRIES: usize = 100_000;

/// One entry of a table of contents, in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocEntry {
    pub label: String,
    /// The archive entry it points at; `None` for a heading that is not a link.
    pub path: Option<String>,
    /// The fragment of its link (`#s3` → `s3`).
    pub fragment: Option<String>,
    /// Nesting, from 1.
    pub level: u8,
}

fn entry(label: String, src: Option<&str>, base: &str, level: usize) -> TocEntry {
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    let fragment = src.and_then(|s| s.split_once('#')).map(|(_, f)| f.to_string()).filter(|f| !f.is_empty());
    // A link with no file part (`#s3`) points into the document that holds it.
    let path = src.map(|s| if s.split(['#', '?']).next().unwrap_or("").is_empty() { base.to_string() } else { resolve(dir_of(base), s) });
    let path = path.filter(|p| !p.is_empty());
    TocEntry { label, path, fragment, level: u8::try_from(level).unwrap_or(u8::MAX) }
}

/// Walks nested lists without recursing: `children` gives an item's nested items.
fn walk<'a>(top: Vec<&'a El>, children: impl Fn(&'a El) -> Vec<&'a El>, mut each: impl FnMut(&'a El, usize)) {
    let mut stack: Vec<(&El, usize)> = top.into_iter().rev().map(|e| (e, 1)).collect();
    let mut seen = 0;
    while let Some((item, level)) = stack.pop() {
        if seen == MAX_TOC_ENTRIES {
            return;
        }
        seen += 1;
        each(item, level);
        stack.extend(children(item).into_iter().rev().map(|e| (e, level + 1)));
    }
}

fn list_items(list: &El) -> Vec<&El> {
    list.children("li").collect()
}

fn nav_points(e: &El) -> Vec<&El> {
    e.children("navPoint").collect()
}

/// The table of contents of an EPUB 3 navigation document: the `nav` whose `epub:type` has
/// `toc` (else the first `nav`), its nested `ol`/`li` flattened with levels. `path` is the
/// navigation document's archive path, against which its links resolve.
///
/// The document is XHTML made by many tools, so it is read leniently.
pub fn parse_nav(xml: &[u8], path: &str) -> Result<Vec<TocEntry>, Error> {
    let root = Options::local().parse(xml).map_err(|e| Error::Part { part: path.to_string(), msg: e.to_string() })?;
    let is_toc = |n: &&El| n.attr_local("type").is_some_and(|t| t.split_ascii_whitespace().any(|t| t == "toc"));
    // `find` looks below the root; a navigation fragment may be the root itself.
    let navs = || std::iter::once(&root).filter(|r| r.is(Local("nav"))).chain(root.find_all(Local("nav")));
    let Some(nav) = navs().find(is_toc).or_else(|| navs().next()) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    walk(
        nav.child("ol").map(list_items).unwrap_or_default(),
        |li| li.child("ol").map(list_items).unwrap_or_default(),
        |li, level| {
            let Some(head) = li.child("a").or_else(|| li.child("span")) else { return };
            out.push(entry(head.deep_text(), head.attr("href"), path, level));
        },
    );
    Ok(out)
}

/// The table of contents of an EPUB 2 NCX: its `navMap`'s nested `navPoint`s flattened with
/// levels. `path` is the NCX's archive path.
pub fn parse_ncx(xml: &[u8], path: &str) -> Result<Vec<TocEntry>, Error> {
    let root = Options::local().parse(xml).map_err(|e| Error::Part { part: path.to_string(), msg: e.to_string() })?;
    let mut out = Vec::new();
    walk(root.child("navMap").map(nav_points).unwrap_or_default(), nav_points, |point, level| {
        let label = point.child("navLabel").map(El::deep_text).unwrap_or_default();
        out.push(entry(label, point.child_attr("content", "src"), path, level));
    });
    Ok(out)
}

/// A resource `META-INF/encryption.xml` lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encrypted {
    /// The archive entry (the `CipherReference` URI, which is relative to the archive's root).
    pub path: String,
    /// The `EncryptionMethod` algorithm URI.
    pub algorithm: String,
}

impl Encrypted {
    /// Font obfuscation (the IDPF or the Adobe algorithm): the font's first bytes are XORed with
    /// a key derived from the book's identifier. Not DRM, but the font is unusable as stored.
    pub fn is_font_obfuscation(&self) -> bool {
        matches!(self.algorithm.as_str(), "http://www.idpf.org/2008/embedding" | "http://ns.adobe.com/pdf/enc#RC")
    }
}

/// `META-INF/encryption.xml`: each `EncryptedData` with its algorithm and resource.
pub fn parse_encryption(xml: &[u8]) -> Result<Vec<Encrypted>, Error> {
    let root = parse_xml("META-INF/encryption.xml", xml, &Limits::DEFAULT)?;
    let one = |data: &El| {
        let path = resolve("", data.find(Local("CipherReference"))?.attr("URI")?);
        let algorithm = data.child_attr("EncryptionMethod", "Algorithm").unwrap_or_default().to_string();
        (!path.is_empty()).then_some(Encrypted { path, algorithm })
    };
    Ok(root.find_all(Local("EncryptedData")).filter_map(one).take(Limits::DEFAULT.max_manifest).collect())
}
