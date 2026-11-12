//! The ZIP container (ODF 1.3 Part 2 section 3): a `mimetype` file first and stored, a
//! manifest, and the document's files.

use std::io::{Cursor, Read, Write};

use crate::manifest::{FileEntry, MANIFEST_PATH, Manifest};
use crate::{Error, zip_guard};

/// What a [`Package`] refuses to exceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most entries the ZIP may declare, checked before `zip` parses them. Default 20,000.
    pub max_entries: u64,
    /// Most bytes one file may inflate to. Default 256 MiB.
    pub max_entry: u64,
    /// Most bytes all the files read from one [`Package`] may inflate to together (a file read
    /// twice counts twice). Default 2 GiB.
    pub max_total: u64,
}

impl Limits {
    pub const DEFAULT: Limits = Limits { max_entries: 20_000, max_entry: 256 << 20, max_total: 2 << 30 };
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// The longest `mimetype` file read: media types are short, and a longer one is no media type.
const MAX_MIMETYPE: u64 = 255;

pub(crate) fn zip_err(e: zip::result::ZipError) -> Error {
    Error::Zip(e.to_string())
}

pub(crate) fn io_err(e: std::io::Error) -> Error {
    Error::Zip(e.to_string())
}

/// An OpenDocument package open for reading.
pub struct Package<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// The files' names (folders and names that are not text left out), in the ZIP's order.
    names: Vec<String>,
    limits: Limits,
    /// Bytes inflated so far, against [`Limits::max_total`].
    spent: u64,
}

impl<'a> Package<'a> {
    /// Opens `bytes` as a ZIP package. The declared entry count is checked against
    /// [`Limits::max_entries`] before anything is parsed. No file is inflated yet.
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<Package<'a>, Error> {
        zip_guard::check(bytes, limits.max_entries).map_err(Error::TooManyEntries)?;
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(zip_err)?;
        if zip.len() as u64 > limits.max_entries {
            return Err(Error::TooManyEntries(format!("the package has {} entries, more than {}", zip.len(), limits.max_entries)));
        }
        let names = zip.file_names().filter_map(|n| n.ok()).filter(|n| !n.ends_with('/')).map(|n| n.into_owned()).collect();
        Ok(Package { zip, names, limits: *limits, spent: 0 })
    }

    /// The names of the package's files, folders left out, in the order the ZIP lists them.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// Is there a file `name`?
    pub fn contains(&self, name: &str) -> bool {
        self.zip.index_for_name(name).is_some()
    }

    /// The size the ZIP declares for `name` (what it claims to inflate to; [`Package::read`]
    /// does not trust it).
    pub fn declared_size(&mut self, name: &str) -> Option<u64> {
        self.zip.by_name(name).ok().map(|f| f.size())
    }

    /// File `name` inflated. `Ok(None)` when the package has no such file, or it cannot be
    /// inflated (damaged data); an error when it is larger than [`Limits::max_entry`] or takes
    /// the package past [`Limits::max_total`]. A file that inflates to more than it declares is
    /// held to the same limits.
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let (max_entry, room) = (self.limits.max_entry, self.limits.max_total.saturating_sub(self.spent));
        let Ok(f) = self.zip.by_name(name) else { return Ok(None) };
        if f.size() > max_entry {
            return Err(Error::EntryTooLarge(name.to_string(), max_entry));
        }
        if f.size() > room {
            return Err(Error::TotalTooLarge(self.limits.max_total));
        }
        let cap = max_entry.min(room);
        let mut out = Vec::new();
        // One byte past the cap tells a file at the cap from one beyond it.
        if f.take(cap.saturating_add(1)).read_to_end(&mut out).is_err() {
            self.spent = self.spent.saturating_add(out.len() as u64);
            return Ok(None);
        }
        self.spent = self.spent.saturating_add(out.len() as u64);
        if out.len() as u64 > cap {
            return Err(if cap == max_entry {
                Error::EntryTooLarge(name.to_string(), max_entry)
            } else {
                Error::TotalTooLarge(self.limits.max_total)
            });
        }
        Ok(Some(out))
    }

    /// Bytes inflated from this package so far.
    pub fn bytes_read(&self) -> u64 {
        self.spent
    }

    /// The package's media type: what its `mimetype` file holds, surrounding whitespace
    /// dropped. `None` when it has none, or one longer than a media type can be.
    pub fn mimetype(&mut self) -> Option<String> {
        let f = self.zip.by_name("mimetype").ok()?;
        if f.size() > MAX_MIMETYPE {
            return None;
        }
        let mut out = Vec::new();
        f.take(MAX_MIMETYPE + 1).read_to_end(&mut out).ok()?;
        if out.len() as u64 > MAX_MIMETYPE {
            return None;
        }
        self.spent = self.spent.saturating_add(out.len() as u64);
        Some(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// The package's manifest; empty when it has none or it cannot be read.
    pub fn manifest(&mut self) -> Manifest {
        self.read(MANIFEST_PATH).ok().flatten().map(|xml| Manifest::parse(&xml)).unwrap_or_default()
    }

    /// The package's manifest read with [`Manifest::parse_strict`]: an error when the package
    /// has none, or one a strict read refuses, or it is over a limit.
    pub fn manifest_strict(&mut self) -> Result<Manifest, Error> {
        let xml = self.read(MANIFEST_PATH)?.ok_or_else(|| Error::Manifest("the package has no manifest".into()))?;
        Manifest::parse_strict(&xml)
    }

    /// Does the manifest say any file is encrypted ([`crate::crypt`])?
    pub fn is_encrypted(&mut self) -> bool {
        self.manifest().is_encrypted()
    }
}

struct Entry {
    path: String,
    data: Vec<u8>,
    stored: bool,
}

/// Writes an OpenDocument package: the `mimetype` file first and stored (so the media type can
/// be read at a fixed offset, section 3.3), then the manifest, then the files in the order they
/// were added.
pub struct PackageWriter {
    mime: String,
    manifest: Manifest,
    manifest_xml: Option<String>,
    entries: Vec<Entry>,
}

impl PackageWriter {
    /// A package of media type `mime` ([`crate::mime`]), ODF version 1.3.
    pub fn new(mime: &str) -> PackageWriter {
        let root = FileEntry { path: "/".into(), media_type: mime.into(), version: Some("1.3".into()), ..Default::default() };
        PackageWriter {
            mime: mime.into(),
            manifest: Manifest { version: Some("1.3".into()), entries: vec![root] },
            manifest_xml: None,
            entries: Vec::new(),
        }
    }

    fn add(&mut self, path: &str, media_type: &str, data: &[u8], stored: bool) {
        self.entries.push(Entry { path: path.into(), data: data.to_vec(), stored });
        if self.manifest.entry(path).is_none() {
            self.manifest.entries.push(FileEntry { path: path.into(), media_type: media_type.into(), ..Default::default() });
        }
    }

    /// Adds file `path`, deflated, listed in the manifest with `media_type`.
    pub fn file(&mut self, path: &str, media_type: &str, data: &[u8]) {
        self.add(path, media_type, data, false);
    }

    /// Adds file `path` stored as it is: for data that is compressed already (PNG, JPEG).
    pub fn stored(&mut self, path: &str, media_type: &str, data: &[u8]) {
        self.add(path, media_type, data, true);
    }

    /// Lists a folder in the manifest (`Object 1/`, a sub-document with its own media type and
    /// version 1.3). It has no ZIP entry of its own.
    pub fn folder(&mut self, path: &str, media_type: &str) {
        if self.manifest.entry(path).is_none() {
            self.manifest.entries.push(FileEntry {
                path: path.into(),
                media_type: media_type.into(),
                version: Some("1.3".into()),
                ..Default::default()
            });
        }
    }

    /// The manifest that will be written, to adjust (an entry's version, entries for files
    /// written some other way).
    pub fn manifest_mut(&mut self) -> &mut Manifest {
        &mut self.manifest
    }

    /// Writes `xml` as the manifest instead of the one built from the files added: for a
    /// writer that keeps a manifest as it was read.
    pub fn manifest_xml(&mut self, xml: String) {
        self.manifest_xml = Some(xml);
    }

    /// The package's bytes.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file("mimetype", stored).map_err(zip_err)?;
        z.write_all(self.mime.as_bytes()).map_err(io_err)?;
        let manifest = self.manifest_xml.unwrap_or_else(|| self.manifest.to_xml());
        z.start_file(MANIFEST_PATH, deflated).map_err(zip_err)?;
        z.write_all(manifest.as_bytes()).map_err(io_err)?;
        for e in &self.entries {
            z.start_file(e.path.as_str(), if e.stored { stored } else { deflated }).map_err(zip_err)?;
            z.write_all(&e.data).map_err(io_err)?;
        }
        Ok(z.finish().map_err(zip_err)?.into_inner())
    }
}
