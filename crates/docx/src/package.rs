//! OPC package access: bounded zip reading, relationships, part names.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::DocxError;
use crate::xml::{self, El};

/// Largest single part we decompress.
pub const MAX_PART: u64 = 256 * 1024 * 1024;
/// Largest total of all decompressed parts.
pub const MAX_TOTAL: u64 = 512 * 1024 * 1024;
/// Most zip entries we look at.
pub const MAX_ENTRIES: usize = 20_000;

/// Relationship type suffixes (after `.../relationships/`).
pub mod rt {
    pub const OFFICE_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
    pub const CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
    pub const APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
    pub const CUSTOM: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";
    pub const CUSTOM_XML_PROPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps";
    pub const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
    pub const NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
    pub const SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
    pub const ATTACHED_TEMPLATE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate";
    pub const FONT_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable";
    pub const THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
    pub const FOOTNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
    pub const ENDNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";
    pub const COMMENTS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
    pub const COMMENTS_EX: &str = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
    pub const COMMENTS_IDS: &str = "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds";
    pub const COMMENTS_EXTENSIBLE: &str = "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible";
    pub const HEADER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
    pub const FOOTER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
    pub const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
    pub const HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    pub const VBA_PROJECT: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
    pub const CHART: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
    pub const DIAGRAM_DATA: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData";
    pub const DIAGRAM_DRAWING: &str = "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";
    pub const DIAGRAM_LAYOUT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramLayout";
    pub const DIAGRAM_QUICK_STYLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramQuickStyle";
    pub const DIAGRAM_COLORS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramColors";
    pub const CUSTOM_XML: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
    pub const GLOSSARY: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument";
    pub const AF_CHUNK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk";
}

/// Content type of a glossary document part (ECMA-376 Part 1 §11.3.8).
pub const GLOSSARY_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml";
/// `Document::passthrough` key: a fingerprint of `Document::glossary` as read, so a save can tell
/// an untouched glossary (written back verbatim) from an edited one (written from the model).
/// Not a package part: WordCraft's own bookkeeping.
pub const GLOSSARY_READ: &str = "wordcraft:glossary.read";
/// Most building blocks read from one glossary.
pub const MAX_GLOSSARY_ENTRIES: usize = 10_000;
/// The glossary entry types `w:type` takes (ST_DocPartType); others aren't kept.
pub const GLOSSARY_TYPES: [&str; 7] = ["none", "normal", "autoExp", "toolbar", "speller", "formFld", "bbPlcHdr"];
/// The insertion behaviours `w:behavior` takes (ST_DocPartBehavior); others aren't kept.
pub const GLOSSARY_BEHAVIORS: [&str; 3] = ["content", "p", "pg"];

/// A fingerprint of `glossary` (FNV-1a over its debug form): stable across runs and builds that
/// keep the model's shape, and cheap (nothing is allocated).
pub fn glossary_fingerprint(glossary: &[craft_wordmodel::BuildingBlock]) -> String {
    struct Fnv(u64);
    impl std::fmt::Write for Fnv {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for b in s.bytes() {
                self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
            Ok(())
        }
    }
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    // Writing into `Fnv` never fails.
    let _ = std::fmt::Write::write_fmt(&mut h, format_args!("{glossary:?}"));
    format!("{:016x}", h.0)
}

/// `Document::passthrough` key for a macro project, kept as opaque bytes (never parsed or run).
pub const VBA_PROJECT_PART: &str = "word/vbaProject.bin";
/// `Document::passthrough` key listing the parts the project relates to (VBA data, signatures…),
/// one `relationship type \t part name \t content type` line each; each part's bytes are kept
/// under its part name. Not a package part: WordCraft's own bookkeeping.
pub const VBA_RELATED: &str = "wordcraft:vbaProject.related";
/// Most parts we carry along with a macro project.
pub const MAX_VBA_RELATED: usize = 64;

/// `Document::passthrough` key listing the parts that objects read from a file refer to (charts,
/// SmartArt diagrams, OLE objects: see `craft_wordmodel::graphic::Embedded`), with their content
/// types and own relationships, as [`EmbeddedManifest`] lines. Each part's bytes are kept under its
/// part name. Not a package part: WordCraft's own bookkeeping.
pub const EMBEDDED_PARTS: &str = "wordcraft:embedded.parts";
/// The font table kept from a file (read by `Reader::keep_part_tree`), with the fonts it embeds.
pub const FONT_TABLE_PART: &str = "word/fontTable.xml";

/// The spelling the font table is stored under in `parts` (part names are case-insensitive, so a
/// package may spell it `WORD/FONTTABLE.XML`), preferring the exact [`FONT_TABLE_PART`].
pub fn font_table_key<V>(parts: &BTreeMap<String, V>) -> Option<&str> {
    if let Some((k, _)) = parts.get_key_value(FONT_TABLE_PART) {
        return Some(k.as_str());
    }
    parts.keys().find(|k| k.eq_ignore_ascii_case(FONT_TABLE_PART)).map(String::as_str)
}
/// Custom XML roots kept by the package (P) or main document (D): `owner\ttype\tpart`.
/// Their opaque bytes and related parts use the same bounded manifest as embedded objects.
pub const CUSTOM_XML_ROOTS: &str = "wordcraft:customXml.roots";
/// The original `word/settings.xml` when it holds settings the model doesn't (merged into on
/// save by `write::settings_merge`). Not a package part: WordCraft's own bookkeeping.
pub const SETTINGS_ORIG: &str = "wordcraft:settings.orig";
/// Extended properties (`docProps/app.xml`) the model doesn't hold but a save keeps
/// ([`APP_KEPT_PROPS`]): `name\tvalue` lines. Not a package part: WordCraft's own bookkeeping.
pub const APP_KEPT: &str = "wordcraft:app.kept";
/// The `docProps/app.xml` properties carried over from the file. Counts (pages, lines,
/// characters, editing time) and the producing application's version are not: they describe
/// the file Word saved, not the one WordCraft writes.
pub const APP_KEPT_PROPS: &[&str] =
    &["Template", "Manager", "Company", "HyperlinkBase", "ScaleCrop", "LinksUpToDate", "SharedDoc", "HyperlinksChanged"];
/// Parts WordCraft doesn't model, carried verbatim: `owner\ttype\tpart` lines (owner `P` for
/// the package, `D` for the main document), with their bytes and relationships in the
/// [`EmbeddedManifest`]. Not a package part: WordCraft's own bookkeeping.
pub const UNMODELLED_ROOTS: &str = "wordcraft:unmodelled.roots";
/// Most parts we carry along for a file's embedded objects.
pub const MAX_EMBEDDED_PARTS: usize = 4096;
/// Most relationships we keep for one such part.
pub const MAX_EMBEDDED_RELS: usize = 1024;
/// Longest markup we keep for one embedded object.
pub const MAX_EMBEDDED_XML: usize = 1024 * 1024;

/// One part an embedded object needs: its content type and its own relationships (internal
/// targets are part names).
#[derive(Clone, Debug, Default)]
pub struct EmbeddedPart {
    pub content_type: String,
    pub rels: Vec<Rel>,
}

/// The parts kept for embedded objects, by part name (see [`EMBEDDED_PARTS`]). Stored as text:
/// `P\tpart\tcontent type` per part, then `R\tpart\tid\ttype\ttarget\tI|E` per relationship of
/// that part (`E`: `target` is an external URL).
#[derive(Clone, Debug, Default)]
pub struct EmbeddedManifest {
    pub parts: BTreeMap<String, EmbeddedPart>,
}

/// Can `s` be a field of a manifest line?
pub fn manifest_field_ok(s: &str) -> bool {
    !s.is_empty() && !s.contains(['\t', '\r', '\n'])
}

impl EmbeddedManifest {
    pub fn parse(bytes: &[u8]) -> EmbeddedManifest {
        let mut m = EmbeddedManifest::default();
        for line in String::from_utf8_lossy(bytes).lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["P", part, ct] if m.parts.len() < MAX_EMBEDDED_PARTS => {
                    m.parts.entry((*part).to_string()).or_insert_with(|| EmbeddedPart { content_type: (*ct).to_string(), rels: Vec::new() });
                }
                ["R", part, id, kind, target, mode] => {
                    if let Some(p) = m.parts.get_mut(*part)
                        && p.rels.len() < MAX_EMBEDDED_RELS
                        && !p.rels.iter().any(|r| r.id == *id)
                    {
                        p.rels.push(Rel { id: (*id).to_string(), kind: (*kind).to_string(), target: (*target).to_string(), external: *mode == "E" });
                    }
                }
                _ => {}
            }
        }
        m
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = String::new();
        for (part, p) in &self.parts {
            if !manifest_field_ok(part) || !manifest_field_ok(&p.content_type) {
                continue;
            }
            s.push_str(&format!("P\t{part}\t{}\n", p.content_type));
        }
        for (part, p) in &self.parts {
            for r in p.rels.iter().filter(|r| [&r.id, &r.kind, &r.target].iter().all(|f| manifest_field_ok(f))) {
                s.push_str(&format!("R\t{part}\t{}\t{}\t{}\t{}\n", r.id, r.kind, r.target, if r.external { "E" } else { "I" }));
            }
        }
        s.into_bytes()
    }
}

/// Does relationship type `t` end with `suffix` (ignoring the transitional/strict prefix)?
pub fn rel_is(t: &str, full: &str) -> bool {
    let tail = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    t == full || tail(t) == tail(full)
}

/// The limits a DOCX package is read within.
pub const LIMITS: craft_opc::Limits = craft_opc::Limits::new(MAX_ENTRIES, MAX_PART, MAX_TOTAL);

fn opc_error(e: craft_opc::Error) -> DocxError {
    match e {
        craft_opc::Error::Zip(m) => DocxError::Zip(m),
        craft_opc::Error::Xml { part, error } if error.is_limit() => DocxError::Limit(format!("{part}: {error}")),
        craft_opc::Error::Xml { part, error } => DocxError::Xml(format!("{part}: {error}")),
        craft_opc::Error::Write(m) => DocxError::Zip(m),
        other => DocxError::Limit(other.to_string()),
    }
}

/// The decompressed parts of a package (bounded), read with `craft-opc`. Shared, so a part the
/// document keeps as it is (a picture, an embedded object) is handed over rather than copied.
pub struct Package {
    opc: craft_opc::Package,
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Package, DocxError> {
        Package::open_limited(bytes, MAX_PART, MAX_TOTAL)
    }

    /// Every part inflated within these limits: a part over `max_part`, or a package over
    /// `max_total` (counting entries that fail their checksum), is an error. A damaged entry is
    /// left out with a logged warning.
    pub fn open_limited(bytes: &[u8], max_part: u64, max_total: u64) -> Result<Package, DocxError> {
        let limits = craft_opc::Limits::new(MAX_ENTRIES, max_part, max_total);
        craft_opc::Package::open(bytes, &limits).map(|opc| Package { opc }).map_err(opc_error)
    }

    /// The bytes of all its parts.
    pub fn size(&self) -> u64 {
        self.opc.size()
    }

    /// A package from parts already in memory (a Flat OPC file, a converted document), by part
    /// name without the leading `/`.
    pub fn from_parts(files: BTreeMap<String, Vec<u8>>) -> Package {
        Package { opc: craft_opc::Package::from_parts(files) }
    }

    /// Part bytes by name (leading `/` ignored; case-insensitive).
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.opc.get(name)
    }

    /// [`Package::get`], as the shared bytes themselves: for a part the document keeps whole, which
    /// then costs no copy.
    pub fn get_shared(&self, name: &str) -> Option<&Arc<Vec<u8>>> {
        self.opc.get_shared(name)
    }

    /// [`Package::xml`] for the main document part: malformed XML is an error, not a truncated read.
    pub fn xml_strict(&self, name: &str) -> Result<Option<El>, DocxError> {
        match self.get(name) {
            Some(b) => xml::parse_strict(b).map(Some),
            None => Ok(None),
        }
    }

    pub fn xml(&self, name: &str) -> Result<Option<El>, DocxError> {
        match self.get(name) {
            Some(b) => xml::parse(b).map(Some),
            None => Ok(None),
        }
    }

    /// Relationships of `part` (from `dir/_rels/name.rels`). Internal targets name the part as
    /// stored, so a part reached under several spellings is kept, cached and written once.
    pub fn rels(&self, part: &str) -> Rels {
        Rels::from_opc(self.opc.rels(part))
    }
}

/// `[Content_Types].xml`: Override by part name, then Default by extension (both case-insensitive).
#[derive(Default)]
pub struct ContentTypes(craft_opc::ContentTypes);

impl ContentTypes {
    pub fn read(pkg: &Package) -> ContentTypes {
        ContentTypes(pkg.opc.content_types())
    }

    /// Content type of `part` (a package path without the leading `/`).
    pub fn of(&self, part: &str) -> Option<&str> {
        self.0.of(part)
    }
}

#[derive(Clone, Debug)]
pub struct Rel {
    pub id: String,
    /// The full relationship type URI.
    pub kind: String,
    /// Package path (internal) or URL (external).
    pub target: String,
    pub external: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Rels {
    pub list: Vec<Rel>,
    /// Each id's first relationship in `list`, so a lookup doesn't scan every relationship.
    ids: HashMap<String, usize>,
}

impl Rels {
    fn from_opc(r: craft_opc::Rels) -> Rels {
        let mut rels = Rels::default();
        for x in r.list {
            rels.ids.entry(x.id.clone()).or_insert(rels.list.len());
            rels.list.push(Rel { id: x.id, kind: x.ty, target: x.target, external: x.external });
        }
        rels
    }
    pub fn by_id(&self, id: &str) -> Option<&Rel> {
        self.ids.get(id).and_then(|&i| self.list.get(i))
    }
    pub fn by_type(&self, full: &str) -> Option<&Rel> {
        self.list.iter().find(|r| rel_is(&r.kind, full))
    }
}

pub use craft_opc::relative;

/// Build a zip from (name, bytes) entries.
pub fn zip_entries(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, DocxError> {
    craft_opc::zip_entries(entries).map_err(opc_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let e: Vec<(String, Vec<u8>)> = entries.iter().map(|(n, b)| (n.to_string(), b.clone())).collect();
        zip_entries(&e).unwrap()
    }

    #[test]
    fn a_package_declaring_too_many_entries_is_refused_before_zip_reads_them() {
        // #3660: a ZIP64 end record declaring more than MAX_ENTRIES.
        let bomb = craft_opc::zip64_entry_count_sample(MAX_ENTRIES as u64 + 1);
        assert!(matches!(Package::open(&bomb), Err(DocxError::Limit(e)) if e.contains("declares")));
        assert_eq!(crate::package_title(std::io::Cursor::new(bomb), 1 << 20), None);
        // A real package still opens, and its title still reads.
        let z = zip_of(&[(
            "docProps/core.xml",
            br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Plan</dc:title></cp:coreProperties>"#.to_vec(),
        )]);
        assert!(Package::open(&z).is_ok());
        assert_eq!(crate::package_title(std::io::Cursor::new(z), 1 << 20).as_deref(), Some("Plan"));
    }

    #[test]
    fn the_document_shares_its_pictures_with_the_package() {
        // Each picture was copied out of the package, so a picture document held its pictures
        // twice while it opened, and left the package's copies behind as freed memory.
        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
        let document = format!(
            r#"<w:document {ns}><w:body><w:p><w:r><w:drawing><wp:inline><wp:extent cx="635000" cy="635000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdImg"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p></w:body></w:document>"#
        );
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/></Relationships>"#;
        let z = zip_of(&[
            ("word/document.xml", document.into_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes().to_vec()),
            ("word/media/pic.png", b"\x89PNG picture bytes".to_vec()),
        ]);
        let pkg = Package::open(&z).unwrap();
        let doc = crate::read::read_opened(&pkg, None).unwrap();
        let held = doc.media.values().next().expect("the picture is read");
        assert!(Arc::ptr_eq(held, pkg.get_shared("word/media/pic.png").unwrap()), "the same bytes, not a copy");
        assert_eq!(pkg.get("/WORD/media/PIC.png"), Some(&b"\x89PNG picture bytes"[..]), "lookups are unchanged");
    }

    #[test]
    fn embedded_manifest_round_trips_and_ignores_junk() {
        let mut m = EmbeddedManifest::default();
        let rel = Rel { id: "rId1".into(), kind: "k".into(), target: "word/embeddings/x.xlsx".into(), external: false };
        let url = Rel { id: "rId2".into(), kind: "k".into(), target: "https://example.com/x".into(), external: true };
        m.parts.insert("word/charts/chart1.xml".into(), EmbeddedPart { content_type: "ct".into(), rels: vec![rel, url] });
        let back = EmbeddedManifest::parse(&m.to_bytes());
        let p = &back.parts["word/charts/chart1.xml"];
        assert_eq!((p.content_type.as_str(), p.rels.len(), p.rels[1].external), ("ct", 2, true));
        let junk = EmbeddedManifest::parse(b"R\tnope\trId1\tk\tt\tI\nP\tonly-two\n\t\t\n\xff\xfe");
        assert!(junk.parts.is_empty());
    }
}
