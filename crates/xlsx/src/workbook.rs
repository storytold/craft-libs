//! The workbook part (`xl/workbook.xml`, ECMA-376 Part 1, 18.2).

use craft_opc::Rels;
use craft_sheet::Visibility;
use craft_xml::El;

use crate::write::{assemble, flag, set};

/// The children of a workbook in schema order (`CT_Workbook`).
const WORKBOOK: &[&str] = &[
    "fileVersion",
    "fileSharing",
    "workbookPr",
    "workbookProtection",
    "bookViews",
    "sheets",
    "functionGroups",
    "externalReferences",
    "definedNames",
    "calcPr",
    "oleSize",
    "customWorkbookViews",
    "pivotCaches",
    "smartTagPr",
    "smartTagTypes",
    "webPublishing",
    "fileRecoveryPr",
    "webPublishObjects",
    "extLst",
];

/// `<definedName>` attributes read into [`DefinedName`]'s fields.
const NAME_ATTRS: &[&str] = &["name", "localSheetId", "comment", "hidden"];

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
    /// The relationship id (`r:id`, or a plain `id` from a writer that left the namespace
    /// out), empty when absent.
    pub rel_id: String,
    pub kind: SheetKind,
    /// The sheet's part, `None` when the relationship is missing or points outside the package.
    pub part: Option<String>,
}

/// One `<definedName>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefinedName {
    /// The name as written, built-in names included (`_xlnm.Print_Area`, `_xlnm._FilterDatabase`).
    pub name: String,
    /// `localSheetId`: an index into [`Workbook::sheets`] for a sheet-scoped name.
    pub local_sheet_id: Option<u32>,
    /// The formula text as the file spells it, trimmed, without a leading `=`.
    pub formula: String,
    pub comment: String,
    pub hidden: bool,
    /// The other attributes as written (`function`, `vbProcedure`, `shortcutKey`, ...).
    pub attrs: Vec<(String, String)>,
}

/// `<pivotCache>`: a pivot cache of the workbook, by the id its PivotTables name it with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PivotCacheEntry {
    pub cache_id: u32,
    /// The relationship to the cache definition part.
    pub rel_id: String,
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
    /// `pivotCaches`, in order.
    pub pivot_caches: Vec<PivotCacheEntry>,
    /// The whole part, for everything not lifted out above (calculation settings, views,
    /// protection, pivot caches, external references). Empty for a binary workbook.
    pub root: El,
}

impl Workbook {
    pub(crate) fn from_xml(root: El, rels: &Rels) -> Workbook {
        let mut wb = Workbook { date_1904: root.child("workbookPr").is_some_and(|p| p.flag("date1904", false)), ..Workbook::default() };
        if let Some(sheets) = root.child("sheets") {
            for s in sheets.children("sheet") {
                let rel_id = s.attr_or_plain("r:id").unwrap_or_default().to_string();
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
                    attrs: d.attrs.iter().filter(|(k, _)| !NAME_ATTRS.contains(&k.as_str())).cloned().collect(),
                });
            }
        }
        if let Some(caches) = root.child("pivotCaches") {
            wb.pivot_caches = caches
                .children("pivotCache")
                .filter_map(|c| Some(PivotCacheEntry { cache_id: c.attr_u32("cacheId")?, rel_id: c.attr_or_plain("r:id")?.to_string() }))
                .collect();
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

    /// The workbook part's root: `sheets` and `definedNames` regenerated from the fields, the
    /// date system set in `workbookPr` and the active tab in the view it was read from;
    /// `pivotCaches` from [`Workbook::pivot_caches`]; everything else in [`Workbook::root`] as
    /// kept. Each sheet's `r:id` is written as [`SheetEntry::rel_id`] holds it.
    pub fn to_el(&self) -> El {
        let root = if self.root.name.is_empty() { El::new("workbook") } else { self.root.clone() };
        let mut pr = root.child("workbookPr").cloned().unwrap_or_else(|| El::new("workbookPr"));
        set(&mut pr, "date1904", flag(self.date_1904));
        let mut views = root.child("bookViews").cloned();
        if let Some(tab) = self.active_tab {
            let views = views.get_or_insert_with(|| El::new("bookViews"));
            if views.child("workbookView").is_none() {
                views.push(El::new("workbookView"));
            }
            let visible = views.children("workbookView").position(|v| v.attr("visibility").unwrap_or("visible") == "visible").unwrap_or(0);
            if let Some(v) = views.elements_mut().filter(|v| v.name == "workbookView").nth(visible) {
                v.set_attr("activeTab", tab);
            }
        }
        let mut sheets = El::new("sheets");
        for s in &self.sheets {
            let mut e = El::new("sheet").with_attr("name", &s.name).with_attr("sheetId", s.sheet_id);
            set(
                &mut e,
                "state",
                match s.visibility {
                    Visibility::Visible => None,
                    Visibility::Hidden => Some("hidden".into()),
                    Visibility::VeryHidden => Some("veryHidden".into()),
                },
            );
            e.set_attr("r:id", &s.rel_id);
            sheets.push(e);
        }
        let mut names = El::new("definedNames");
        for d in &self.defined_names {
            let mut e = El::new("definedName").with_attr("name", &d.name);
            e.attrs.extend(d.attrs.iter().cloned());
            set(&mut e, "comment", (!d.comment.is_empty()).then(|| d.comment.clone()));
            set(&mut e, "hidden", flag(d.hidden));
            set(&mut e, "localSheetId", d.local_sheet_id.map(|v| v.to_string()));
            names.push(if d.formula.is_empty() { e } else { e.with_text(&d.formula) });
        }
        let owned = vec![
            ("workbookPr", Some(pr)),
            ("bookViews", views),
            ("sheets", Some(sheets)),
            ("definedNames", (!self.defined_names.is_empty()).then_some(names)),
            (
                "pivotCaches",
                (!self.pivot_caches.is_empty()).then(|| {
                    let mut e = El::new("pivotCaches");
                    self.pivot_caches
                        .iter()
                        .for_each(|c| e.push(El::new("pivotCache").with_attr("cacheId", c.cache_id).with_attr("r:id", &c.rel_id)));
                    e
                }),
            ),
        ];
        assemble(&root, WORKBOOK, owned)
    }
}
