//! A whole workbook held in memory and written back: the parts this crate reads, regenerated,
//! and every other part carried through as it was.

use craft_opc::{ContentTypes, PackageOut, RT_BASE, Rel, RelOut, Rels, RelsOut, relative, rels_path};

use crate::sheet::{CellValue, Row, SheetPart};
use crate::strings::StringItem;
use crate::styles::Styles;
use crate::workbook::{SheetKind, Workbook};
use crate::{Error, Limits, Package};

const CT_WORKSHEET: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CT_STRINGS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
/// The relationship type namespace of strict files: kept relationships are written with the
/// transitional one, as the parts this crate writes are.
const RT_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/";

/// Which member of the family a package is: its workbook part's content type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum XlsxKind {
    /// `.xlsx`
    #[default]
    Workbook,
    /// `.xlsm`
    MacroWorkbook,
    /// `.xltx`
    Template,
    /// `.xltm`
    MacroTemplate,
}

impl XlsxKind {
    const ALL: [XlsxKind; 4] = [XlsxKind::Workbook, XlsxKind::MacroWorkbook, XlsxKind::Template, XlsxKind::MacroTemplate];

    /// The kind a file name's extension asks for; anything else is a plain workbook.
    pub fn from_path(path: &str) -> XlsxKind {
        let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "xlsm" => XlsxKind::MacroWorkbook,
            "xltx" => XlsxKind::Template,
            "xltm" => XlsxKind::MacroTemplate,
            _ => XlsxKind::Workbook,
        }
    }

    /// The content type of the workbook part.
    pub fn main_content_type(self) -> &'static str {
        match self {
            XlsxKind::Workbook => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            XlsxKind::MacroWorkbook => "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
            XlsxKind::Template => "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
            XlsxKind::MacroTemplate => "application/vnd.ms-excel.template.macroEnabled.main+xml",
        }
    }

    /// May carry a VBA project (`.xlsm`, `.xltm`).
    pub fn is_macro_enabled(self) -> bool {
        matches!(self, XlsxKind::MacroWorkbook | XlsxKind::MacroTemplate)
    }

    fn of_content_type(ct: &str) -> Option<XlsxKind> {
        XlsxKind::ALL.into_iter().find(|k| k.main_content_type() == ct)
    }
}

/// One worksheet: its rows and the rest of its part.
#[derive(Clone, Debug, Default)]
pub struct Worksheet {
    pub rows: Vec<Row>,
    pub part: SheetPart,
}

/// A part carried through unchanged: drawings, charts, the theme, document properties, a VBA
/// project, and the relationships of each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeptPart {
    pub name: String,
    /// From `[Content_Types].xml`; `None` when it names none.
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// A whole SpreadsheetML workbook.
///
/// [`Document::read`] holds every part; [`Document::write`] regenerates the workbook part, the
/// worksheets, the shared strings, the styles and their relationships from the fields, and
/// writes every [`KeptPart`] back as it was. A read file written without changes keeps its part
/// names and relationship ids, so kept parts still point where they did. The calculation chain
/// is dropped: Excel rebuilds it, and a stale one makes it repair the file.
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub kind: XlsxKind,
    /// The workbook part's name; `xl/workbook.xml` when empty.
    pub workbook_part: String,
    pub workbook: Workbook,
    /// The workbook part's relationships as read. Those to worksheets, shared strings, styles
    /// and the calculation chain are made again on writing; the rest are kept with their ids.
    pub workbook_rels: Rels,
    pub shared_strings: Vec<StringItem>,
    pub styles: Option<Styles>,
    /// One per [`Workbook::sheets`] entry: the worksheet, or `None` for a sheet kept as its
    /// parts (a chart sheet, a macro sheet, a worksheet whose part could not be read).
    pub worksheets: Vec<Option<Worksheet>>,
    pub parts: Vec<KeptPart>,
    /// What reading skipped or repaired.
    pub warnings: Vec<String>,
}

impl Document {
    /// Reads every part. A binary workbook is refused: its formulas and styles are not read, so
    /// it cannot be written back as SpreadsheetML without loss.
    pub fn read(bytes: &[u8], limits: &Limits) -> Result<Document, Error> {
        let mut p = Package::open(bytes, limits)?;
        if p.is_binary() {
            return Err(Error::Invalid("a binary workbook (.xlsb) cannot be held as SpreadsheetML".into()));
        }
        let workbook = p.workbook()?;
        let workbook_part = p.workbook_part().to_string();
        let workbook_rels = p.workbook_rels()?;
        let shared_strings = p.shared_strings()?;
        let styles = p.styles()?;
        let mut worksheets = Vec::with_capacity(workbook.sheets.len());
        for entry in &workbook.sheets {
            if entry.kind != SheetKind::Worksheet || entry.part.is_none() {
                worksheets.push(None);
                continue;
            }
            let mut rows = Vec::new();
            match p.sheet(entry, |row| {
                rows.push(row);
                Ok(())
            }) {
                Ok(part) => worksheets.push(Some(Worksheet { rows, part })),
                Err(Error::TooLarge(m)) => return Err(Error::TooLarge(m)),
                Err(e) => {
                    p.warn(format!("sheet \"{}\" kept unread: {e}", entry.name));
                    worksheets.push(None);
                }
            }
        }
        let types = p.opc().read("[Content_Types].xml")?.map(|b| ContentTypes::parse(&b)).unwrap_or_default();
        let mut made = vec!["[Content_Types].xml".to_string(), workbook_part.clone(), rels_path(&workbook_part)];
        for ws in worksheets.iter().flatten() {
            made.push(ws.part.part.clone());
            made.push(rels_path(&ws.part.part));
        }
        made.extend(workbook_rels.list.iter().filter(|r| !r.external && regenerated(r)).map(|r| r.target.clone()));
        let opc = p.opc();
        let names: Vec<String> = opc.names().iter().filter_map(|n| opc.canonical(n)).collect();
        let mut parts = Vec::new();
        for name in names {
            if made.iter().any(|m| m.eq_ignore_ascii_case(&name)) || name.ends_with('/') {
                continue;
            }
            let Some(bytes) = p.opc().read(&name)? else { continue };
            parts.push(KeptPart { content_type: types.of(&name).map(str::to_string), name, bytes });
        }
        let kind = types.of(&workbook_part).and_then(XlsxKind::of_content_type).unwrap_or_default();
        Ok(Document { kind, workbook_part, workbook, workbook_rels, shared_strings, styles, worksheets, parts, warnings: p.warnings().to_vec() })
    }

    /// The package. Refused when [`Document::worksheets`] does not match the workbook's sheets,
    /// or a sheet without a worksheet has no kept relationship to point at.
    pub fn write(&self) -> Result<Vec<u8>, Error> {
        if self.worksheets.len() != self.workbook.sheets.len() {
            return Err(Error::Invalid(format!("{} worksheets for {} sheets", self.worksheets.len(), self.workbook.sheets.len())));
        }
        let book_part = if self.workbook_part.is_empty() { "xl/workbook.xml" } else { &self.workbook_part };
        let mut out = PackageOut::default();
        let mut rels = RelsOut::default();
        // A worksheet's relationship is kept only for a sheet kept as its parts: the others are
        // made again, and one whose sheet was removed goes with it.
        let unread = |r: &Rel| self.workbook.sheets.iter().zip(&self.worksheets).any(|(e, ws)| ws.is_none() && e.rel_id == r.id);
        for r in self.workbook_rels.list.iter().filter(|r| !regenerated(r) && (r.kind() != "worksheet" || unread(r))) {
            rels.items.push(kept(r));
        }
        let mut workbook = self.workbook.clone();
        let mut sheet_parts = Vec::new();
        let mut taken: Vec<String> = self.parts.iter().map(|p| p.name.to_ascii_lowercase()).collect();
        for (i, (entry, ws)) in workbook.sheets.iter_mut().zip(&self.worksheets).enumerate() {
            let Some(ws) = ws else {
                if rels.items.iter().any(|r| r.id == entry.rel_id) {
                    continue;
                }
                return Err(Error::Invalid(format!("sheet \"{}\" has neither a worksheet nor a kept part", entry.name)));
            };
            let mut name = ws.part.part.clone();
            if name.is_empty() || taken.contains(&name.to_ascii_lowercase()) {
                name = fresh(&taken, |n| format!("xl/worksheets/sheet{}.xml", i + n));
            }
            taken.push(name.to_ascii_lowercase());
            if entry.rel_id.is_empty() || rels.items.iter().any(|r| r.id == entry.rel_id) {
                entry.rel_id = fresh_id(&rels);
            }
            rels.items.push(RelOut {
                id: entry.rel_id.clone(),
                ty: format!("{RT_BASE}worksheet"),
                target: relative(book_part, &name),
                external: false,
            });
            sheet_parts.push((name, ws));
        }
        let mut add = |kind: &str, default: &str, taken: &mut Vec<String>| {
            let old = self.workbook_rels.list.iter().find(|r| r.kind() == kind && !r.external).map(|r| r.target.clone());
            let name = old.filter(|n| !taken.contains(&n.to_ascii_lowercase())).unwrap_or_else(|| default.to_string());
            taken.push(name.to_ascii_lowercase());
            let id = fresh_id(&rels);
            rels.items.push(RelOut { id, ty: format!("{RT_BASE}{kind}"), target: relative(book_part, &name), external: false });
            name
        };
        let strings = (!self.shared_strings.is_empty()).then(|| add("sharedStrings", "xl/sharedStrings.xml", &mut taken));
        let styles = self.styles.as_ref().map(|s| (add("styles", "xl/styles.xml", &mut taken), s));

        let mut root = workbook.to_el();
        crate::ns::declare(&mut root);
        let mut w = craft_xml::Writer::document();
        w.tree(&root);
        out.xml_part(book_part, self.kind.main_content_type(), w.into_bytes());
        out.rels(book_part, &rels);
        if !self.parts.iter().any(|p| p.name == "_rels/.rels") {
            let mut package = RelsOut::default();
            package.add("officeDocument", book_part, false);
            out.rels("", &package);
        }
        for (name, ws) in &sheet_parts {
            out.xml_part(name, CT_WORKSHEET, ws.part.to_xml(&ws.rows));
            let mut sheet_rels = RelsOut::default();
            sheet_rels.items.extend(ws.part.rels.list.iter().map(kept));
            out.rels(name, &sheet_rels);
        }
        if let Some(name) = &strings {
            let count: usize =
                sheet_parts.iter().flat_map(|(_, ws)| &ws.rows).flat_map(|r| &r.cells).filter(|c| matches!(c.value, CellValue::Shared(_))).count();
            let mut w = craft_xml::Writer::document();
            w.tag("sst").attr("xmlns", crate::ns::MAIN).attr("count", count).attr("uniqueCount", self.shared_strings.len()).open();
            for s in &self.shared_strings {
                w.tree(&s.to_el("si"));
            }
            out.xml_part(name, CT_STRINGS, w.into_bytes());
        }
        if let Some((name, s)) = &styles {
            out.xml_part(name, CT_STYLES, s.to_xml());
        }
        for p in &self.parts {
            match &p.content_type {
                Some(ct) => out.binary(&p.name, ct, p.bytes.clone()),
                None => out.raw(&p.name, p.bytes.clone()),
            }
        }
        Ok(out.finish()?)
    }
}

/// A workbook relationship to a part that [`Document::write`] makes again (or drops).
fn regenerated(r: &Rel) -> bool {
    matches!(r.kind(), "sharedStrings" | "styles" | "calcChain")
}

/// A relationship as read, to be written back with its id and target as written.
fn kept(r: &Rel) -> RelOut {
    let ty = match r.ty.strip_prefix(RT_STRICT) {
        Some(kind) => format!("{RT_BASE}{kind}"),
        None => r.ty.clone(),
    };
    RelOut { id: r.id.clone(), ty, target: r.target_raw.clone(), external: r.external }
}

/// An `rIdN` no relationship in `rels` has.
fn fresh_id(rels: &RelsOut) -> String {
    fresh(&rels.items.iter().map(|r| r.id.to_ascii_lowercase()).collect::<Vec<_>>(), |n| format!("rId{n}"))
}

/// The first of `name(1)`, `name(2)`, ... that is not `taken` (compared in lower case).
fn fresh(taken: &[String], name: impl Fn(usize) -> String) -> String {
    (1..).map(name).find(|n| !taken.contains(&n.to_ascii_lowercase())).unwrap_or_default()
}
