//! Writing a package.

use std::collections::{HashMap, HashSet};

use craft_xml::Writer;

use crate::name::{normalize, rels_path};
use crate::{CT_RELATIONSHIPS, Error, NS_CONTENT_TYPES, NS_RELATIONSHIPS, RT_BASE};

/// How an entry is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Deflate,
    Store,
}

/// [`Compression::Store`] for a name whose extension says the data is already compressed
/// (pictures, video, audio, archives), which gains nothing from deflate; otherwise
/// [`Compression::Deflate`].
pub fn compression_for(name: &str) -> Compression {
    const PACKED: [&str; 19] =
        ["png", "jpg", "jpeg", "gif", "webp", "mp4", "m4v", "m4a", "mov", "mp3", "wmv", "wma", "avi", "ogg", "webm", "zip", "docx", "xlsx", "pptx"];
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    if PACKED.iter().any(|p| ext.eq_ignore_ascii_case(p)) { Compression::Store } else { Compression::Deflate }
}

/// A ZIP archive being written. The output is deterministic: deflate level 6 and the same
/// (1980) timestamp on every entry.
#[derive(Default)]
pub struct ZipOut {
    zip: craft_zip::Writer,
}

impl ZipOut {
    pub fn new() -> ZipOut {
        ZipOut::default()
    }

    /// Adds an entry. The caller keeps names unique.
    pub fn add(&mut self, name: &str, data: &[u8], compression: Compression) -> Result<(), Error> {
        let compression = match compression {
            Compression::Deflate => craft_zip::Compression::Deflated,
            Compression::Store => craft_zip::Compression::Stored,
        };
        self.zip.add(name, data, compression).map_err(crate::zip_error)
    }

    pub fn finish(self) -> Result<Vec<u8>, Error> {
        self.zip.finish().map_err(crate::zip_error)
    }
}

/// A ZIP archive of `entries` in order, each stored as [`compression_for`] its name says.
pub fn zip_entries(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, Error> {
    let mut zip = ZipOut::new();
    for (name, bytes) in entries {
        zip.add(name, bytes, compression_for(name))?;
    }
    zip.finish()
}

/// One relationship being written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelOut {
    pub id: String,
    /// The full relationship type URI.
    pub ty: String,
    pub target: String,
    pub external: bool,
}

/// The relationships being built for one part. Ids are `rId1`, `rId2`… in the order added.
#[derive(Clone, Debug, Default)]
pub struct RelsOut {
    pub items: Vec<RelOut>,
    /// (type, target, external) → id of the first relationship made for it.
    added: HashMap<(String, String, bool), String>,
}

fn full_type(kind: &str) -> String {
    if kind.starts_with("http") { kind.to_string() } else { format!("{RT_BASE}{kind}") }
}

impl RelsOut {
    /// Adds a relationship, or reuses the one already made for this type, target and mode.
    /// `kind` is a full type URI or a last segment of the standard namespace ([`RT_BASE`]).
    /// Returns its id.
    pub fn add(&mut self, kind: &str, target: &str, external: bool) -> String {
        let key = (full_type(kind), target.to_string(), external);
        match self.added.get(&key) {
            Some(id) => id.clone(),
            None => self.add_new(kind, target, external),
        }
    }

    /// Always adds a new relationship, even if an identical one exists. [`RelsOut::add`] reuses
    /// the first relationship for a target, whichever call made it.
    pub fn add_new(&mut self, kind: &str, target: &str, external: bool) -> String {
        let ty = full_type(kind);
        let id = format!("rId{}", self.items.len() + 1);
        self.added.entry((ty.clone(), target.to_string(), external)).or_insert_with(|| id.clone());
        self.items.push(RelOut { id: id.clone(), ty, target: target.to_string(), external });
        id
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The relationships part.
    pub fn to_xml(&self) -> Vec<u8> {
        let mut w = Writer::document();
        w.open("Relationships", &[("xmlns", NS_RELATIONSHIPS)]);
        for r in &self.items {
            w.tag("Relationship")
                .attr("Id", &r.id)
                .attr("Type", &r.ty)
                .attr("Target", &r.target)
                .attr_opt("TargetMode", r.external.then_some("External"))
                .empty();
        }
        w.into_bytes()
    }
}

/// A package being written: parts, their content types, and `[Content_Types].xml` as the first
/// entry.
pub struct PackageOut {
    files: Vec<(String, Vec<u8>, Compression)>,
    defaults: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
}

impl Default for PackageOut {
    fn default() -> PackageOut {
        PackageOut {
            files: Vec::new(),
            defaults: vec![("rels".into(), CT_RELATIONSHIPS.into()), ("xml".into(), "application/xml".into())],
            overrides: Vec::new(),
        }
    }
}

impl PackageOut {
    /// Adds an XML part with a content type Override.
    pub fn xml_part(&mut self, name: &str, content_type: &str, data: Vec<u8>) {
        self.overrides.push((format!("/{name}"), content_type.to_string()));
        self.files.push((name.to_string(), data, Compression::Deflate));
    }

    /// Adds (or replaces) the relationships part of `part`; of the package itself for `""`.
    /// Nothing is written for no relationships.
    pub fn rels(&mut self, part: &str, rels: &RelsOut) {
        if rels.is_empty() {
            return;
        }
        let path = rels_path(part);
        match self.files.iter_mut().find(|(name, ..)| *name == path) {
            Some(existing) => existing.1 = rels.to_xml(),
            None => self.files.push((path, rels.to_xml(), Compression::Deflate)),
        }
    }

    /// Adds a part with no content type entry of its own: one the `rels` and `xml` Defaults
    /// cover.
    pub fn raw(&mut self, name: &str, data: Vec<u8>) {
        self.files.push((name.to_string(), data, compression_for(name)));
    }

    /// Adds a binary part. Its extension gets a Default content type; when the extension
    /// already has another, the part gets an Override.
    pub fn binary(&mut self, name: &str, content_type: &str, data: Vec<u8>) {
        let ext = name.rsplit('.').next().unwrap_or("bin").to_ascii_lowercase();
        if !self.defaults.iter().any(|(e, _)| *e == ext) {
            self.defaults.push((ext, content_type.to_string()));
        } else if !self.defaults.iter().any(|(e, c)| *e == ext && c == content_type) {
            self.overrides.push((format!("/{name}"), content_type.to_string()));
        }
        self.files.push((name.to_string(), data, compression_for(name)));
    }

    /// `[Content_Types].xml` for what has been added.
    pub fn content_types_xml(&self) -> Vec<u8> {
        let mut w = Writer::document();
        w.open("Types", &[("xmlns", NS_CONTENT_TYPES)]);
        for (e, c) in &self.defaults {
            w.empty("Default", &[("Extension", e), ("ContentType", c)]);
        }
        for (p, c) in &self.overrides {
            w.empty("Override", &[("PartName", p), ("ContentType", c)]);
        }
        w.into_bytes()
    }

    /// The archive. A second part under a name already written (in any case) is left out with
    /// a logged warning: entry names must be unique, or readers reject the package or resolve
    /// it differently.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        let mut zip = ZipOut::new();
        zip.add("[Content_Types].xml", &self.content_types_xml(), Compression::Deflate)?;
        let mut written = HashSet::new();
        for (name, data, compression) in &self.files {
            if !written.insert(normalize(name)) {
                log::warn!("opc: part {name} written twice; keeping the first");
                continue;
            }
            zip.add(name, data, *compression)?;
        }
        zip.finish()
    }
}
