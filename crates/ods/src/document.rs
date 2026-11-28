//! A whole spreadsheet document: a package (`.ods`, `.ots`), a password-protected package, or a
//! flat document (`.fods`). Its tables come from `content.xml`; its other parts (styles,
//! settings, metadata) are kept as trees, and its pictures and embedded objects are read on
//! request.

use std::borrow::Cow;

use craft_odf::crypt::{self, CryptError};
use craft_odf::flat;
use craft_xml::{Canonical, Doctype, El, Options as XmlOptions};

use crate::content::{read_content, tree_cost};
use crate::model::Workbook;
use crate::ns::NAMESPACES;
use crate::{Error, Options};

/// Where the document's files are.
enum Files<'a> {
    Package(Cow<'a, [u8]>),
    /// A flat document's pictures and embedded objects, named as a package names them.
    Flat(Vec<(String, Vec<u8>)>),
}

/// A spreadsheet document. See [`Document::open`].
pub struct Document<'a> {
    /// The tables of `content.xml` and what it holds besides them.
    pub workbook: Workbook,
    /// `styles.xml` (`office:document-styles`): fonts, common and automatic styles, page layouts
    /// and master pages.
    pub styles: Option<El>,
    /// `settings.xml` (`office:document-settings`).
    pub settings: Option<El>,
    /// `meta.xml` (`office:document-meta`).
    pub meta: Option<El>,
    /// The optional parts that were damaged or too large and were left out, and why.
    pub warnings: Vec<String>,
    files: Files<'a>,
    options: Options,
}

impl std::fmt::Debug for Document<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document").field("workbook", &self.workbook).field("warnings", &self.warnings).finish_non_exhaustive()
    }
}

/// Is this a flat document: XML text, not a ZIP package?
fn is_flat(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<')
}

fn is_spreadsheet(mime: Option<&str>) -> bool {
    matches!(mime, Some(craft_odf::mime::SPREADSHEET | craft_odf::mime::SPREADSHEET_TEMPLATE))
}

/// The package path an `xlink:href` names (`./Object 1`, `Pictures/1.png`), or `None` for a
/// link outside the package: another file, a URL, a path that climbs out.
pub fn package_path(href: &str) -> Option<String> {
    let mut path = href.trim();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    let path = path.trim_end_matches('/');
    let outside = path.is_empty()
        || path.starts_with('/')
        || path.starts_with('#')
        || path.split('/').next().is_some_and(|first| first.contains(':'))
        || path.split('/').any(|part| part == ".." || part.is_empty());
    (!outside).then(|| path.to_string())
}

impl<'a> Document<'a> {
    /// Opens a spreadsheet: a package, or a flat document (XML text). A password-protected
    /// package needs `password`: without one it is [`Error::Encrypted`], with the wrong one
    /// [`Error::Password`]. `content.xml` must be readable; a damaged or oversized `styles.xml`,
    /// `settings.xml` or `meta.xml` is left out and named in [`Document::warnings`].
    pub fn open(bytes: &'a [u8], password: Option<&str>, options: &Options) -> Result<Document<'a>, Error> {
        if is_flat(bytes) {
            let doc = flat::split(bytes, &options.package)?;
            if !is_spreadsheet(doc.mimetype.as_deref()) {
                return Err(Error::Invalid("missing or invalid ODS mimetype".into()));
            }
            let workbook = read_content(doc.content.as_bytes(), options)?;
            let mut d =
                Document { workbook, styles: None, settings: None, meta: None, warnings: vec![], files: Files::Flat(doc.files), options: *options };
            d.styles = d.part_tree("styles.xml", doc.styles.as_bytes(), "office:document-styles");
            d.settings = d.part_tree("settings.xml", doc.settings.as_bytes(), "office:document-settings");
            d.meta = d.part_tree("meta.xml", doc.meta.as_bytes(), "office:document-meta");
            return Ok(d);
        }
        let mut package = craft_odf::Package::open(bytes, &options.package)?;
        if !is_spreadsheet(package.mimetype().as_deref()) {
            return Err(Error::Invalid("missing or invalid ODS mimetype".into()));
        }
        let bytes = if package.is_encrypted() {
            let Some(password) = password else { return Err(Error::Encrypted) };
            match crypt::decrypt(bytes, password) {
                Ok(plain) => Cow::Owned(plain),
                Err(CryptError::WrongPassword) => return Err(Error::Password),
                Err(CryptError::Unsupported(why)) => return Err(Error::Invalid(why)),
            }
        } else {
            Cow::Borrowed(bytes)
        };
        let mut package = craft_odf::Package::open(&bytes, &options.package)?;
        let content = package.read("content.xml")?.ok_or_else(|| Error::Invalid("content.xml is missing".into()))?;
        let workbook = read_content(&content, options)?;
        let mut warnings = vec![];
        let mut read = |name: &str| {
            package.read(name).unwrap_or_else(|e| {
                warnings.push(format!("{name} could not be read: {e}"));
                None
            })
        };
        let (styles, settings, meta) = (read("styles.xml"), read("settings.xml"), read("meta.xml"));
        drop(package);
        let mut d = Document { workbook, styles: None, settings: None, meta: None, warnings, files: Files::Package(bytes), options: *options };
        d.styles = styles.and_then(|x| d.part_tree("styles.xml", &x, "office:document-styles"));
        d.settings = settings.and_then(|x| d.part_tree("settings.xml", &x, "office:document-settings"));
        d.meta = meta.and_then(|x| d.part_tree("meta.xml", &x, "office:document-meta"));
        Ok(d)
    }

    /// An optional part as a tree with root `root`, or `None` with a warning. A flat document
    /// without the part gives an empty text, which is no part.
    fn part_tree(&mut self, name: &str, xml: &[u8], root: &str) -> Option<El> {
        if xml.iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        match self.parse(xml) {
            Ok(el) if el.name == root => Some(el),
            Ok(_) => {
                self.warnings.push(format!("{name} is not an OpenDocument {root} part"));
                None
            }
            Err(e) => {
                self.warnings.push(format!("{name} could not be read: {e}"));
                None
            }
        }
    }

    /// XML as a tree with this crate's prefixes, within [`Options::max_kept`].
    fn parse(&self, xml: &[u8]) -> Result<El, Error> {
        let canonical = Canonical::new(NAMESPACES);
        let opts = XmlOptions { doctype: Doctype::Reject, ..XmlOptions::canonical(&canonical).strict() };
        let el = opts.parse(xml).map_err(|e| if e.is_limit() { Error::TooLarge(e.to_string()) } else { Error::Xml(e.to_string()) })?;
        if tree_cost(&el) > self.options.max_kept {
            return Err(Error::TooLarge(format!("more than {} MiB", self.options.max_kept >> 20)));
        }
        Ok(el)
    }

    /// A file of the document by its package path (see [`package_path`]): a picture, or a part
    /// of an embedded object (`Object 1/content.xml`). `None` when there is no such file.
    pub fn file(&self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        match &self.files {
            Files::Package(bytes) => Ok(craft_odf::Package::open(bytes, &self.options.package)?.read(path)?),
            Files::Flat(files) => Ok(files.iter().find(|(name, _)| name == path).map(|(_, data)| data.clone())),
        }
    }

    /// An XML file of the document as a tree, with the prefixes of [`Document::styles`]: an
    /// embedded chart's `Object 1/content.xml`, say. `None` when there is no such file.
    pub fn xml(&self, path: &str) -> Result<Option<El>, Error> {
        self.file(path)?.map(|xml| self.parse(&xml)).transpose()
    }
}
