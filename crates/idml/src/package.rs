//! The ZIP container: bounded part reads, lenient part names, designmap includes, and the writer.

use std::collections::HashMap;

use craft_xml::El;

use crate::value::percent_decode;
use crate::{Error, MIMETYPE, Result, part};

/// What the reader refuses to exceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most entries a package may hold. Default 100,000.
    pub max_entries: usize,
    /// Most bytes one part may expand to. Default 512 MiB.
    pub max_part_bytes: u64,
    /// Most bytes all the parts read from one package may expand to together. Default 2 GiB.
    pub max_total_bytes: u64,
    /// The limits of one part's XML ([`crate::xml_limits`]).
    pub xml: craft_xml::Limits,
}

impl Limits {
    pub const DEFAULT: Limits = Limits { max_entries: 100_000, max_part_bytes: 512 << 20, max_total_bytes: 2 << 30, xml: part::XML_LIMITS };
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// An IDML package whose parts are decompressed only when asked for, within byte budgets that
/// don't trust the sizes the archive claims.
pub struct Package<'a> {
    zip: craft_zip::Archive<'a>,
    /// Normalised part name → entry name, first entry wins (zip order, so the choice is stable).
    lenient: HashMap<String, String>,
    limits: Limits,
}

/// `designmap.xml` with the parts it includes.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    /// The `Document` element as written: its attributes, and its children with each included
    /// part still an `idPkg:*` element.
    pub root: El,
    /// The children of `root` in order, with every `idPkg:*` include replaced by the children
    /// of its part's root element.
    pub top: Vec<El>,
    /// One line per include whose part the package lacks; those contents are missing from `top`.
    pub warnings: Vec<String>,
    /// `META-INF/metadata.xml` (an XMP packet), when the package has one.
    pub metadata: Option<String>,
}

impl<'a> Package<'a> {
    /// Opens `bytes` with [`Limits::DEFAULT`].
    pub fn open(bytes: &'a [u8]) -> Result<Self> {
        Self::open_with(bytes, Limits::DEFAULT)
    }

    /// The declared entry count is checked against [`Limits::max_entries`] before the archive
    /// is indexed.
    pub fn open_with(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        let zip =
            craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries as u64, max_total_bytes: limits.max_total_bytes })
                .map_err(|e| match e {
                    craft_zip::Error::TooManyEntries { declared, limit } => Error::TooManyEntries {
                        found: usize::try_from(declared).unwrap_or(usize::MAX),
                        limit: usize::try_from(limit).unwrap_or(usize::MAX),
                    },
                    e => Error::NotIdml(e.to_string()),
                })?;
        let mut lenient: HashMap<String, String> = HashMap::new();
        for name in zip.names().filter(|n| !n.ends_with('/')) {
            lenient.entry(normalize_part_name(&name)).or_insert_with(|| name.into_owned());
        }
        Ok(Package { zip, lenient, limits })
    }

    /// The bytes of part `name` (matched exactly, then leniently: writers differ in
    /// percent-encoding, case, path separators and a leading `./` or `/`), or `None` if the
    /// package has no such part.
    pub fn part(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        let exact = (self.zip.contains(name) && !name.ends_with('/')).then_some(name);
        let Some(stored) = exact.or_else(|| self.lenient.get(&normalize_part_name(name)).map(String::as_str)) else {
            return Ok(None);
        };
        let limit = self.limits.max_part_bytes.min(self.zip.budget().remaining());
        let stored = stored.to_string();
        self.zip.read(&stored, self.limits.max_part_bytes).map_err(|e| match e {
            craft_zip::Error::EntryTooLarge { .. } | craft_zip::Error::BudgetSpent { .. } => Error::TooLarge { part: stored.clone(), limit },
            craft_zip::Error::Entry { msg, .. } => Error::Part { part: stored.clone(), msg },
            e => Error::NotIdml(e.to_string()),
        })
    }

    /// Part `name` as an element tree ([`crate::parse_part`]).
    pub fn xml(&mut self, name: &str) -> Result<Option<El>> {
        let Some(data) = self.part(name)? else { return Ok(None) };
        let limits = self.limits.xml;
        part::parse_with(&data, limits).map(Some).map_err(|e| Error::Part { part: name.to_string(), msg: e.to_string() })
    }

    /// Bytes the parts still to be read may expand to.
    pub fn budget_left(&self) -> u64 {
        self.zip.budget().remaining()
    }

    /// Reads `designmap.xml` and every part it includes.
    ///
    /// A `mimetype` part that says something else, a missing `designmap.xml`, or one whose root
    /// is not `Document` is not an IDML package. A part that is listed and missing is a warning;
    /// one that is there and cannot be read is an error.
    pub fn document(&mut self) -> Result<Document> {
        if let Some(m) = self.part("mimetype")? {
            let m = String::from_utf8_lossy(&m);
            if m.trim() != MIMETYPE {
                return Err(Error::NotIdml(format!("unexpected mimetype `{}`", m.trim())));
            }
        }
        let root = self.xml("designmap.xml")?.ok_or_else(|| Error::NotIdml("missing designmap.xml".into()))?;
        if root.local() != "Document" {
            return Err(Error::NotIdml(format!("designmap root is <{}>", root.name)));
        }
        let mut top: Vec<El> = Vec::new();
        let mut warnings = Vec::new();
        for c in root.elements() {
            if c.name.starts_with("idPkg:") {
                let Some(src) = c.attr("src") else { continue };
                let Some(el) = self.xml(src)? else {
                    warnings.push(format!("{src}: listed in designmap.xml but missing from the package; its contents were not imported"));
                    continue;
                };
                top.extend(el.children.into_iter().filter_map(|n| match n {
                    craft_xml::Node::El(e) => Some(e),
                    _ => None,
                }));
            } else {
                top.push(c.clone());
            }
        }
        let metadata = self.part("META-INF/metadata.xml")?.map(|m| String::from_utf8_lossy(&m).into_owned());
        Ok(Document { root, top, warnings, metadata })
    }
}

/// A package part name in the form the lookup compares: InDesign and other IDML writers differ
/// in percent-encoding, case, path separators and a leading `./` or `/`.
fn normalize_part_name(name: &str) -> String {
    let decoded = percent_decode(&name.replace('\\', "/"));
    let mut rest = decoded.as_str();
    loop {
        if let Some(r) = rest.strip_prefix("./") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix('/') {
            rest = r;
        } else {
            break;
        }
    }
    rest.to_lowercase()
}

/// Zips an IDML package: the stored `mimetype` first, then `parts` in order as (name, bytes).
/// Equal parts give equal bytes.
pub fn write_package<N: AsRef<str>, D: AsRef<[u8]>>(parts: &[(N, D)]) -> Result<Vec<u8>> {
    use craft_zip::Compression;
    let fail = |e: craft_zip::Error| Error::Write(e.to_string());
    let mut w = craft_zip::Writer::new();
    w.add("mimetype", MIMETYPE.as_bytes(), Compression::Stored).map_err(fail)?;
    for (name, data) in parts {
        w.add(name.as_ref(), data.as_ref(), Compression::Deflated).map_err(fail)?;
    }
    w.finish().map_err(fail)
}
