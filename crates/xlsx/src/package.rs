//! The OPC package of a workbook and the parts this crate reads from it.

use craft_opc::{ContentTypes, LazyPackage, Rels};
use craft_xml::{El, Options, UnknownEntity};

use crate::sheet::{Row, RowReader, SheetPart};
use crate::strings::StringItem;
use crate::styles::Styles;
use crate::workbook::{SheetEntry, Workbook};
use crate::{Error, Limits, Warnings, xlsb};

/// Most bytes of `[Content_Types].xml` read from a binary workbook.
const MAX_CONTENT_TYPES: u64 = 1 << 20;

/// An open workbook package. Parts are inflated when they are asked for, against the
/// package's [`Limits`].
pub struct Package<'a> {
    opc: LazyPackage<'a>,
    limits: Limits,
    warnings: Warnings,
    workbook_part: String,
    /// Content types, for a binary workbook (which checks every part it reads against them).
    binary: Option<ContentTypes>,
    workbook_rels: Option<Rels>,
    budget: xlsb::Budget,
    /// Shared strings in a binary workbook, once counted: a cell's index is checked against it.
    binary_strings: Option<usize>,
}

/// Is this a binary workbook (`.xlsb`): a package whose one workbook relationship points at a
/// part of the XLSB workbook content type?
pub fn is_xlsb(bytes: &[u8]) -> bool {
    Package::open(bytes, &Limits::default()).is_ok_and(|p| p.is_binary())
}

impl<'a> Package<'a> {
    /// Opens the package and finds the workbook part. Nothing else is read yet.
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<Package<'a>, Error> {
        let mut opc = LazyPackage::open(bytes, &limits.package)?;
        let root = opc.rels("")?;
        let documents: Vec<_> = root.all("officeDocument").collect();
        let mut binary = None;
        let workbook_part = match documents.first() {
            Some(rel) if !rel.external && rel.target.to_ascii_lowercase().ends_with(".bin") => {
                let types = opc.read_limited("[Content_Types].xml", MAX_CONTENT_TYPES)?.map(|b| ContentTypes::parse(&b)).unwrap_or_default();
                if documents.len() != 1 {
                    return Err(xlsb::invalid("expected one workbook relationship"));
                }
                xlsb::check_type(&types, &rel.target, xlsb::MAIN)?;
                binary = Some(types);
                rel.target.clone()
            }
            Some(rel) if !rel.external => rel.target.clone(),
            _ => "xl/workbook.xml".to_string(),
        };
        Ok(Package {
            opc,
            limits: *limits,
            warnings: Warnings::default(),
            workbook_part,
            binary,
            workbook_rels: None,
            budget: xlsb::Budget::default(),
            binary_strings: None,
        })
    }

    /// A binary workbook (`.xlsb`).
    pub fn is_binary(&self) -> bool {
        self.binary.is_some()
    }

    /// The workbook part's name.
    pub fn workbook_part(&self) -> &str {
        &self.workbook_part
    }

    /// What reading skipped or repaired so far, each message once.
    pub fn warnings(&self) -> &[String] {
        &self.warnings.0
    }

    /// Adds a warning (a caller reading further parts keeps one list).
    pub fn warn(&mut self, m: impl Into<String>) {
        self.warnings.warn(m);
    }

    /// The package underneath, for the parts this crate does not read.
    pub fn opc(&mut self) -> &mut LazyPackage<'a> {
        &mut self.opc
    }

    /// The XML options parts are read with: local names, an undefined entity kept as written,
    /// [`Limits::xml`].
    pub fn xml_options(&self) -> Options<'static> {
        Options { unknown_entity: UnknownEntity::Literal, ..Options::local() }.with_limits(self.limits.xml)
    }

    /// Part `part` as XML; `Ok(None)` when there is no such part.
    pub fn read_xml(&mut self, part: &str) -> Result<Option<El>, Error> {
        let options = self.xml_options();
        Ok(self.opc.read_xml(part, &options)?)
    }

    /// A part the workbook can do without: missing or unreadable is a warning and `None`;
    /// only a limit is an error.
    pub fn optional_xml(&mut self, part: &str) -> Result<Option<El>, Error> {
        match self.read_xml(part) {
            Ok(Some(e)) => Ok(Some(e)),
            Ok(None) => {
                self.warn(format!("missing part {part}"));
                Ok(None)
            }
            Err(Error::TooLarge(m)) => Err(Error::TooLarge(m)),
            Err(e) => {
                self.warn(format!("skipped {part}: {e}"));
                Ok(None)
            }
        }
    }

    /// The relationships of `part`.
    pub fn rels(&mut self, part: &str) -> Result<Rels, Error> {
        Ok(self.opc.rels(part)?)
    }

    /// The workbook part's relationships.
    pub fn workbook_rels(&mut self) -> Result<Rels, Error> {
        if let Some(r) = &self.workbook_rels {
            return Ok(r.clone());
        }
        let rels = self.opc.rels(&self.workbook_part)?;
        self.workbook_rels = Some(rels.clone());
        Ok(rels)
    }

    /// The workbook part.
    pub fn workbook(&mut self) -> Result<Workbook, Error> {
        if self.binary.is_some() {
            let bytes = self.required(&self.workbook_part.clone())?;
            let rels = self.workbook_rels()?;
            return xlsb::workbook(&bytes, &rels, &self.limits, &self.budget, &mut self.warnings);
        }
        let options = self.xml_options();
        let root = match self.opc.read_xml(&self.workbook_part, &options)? {
            Some(root) => root,
            None if self.workbook_part != "xl/workbook.xml" && self.opc.has("xl/workbook.xml") => {
                self.workbook_part = "xl/workbook.xml".into();
                self.workbook_rels = None;
                self.opc.read_xml("xl/workbook.xml", &options)?.ok_or_else(|| Error::Invalid("no workbook part".into()))?
            }
            None if self.opc.has("content.xml") => return Err(Error::Invalid("this is an OpenDocument file, not XLSX".into())),
            None => return Err(Error::Invalid("no workbook part".into())),
        };
        if root.name != "workbook" {
            return Err(Error::Invalid(format!("unexpected root element <{}> in workbook part", root.name)));
        }
        let rels = self.workbook_rels()?;
        Ok(Workbook::from_xml(root, &rels))
    }

    /// The shared strings, empty when the workbook has none.
    pub fn shared_strings(&mut self) -> Result<Vec<StringItem>, Error> {
        let rels = self.workbook_rels()?;
        if self.binary.is_some() {
            let found: Vec<_> = rels.all("sharedStrings").collect();
            let Some(rel) = found.first() else {
                self.binary_strings = Some(0);
                return Ok(Vec::new());
            };
            if found.len() > 1 {
                return Err(xlsb::invalid("multiple shared-string relationships"));
            }
            if rel.external {
                return Err(xlsb::invalid("external shared-string relationship"));
            }
            if let Some(types) = &self.binary {
                xlsb::check_type(types, &rel.target, xlsb::STRINGS)?;
            }
            let bytes = self.required(&rel.target)?;
            let strings = xlsb::shared_strings(&bytes, &self.limits, &self.budget, &mut self.warnings)?;
            self.binary_strings = Some(strings.len());
            return Ok(strings);
        }
        let Some(rel) = rels.first("sharedStrings").filter(|r| !r.external) else { return Ok(Vec::new()) };
        let Some(bytes) = self.opc.read(&rel.target)? else {
            self.warn(format!("missing part {}", rel.target));
            return Ok(Vec::new());
        };
        let mut items = Vec::new();
        let (_, err) = self.xml_options().parse_streaming_partial(&bytes, &["si"], |si| {
            items.push(StringItem::read(&si));
            Ok::<(), craft_xml::Error>(())
        });
        if let Some(e) = err {
            self.warn(format!("shared strings are damaged ({e}); kept {} strings", items.len()));
        }
        let dropped: usize = items.iter().map(|s| s.dropped_phonetic_runs).sum();
        if dropped > 0 {
            self.warn(format!("shared strings: {dropped} phonetic runs were dropped (bad sb/eb or no reading)"));
        }
        Ok(items)
    }

    /// The styles part, `None` when the workbook has none (or is binary: its styles are not read).
    pub fn styles(&mut self) -> Result<Option<Styles>, Error> {
        if self.binary.is_some() {
            return Ok(None);
        }
        let rels = self.workbook_rels()?;
        let Some(rel) = rels.first("styles").filter(|r| !r.external) else {
            self.warn("no styles part");
            return Ok(None);
        };
        Ok(self.optional_xml(&rel.target.clone())?.map(Styles::read))
    }

    /// Reads a worksheet, handing each row to `each` in file order; an error from `each` stops the
    /// read and is returned. A damaged worksheet gives the rows before the damage and says so
    /// in [`SheetPart::damaged`] and in the warnings.
    pub fn sheet(&mut self, entry: &SheetEntry, mut each: impl FnMut(Row) -> Result<(), Error>) -> Result<SheetPart, Error> {
        let part = entry.part.clone().ok_or_else(|| Error::Invalid(format!("sheet \"{}\" has no part", entry.name)))?;
        if self.binary.is_some() {
            if let Some(types) = &self.binary {
                xlsb::check_type(types, &part, xlsb::WORKSHEET)?;
            }
            let strings = match self.binary_strings {
                Some(n) => n,
                None => self.shared_strings()?.len(),
            };
            let bytes = self.required(&part)?;
            let mut sheet = xlsb::worksheet(&bytes, strings, &self.limits, &self.budget, &mut self.warnings, &mut each)?;
            sheet.part = part;
            return Ok(sheet);
        }
        let bytes = self.required(&part)?;
        let rels = self.opc.rels(&part)?;
        let mut rows = RowReader::default();
        let (root, err) = self.xml_options().parse_streaming_partial(&bytes, &["row"], |e| match rows.row(&e) {
            Some(row) => each(row).map_err(Stop::Caller),
            None => Ok(()),
        });
        let damaged = match err {
            Some(Stop::Caller(e)) => return Err(e),
            Some(Stop::Xml(e)) => {
                self.warn(format!("sheet \"{}\" is damaged ({e}); kept {} rows", entry.name, rows.rows));
                Some(e.to_string())
            }
            None => None,
        };
        if rows.dropped > 0 {
            self.warn(format!("sheet \"{}\": {} cells or rows outside the 1,048,576 × 16,384 grid were dropped", entry.name, rows.dropped));
        }
        let mut sheet =
            SheetPart { part, root: root.unwrap_or_default(), rels, rows: rows.rows, dropped: rows.dropped, damaged, ..SheetPart::default() };
        sheet.lift();
        Ok(sheet)
    }

    fn required(&mut self, part: &str) -> Result<Vec<u8>, Error> {
        self.opc.read(part)?.ok_or_else(|| Error::Invalid(format!("missing part {part}")))
    }
}

/// Why a streamed worksheet stopped: the XML, or the caller.
enum Stop {
    Xml(craft_xml::Error),
    Caller(Error),
}

impl From<craft_xml::Error> for Stop {
    fn from(e: craft_xml::Error) -> Self {
        Stop::Xml(e)
    }
}
