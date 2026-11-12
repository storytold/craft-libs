//! The workbook part (`xl/workbook.xml`, ECMA-376 Part 1, 18.2).

use craft_opc::Rels;
use craft_sheet::Visibility;
use craft_xml::El;

/// What kind of sheet a workbook entry points at, from its relationship type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SheetKind {
    Worksheet,
    Chartsheet,
    Dialogsheet,
    /// An Excel 4.0 macro sheet (`xlMacrosheet`, `xlIntlMacrosheet`).
    Macrosheet,
    /// Another relationship type, by its last segment.
    Other(String),
}

impl SheetKind {
    pub(crate) fn of(kind: &str) -> SheetKind {
        match kind {
            "worksheet" => SheetKind::Worksheet,
            "chartsheet" => SheetKind::Chartsheet,
            "dialogsheet" => SheetKind::Dialogsheet,
            "xlMacrosheet" | "xlIntlMacrosheet" => SheetKind::Macrosheet,
            other => SheetKind::Other(other.to_string()),
        }
    }
}

/// One `<sheet>` of the workbook, in tab order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetEntry {
    /// The name as the file has it: not checked against Excel's rules for sheet names and not
    /// made unique.
    pub name: String,
    /// `sheetId`, 0 when absent.
    pub sheet_id: u32,
    pub visibility: Visibility,
    /// The relationship id (`r:id`), empty when absent.
    pub rel_id: String,
    pub kind: SheetKind,
    /// The sheet's part, `None` when the relationship is missing or points outside the package.
    pub part: Option<String>,
}

/// One `<definedName>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinedName {
    /// The name as written, built-in names included (`_xlnm.Print_Area`, `_xlnm._FilterDatabase`).
    pub name: String,
    /// `localSheetId`: an index into [`Workbook::sheets`] for a sheet-scoped name.
    pub local_sheet_id: Option<u32>,
    /// The formula text as the file spells it, trimmed, without a leading `=`.
    pub formula: String,
    pub comment: String,
    pub hidden: bool,
}

/// The workbook part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workbook {
    pub sheets: Vec<SheetEntry>,
    /// Dates count from 1904-01-01 instead of 1900 (`workbookPr/@date1904`).
    pub date_1904: bool,
    pub defined_names: Vec<DefinedName>,
    /// `activeTab` of the first visible workbook view (of the first view when none is visible).
    pub active_tab: Option<u32>,
    /// The whole part, for everything not lifted out above (calculation settings, views,
    /// protection, pivot caches, external references). Empty for a binary workbook.
    pub root: El,
}

impl Workbook {
    pub(crate) fn from_xml(root: El, rels: &Rels) -> Workbook {
        let mut wb = Workbook { date_1904: root.child("workbookPr").is_some_and(|p| p.flag("date1904", false)), ..Workbook::default() };
        if let Some(sheets) = root.child("sheets") {
            for s in sheets.children("sheet") {
                let rel_id = s.attr("id").unwrap_or_default().to_string();
                let rel = rels.by_id(&rel_id);
                wb.sheets.push(SheetEntry {
                    name: s.attr("name").unwrap_or_default().to_string(),
                    sheet_id: s.attr_u32("sheetId").unwrap_or(0),
                    visibility: match s.attr("state") {
                        Some("hidden") => Visibility::Hidden,
                        Some("veryHidden") => Visibility::VeryHidden,
                        _ => Visibility::Visible,
                    },
                    kind: rel.map_or(SheetKind::Other(String::new()), |r| SheetKind::of(r.kind())),
                    part: rel.filter(|r| !r.external).map(|r| r.target.clone()),
                    rel_id,
                });
            }
        }
        if let Some(names) = root.child("definedNames") {
            for d in names.children("definedName") {
                let Some(name) = d.attr("name") else { continue };
                wb.defined_names.push(DefinedName {
                    name: name.to_string(),
                    local_sheet_id: d.attr_u32("localSheetId"),
                    formula: d.text().trim().to_string(),
                    comment: d.attr("comment").unwrap_or_default().to_string(),
                    hidden: d.flag("hidden", false),
                });
            }
        }
        if let Some(views) = root.child("bookViews") {
            let view = views
                .children("workbookView")
                .find(|v| v.attr("visibility").unwrap_or("visible") == "visible")
                .or_else(|| views.child("workbookView"));
            wb.active_tab = view.and_then(|v| v.attr_u32("activeTab"));
        }
        wb.root = root;
        wb
    }
}
