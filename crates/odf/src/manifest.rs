//! The manifest (`META-INF/manifest.xml`, ODF 1.3 Part 2 section 4): each file's path and
//! media type, and how an encrypted file is encrypted.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use craft_xml::{Canonical, Doctype, El, Options, esc_attr};

use crate::Error;

/// The manifest's path in a package.
pub const MANIFEST_PATH: &str = "META-INF/manifest.xml";
/// The manifest's XML namespace.
pub const MANIFEST_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";

/// Most file entries read from a manifest (as many as a package may hold by default).
pub const MAX_ENTRIES: usize = 20_000;

/// How one file is encrypted (a `manifest:encryption-data`, section 4.4 to 4.7).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Encryption {
    /// `manifest:checksum-type` (`SHA1/1K` when absent).
    pub checksum_type: String,
    /// `manifest:checksum`, decoded.
    pub checksum: Vec<u8>,
    /// `manifest:algorithm-name`.
    pub algorithm: String,
    /// `manifest:initialisation-vector`, decoded.
    pub iv: Vec<u8>,
    /// `manifest:start-key-generation-name` (`SHA1` when absent).
    pub start_key: String,
    /// The start key's `manifest:key-size`.
    pub start_key_size: Option<usize>,
    /// `manifest:key-derivation-name`.
    pub derivation: String,
    /// `manifest:iteration-count` (1024 when absent).
    pub iterations: u32,
    /// `manifest:salt`, decoded.
    pub salt: Vec<u8>,
    /// The derived key's `manifest:key-size`.
    pub key_size: Option<usize>,
}

/// One `manifest:file-entry`.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// `manifest:full-path`: `/` is the package itself, a path ending in `/` a folder (a
    /// sub-document).
    pub path: String,
    /// `manifest:media-type` (may be empty).
    pub media_type: String,
    /// `manifest:version`.
    pub version: Option<String>,
    /// `manifest:size`: the file's size before encryption.
    pub size: Option<u64>,
    /// Present when the file is encrypted.
    pub encryption: Option<Encryption>,
}

/// A package's manifest.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// `manifest:version` of the root element.
    pub version: Option<String>,
    pub entries: Vec<FileEntry>,
}

impl Manifest {
    /// Reads a manifest. Elements are found by namespace, whatever prefix the file uses. What
    /// can't be read is left out: damaged XML gives the entries before the damage, and entries
    /// past [`MAX_ENTRIES`] are dropped.
    pub fn parse(xml: &[u8]) -> Manifest {
        let canonical = Canonical::new(&[("manifest", MANIFEST_NS)]);
        let (root, _) = Options::canonical(&canonical).parse_streaming_partial(xml, &[], |_| Ok::<(), craft_xml::Error>(()));
        let Some(root) = root.filter(|r| named(r, "manifest")) else { return Manifest::default() };
        let entries = root.elements().filter(|e| named(e, "file-entry")).take(MAX_ENTRIES).filter_map(file_entry).collect();
        Manifest { version: attr(&root, "version").map(str::to_string), entries }
    }

    /// Reads a manifest, refusing one a lenient read would repair: malformed or unbalanced
    /// XML, a document type declaration (of any kind), an undefined entity, text or a second
    /// element outside the root, a root that is not `manifest` in the manifest namespace, a
    /// `file-entry` without a `full-path`, or more than [`MAX_ENTRIES`] entries. For a reader
    /// that refuses a damaged package rather than open what it can.
    pub fn parse_strict(xml: &[u8]) -> Result<Manifest, Error> {
        let bad = |why: String| Error::Manifest(why);
        let canonical = Canonical::new(&[("manifest", MANIFEST_NS)]);
        let opts = Options { doctype: Doctype::Reject, ..Options::canonical(&canonical).strict() };
        let root = opts.parse(xml).map_err(|e| bad(e.to_string()))?;
        if root.name != "manifest:manifest" {
            return Err(bad(format!("the root element is {}, not manifest:manifest", root.name)));
        }
        let mut entries = Vec::new();
        for e in root.elements().filter(|e| e.name == "manifest:file-entry") {
            if entries.len() == MAX_ENTRIES {
                return Err(bad(format!("more than {MAX_ENTRIES} file entries")));
            }
            entries.push(file_entry(e).ok_or_else(|| bad("a file entry has no full-path".into()))?);
        }
        Ok(Manifest { version: attr(&root, "version").map(str::to_string), entries })
    }

    /// The entry for `path`.
    pub fn entry(&self, path: &str) -> Option<&FileEntry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// The media type the manifest gives `path` (`/` for the package, `Object 1/` for a
    /// sub-document).
    pub fn media_type(&self, path: &str) -> Option<&str> {
        self.entry(path).map(|e| e.media_type.as_str())
    }

    /// Does any file say it is encrypted?
    pub fn is_encrypted(&self) -> bool {
        self.entries.iter().any(|e| e.encryption.is_some())
    }

    /// The manifest as XML, as LibreOffice lays it out: one entry a line, encryption data
    /// included.
    pub fn to_xml(&self) -> String {
        let mut o = format!("{}<manifest:manifest xmlns:manifest=\"{MANIFEST_NS}\"", craft_xml::DECL);
        if let Some(v) = &self.version {
            let _ = write!(o, " manifest:version=\"{}\"", esc_attr(v));
        }
        o.push_str(">\n");
        for e in &self.entries {
            let _ = write!(o, "<manifest:file-entry manifest:full-path=\"{}\"", esc_attr(&e.path));
            if let Some(v) = &e.version {
                let _ = write!(o, " manifest:version=\"{}\"", esc_attr(v));
            }
            let _ = write!(o, " manifest:media-type=\"{}\"", esc_attr(&e.media_type));
            if let Some(size) = e.size {
                let _ = write!(o, " manifest:size=\"{size}\"");
            }
            match &e.encryption {
                Some(c) => {
                    o.push('>');
                    encryption_xml(&mut o, c);
                    o.push_str("</manifest:file-entry>\n");
                }
                None => o.push_str("/>\n"),
            }
        }
        o.push_str("</manifest:manifest>\n");
        o
    }
}

fn encryption_xml(o: &mut String, c: &Encryption) {
    let size = |s: Option<usize>| s.map(|s| format!(" manifest:key-size=\"{s}\"")).unwrap_or_default();
    let _ = write!(
        o,
        "<manifest:encryption-data manifest:checksum-type=\"{}\" manifest:checksum=\"{}\"><manifest:algorithm manifest:algorithm-name=\"{}\" manifest:initialisation-vector=\"{}\"/><manifest:start-key-generation manifest:start-key-generation-name=\"{}\"{}/><manifest:key-derivation manifest:key-derivation-name=\"{}\" manifest:iteration-count=\"{}\" manifest:salt=\"{}\"{}/></manifest:encryption-data>",
        esc_attr(&c.checksum_type),
        STANDARD.encode(&c.checksum),
        esc_attr(&c.algorithm),
        STANDARD.encode(&c.iv),
        esc_attr(&c.start_key),
        size(c.start_key_size),
        esc_attr(&c.derivation),
        c.iterations,
        STANDARD.encode(&c.salt),
        size(c.key_size)
    );
}

/// Is `e` the manifest element `local`: in the manifest namespace, or (a prefix nothing declares,
/// a file with no namespace at all) in none?
fn named(e: &El, local: &str) -> bool {
    e.name.strip_prefix("manifest:").unwrap_or(&e.name) == local
}

fn child<'a>(e: &'a El, local: &str) -> Option<&'a El> {
    e.elements().find(|c| named(c, local))
}

/// A manifest attribute: in the manifest namespace, or (as some writers leave it) in none.
fn attr<'a>(e: &'a El, name: &str) -> Option<&'a str> {
    e.attr(&format!("manifest:{name}")).or_else(|| e.attr(name))
}

fn file_entry(e: &El) -> Option<FileEntry> {
    fn num<T: std::str::FromStr>(e: &El, name: &str) -> Option<T> {
        attr(e, name).and_then(|s| s.trim().parse().ok())
    }
    let b64 = |e: &El, name: &str| attr(e, name).and_then(|s| STANDARD.decode(s.trim()).ok()).unwrap_or_default();
    let encryption = child(e, "encryption-data").map(|d| {
        let mut c = Encryption {
            checksum_type: attr(d, "checksum-type").unwrap_or("SHA1/1K").to_string(),
            checksum: b64(d, "checksum"),
            ..Default::default()
        };
        if let Some(a) = child(d, "algorithm") {
            c.algorithm = attr(a, "algorithm-name").unwrap_or_default().to_string();
            c.iv = b64(a, "initialisation-vector");
        }
        if let Some(k) = child(d, "start-key-generation") {
            c.start_key = attr(k, "start-key-generation-name").unwrap_or("SHA1").to_string();
            c.start_key_size = num(k, "key-size");
        }
        if let Some(k) = child(d, "key-derivation") {
            c.derivation = attr(k, "key-derivation-name").unwrap_or_default().to_string();
            c.iterations = num(k, "iteration-count").unwrap_or(1024);
            c.salt = b64(k, "salt");
            c.key_size = num(k, "key-size");
        }
        c
    });
    Some(FileEntry {
        path: attr(e, "full-path")?.to_string(),
        media_type: attr(e, "media-type").unwrap_or_default().to_string(),
        version: attr(e, "version").map(str::to_string),
        size: num(e, "size"),
        encryption,
    })
}
