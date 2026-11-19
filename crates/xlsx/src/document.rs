//! A whole workbook held in memory and written back: the parts this crate reads, regenerated,
//! and every other part carried through as it was.

use craft_opc::{ContentTypes, PackageOut, RT_BASE, Rel, RelOut, Rels, RelsOut, relative, rels_path};

use craft_drawingml::chart::Chart;

use crate::drawing::{Anchor, Drawing, Item, Placement};
use crate::features::{Comments, NoteShape, Table, notes_vml};
use crate::pivot::{PivotCacheDefinition, PivotTableDefinition};
use crate::richdata::{CellPicture, metadata_parts};
use crate::sheet::{CellValue, Row, SheetPart};
use crate::strings::StringItem;
use crate::styles::Styles;
use crate::workbook::{PivotCacheEntry, SheetKind, Workbook};
use crate::{Error, Limits, Package};

const CT_WORKSHEET: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CT_STRINGS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const CT_COMMENTS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";
const CT_VML: &str = "application/vnd.openxmlformats-officedocument.vmlDrawing";
const CT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";
const CT_DRAWING: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
const CT_CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const CT_CHARTEX: &str = "application/vnd.ms-office.chartex+xml";
const CT_PIVOT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";
const CT_PIVOT_CACHE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
const CT_PIVOT_RECORDS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml";
const RT_CHARTEX: &str = "http://schemas.microsoft.com/office/2014/relationships/chartEx";
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

/// One worksheet: its rows, the rest of its part, and its comments.
#[derive(Clone, Debug, Default)]
pub struct Worksheet {
    pub rows: Vec<Row>,
    pub part: SheetPart,
    /// The comments part, written from here. `None` removes it.
    pub comments: Option<Comments>,
    /// How the comments are shown, from the legacy VML drawing. The drawing itself is kept as
    /// read; these are used only to draw the comments of a sheet that has no VML drawing.
    pub notes: Vec<NoteShape>,
    /// The drawing part, written from here. `None` removes it.
    pub drawing: Option<DrawingPart>,
    /// The table parts, written from here, in the order of the sheet's `tableParts`.
    pub tables: Vec<TablePart>,
    /// The PivotTable parts, written from here. A PivotTable that did not read, or whose cache
    /// did not, is a kept part.
    pub pivot_tables: Vec<PivotTablePart>,
}

/// A PivotTable part. It is related to the cache of [`Document::pivot_caches`] its `cache_id`
/// names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PivotTablePart {
    /// The part's name; a new name is chosen on writing when empty.
    pub part: String,
    pub table: PivotTableDefinition,
}

/// A pivot cache: its definition part and its records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PivotCachePart {
    /// The definition part's name; a new name is chosen on writing when empty.
    pub part: String,
    /// The id the workbook and its PivotTables know the cache by.
    pub cache_id: u32,
    pub definition: PivotCacheDefinition,
    /// The records part's name; a new name is chosen on writing when empty.
    pub records_part: String,
    /// The records part as written ([`crate::records_xml`] makes one); `None` for a cache saved
    /// without records (`saveData="0"`).
    pub records: Option<Vec<u8>>,
}

/// A table part. Its relationships (a query table) are kept parts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TablePart {
    /// The part's name; a new name is chosen on writing when empty.
    pub part: String,
    pub table: Table,
}

/// A worksheet's drawing part and its relationships (to charts and pictures, kept with their
/// ids).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawingPart {
    /// The part's name; a new name is chosen on writing when empty.
    pub part: String,
    pub drawing: Drawing,
    /// `target` is the part name for an internal relationship; the target written is made
    /// relative to the drawing part again.
    pub rels: Vec<Rel>,
}

impl DrawingPart {
    /// Adds a relationship to part `target` (`kind` a last segment of the standard namespace or
    /// a full type URI) and returns its id.
    pub fn add_rel(&mut self, kind: &str, target: &str) -> String {
        let taken: Vec<String> = self.rels.iter().map(|r| r.id.to_ascii_lowercase()).collect();
        let id = fresh(&taken, |n| format!("rId{n}"));
        let ty = if kind.starts_with("http") { kind.to_string() } else { format!("{RT_BASE}{kind}") };
        self.rels.push(Rel { id: id.clone(), ty, target_raw: String::new(), target: target.to_string(), external: false });
        id
    }

    /// The next free `xdr:cNvPr/@id`.
    fn next_shape_id(&self) -> u32 {
        let ids = self.drawing.items.iter().filter_map(|i| match i {
            Item::Anchor(a) => a.props().and_then(|p| p.attr_u32("id")),
            Item::Other(e) => e.descendants().filter(|d| d.name == "xdr:cNvPr").filter_map(|d| d.attr_u32("id")).max(),
        });
        ids.max().unwrap_or(1).saturating_add(1)
    }
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
    /// The pivot caches, written from here with their entries in [`Workbook::pivot_caches`]. A
    /// cache that did not read (OLAP data, relationships other than its records) is a kept part
    /// whose entry stays.
    pub pivot_caches: Vec<PivotCachePart>,
    /// The pictures in cells, read from the metadata and rich-value parts: entry `vm - 1` for a
    /// cell's `vm` (see [`Package::cell_pictures`]). Those parts stay kept parts, written as
    /// read; this is not written. [`Document::add_cell_metadata`] makes new ones.
    pub cell_pictures: Vec<Option<CellPicture>>,
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
        let cell_pictures = p.cell_pictures()?;
        let mut pivot_caches = Vec::new();
        for entry in &workbook.pivot_caches {
            let Some(rel) = workbook_rels.by_id(&entry.rel_id).filter(|r| !r.external && r.kind() == "pivotCacheDefinition") else { continue };
            let rels = p.opc().rels(&rel.target)?;
            if rels.list.iter().any(|r| r.kind() != "pivotCacheRecords" || r.external) {
                continue;
            }
            let Some(definition) = p.pivot_cache(&rel.target)? else { continue };
            let records_part = rels.first("pivotCacheRecords").map(|r| r.target.clone()).unwrap_or_default();
            let records = if records_part.is_empty() { None } else { p.opc().read(&records_part)? };
            pivot_caches.push(PivotCachePart { part: rel.target.clone(), cache_id: entry.cache_id, definition, records_part, records });
        }
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
                Ok(part) => {
                    let comments = p.comments(&part)?;
                    let notes = if comments.is_some() { p.note_shapes(&part)? } else { Vec::new() };
                    let drawing = p.drawing(&part)?.map(|(name, drawing, rels)| DrawingPart { part: name, drawing, rels: rels.list });
                    let tables = p.tables(&part)?.into_iter().map(|(part, table)| TablePart { part, table }).collect();
                    let mut pivot_tables = Vec::new();
                    for (name, table, cache) in p.pivot_tables(&workbook, &part)? {
                        // Typed only when its cache is, by the same id, and it relates to nothing else.
                        let typed = pivot_caches.iter().any(|c| Some(&c.part) == cache.as_ref() && Some(c.cache_id) == table.cache_id);
                        let rels = p.opc().rels(&name)?;
                        if typed && rels.list.iter().all(|r| r.kind() == "pivotCacheDefinition" && !r.external) {
                            pivot_tables.push(PivotTablePart { part: name, table });
                        }
                    }
                    worksheets.push(Some(Worksheet { rows, part, comments, notes, drawing, tables, pivot_tables }));
                }
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
            if ws.comments.is_some() {
                made.extend(ws.part.rels.first("comments").filter(|r| !r.external).map(|r| r.target.clone()));
            }
            if let Some(d) = &ws.drawing {
                made.push(d.part.clone());
                made.push(rels_path(&d.part));
            }
            made.extend(ws.tables.iter().map(|t| t.part.clone()));
            made.extend(ws.pivot_tables.iter().flat_map(|t| [t.part.clone(), rels_path(&t.part)]));
        }
        for c in &pivot_caches {
            made.extend([c.part.clone(), rels_path(&c.part), c.records_part.clone()]);
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
        Ok(Document {
            kind,
            workbook_part,
            workbook,
            workbook_rels,
            shared_strings,
            styles,
            worksheets,
            pivot_caches,
            cell_pictures,
            parts,
            warnings: p.warnings().to_vec(),
        })
    }

    /// Adds a chart to worksheet `sheet` (an index into [`Workbook::sheets`]): a new chart part
    /// written by `craft-drawingml` (an Office 2016 `chartEx` part for the chart types that need
    /// one, behind an `mc:AlternateContent` so older readers skip it), and an anchor for it in
    /// the sheet's drawing, made when the sheet has none.
    pub fn add_chart(&mut self, sheet: usize, placement: Placement, chart: &Chart, name: &str) -> Result<(), Error> {
        let taken = self.part_names();
        let ex = craft_drawingml::chart::write::chartex_part(chart);
        let (part, ct, bytes) = match ex {
            Some((bytes, _)) => (fresh(&taken, |n| format!("xl/charts/chartEx{n}.xml")), CT_CHARTEX, bytes),
            None => (fresh(&taken, |n| format!("xl/charts/chart{n}.xml")), CT_CHART, craft_drawingml::chart::write::chart_xml(chart).0),
        };
        let d = self.drawing_of(sheet)?;
        let id = d.next_shape_id();
        if ct == CT_CHARTEX {
            let rel = d.add_rel(RT_CHARTEX, &part);
            let mut anchor = Anchor::chart(placement, id, name, &rel);
            if let Some(data) = anchor.object.child_mut("a:graphic").and_then(|g| g.child_mut("a:graphicData")) {
                data.set_attr("uri", "http://schemas.microsoft.com/office/drawing/2014/chartex");
                if let Some(c) = data.child_mut("c:chart") {
                    c.name = "cx:chart".into();
                }
            }
            let choice = craft_xml::El::new("mc:Choice").with_attr("Requires", "cx1").with_child(anchor.to_el());
            d.drawing
                .items
                .push(Item::Other(craft_xml::El::new("mc:AlternateContent").with_child(choice).with_child(craft_xml::El::new("mc:Fallback"))));
        } else {
            let rel = d.add_rel("chart", &part);
            d.drawing.items.push(Item::Anchor(Anchor::chart(placement, id, name, &rel)));
        }
        self.parts.push(KeptPart { name: part, content_type: Some(ct.into()), bytes });
        Ok(())
    }

    /// Adds a picture to worksheet `sheet`: the image bytes as a new media part (`ext` its file
    /// extension: `png`, `jpeg`, `gif`, ...), and an anchor for it in the sheet's drawing.
    pub fn add_picture(&mut self, sheet: usize, placement: Placement, image: Vec<u8>, ext: &str, descr: &str) -> Result<(), Error> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        let ct = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "tif" | "tiff" => "image/tiff",
            "svg" => "image/svg+xml",
            "emf" => "image/x-emf",
            "wmf" => "image/x-wmf",
            "webp" => "image/webp",
            _ => return Err(Error::Invalid(format!("no image type for .{ext}"))),
        };
        let part = fresh(&self.part_names(), |n| format!("xl/media/image{n}.{ext}"));
        let d = self.drawing_of(sheet)?;
        let id = d.next_shape_id();
        let rel = d.add_rel("image", &part);
        d.drawing.items.push(Item::Anchor(Anchor::picture(placement, id, &format!("Picture {}", id.saturating_sub(1)), descr, &rel)));
        self.parts.push(KeptPart { name: part, content_type: Some(ct.into()), bytes: image });
        Ok(())
    }

    /// Adds the cell metadata part, and for `pictures` the rich-value parts and their images:
    /// a cell shows picture `i` with `vm = i + 1`, and with `dynamic_arrays` a dynamic-array
    /// formula is marked by `cm = 1`. Refused for a workbook that has cell metadata already,
    /// and for a picture that is not a PNG or JPEG Excel shows in a cell
    /// ([`CellPicture::image_type`]) or over the size limits.
    pub fn add_cell_metadata(&mut self, pictures: &[CellPicture], dynamic_arrays: bool) -> Result<(), Error> {
        if self.workbook_rels.list.iter().any(|r| r.kind() == "sheetMetadata") {
            return Err(Error::Invalid("the workbook has cell metadata already".into()));
        }
        let mut taken = self.part_names();
        let made = metadata_parts(pictures, dynamic_arrays, |stem, ext, numbered| {
            // `xl/metadata.xml`, `xl/media/cellImage1.png`, as Excel names them.
            let name = fresh(&taken, |n| if n == 1 && !numbered { format!("{stem}.{ext}") } else { format!("{stem}{n}.{ext}") });
            taken.push(name.to_ascii_lowercase());
            name
        })?;
        let book = if self.workbook_part.is_empty() { "xl/workbook.xml".to_string() } else { self.workbook_part.clone() };
        for (ty, target) in made.workbook_rels {
            let ids: Vec<String> = self.workbook_rels.list.iter().map(|r| r.id.to_ascii_lowercase()).collect();
            let id = fresh(&ids, |n| format!("rId{n}"));
            self.workbook_rels.list.push(Rel { id, ty: ty.into(), target_raw: relative(&book, &target), target, external: false });
        }
        for (name, ct, bytes) in made.parts {
            self.parts.push(KeptPart { name, content_type: ct.map(str::to_string), bytes });
        }
        Ok(())
    }

    /// Every part name in use, in lower case: kept parts and the drawings of worksheets.
    fn part_names(&self) -> Vec<String> {
        let drawings = self.worksheets.iter().flatten().filter_map(|w| w.drawing.as_ref()).map(|d| d.part.clone());
        self.parts.iter().map(|p| p.name.clone()).chain(drawings).map(|n| n.to_ascii_lowercase()).collect()
    }

    fn drawing_of(&mut self, sheet: usize) -> Result<&mut DrawingPart, Error> {
        let ws =
            self.worksheets.get_mut(sheet).and_then(Option::as_mut).ok_or_else(|| Error::Invalid(format!("sheet {sheet} is not a worksheet")))?;
        Ok(ws.drawing.get_or_insert_with(DrawingPart::default))
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
        let kept_part = |r: &Rel| self.parts.iter().any(|p| p.name.eq_ignore_ascii_case(&r.target));
        let keep = |r: &&Rel| match r.kind() {
            "worksheet" => unread(r),
            // A cache this writes is related again; one removed goes with its relationship.
            "pivotCacheDefinition" => r.external || kept_part(r),
            _ => !regenerated(r),
        };
        for r in self.workbook_rels.list.iter().filter(keep) {
            rels.items.push(kept(r));
        }
        let mut workbook = self.workbook.clone();
        let mut sheet_parts = Vec::new();
        // Every name a part keeps is taken before any new part is named, so a new one never
        // takes the name of a part written later.
        let mut taken: Vec<String> = self.parts.iter().map(|p| p.name.to_ascii_lowercase()).collect();
        for ws in self.worksheets.iter().flatten() {
            let comments = ws.comments.as_ref().and(ws.part.rels.first("comments")).map(|r| r.target.clone());
            let own = [ws.part.part.clone()].into_iter().chain(comments).chain(ws.drawing.iter().map(|d| d.part.clone()));
            let own = own.chain(ws.tables.iter().map(|t| t.part.clone())).chain(ws.pivot_tables.iter().map(|t| t.part.clone()));
            taken.extend(own.filter(|n| !n.is_empty()).map(|n| n.to_ascii_lowercase()));
        }
        for c in &self.pivot_caches {
            taken.extend([&c.part, &c.records_part].into_iter().filter(|n| !n.is_empty()).map(|n| n.to_ascii_lowercase()));
        }
        for (i, (entry, ws)) in workbook.sheets.iter_mut().zip(&self.worksheets).enumerate() {
            let Some(ws) = ws else {
                if rels.items.iter().any(|r| r.id == entry.rel_id) {
                    continue;
                }
                return Err(Error::Invalid(format!("sheet \"{}\" has neither a worksheet nor a kept part", entry.name)));
            };
            let mut name = ws.part.part.clone();
            if name.is_empty() {
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
        // Pivot caches: each written cache related again (with the id it had), and the entries of
        // kept caches whose relationship stays.
        let mut caches: Vec<(u32, String)> = Vec::new();
        let mut entries: Vec<PivotCacheEntry> = Vec::new();
        for c in &self.pivot_caches {
            if caches.iter().any(|(id, _)| *id == c.cache_id) {
                return Err(Error::Invalid(format!("two pivot caches have the id {}", c.cache_id)));
            }
            let name = if c.part.is_empty() { fresh(&taken, |n| format!("xl/pivotCache/pivotCacheDefinition{n}.xml")) } else { c.part.clone() };
            taken.push(name.to_ascii_lowercase());
            let old = self
                .workbook_rels
                .list
                .iter()
                .find(|r| r.kind() == "pivotCacheDefinition" && r.target.eq_ignore_ascii_case(&name) && !rels.items.iter().any(|o| o.id == r.id));
            let id = old.map_or_else(|| fresh_id(&rels), |r| r.id.clone());
            rels.items.push(RelOut {
                id: id.clone(),
                ty: format!("{RT_BASE}pivotCacheDefinition"),
                target: relative(book_part, &name),
                external: false,
            });
            entries.push(PivotCacheEntry { cache_id: c.cache_id, rel_id: id });
            let mut crels = RelsOut::default();
            let records_rel = c.records.as_ref().map(|bytes| {
                let rec = if c.records_part.is_empty() {
                    fresh(&taken, |n| format!("xl/pivotCache/pivotCacheRecords{n}.xml"))
                } else {
                    c.records_part.clone()
                };
                taken.push(rec.to_ascii_lowercase());
                out.binary(&rec, CT_PIVOT_RECORDS, bytes.clone());
                crels.add("pivotCacheRecords", &relative(&name, &rec), false)
            });
            out.xml_part(&name, CT_PIVOT_CACHE, c.definition.to_xml(records_rel.as_deref()));
            out.rels(&name, &crels);
            caches.push((c.cache_id, name));
        }
        for e in &self.workbook.pivot_caches {
            let target = rels.items.iter().find(|r| r.id == e.rel_id).and_then(|_| self.workbook_rels.by_id(&e.rel_id));
            if let Some(r) = target.filter(|_| !caches.iter().any(|(id, _)| *id == e.cache_id)) {
                caches.push((e.cache_id, r.target.clone()));
                entries.push(e.clone());
            }
        }
        workbook.pivot_caches = entries;
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
        let mut blocks = vml_blocks(&self.parts);
        let kept_names: Vec<String> = self.parts.iter().map(|p| p.name.to_ascii_lowercase()).collect();
        for (name, ws) in &sheet_parts {
            write_sheet(&mut out, name, ws, &kept_names, &mut taken, &mut blocks, &caches)?;
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

/// A worksheet, its relationships, and its comments with a VML drawing to show them when it has
/// none. A hyperlink to a URL its relationship does not lead to gets a relationship of its own.
fn write_sheet(
    out: &mut PackageOut,
    name: &str,
    ws: &Worksheet,
    kept_names: &[String],
    taken: &mut Vec<String>,
    blocks: &mut Vec<u32>,
    caches: &[(u32, String)],
) -> Result<(), Error> {
    let mut part = ws.part.clone();
    let mut rels = RelsOut::default();
    let kept_part = |r: &Rel| kept_names.contains(&r.target.to_ascii_lowercase());
    // A comments or drawing relationship stays when this writes that part, or the part is kept.
    let stays = |r: &&Rel| match r.kind() {
        "comments" => ws.comments.is_some(),
        "drawing" => ws.drawing.as_ref().map_or(kept_part(r), |d| d.part.eq_ignore_ascii_case(&r.target)),
        "table" => ws.tables.iter().any(|t| t.part.eq_ignore_ascii_case(&r.target)) || kept_part(r),
        "pivotTable" => ws.pivot_tables.iter().any(|t| t.part.eq_ignore_ascii_case(&r.target)) || kept_part(r),
        _ => true,
    };
    rels.items.extend(ws.part.rels.list.iter().filter(stays).map(kept));
    if ws.drawing.is_none() && !rels.items.iter().any(|r| part.features.drawing.as_deref() == Some(&r.id)) {
        part.features.drawing = None;
    }
    if let Some(d) = &ws.drawing {
        let target = if d.part.is_empty() { fresh(taken, |n| format!("xl/drawings/drawing{n}.xml")) } else { d.part.clone() };
        taken.push(target.to_ascii_lowercase());
        let id = match ws.part.rels.list.iter().find(|r| r.kind() == "drawing" && r.target.eq_ignore_ascii_case(&target)) {
            Some(r) => r.id.clone(),
            None => {
                let id = fresh_id(&rels);
                rels.items.push(RelOut { id: id.clone(), ty: format!("{RT_BASE}drawing"), target: relative(name, &target), external: false });
                id
            }
        };
        part.features.drawing = Some(id);
        out.xml_part(&target, CT_DRAWING, d.drawing.to_xml());
        let mut drels = RelsOut::default();
        for r in &d.rels {
            let mut o = kept(r);
            if !r.external {
                o.target = relative(&target, &r.target);
            }
            drels.items.push(o);
        }
        out.rels(&target, &drels);
    }
    for h in &mut part.features.hyperlinks {
        let Some(url) = &h.url else { continue };
        let ok = h.rel_id.as_deref().and_then(|id| ws.part.rels.by_id(id)).is_some_and(|r| r.external && r.target == *url);
        if !ok {
            let id = fresh_id(&rels);
            rels.items.push(RelOut { id: id.clone(), ty: format!("{RT_BASE}hyperlink"), target: url.clone(), external: true });
            h.rel_id = Some(id);
        }
    }
    if let Some(c) = &ws.comments {
        let target = match ws.part.rels.first("comments").filter(|r| !r.external) {
            Some(r) => r.target.clone(),
            None => {
                let target = fresh(taken, |n| format!("xl/comments{n}.xml"));
                let id = fresh_id(&rels);
                rels.items.push(RelOut { id, ty: format!("{RT_BASE}comments"), target: relative(name, &target), external: false });
                target
            }
        };
        taken.push(target.to_ascii_lowercase());
        out.xml_part(&target, CT_COMMENTS, c.to_xml());
        if !rels.items.iter().any(|r| r.ty.ends_with("/vmlDrawing")) && !c.comments.is_empty() {
            let block = (1..).find(|b| !blocks.contains(b)).unwrap_or(1);
            blocks.push(block);
            let vml = fresh(taken, |n| format!("xl/drawings/vmlDrawing{n}.vml"));
            taken.push(vml.to_ascii_lowercase());
            let id = fresh_id(&rels);
            rels.items.push(RelOut { id: id.clone(), ty: format!("{RT_BASE}vmlDrawing"), target: relative(name, &vml), external: false });
            part.features.legacy_drawing = Some(id);
            out.binary(&vml, CT_VML, notes_vml(&c.comments, &ws.notes, block));
        }
    }
    let mut table_ids = Vec::new();
    for t in &ws.tables {
        let target = if t.part.is_empty() { fresh(taken, |n| format!("xl/tables/table{n}.xml")) } else { t.part.clone() };
        taken.push(target.to_ascii_lowercase());
        let id = match ws.part.rels.list.iter().find(|r| r.kind() == "table" && r.target.eq_ignore_ascii_case(&target)) {
            Some(r) => r.id.clone(),
            None => {
                let id = fresh_id(&rels);
                rels.items.push(RelOut { id: id.clone(), ty: format!("{RT_BASE}table"), target: relative(name, &target), external: false });
                id
            }
        };
        table_ids.push(id);
        out.xml_part(&target, CT_TABLE, t.table.to_xml());
    }
    // Tables kept as parts (unreadable ones) keep their place after those written.
    let kept_tables =
        part.features.table_parts.iter().filter(|id| !table_ids.contains(id) && rels.items.iter().any(|r| r.id == **id && r.ty.ends_with("/table")));
    table_ids.extend(kept_tables.cloned().collect::<Vec<_>>());
    part.features.table_parts = table_ids;
    for t in &ws.pivot_tables {
        let cache = t.table.cache_id.and_then(|id| caches.iter().find(|(c, _)| *c == id));
        let Some((_, cache)) = cache else {
            return Err(Error::Invalid(format!("PivotTable {:?} names no pivot cache of the workbook", t.table.name.as_deref().unwrap_or(""))));
        };
        let target = if t.part.is_empty() { fresh(taken, |n| format!("xl/pivotTables/pivotTable{n}.xml")) } else { t.part.clone() };
        taken.push(target.to_ascii_lowercase());
        if !ws.part.rels.list.iter().any(|r| r.kind() == "pivotTable" && r.target.eq_ignore_ascii_case(&target)) {
            let id = fresh_id(&rels);
            rels.items.push(RelOut { id, ty: format!("{RT_BASE}pivotTable"), target: relative(name, &target), external: false });
        }
        out.xml_part(&target, CT_PIVOT_TABLE, t.table.to_xml());
        let mut trels = RelsOut::default();
        trels.add("pivotCacheDefinition", &relative(&target, cache), false);
        out.rels(&target, &trels);
    }
    out.xml_part(name, CT_WORKSHEET, part.to_xml(&ws.rows));
    out.rels(name, &rels);
    Ok(())
}

/// The shape-id blocks (`o:idmap`) the kept VML drawings use, so a new drawing takes another.
fn vml_blocks(parts: &[KeptPart]) -> Vec<u32> {
    let mut used = Vec::new();
    for p in parts.iter().filter(|p| p.name.to_ascii_lowercase().ends_with(".vml")) {
        let text = String::from_utf8_lossy(&p.bytes);
        for (_, after) in text.match_indices("idmap").map(|(i, _)| text.split_at(i)) {
            let data = after.split("data=\"").nth(1).and_then(|d| d.split('"').next()).unwrap_or("");
            used.extend(data.split(',').filter_map(|n| n.trim().parse::<u32>().ok()));
        }
    }
    used
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
