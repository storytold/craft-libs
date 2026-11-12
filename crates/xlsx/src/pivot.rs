//! PivotTables (ECMA-376 Part 1, 18.10): the pivot cache definition (the source and its fields
//! with their shared items and grouping), the cache records, and the PivotTable definition (its
//! location, fields, axes, values and style).
//!
//! As for the worksheet features, an attribute the file may leave out is an `Option`, the
//! attributes a type does not name are kept in its `attrs`, and the children it does not model
//! (OLAP hierarchies, formats, filters, extensions) are kept as written. A cache whose items
//! carry OLAP member data does not read: it stays a kept part, so nothing is lost. A part is
//! read as one tree, so [`crate::Limits::xml`] bounds every list in it; records are streamed.

use craft_sheet::Range;
use craft_xml::{El, Node};

use crate::features::{Attr, kids, rest};
use crate::write::num;

/// One value of a pivot cache: a shared or group item, or a field of a record.
#[derive(Clone, Debug, PartialEq)]
pub enum CacheValue {
    /// `<m/>`: no value.
    Missing,
    /// `<n v>`
    Number(f64),
    /// `<b v>`
    Bool(bool),
    /// `<e v>`: the error as written (`#N/A`).
    Error(String),
    /// `<s v>`
    Text(String),
    /// `<d v>`: an ISO 8601 date-time as written (`2024-03-05T00:00:00`).
    Date(String),
    /// `<x v>`: an index into the field's shared items (records only).
    Index(u32),
}

impl CacheValue {
    /// `None` for an element that is not a value, or one with children (OLAP member tuples).
    fn read(e: &El) -> Option<CacheValue> {
        if e.elements().next().is_some() {
            return None;
        }
        let v = e.attr("v");
        Some(match e.name.as_str() {
            "m" => CacheValue::Missing,
            "n" => CacheValue::Number(craft_xml::value::to_f64(v?)?),
            "b" => CacheValue::Bool(craft_xml::value::to_bool(v?)?),
            "e" => CacheValue::Error(v?.to_string()),
            "s" => CacheValue::Text(v?.to_string()),
            "d" => CacheValue::Date(v?.to_string()),
            "x" => CacheValue::Index(craft_xml::value::to_u32(v.unwrap_or("0"))?),
            _ => return None,
        })
    }

    fn to_el(&self) -> El {
        match self {
            CacheValue::Missing => El::new("m"),
            CacheValue::Number(n) => El::new("n").with_attr("v", num(*n)),
            CacheValue::Bool(b) => El::new("b").with_attr("v", u8::from(*b)),
            CacheValue::Error(s) => El::new("e").with_attr("v", s),
            CacheValue::Text(s) => El::new("s").with_attr("v", s),
            CacheValue::Date(s) => El::new("d").with_attr("v", s),
            CacheValue::Index(i) => El::new("x").with_attr("v", i),
        }
    }
}

/// A shared or group item: its value and its other attributes (`u` unused, `f` formula, `c`
/// calculated, `cp`, `in`, ...).
#[derive(Clone, Debug, PartialEq)]
pub struct CacheItem {
    pub value: CacheValue,
    pub attrs: Vec<(String, String)>,
}

impl CacheItem {
    pub fn new(value: CacheValue) -> CacheItem {
        CacheItem { value, attrs: Vec::new() }
    }

    fn read(e: &El) -> Option<CacheItem> {
        Some(CacheItem { value: CacheValue::read(e)?, attrs: rest(e, &["v"]) })
    }

    fn to_el(&self) -> El {
        let mut e = self.value.to_el();
        e.attrs.extend(self.attrs.iter().cloned());
        e
    }
}

fn read_items(e: &El) -> Option<Vec<CacheItem>> {
    e.elements().map(CacheItem::read).collect()
}

fn items_el(name: &str, items: &[CacheItem]) -> El {
    let mut e = El::new(name).with_attr("count", items.len());
    items.iter().for_each(|i| e.push(i.to_el()));
    e
}

element! {
    /// `<sharedItems>`: what a field's values are, and the items an axis refers to by index.
    pub struct SharedItems {
        contains_semi_mixed_types: bool = "containsSemiMixedTypes",
        contains_non_date: bool = "containsNonDate",
        contains_date: bool = "containsDate",
        contains_string: bool = "containsString",
        contains_blank: bool = "containsBlank",
        contains_mixed_types: bool = "containsMixedTypes",
        contains_number: bool = "containsNumber",
        contains_integer: bool = "containsInteger",
        min_value: f64 = "minValue",
        max_value: f64 = "maxValue",
        min_date: String = "minDate",
        max_date: String = "maxDate",
        long_text: bool = "longText",
    }
    + {
        items: Vec<CacheItem>,
    }
}

impl SharedItems {
    /// The flags Excel writes for a field holding `values` (no items: add those that an axis
    /// needs). Booleans and errors count as strings, as Excel counts them; a field of dates
    /// (and blanks) gets the date flags and its first and last date.
    pub fn describe<'a>(values: impl IntoIterator<Item = &'a CacheValue>) -> SharedItems {
        let (mut blank, mut strings, mut long, mut dates, mut non_int) = (false, false, false, false, false);
        let mut range: Option<(f64, f64)> = None;
        let mut date_range: Option<(&str, &str)> = None;
        for v in values {
            match v {
                CacheValue::Missing => blank = true,
                CacheValue::Number(n) => {
                    non_int |= n.fract() != 0.0;
                    range = Some(range.map_or((*n, *n), |(a, b)| (a.min(*n), b.max(*n))));
                }
                CacheValue::Text(t) => {
                    strings = true;
                    long |= t.chars().count() > 255;
                }
                CacheValue::Bool(_) | CacheValue::Error(_) => strings = true,
                CacheValue::Date(d) => {
                    dates = true;
                    let d = d.as_str();
                    date_range = Some(date_range.map_or((d, d), |(a, b)| (a.min(d), b.max(d))));
                }
                CacheValue::Index(_) => {}
            }
        }
        let number = range.is_some();
        let some = |b: bool| b.then_some(true);
        if dates && !strings && !number {
            return SharedItems {
                contains_semi_mixed_types: Some(false),
                contains_non_date: Some(false),
                contains_date: Some(true),
                contains_string: Some(false),
                contains_blank: some(blank),
                min_date: date_range.map(|r| r.0.to_string()),
                max_date: date_range.map(|r| r.1.to_string()),
                ..SharedItems::default()
            };
        }
        let kinds = usize::from(strings) + usize::from(number) + usize::from(dates);
        SharedItems {
            contains_semi_mixed_types: (!strings && !blank).then_some(false),
            contains_date: some(dates),
            contains_string: (!strings).then_some(false),
            contains_blank: some(blank),
            contains_mixed_types: some(kinds > 1),
            contains_number: some(number),
            contains_integer: some(number && !non_int),
            min_value: range.map(|r| r.0),
            max_value: range.map(|r| r.1),
            min_date: date_range.map(|r| r.0.to_string()),
            max_date: date_range.map(|r| r.1.to_string()),
            long_text: some(long),
            ..SharedItems::default()
        }
    }
}

element! {
    /// `<rangePr>`: grouping by numeric or date ranges.
    pub struct RangePr {
        auto_start: bool = "autoStart",
        auto_end: bool = "autoEnd",
        /// `range`, `seconds`, `minutes`, `hours`, `days`, `months`, `quarters`, `years`.
        group_by: String = "groupBy",
        start_num: f64 = "startNum",
        end_num: f64 = "endNum",
        start_date: String = "startDate",
        end_date: String = "endDate",
        group_interval: f64 = "groupInterval",
    }
}

element! {
    /// `<fieldGroup>`: a field grouped from another (`base`), into ranges or chosen items.
    pub struct FieldGroup {
        /// The next coarser grouping field of the same base.
        par: u32 = "par",
        base: u32 = "base",
    }
    + {
        range: Option<RangePr>,
        /// `discretePr`: for each base item, the group item it falls in.
        discrete: Option<Vec<u32>>,
        /// `groupItems`: the groups' captions.
        items: Option<Vec<CacheItem>>,
    }
}

element! {
    /// `<cacheField>`: one column of the source, or a field grouped or calculated from others.
    pub struct CacheField {
        name: String = "name",
        caption: String = "caption",
        num_fmt_id: u32 = "numFmtId",
        /// A calculated field's formula, without `=`.
        formula: String = "formula",
        /// `false` for a grouping or calculated field.
        database_field: bool = "databaseField",
        sql_type: i32 = "sqlType",
        hierarchy: i32 = "hierarchy",
        level: u32 = "level",
    }
    + {
        shared_items: Option<SharedItems>,
        group: Option<FieldGroup>,
        /// `mpMap` and `extLst`, as written.
        other: Vec<El>,
    }
}

element! {
    /// `<worksheetSource>`: a range on a sheet (`reference` and `sheet`), or a defined name or
    /// table (`name`); `r:id` for a source in another workbook.
    pub struct WorksheetSource {
        reference: String = "ref",
        name: String = "name",
        sheet: String = "sheet",
        rel_id: String = "r:id",
    }
}

element! {
    /// `<cacheSource>`.
    pub struct CacheSource {
        /// `worksheet`, `external`, `consolidation`, `scenario`.
        source_type: String = "type",
        connection_id: u32 = "connectionId",
    }
    + {
        worksheet: Option<WorksheetSource>,
        /// `consolidation` and `extLst`, as written.
        other: Vec<El>,
    }
}

element! {
    /// A pivot cache definition (`<pivotCacheDefinition>`, 18.10.1.67). Its records part is
    /// related from it (`r:id`), which [`crate::Document`] writes.
    pub struct PivotCacheDefinition {
        invalid: bool = "invalid",
        /// `false`: no records are saved, and the cache is refreshed before use.
        save_data: bool = "saveData",
        refresh_on_load: bool = "refreshOnLoad",
        optimize_memory: bool = "optimizeMemory",
        enable_refresh: bool = "enableRefresh",
        refreshed_by: String = "refreshedBy",
        refreshed_date: f64 = "refreshedDate",
        background_query: bool = "backgroundQuery",
        missing_items_limit: u32 = "missingItemsLimit",
        created_version: u32 = "createdVersion",
        refreshed_version: u32 = "refreshedVersion",
        min_refreshable_version: u32 = "minRefreshableVersion",
        record_count: u32 = "recordCount",
        upgrade_on_refresh: bool = "upgradeOnRefresh",
    }
    + {
        source: CacheSource,
        fields: Vec<CacheField>,
        /// `cacheHierarchies`, `kpis`, `tupleCache`, `calculatedItems`, `calculatedMembers`,
        /// `dimensions`, `measureGroups`, `maps`, `extLst`, as written.
        other: Vec<El>,
    }
}

impl PivotCacheDefinition {
    /// A part read with [`crate::ns::CANONICAL`]. `None` when it is not a cache definition, or
    /// an item holds OLAP member data this model does not keep.
    pub fn read(x: &El) -> Option<PivotCacheDefinition> {
        if x.name != "pivotCacheDefinition" {
            return None;
        }
        let source = x.child("cacheSource").map_or_else(CacheSource::default, |s| CacheSource {
            worksheet: s.child("worksheetSource").map(WorksheetSource::read_attrs),
            other: kids(s, &["worksheetSource"]),
            ..CacheSource::read_attrs(s)
        });
        let mut fields = Vec::new();
        for f in x.child("cacheFields").into_iter().flat_map(|c| c.children("cacheField")) {
            let shared_items = match f.child("sharedItems") {
                Some(s) => Some(SharedItems { items: read_items(s)?, ..SharedItems::read_attrs(s) }),
                None => None,
            };
            let group = match f.child("fieldGroup") {
                Some(g) => Some(FieldGroup {
                    range: g.child("rangePr").map(RangePr::read_attrs),
                    discrete: g.child("discretePr").map(|d| d.children("x").filter_map(|x| x.attr_u32("v")).collect()),
                    items: match g.child("groupItems") {
                        Some(i) => Some(read_items(i)?),
                        None => None,
                    },
                    ..FieldGroup::read_attrs(g)
                }),
                None => None,
            };
            fields.push(CacheField { shared_items, group, other: kids(f, &["sharedItems", "fieldGroup"]), ..CacheField::read_attrs(f) });
        }
        let mut def = PivotCacheDefinition { source, fields, other: kids(x, &["cacheSource", "cacheFields"]), ..PivotCacheDefinition::read_attrs(x) };
        // The records relationship is the writer's.
        def.attrs.retain(|(k, _)| k != "r:id");
        Some(def)
    }

    /// The part, related to its records by `records_rel` when it has them.
    pub fn to_xml(&self, records_rel: Option<&str>) -> Vec<u8> {
        let mut e = El::new("pivotCacheDefinition");
        if let Some(id) = records_rel {
            e.set_attr("r:id", id);
        }
        self.write_attrs(&mut e);
        let mut src = El::new("cacheSource");
        self.source.write_attrs(&mut src);
        if let Some(w) = &self.source.worksheet {
            let mut we = El::new("worksheetSource");
            w.write_attrs(&mut we);
            src.push(we);
        }
        src.children.extend(self.source.other.iter().cloned().map(Node::El));
        e.push(src);
        let mut fields = El::new("cacheFields").with_attr("count", self.fields.len());
        for f in &self.fields {
            let mut fe = El::new("cacheField");
            f.write_attrs(&mut fe);
            if let Some(s) = &f.shared_items {
                let mut se = El::new("sharedItems");
                s.write_attrs(&mut se);
                if !s.items.is_empty() {
                    se.set_attr("count", s.items.len());
                    s.items.iter().for_each(|i| se.push(i.to_el()));
                }
                fe.push(se);
            }
            if let Some(g) = &f.group {
                let mut ge = El::new("fieldGroup");
                g.write_attrs(&mut ge);
                if let Some(r) = &g.range {
                    let mut re = El::new("rangePr");
                    r.write_attrs(&mut re);
                    ge.push(re);
                }
                if let Some(d) = &g.discrete {
                    let mut de = El::new("discretePr").with_attr("count", d.len());
                    d.iter().for_each(|v| de.push(El::new("x").with_attr("v", v)));
                    ge.push(de);
                }
                if let Some(items) = &g.items {
                    ge.push(items_el("groupItems", items));
                }
                fe.push(ge);
            }
            fe.children.extend(f.other.iter().cloned().map(Node::El));
            fields.push(fe);
        }
        e.push(fields);
        e.children.extend(self.other.iter().cloned().map(Node::El));
        document(e)
    }
}

/// One record of a pivot cache: a value per database field, in field order.
pub type CacheRecord = Vec<CacheValue>;

/// A pivot cache records part (`<pivotCacheRecords>`) for `records`.
pub fn records_xml(records: &[CacheRecord]) -> Vec<u8> {
    let mut w = craft_xml::Writer::document();
    w.tag("pivotCacheRecords").attr("xmlns", crate::ns::MAIN).attr("xmlns:r", crate::ns::R).attr("count", records.len()).open();
    for r in records {
        let mut e = El::new("r");
        r.iter().for_each(|v| e.push(v.to_el()));
        w.tree(&e);
    }
    w.into_bytes()
}

/// One `<r>` of a records part, read with [`crate::ns::CANONICAL`]; values with OLAP data and
/// unknown elements are left out, so a field may be missing.
pub(crate) fn read_record(r: &El) -> CacheRecord {
    r.elements().filter_map(CacheValue::read).collect()
}

element! {
    /// `<location>`: where the PivotTable is (its body: report filters sit above it).
    pub struct Location {
        first_header_row: u32 = "firstHeaderRow",
        first_data_row: u32 = "firstDataRow",
        first_data_col: u32 = "firstDataCol",
        row_page_count: u32 = "rowPageCount",
        col_page_count: u32 = "colPageCount",
    }
    + {
        range: Range,
    }
}

element! {
    /// `<item>` of a pivot field: a shared item shown (`x`), or a subtotal (`t`).
    pub struct PivotItem {
        /// The index of the item in the cache field's shared or group items.
        x: u32 = "x",
        /// `data` (the default), `default`, `sum`, `countA`, `avg`, ..., `grand`, `blank`.
        item_type: String = "t",
        /// Hidden (filtered out).
        h: bool = "h",
        /// Details shown (expanded); `false` is collapsed.
        sd: bool = "sd",
        /// A caption replacing the item's.
        n: String = "n",
        s: bool = "s",
        f: bool = "f",
        m: bool = "m",
        c: bool = "c",
        d: bool = "d",
        e: bool = "e",
    }
}

element! {
    /// `<pivotField>`: how one cache field is used (by position: field `i` is cache field `i`).
    pub struct PivotField {
        name: String = "name",
        /// `axisRow`, `axisCol`, `axisPage`, `axisValues`; `None` when the field is on no axis.
        axis: String = "axis",
        data_field: bool = "dataField",
        num_fmt_id: u32 = "numFmtId",
        compact: bool = "compact",
        outline: bool = "outline",
        subtotal_top: bool = "subtotalTop",
        show_all: bool = "showAll",
        /// `manual` (the default), `ascending`, `descending`.
        sort_type: String = "sortType",
        default_subtotal: bool = "defaultSubtotal",
        multiple_item_selection_allowed: bool = "multipleItemSelectionAllowed",
        drag_to_row: bool = "dragToRow",
        drag_to_col: bool = "dragToCol",
        drag_to_page: bool = "dragToPage",
        drag_to_data: bool = "dragToData",
        insert_blank_row: bool = "insertBlankRow",
        insert_page_break: bool = "insertPageBreak",
    }
    + {
        /// `None` when the field has no `<items>`.
        items: Option<Vec<PivotItem>>,
        /// `autoSortScope` and `extLst`, as written.
        other: Vec<El>,
    }
}

element! {
    /// `<pageField>`: a report filter.
    pub struct PageField {
        fld: i32 = "fld",
        /// The one item shown; `None` shows all (or those not hidden in the field's items).
        item: u32 = "item",
        hier: i32 = "hier",
        name: String = "name",
        cap: String = "cap",
    }
    + {
        ext: Option<El>,
    }
}

element! {
    /// `<dataField>`: a value field.
    pub struct DataField {
        name: String = "name",
        fld: u32 = "fld",
        /// `sum` (the default), `count`, `average`, `max`, `min`, `product`, `countNums`,
        /// `stdDev`, `stdDevp`, `var`, `varp`.
        subtotal: String = "subtotal",
        /// `normal`, `difference`, `percent`, `percentDiff`, `runTotal`, `percentOfRow`,
        /// `percentOfCol`, `percentOfTotal`, `index`.
        show_data_as: String = "showDataAs",
        base_field: i32 = "baseField",
        base_item: u32 = "baseItem",
        num_fmt_id: u32 = "numFmtId",
    }
    + {
        /// `extLst`, as written: `x14:dataField` holds the Excel 2010 show-values-as kinds
        /// (`pivotShowAs="rankDescending"`).
        ext: Option<El>,
    }
}

element! {
    /// `<pivotTableStyleInfo>`.
    pub struct PivotTableStyleInfo {
        name: String = "name",
        show_row_headers: bool = "showRowHeaders",
        show_col_headers: bool = "showColHeaders",
        show_row_stripes: bool = "showRowStripes",
        show_col_stripes: bool = "showColStripes",
        show_last_column: bool = "showLastColumn",
    }
}

/// The children of a PivotTable definition in schema order (`CT_pivotTableDefinition`).
const TABLE: &[&str] = &[
    "location",
    "pivotFields",
    "rowFields",
    "rowItems",
    "colFields",
    "colItems",
    "pageFields",
    "dataFields",
    "formats",
    "conditionalFormats",
    "chartFormats",
    "pivotHierarchies",
    "pivotTableStyleInfo",
    "filters",
    "rowHierarchiesUsage",
    "colHierarchiesUsage",
    "extLst",
];

element! {
    /// A PivotTable (`<pivotTableDefinition>`, 18.10.1.73) on the cache `cache_id` names.
    pub struct PivotTableDefinition {
        name: String = "name",
        cache_id: u32 = "cacheId",
        /// Σ Values is on the rows axis.
        data_on_rows: bool = "dataOnRows",
        data_position: u32 = "dataPosition",
        data_caption: String = "dataCaption",
        grand_total_caption: String = "grandTotalCaption",
        error_caption: String = "errorCaption",
        show_error: bool = "showError",
        missing_caption: String = "missingCaption",
        show_missing: bool = "showMissing",
        apply_number_formats: bool = "applyNumberFormats",
        apply_border_formats: bool = "applyBorderFormats",
        apply_font_formats: bool = "applyFontFormats",
        apply_pattern_formats: bool = "applyPatternFormats",
        apply_alignment_formats: bool = "applyAlignmentFormats",
        apply_width_height_formats: bool = "applyWidthHeightFormats",
        updated_version: u32 = "updatedVersion",
        min_refreshable_version: u32 = "minRefreshableVersion",
        enable_drill: bool = "enableDrill",
        use_auto_formatting: bool = "useAutoFormatting",
        /// The grand total row below the report.
        row_grand_totals: bool = "rowGrandTotals",
        /// The grand total column at its right.
        col_grand_totals: bool = "colGrandTotals",
        item_print_titles: bool = "itemPrintTitles",
        created_version: u32 = "createdVersion",
        indent: u32 = "indent",
        show_headers: bool = "showHeaders",
        compact: bool = "compact",
        outline: bool = "outline",
        outline_data: bool = "outlineData",
        compact_data: bool = "compactData",
        grid_drop_zones: bool = "gridDropZones",
        multiple_field_filters: bool = "multipleFieldFilters",
        row_header_caption: String = "rowHeaderCaption",
        col_header_caption: String = "colHeaderCaption",
    }
    + {
        location: Location,
        fields: Vec<PivotField>,
        /// `rowFields`: cache field indices, `-2` for Σ Values.
        row_fields: Vec<i32>,
        col_fields: Vec<i32>,
        page_fields: Vec<PageField>,
        data_fields: Vec<DataField>,
        style: Option<PivotTableStyleInfo>,
        /// `rowItems`, `colItems`, `formats`, `conditionalFormats`, `chartFormats`,
        /// `pivotHierarchies`, `filters`, the hierarchy usages and `extLst`, as written.
        other: Vec<El>,
    }
}

impl PivotTableDefinition {
    /// A part read with [`crate::ns::CANONICAL`]. `None` when it is not a PivotTable definition
    /// or has no readable location.
    pub fn read(x: &El) -> Option<PivotTableDefinition> {
        if x.name != "pivotTableDefinition" {
            return None;
        }
        let loc = x.child("location")?;
        let location = Location { range: Range::parse(&loc.attr("ref")?.replace('$', ""))?, ..Location::read_attrs(loc) };
        let list = |name: &str| -> Vec<i32> {
            x.child(name).map(|e| e.children("field").filter_map(|f| f.attr("x").and_then(craft_xml::value::to_i32)).collect()).unwrap_or_default()
        };
        let fields = x
            .child("pivotFields")
            .map(|p| {
                p.children("pivotField")
                    .map(|f| PivotField {
                        items: f.child("items").map(|i| i.children("item").map(PivotItem::read_attrs).collect()),
                        other: kids(f, &["items"]),
                        ..PivotField::read_attrs(f)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(PivotTableDefinition {
            location,
            fields,
            row_fields: list("rowFields"),
            col_fields: list("colFields"),
            page_fields: x
                .child("pageFields")
                .map(|p| p.children("pageField").map(|f| PageField { ext: f.child("extLst").cloned(), ..PageField::read_attrs(f) }).collect())
                .unwrap_or_default(),
            data_fields: x
                .child("dataFields")
                .map(|p| p.children("dataField").map(|f| DataField { ext: f.child("extLst").cloned(), ..DataField::read_attrs(f) }).collect())
                .unwrap_or_default(),
            style: x.child("pivotTableStyleInfo").map(PivotTableStyleInfo::read_attrs),
            other: kids(x, &["location", "pivotFields", "rowFields", "colFields", "pageFields", "dataFields", "pivotTableStyleInfo"]),
            ..PivotTableDefinition::read_attrs(x)
        })
    }

    /// The part.
    pub fn to_xml(&self) -> Vec<u8> {
        let mut e = El::new("pivotTableDefinition");
        self.write_attrs(&mut e);
        let mut kids: Vec<El> = Vec::new();
        let mut loc = El::new("location").with_attr("ref", self.location.range.a1());
        self.location.write_attrs(&mut loc);
        kids.push(loc);
        if !self.fields.is_empty() {
            let mut pf = El::new("pivotFields").with_attr("count", self.fields.len());
            for f in &self.fields {
                let mut fe = El::new("pivotField");
                f.write_attrs(&mut fe);
                if let Some(items) = &f.items {
                    let mut ie = El::new("items").with_attr("count", items.len());
                    for i in items {
                        let mut it = El::new("item");
                        i.write_attrs(&mut it);
                        ie.push(it);
                    }
                    fe.push(ie);
                }
                fe.children.extend(f.other.iter().cloned().map(Node::El));
                pf.push(fe);
            }
            kids.push(pf);
        }
        for (name, list) in [("rowFields", &self.row_fields), ("colFields", &self.col_fields)] {
            if !list.is_empty() {
                let mut l = El::new(name).with_attr("count", list.len());
                list.iter().for_each(|x| l.push(El::new("field").with_attr("x", x)));
                kids.push(l);
            }
        }
        if !self.page_fields.is_empty() {
            let mut l = El::new("pageFields").with_attr("count", self.page_fields.len());
            for p in &self.page_fields {
                let mut pe = El::new("pageField");
                p.write_attrs(&mut pe);
                pe.children.extend(p.ext.iter().cloned().map(Node::El));
                l.push(pe);
            }
            kids.push(l);
        }
        if !self.data_fields.is_empty() {
            let mut l = El::new("dataFields").with_attr("count", self.data_fields.len());
            for d in &self.data_fields {
                let mut de = El::new("dataField");
                d.write_attrs(&mut de);
                de.children.extend(d.ext.iter().cloned().map(Node::El));
                l.push(de);
            }
            kids.push(l);
        }
        if let Some(s) = &self.style {
            let mut se = El::new("pivotTableStyleInfo");
            s.write_attrs(&mut se);
            kids.push(se);
        }
        kids.extend(self.other.iter().cloned());
        let pos = |n: &str| TABLE.iter().position(|o| *o == n).unwrap_or(TABLE.len());
        kids.sort_by_key(|k| pos(&k.name));
        e.children.extend(kids.into_iter().map(Node::El));
        document(e)
    }
}

fn document(mut root: El) -> Vec<u8> {
    crate::ns::declare(&mut root);
    let mut w = craft_xml::Writer::document();
    w.tree(&root);
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(v: &[CacheValue]) -> SharedItems {
        SharedItems::describe(v)
    }

    #[test]
    fn shared_item_flags_follow_excel() {
        use CacheValue::*;
        let s = values(&[Number(1.0), Number(2.5)]);
        assert_eq!(
            (s.contains_semi_mixed_types, s.contains_string, s.contains_number, s.contains_integer, s.min_value, s.max_value),
            (Some(false), Some(false), Some(true), None, Some(1.0), Some(2.5))
        );
        let s = values(&[Text("a".into()), Number(3.0), Missing]);
        assert_eq!(
            (s.contains_semi_mixed_types, s.contains_string, s.contains_blank, s.contains_mixed_types, s.contains_integer),
            (None, None, Some(true), Some(true), Some(true))
        );
        // Booleans and errors count as strings.
        let s = values(&[Bool(true), Error("#N/A".into())]);
        assert_eq!((s.contains_string, s.contains_number, s.contains_mixed_types), (None, None, None));
        let s = values(&[Text("x".repeat(256))]);
        assert_eq!(s.long_text, Some(true));
        let s = values(&[Date("2024-03-05T00:00:00".into()), Missing, Date("2023-01-01T00:00:00".into())]);
        assert_eq!(
            (s.contains_semi_mixed_types, s.contains_non_date, s.contains_date, s.contains_string, s.contains_blank),
            (Some(false), Some(false), Some(true), Some(false), Some(true))
        );
        assert_eq!((s.min_date.as_deref(), s.max_date.as_deref()), (Some("2023-01-01T00:00:00"), Some("2024-03-05T00:00:00")));
    }

    #[test]
    fn olap_items_do_not_read() {
        let x = craft_xml::parse(
            br#"<pivotCacheDefinition><cacheSource type="external"/><cacheFields count="1"><cacheField name="a"><sharedItems><s v="x"><x v="0"/></s></sharedItems></cacheField></cacheFields></pivotCacheDefinition>"#,
        )
        .unwrap();
        assert!(PivotCacheDefinition::read(&x).is_none());
        let bad = craft_xml::parse(br#"<pivotCacheDefinition><cacheFields><cacheField name="a"><sharedItems><n v="NaN-ish"/></sharedItems></cacheField></cacheFields></pivotCacheDefinition>"#).unwrap();
        assert!(PivotCacheDefinition::read(&bad).is_none());
    }

    #[test]
    fn a_table_without_location_does_not_read() {
        let x = craft_xml::parse(br#"<pivotTableDefinition name="P" cacheId="1"><pivotFields count="0"/></pivotTableDefinition>"#).unwrap();
        assert!(PivotTableDefinition::read(&x).is_none());
        let x = craft_xml::parse(br#"<pivotTableDefinition name="P" cacheId="1"><location ref="nonsense"/></pivotTableDefinition>"#).unwrap();
        assert!(PivotTableDefinition::read(&x).is_none());
    }
}
