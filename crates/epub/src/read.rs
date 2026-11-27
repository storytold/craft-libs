//! An EPUB 2 or 3 package, read: container, package document, and bounded entry reads.

use std::collections::HashMap;

use craft_xml::{El, Local, Options};

use crate::Error;

/// What the reader refuses to exceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most entries the archive may hold. Default 100,000.
    pub max_entries: usize,
    /// Most bytes one entry may expand to. Default 64 MiB.
    pub max_entry_bytes: u64,
    /// Most bytes all the entries read from one package may expand to together (many entries
    /// with small declared sizes can't add up past this). Default 256 MiB.
    pub max_total_bytes: u64,
    /// Most manifest items kept; further ones are ignored. Default 10,000.
    pub max_manifest: usize,
    /// Most spine items kept; further ones are ignored. Default 10,000.
    pub max_spine: usize,
}

impl Limits {
    pub const DEFAULT: Limits =
        Limits { max_entries: 100_000, max_entry_bytes: 64 << 20, max_total_bytes: 256 << 20, max_manifest: 10_000, max_spine: 10_000 };
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// A manifest item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    /// The `href` as written.
    pub href: String,
    /// The archive entry the `href` names: resolved against the package document, decoded,
    /// fragment removed.
    pub path: String,
    pub media_type: String,
    /// EPUB 3 `properties` (`nav`, `cover-image`, `mathml`, …), space separated.
    pub properties: String,
}

impl Item {
    pub fn has_property(&self, property: &str) -> bool {
        self.properties.split_ascii_whitespace().any(|p| p == property)
    }
}

/// A spine entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpineItem {
    pub idref: String,
    /// `linear="no"`: auxiliary content, outside the main reading order.
    pub linear: bool,
    /// The archive entry of the manifest item it refers to; `None` when no item has that id.
    pub path: Option<String>,
}

/// The package document (OPF).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    /// The `version` attribute (`2.0`, `3.0`).
    pub version: String,
    /// The first `dc:title`.
    pub title: String,
    /// Every `dc:creator`, in order.
    pub creators: Vec<String>,
    /// The first `dc:language` that is not blank.
    pub language: Option<String>,
    /// The `dc:identifier` the package's `unique-identifier` names, else the first.
    pub identifier: Option<String>,
    /// `dcterms:modified` (EPUB 3).
    pub modified: Option<String>,
    /// `rendition:layout` is `pre-paginated`: a fixed-layout book.
    pub pre_paginated: bool,
    pub manifest: Vec<Item>,
    pub spine: Vec<SpineItem>,
    /// The archive entry of the EPUB 2 NCX the spine's `toc` attribute names.
    pub ncx: Option<String>,
    /// The id of the EPUB 2 cover image (`<meta name="cover">`).
    cover_id: Option<String>,
}

impl Package {
    /// The archive entries of the spine, in reading order, for the items the manifest has.
    pub fn spine_paths(&self) -> impl Iterator<Item = &str> {
        self.spine.iter().filter_map(|s| s.path.as_deref())
    }

    /// The EPUB 3 navigation document.
    pub fn nav(&self) -> Option<&Item> {
        self.manifest.iter().find(|i| i.has_property("nav"))
    }

    /// The cover image: the item with the `cover-image` property, else the one an EPUB 2
    /// `<meta name="cover">` names.
    pub fn cover_image(&self) -> Option<&Item> {
        let by_id = || self.cover_id.as_ref().and_then(|id| self.manifest.iter().find(|i| &i.id == id));
        self.manifest.iter().find(|i| i.has_property("cover-image")).or_else(by_id)
    }
}

/// The directory of an archive path (`a/b/c.xhtml` → `a/b`; `c.xhtml` → `""`).
pub fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The archive entry a relative `href` names from a file in `base_dir`: `.` and `..` segments
/// applied, percent escapes decoded, fragment and query removed.
///
/// A hostile `../../../etc/passwd` cannot leave the archive: the result only ever names an
/// archive entry, never a file system path, and `..` stops at the archive's root.
pub fn resolve(base_dir: &str, href: &str) -> String {
    let href = href.split(['#', '?']).next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if href.starts_with('/') { Vec::new() } else { base_dir.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Decode `%XX` escapes; an invalid escape, or a result that is not UTF-8, is kept as written.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |i: usize| b.get(i).and_then(|c| char::from(*c).to_digit(16));
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Both required documents must be well formed: a lenient read would take a container or package
/// document cut off after its useful content for a complete one, and give half a book.
pub(crate) fn parse_xml(part: &str, xml: &[u8], limits: &Limits) -> Result<El, Error> {
    let xml_limits = craft_xml::Limits { max_input: usize::try_from(limits.max_entry_bytes).unwrap_or(usize::MAX), ..craft_xml::Limits::DEFAULT };
    Options::local().strict().with_limits(xml_limits).parse(xml).map_err(|e| Error::Part { part: part.to_string(), msg: e.to_string() })
}

/// The package document's path from `META-INF/container.xml` (the first `rootfile`'s
/// `full-path`). `Ok(None)`: well formed, but it names none.
pub fn parse_container(xml: &[u8]) -> Result<Option<String>, Error> {
    let root = parse_xml("META-INF/container.xml", xml, &Limits::DEFAULT)?;
    Ok(root.find(Local("rootfile")).and_then(|r| r.attr("full-path")).filter(|p| !p.is_empty()).map(str::to_string))
}

/// The package document `xml`, its hrefs resolved against `opf_dir` (the directory of the
/// package document in the archive).
pub fn parse_package(xml: &[u8], opf_dir: &str, limits: &Limits) -> Result<Package, Error> {
    let root = parse_xml("package document", xml, limits)?;
    let mut pkg = Package { version: root.attr("version").unwrap_or_default().to_string(), ..Package::default() };
    let unique = root.attr("unique-identifier");
    if let Some(meta) = root.child("metadata") {
        // EPUB 2 packages may wrap the Dublin Core elements in `dc-metadata`.
        let all = || meta.elements().flat_map(|e| if e.is("dc-metadata") { e.elements().collect::<Vec<_>>() } else { vec![e] });
        let texts = |name: &'static str| all().filter(move |e| e.is(name)).map(|e| e.deep_text().trim().to_string());
        pkg.title = texts("title").next().unwrap_or_default();
        pkg.creators = texts("creator").filter(|c| !c.is_empty()).take(limits.max_manifest).collect();
        pkg.language = texts("language").find(|l| !l.is_empty());
        let ids = || all().filter(|e| e.is("identifier"));
        let chosen = ids().find(|e| unique.is_some() && e.attr("id") == unique).or_else(|| ids().next());
        pkg.identifier = chosen.map(|e| e.deep_text().trim().to_string()).filter(|i| !i.is_empty());
        for m in all().filter(|e| e.is("meta")) {
            let text = || m.deep_text().trim().to_string();
            match (m.attr("property"), m.attr("name")) {
                (Some("dcterms:modified"), _) if pkg.modified.is_none() => pkg.modified = Some(text()),
                (Some("rendition:layout"), _) => pkg.pre_paginated = text() == "pre-paginated",
                (_, Some("cover")) if pkg.cover_id.is_none() => pkg.cover_id = m.attr("content").map(str::to_string),
                _ => {}
            }
        }
    }
    let attr = |e: &El, name: &str| e.attr(name).unwrap_or_default().to_string();
    for item in root.child("manifest").into_iter().flat_map(|m| m.children("item")).take(limits.max_manifest) {
        let (Some(id), Some(href)) = (item.attr("id"), item.attr("href")) else { continue };
        pkg.manifest.push(Item {
            id: id.to_string(),
            href: href.to_string(),
            path: resolve(opf_dir, href),
            media_type: attr(item, "media-type"),
            properties: attr(item, "properties"),
        });
    }
    // The first item with an id wins, as it does when the id is looked up in the document.
    let mut by_id: HashMap<&str, &str> = HashMap::new();
    for item in &pkg.manifest {
        by_id.entry(&item.id).or_insert(&item.path);
    }
    if let Some(spine) = root.child("spine") {
        pkg.ncx = spine.attr("toc").and_then(|id| by_id.get(id)).map(ToString::to_string);
        for itemref in spine.children("itemref").take(limits.max_spine) {
            let Some(idref) = itemref.attr("idref") else { continue };
            let path = by_id.get(idref).map(ToString::to_string);
            pkg.spine.push(SpineItem { idref: idref.to_string(), linear: itemref.attr("linear") != Some("no"), path });
        }
    }
    Ok(pkg)
}

/// An open EPUB package. Entries are decompressed only when asked for, within the byte budgets
/// of its [`Limits`].
pub struct Epub<'a> {
    zip: craft_zip::Archive<'a>,
    limits: Limits,
    /// The archive path of the package document.
    pub opf_path: String,
    pub package: Package,
}

impl<'a> Epub<'a> {
    /// Opens `bytes` with [`Limits::DEFAULT`].
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        Self::open_with(bytes, Limits::DEFAULT)
    }

    /// Reads the container and the package document. The `mimetype` entry is not required:
    /// packages without one, or with it compressed, are common.
    pub fn open_with(bytes: &'a [u8], limits: Limits) -> Result<Self, Error> {
        let zip =
            craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries as u64, max_total_bytes: limits.max_total_bytes })
                .map_err(|e| match e {
                    craft_zip::Error::TooManyEntries { declared, limit } => Error::TooManyEntries {
                        found: usize::try_from(declared).unwrap_or(usize::MAX),
                        limit: usize::try_from(limit).unwrap_or(usize::MAX),
                    },
                    e => Error::NotEpub(e.to_string()),
                })?;
        let mut epub = Epub { zip, limits, opf_path: String::new(), package: Package::default() };
        let container = epub.read("META-INF/container.xml")?.ok_or_else(|| Error::NotEpub("META-INF/container.xml is missing".into()))?;
        let opf_path = parse_container(&container)?.ok_or_else(|| Error::NotEpub("container.xml names no package document".into()))?;
        // The container gives a path from the archive's root; resolving it also drops a leading `/`.
        let opf_path = resolve("", &opf_path);
        let opf = epub.read(&opf_path)?.ok_or_else(|| Error::NotEpub(format!("package document {opf_path} is missing")))?;
        epub.package = parse_package(&opf, dir_of(&opf_path), &limits).map_err(|e| match e {
            Error::Part { msg, .. } => Error::Part { part: opf_path.clone(), msg },
            e => e,
        })?;
        epub.opf_path = opf_path;
        Ok(epub)
    }

    /// Does the archive have an entry `path`?
    pub fn contains(&self, path: &str) -> bool {
        self.zip.contains(path)
    }

    /// The bytes of archive entry `path`, or `None` when the archive has no such entry. An entry
    /// past the limits is an error, and so is one that expands to more than it declared.
    pub fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        if path.ends_with('/') {
            return Ok(None);
        }
        let limit = self.limits.max_entry_bytes.min(self.zip.budget().remaining());
        self.zip.read(path, self.limits.max_entry_bytes).map_err(|e| match e {
            craft_zip::Error::EntryTooLarge { .. } | craft_zip::Error::BudgetSpent { .. } => Error::TooLarge { part: path.to_string(), limit },
            e => Error::Part { part: path.to_string(), msg: e.to_string() },
        })
    }

    /// [`Epub::read`] for what a file of the package refers to: `href` as written in the file at
    /// archive path `from` (an image's `src` in a content document).
    pub fn read_href(&mut self, from: &str, href: &str) -> Result<Option<Vec<u8>>, Error> {
        let path = resolve(dir_of(from), href);
        if path.is_empty() { Ok(None) } else { self.read(&path) }
    }

    /// The table of contents: the EPUB 3 navigation document's `toc`, else the EPUB 2 NCX.
    /// Empty when the package has neither, or the one it names is missing.
    pub fn toc(&mut self) -> Result<Vec<crate::TocEntry>, Error> {
        let nav = self.package.nav().map(|i| i.path.clone());
        if let Some(path) = nav
            && let Some(xml) = self.read(&path)?
        {
            let toc = crate::parse_nav(&xml, &path)?;
            if !toc.is_empty() {
                return Ok(toc);
            }
        }
        match self.package.ncx.clone() {
            Some(path) => match self.read(&path)? {
                Some(xml) => crate::parse_ncx(&xml, &path),
                None => Ok(Vec::new()),
            },
            None => Ok(Vec::new()),
        }
    }

    /// What `META-INF/encryption.xml` says is encrypted; empty when the package has none. A
    /// resource listed here cannot be used as it is read: fonts are usually only obfuscated
    /// ([`crate::Encrypted::is_font_obfuscation`]), anything else is DRM.
    pub fn encryption(&mut self) -> Result<Vec<crate::Encrypted>, Error> {
        match self.read("META-INF/encryption.xml")? {
            Some(xml) => crate::parse_encryption(&xml),
            None => Ok(Vec::new()),
        }
    }

    /// Bytes the entries still to be read may expand to.
    pub fn budget_left(&self) -> u64 {
        self.zip.budget().remaining()
    }
}
