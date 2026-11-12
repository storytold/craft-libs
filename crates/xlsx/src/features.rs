//! The worksheet's typed features, each mirroring its SpreadsheetML element: sheet views, print
//! setup, data validation, conditional formatting, hyperlinks (ECMA-376 Part 1, 18.3), and the
//! comments part (18.7) with the legacy note shapes Excel draws comments with.
//!
//! An attribute the file may leave out is an `Option` (`None`: not written, the schema's
//! default applies), so a feature reads and writes back as it was. Attributes a type does not
//! name are kept in its `attrs` (`xr:uid` and the like).

use craft_sheet::{CellRef, MAX_COLS, MAX_ROWS, Range, col_letters};
use craft_xml::{El, Node};

use crate::strings::StringItem;
use crate::styles::Color;
use crate::write::num;

/// An attribute value type.
pub(crate) trait Attr: Sized {
    fn parse(s: &str) -> Option<Self>;
    fn spell(&self) -> String;
}

impl Attr for bool {
    fn parse(s: &str) -> Option<bool> {
        craft_xml::value::to_bool(s)
    }
    fn spell(&self) -> String {
        if *self { "1" } else { "0" }.into()
    }
}
impl Attr for u32 {
    fn parse(s: &str) -> Option<u32> {
        craft_xml::value::to_u32(s)
    }
    fn spell(&self) -> String {
        self.to_string()
    }
}
impl Attr for i32 {
    fn parse(s: &str) -> Option<i32> {
        craft_xml::value::to_i32(s)
    }
    fn spell(&self) -> String {
        self.to_string()
    }
}
impl Attr for f64 {
    fn parse(s: &str) -> Option<f64> {
        craft_xml::value::to_f64(s)
    }
    fn spell(&self) -> String {
        num(*self)
    }
}
impl Attr for String {
    fn parse(s: &str) -> Option<String> {
        Some(s.to_string())
    }
    fn spell(&self) -> String {
        self.clone()
    }
}

/// The attributes of `e` not in `named`. A container's `count` (written from the items) and
/// the `ref` and `sqref` each type reads into a field of its own are left out.
pub(crate) fn rest(e: &El, named: &[&str]) -> Vec<(String, String)> {
    e.attrs.iter().filter(|(k, _)| !matches!(k.as_str(), "count" | "ref" | "sqref") && !named.contains(&k.as_str())).cloned().collect()
}

/// A struct whose `Option` fields are the element's attributes (`field: type = "xmlName"`), with
/// further fields after `+` that the element's own reader fills, and `attrs` for the rest.
macro_rules! element {
    (
        $(#[$m:meta])* pub struct $name:ident {
            $($(#[$fm:meta])* $f:ident: $t:ty = $x:literal,)*
        }
        $(+ { $($(#[$em:meta])* $e:ident: $et:ty,)* })?
    ) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name {
            $($(#[$fm])* pub $f: Option<$t>,)*
            $($($(#[$em])* pub $e: $et,)*)?
            /// The other attributes as written.
            pub attrs: Vec<(String, String)>,
        }

        impl $name {
            const NAMED: &'static [&'static str] = &[$($x),*];

            #[allow(clippy::needless_update)]
            fn read_attrs(el: &El) -> $name {
                $name { $($f: el.attr($x).and_then(<$t as Attr>::parse),)* attrs: rest(el, Self::NAMED), ..$name::default() }
            }

            fn write_attrs(&self, el: &mut El) {
                $(if let Some(v) = &self.$f { el.set_attr($x, v.spell()); })*
                el.attrs.extend(self.attrs.iter().cloned());
            }
        }
    };
}

/// `sqref`: ranges separated by spaces. Unreadable ranges are dropped.
pub fn parse_sqref(s: &str) -> Vec<Range> {
    s.split_whitespace().filter_map(Range::parse).collect()
}

/// A range as a reference writes it: whole columns as `A:C`, whole rows as `1:3`.
pub fn range_ref(r: &Range) -> String {
    let full_cols = r.first.row == 0 && r.last.row == MAX_ROWS - 1;
    let full_rows = r.first.col == 0 && r.last.col == MAX_COLS - 1;
    match (full_cols, full_rows) {
        (true, false) => format!("{}:{}", col_letters(r.first.col), col_letters(r.last.col)),
        (false, true) => format!("{}:{}", r.first.row + 1, r.last.row + 1),
        _ => r.a1(),
    }
}

fn sqref(ranges: &[Range]) -> String {
    ranges.iter().map(range_ref).collect::<Vec<_>>().join(" ")
}

pub(crate) fn kids(e: &El, except: &[&str]) -> Vec<El> {
    e.elements().filter(|c| !except.contains(&c.name.as_str())).cloned().collect()
}

// ------------------------------------------------------------------------------- sheet views

element! {
    /// `<pane>`: the split or frozen panes of a view.
    pub struct Pane {
        x_split: f64 = "xSplit",
        y_split: f64 = "ySplit",
        top_left_cell: String = "topLeftCell",
        active_pane: String = "activePane",
        state: String = "state",
    }
}

element! {
    /// `<selection>`.
    pub struct Selection {
        pane: String = "pane",
        active_cell: String = "activeCell",
        active_cell_id: u32 = "activeCellId",
    }
    + {
        sqref: Vec<Range>,
    }
}

element! {
    /// `<sheetView>` (18.3.1.87).
    pub struct SheetView {
        window_protection: bool = "windowProtection",
        show_formulas: bool = "showFormulas",
        show_grid_lines: bool = "showGridLines",
        show_row_col_headers: bool = "showRowColHeaders",
        show_zeros: bool = "showZeros",
        right_to_left: bool = "rightToLeft",
        tab_selected: bool = "tabSelected",
        show_ruler: bool = "showRuler",
        show_outline_symbols: bool = "showOutlineSymbols",
        default_grid_color: bool = "defaultGridColor",
        show_white_space: bool = "showWhiteSpace",
        /// `normal`, `pageBreakPreview`, `pageLayout`.
        view: String = "view",
        top_left_cell: String = "topLeftCell",
        color_id: u32 = "colorId",
        zoom_scale: u32 = "zoomScale",
        zoom_scale_normal: u32 = "zoomScaleNormal",
        zoom_scale_sheet_layout_view: u32 = "zoomScaleSheetLayoutView",
        zoom_scale_page_layout_view: u32 = "zoomScalePageLayoutView",
        workbook_view_id: u32 = "workbookViewId",
    }
    + {
        pane: Option<Pane>,
        selections: Vec<Selection>,
        /// `pivotSelection` and `extLst`, as written.
        other: Vec<El>,
    }
}

impl SheetView {
    pub fn read(e: &El) -> SheetView {
        let selection = |s: &El| Selection { sqref: s.attr("sqref").map(parse_sqref).unwrap_or_default(), ..Selection::read_attrs(s) };
        SheetView {
            pane: e.child("pane").map(Pane::read_attrs),
            selections: e.children("selection").map(selection).collect(),
            other: kids(e, &["pane", "selection"]),
            ..SheetView::read_attrs(e)
        }
    }

    pub fn to_el(&self) -> El {
        let mut e = El::new("sheetView");
        self.write_attrs(&mut e);
        if let Some(p) = &self.pane {
            let mut pe = El::new("pane");
            p.write_attrs(&mut pe);
            e.push(pe);
        }
        for s in &self.selections {
            let mut se = El::new("selection");
            s.write_attrs(&mut se);
            if !s.sqref.is_empty() {
                se.set_attr("sqref", sqref(&s.sqref));
            }
            e.push(se);
        }
        e.children.extend(self.other.iter().cloned().map(Node::El));
        e
    }
}

// ------------------------------------------------------------------------------- printing

element! {
    /// `<printOptions>`.
    pub struct PrintOptions {
        horizontal_centered: bool = "horizontalCentered",
        vertical_centered: bool = "verticalCentered",
        headings: bool = "headings",
        grid_lines: bool = "gridLines",
        grid_lines_set: bool = "gridLinesSet",
    }
}

element! {
    /// `<pageMargins>`, inches.
    pub struct PageMargins {
        left: f64 = "left",
        right: f64 = "right",
        top: f64 = "top",
        bottom: f64 = "bottom",
        header: f64 = "header",
        footer: f64 = "footer",
    }
}

element! {
    /// `<pageSetup>`.
    pub struct PageSetup {
        paper_size: u32 = "paperSize",
        /// With its unit (`297mm`).
        paper_height: String = "paperHeight",
        paper_width: String = "paperWidth",
        scale: u32 = "scale",
        first_page_number: u32 = "firstPageNumber",
        fit_to_width: u32 = "fitToWidth",
        fit_to_height: u32 = "fitToHeight",
        /// `downThenOver`, `overThenDown`.
        page_order: String = "pageOrder",
        /// `default`, `portrait`, `landscape`.
        orientation: String = "orientation",
        use_printer_defaults: bool = "usePrinterDefaults",
        black_and_white: bool = "blackAndWhite",
        draft: bool = "draft",
        /// `none`, `asDisplayed`, `atEnd`.
        cell_comments: String = "cellComments",
        use_first_page_number: bool = "useFirstPageNumber",
        /// `displayed`, `blank`, `dash`, `NA`.
        errors: String = "errors",
        horizontal_dpi: u32 = "horizontalDpi",
        vertical_dpi: u32 = "verticalDpi",
        copies: u32 = "copies",
        /// The printer settings part.
        rel_id: String = "r:id",
    }
}

element! {
    /// `<headerFooter>`: the header and footer texts with their `&` codes as written.
    pub struct HeaderFooter {
        different_odd_even: bool = "differentOddEven",
        different_first: bool = "differentFirst",
        scale_with_doc: bool = "scaleWithDoc",
        align_with_margins: bool = "alignWithMargins",
    }
    + {
        odd_header: Option<String>,
        odd_footer: Option<String>,
        even_header: Option<String>,
        even_footer: Option<String>,
        first_header: Option<String>,
        first_footer: Option<String>,
    }
}

const HEADER_FOOTER: [&str; 6] = ["oddHeader", "oddFooter", "evenHeader", "evenFooter", "firstHeader", "firstFooter"];

impl HeaderFooter {
    fn texts(&self) -> [&Option<String>; 6] {
        [&self.odd_header, &self.odd_footer, &self.even_header, &self.even_footer, &self.first_header, &self.first_footer]
    }

    pub fn read(e: &El) -> HeaderFooter {
        let t = |n: &str| e.child(n).map(|c| c.text().into_owned());
        HeaderFooter {
            odd_header: t("oddHeader"),
            odd_footer: t("oddFooter"),
            even_header: t("evenHeader"),
            even_footer: t("evenFooter"),
            first_header: t("firstHeader"),
            first_footer: t("firstFooter"),
            ..HeaderFooter::read_attrs(e)
        }
    }

    pub fn to_el(&self) -> El {
        let mut e = El::new("headerFooter");
        self.write_attrs(&mut e);
        for (name, text) in HEADER_FOOTER.iter().zip(self.texts()) {
            if let Some(t) = text {
                let mut c = El::new(*name);
                if t.starts_with(' ') || t.ends_with(' ') {
                    c.set_attr("xml:space", "preserve");
                }
                e.push(if t.is_empty() { c } else { c.with_text(t.as_str()) });
            }
        }
        e
    }
}

element! {
    /// `<brk>`: a page break before row or column `id` (zero-based is `id - 1`: the break
    /// follows that many rows or columns).
    pub struct Break {
        id: u32 = "id",
        min: u32 = "min",
        max: u32 = "max",
        /// Inserted by hand.
        man: bool = "man",
        /// Inserted for a pivot table.
        pt: bool = "pt",
    }
}

fn breaks_el(name: &str, breaks: &[Break]) -> Option<El> {
    if breaks.is_empty() {
        return None;
    }
    let manual = breaks.iter().filter(|b| b.man == Some(true)).count();
    let mut e = El::new(name).with_attr("count", breaks.len()).with_attr("manualBreakCount", manual);
    for b in breaks {
        let mut be = El::new("brk");
        b.write_attrs(&mut be);
        e.push(be);
    }
    Some(e)
}

/// How a worksheet prints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Print {
    pub options: Option<PrintOptions>,
    pub margins: Option<PageMargins>,
    pub setup: Option<PageSetup>,
    pub header_footer: Option<HeaderFooter>,
    pub row_breaks: Vec<Break>,
    pub col_breaks: Vec<Break>,
}

// ------------------------------------------------------------------------------- data validation

element! {
    /// `<dataValidation>` (18.3.1.32). The formulas are as the file spells them, without `=`.
    pub struct DataValidation {
        /// `none`, `whole`, `decimal`, `list`, `date`, `time`, `textLength`, `custom`.
        kind: String = "type",
        /// `stop`, `warning`, `information`.
        error_style: String = "errorStyle",
        ime_mode: String = "imeMode",
        operator: String = "operator",
        allow_blank: bool = "allowBlank",
        /// Set: the in-cell list arrow is *hidden*.
        show_drop_down: bool = "showDropDown",
        show_input_message: bool = "showInputMessage",
        show_error_message: bool = "showErrorMessage",
        error_title: String = "errorTitle",
        error: String = "error",
        prompt_title: String = "promptTitle",
        prompt: String = "prompt",
    }
    + {
        sqref: Vec<Range>,
        formula1: Option<String>,
        formula2: Option<String>,
    }
}

impl DataValidation {
    /// A `dataValidation` of the main namespace, or of an `x14:dataValidations` extension (its
    /// formulas in `xm:f` and its ranges in `xm:sqref`).
    pub fn read(v: &El) -> DataValidation {
        let formula = |n: &str| {
            let f = v.child(n).or_else(|| v.child(format!("x14:{n}").as_str()))?;
            Some(f.child("xm:f").map_or_else(|| f.text(), |x| x.text()).trim().to_string())
        };
        let sq = v.attr("sqref").map(str::to_string).or_else(|| v.child("xm:sqref").map(|s| s.text().into_owned()));
        DataValidation {
            sqref: sq.as_deref().map(parse_sqref).unwrap_or_default(),
            formula1: formula("formula1"),
            formula2: formula("formula2"),
            ..DataValidation::read_attrs(v)
        }
    }

    /// The main-namespace element.
    pub fn to_el(&self) -> El {
        let mut e = El::new("dataValidation");
        self.write_attrs(&mut e);
        e.set_attr("sqref", sqref(&self.sqref));
        for (n, f) in [("formula1", &self.formula1), ("formula2", &self.formula2)] {
            if let Some(f) = f {
                e.push(El::new(n).with_text(f.as_str()));
            }
        }
        e
    }
}

element! {
    /// `<dataValidations>`.
    pub struct DataValidations {
        disable_prompts: bool = "disablePrompts",
        x_window: u32 = "xWindow",
        y_window: u32 = "yWindow",
    }
    + {
        items: Vec<DataValidation>,
    }
}

// ------------------------------------------------------------------------------- conditional formats

element! {
    /// `<cfvo>`: a threshold of a colour scale, data bar or icon set.
    pub struct Cfvo {
        /// `num`, `percent`, `max`, `min`, `formula`, `percentile`.
        kind: String = "type",
        val: String = "val",
        gte: bool = "gte",
    }
}

fn cfvos(e: &El) -> Vec<Cfvo> {
    e.children("cfvo").map(Cfvo::read_attrs).collect()
}

fn push_cfvos(e: &mut El, cfvos: &[Cfvo]) {
    for c in cfvos {
        let mut ce = El::new("cfvo");
        c.write_attrs(&mut ce);
        e.push(ce);
    }
}

/// `<colorScale>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorScale {
    pub cfvos: Vec<Cfvo>,
    pub colors: Vec<Color>,
}

element! {
    /// `<dataBar>`.
    pub struct DataBar {
        min_length: u32 = "minLength",
        max_length: u32 = "maxLength",
        show_value: bool = "showValue",
    }
    + {
        cfvos: Vec<Cfvo>,
        color: Option<Color>,
    }
}

element! {
    /// `<iconSet>`.
    pub struct IconSet {
        icon_set: String = "iconSet",
        show_value: bool = "showValue",
        percent: bool = "percent",
        reverse: bool = "reverse",
    }
    + {
        cfvos: Vec<Cfvo>,
    }
}

element! {
    /// `<cfRule>` (18.3.1.10).
    pub struct CfRule {
        /// `expression`, `cellIs`, `colorScale`, `dataBar`, `iconSet`, `top10`, `uniqueValues`,
        /// `duplicateValues`, `containsText`, ..., `aboveAverage`.
        kind: String = "type",
        /// An index into [`crate::Styles::dxfs`].
        dxf_id: u32 = "dxfId",
        priority: u32 = "priority",
        stop_if_true: bool = "stopIfTrue",
        above_average: bool = "aboveAverage",
        percent: bool = "percent",
        bottom: bool = "bottom",
        operator: String = "operator",
        text: String = "text",
        time_period: String = "timePeriod",
        rank: u32 = "rank",
        std_dev: i32 = "stdDev",
        equal_average: bool = "equalAverage",
    }
    + {
        /// `<formula>` texts, as written.
        formulas: Vec<String>,
        color_scale: Option<ColorScale>,
        data_bar: Option<DataBar>,
        icon_set: Option<IconSet>,
        /// The rule's `extLst` (the id that links a data bar to its Excel 2010 extension).
        ext: Option<El>,
    }
}

impl CfRule {
    pub fn read(r: &El) -> CfRule {
        CfRule {
            formulas: r.children("formula").map(|f| f.text().into_owned()).collect(),
            color_scale: r.child("colorScale").map(|c| ColorScale { cfvos: cfvos(c), colors: c.children("color").map(Color::read).collect() }),
            data_bar: r.child("dataBar").map(|d| DataBar { cfvos: cfvos(d), color: d.child("color").map(Color::read), ..DataBar::read_attrs(d) }),
            icon_set: r.child("iconSet").map(|i| IconSet { cfvos: cfvos(i), ..IconSet::read_attrs(i) }),
            ext: r.child("extLst").cloned(),
            ..CfRule::read_attrs(r)
        }
    }

    pub fn to_el(&self) -> El {
        let mut e = El::new("cfRule");
        self.write_attrs(&mut e);
        for f in &self.formulas {
            e.push(El::new("formula").with_text(f.as_str()));
        }
        if let Some(c) = &self.color_scale {
            let mut ce = El::new("colorScale");
            push_cfvos(&mut ce, &c.cfvos);
            c.colors.iter().for_each(|col| ce.push(col.to_el("color")));
            e.push(ce);
        }
        if let Some(d) = &self.data_bar {
            let mut de = El::new("dataBar");
            d.write_attrs(&mut de);
            push_cfvos(&mut de, &d.cfvos);
            if let Some(c) = &d.color {
                de.push(c.to_el("color"));
            }
            e.push(de);
        }
        if let Some(i) = &self.icon_set {
            let mut ie = El::new("iconSet");
            i.write_attrs(&mut ie);
            push_cfvos(&mut ie, &i.cfvos);
            e.push(ie);
        }
        if let Some(x) = &self.ext {
            e.push(x.clone());
        }
        e
    }
}

element! {
    /// `<conditionalFormatting>`: rules over ranges.
    pub struct ConditionalFormatting {
        pivot: bool = "pivot",
    }
    + {
        sqref: Vec<Range>,
        rules: Vec<CfRule>,
        ext: Option<El>,
    }
}

impl ConditionalFormatting {
    pub fn read(e: &El) -> ConditionalFormatting {
        ConditionalFormatting {
            sqref: e.attr("sqref").map(parse_sqref).unwrap_or_default(),
            rules: e.children("cfRule").map(CfRule::read).collect(),
            ext: e.child("extLst").cloned(),
            ..ConditionalFormatting::read_attrs(e)
        }
    }

    pub fn to_el(&self) -> El {
        let mut e = El::new("conditionalFormatting");
        self.write_attrs(&mut e);
        e.set_attr("sqref", sqref(&self.sqref));
        self.rules.iter().for_each(|r| e.push(r.to_el()));
        if let Some(x) = &self.ext {
            e.push(x.clone());
        }
        e
    }
}

// ------------------------------------------------------------------------------- hyperlinks

element! {
    /// `<hyperlink>`: to a URL through a relationship, or to a place in the workbook.
    pub struct Hyperlink {
        rel_id: String = "r:id",
        /// A place in the workbook (`Sheet2!A1`, a defined name).
        location: String = "location",
        tooltip: String = "tooltip",
        display: String = "display",
    }
    + {
        range: Range,
        /// The relationship's target when it points outside the package, read from the sheet's
        /// relationships. On writing, a link whose `rel_id` does not lead to this URL gets a
        /// relationship of its own.
        url: Option<String>,
    }
}

impl Hyperlink {
    fn read(e: &El, rels: &craft_opc::Rels) -> Option<Hyperlink> {
        let range = Range::parse(e.attr("ref")?)?;
        let h = Hyperlink::read_attrs(e);
        let url = h.rel_id.as_deref().and_then(|id| rels.by_id(id)).filter(|r| r.external).map(|r| r.target.clone());
        Some(Hyperlink { range, url, ..h })
    }

    pub fn to_el(&self) -> El {
        let mut e = El::new("hyperlink").with_attr("ref", range_ref(&self.range));
        self.write_attrs(&mut e);
        e
    }
}

// ------------------------------------------------------------------------------- the lift

/// The typed features of a worksheet, lifted from its tree. Each one listed here is written from
/// these fields; its element in [`crate::SheetPart::root`] is not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Features {
    pub views: Vec<SheetView>,
    pub print: Print,
    pub validations: Option<DataValidations>,
    pub conditional_formats: Vec<ConditionalFormatting>,
    pub hyperlinks: Vec<Hyperlink>,
    /// `legacyDrawing/@r:id`: the VML part the sheet's notes and form controls are drawn in.
    pub legacy_drawing: Option<String>,
    /// `drawing/@r:id`: the drawing part (shapes, pictures, charts).
    pub drawing: Option<String>,
    /// `tableParts/tablePart/@r:id`: the sheet's table parts, in order.
    pub table_parts: Vec<String>,
}

impl Features {
    pub(crate) fn read(root: &El, rels: &craft_opc::Rels) -> Features {
        let attrs_of = |n: &str| root.child(n);
        Features {
            views: root.child("sheetViews").map(|v| v.children("sheetView").map(SheetView::read).collect()).unwrap_or_default(),
            print: Print {
                options: attrs_of("printOptions").map(PrintOptions::read_attrs),
                margins: attrs_of("pageMargins").map(PageMargins::read_attrs),
                setup: attrs_of("pageSetup").map(PageSetup::read_attrs),
                header_footer: attrs_of("headerFooter").map(HeaderFooter::read),
                row_breaks: root.child("rowBreaks").map(|b| b.children("brk").map(Break::read_attrs).collect()).unwrap_or_default(),
                col_breaks: root.child("colBreaks").map(|b| b.children("brk").map(Break::read_attrs).collect()).unwrap_or_default(),
            },
            validations: root.child("dataValidations").map(|d| DataValidations {
                items: d.children("dataValidation").map(DataValidation::read).collect(),
                ..DataValidations::read_attrs(d)
            }),
            conditional_formats: root.children("conditionalFormatting").map(ConditionalFormatting::read).collect(),
            hyperlinks: root
                .child("hyperlinks")
                .map(|h| h.children("hyperlink").filter_map(|e| Hyperlink::read(e, rels)).collect())
                .unwrap_or_default(),
            legacy_drawing: root.child("legacyDrawing").and_then(|l| l.attr_or_plain("r:id")).map(str::to_string),
            drawing: root.child("drawing").and_then(|l| l.attr_or_plain("r:id")).map(str::to_string),
            table_parts: root
                .child("tableParts")
                .map(|t| t.children("tablePart").filter_map(|p| p.attr_or_plain("r:id")).map(str::to_string).collect())
                .unwrap_or_default(),
        }
    }

    /// The elements these fields own, for [`crate::write::assemble`]. Conditional formats are
    /// one element per block: the first block takes the place of the file's first, the others
    /// follow it.
    pub(crate) fn owned(&self) -> Vec<(&'static str, Vec<El>)> {
        let mut views = El::new("sheetViews");
        self.views.iter().for_each(|v| views.push(v.to_el()));
        let el = |name: &str, write: &dyn Fn(&mut El)| {
            let mut e = El::new(name);
            write(&mut e);
            e
        };
        let validations = self.validations.as_ref().filter(|v| !v.items.is_empty()).map(|v| {
            let mut e = el("dataValidations", &|e| v.write_attrs(e));
            e.attrs.insert(0, ("count".into(), v.items.len().to_string()));
            v.items.iter().for_each(|i| e.push(i.to_el()));
            e
        });
        let mut links = El::new("hyperlinks");
        self.hyperlinks.iter().for_each(|h| links.push(h.to_el()));
        let p = &self.print;
        vec![
            ("sheetViews", Vec::from_iter((!self.views.is_empty()).then_some(views))),
            ("conditionalFormatting", self.conditional_formats.iter().map(ConditionalFormatting::to_el).collect()),
            ("dataValidations", Vec::from_iter(validations)),
            ("hyperlinks", Vec::from_iter((!self.hyperlinks.is_empty()).then_some(links))),
            ("printOptions", Vec::from_iter(p.options.as_ref().map(|o| el("printOptions", &|e| o.write_attrs(e))))),
            ("pageMargins", Vec::from_iter(p.margins.as_ref().map(|m| el("pageMargins", &|e| m.write_attrs(e))))),
            ("pageSetup", Vec::from_iter(p.setup.as_ref().map(|s| el("pageSetup", &|e| s.write_attrs(e))))),
            ("headerFooter", Vec::from_iter(p.header_footer.as_ref().map(HeaderFooter::to_el))),
            ("rowBreaks", Vec::from_iter(breaks_el("rowBreaks", &p.row_breaks))),
            ("colBreaks", Vec::from_iter(breaks_el("colBreaks", &p.col_breaks))),
            ("legacyDrawing", Vec::from_iter(self.legacy_drawing.as_ref().map(|id| El::new("legacyDrawing").with_attr("r:id", id)))),
            ("drawing", Vec::from_iter(self.drawing.as_ref().map(|id| El::new("drawing").with_attr("r:id", id)))),
            (
                "tableParts",
                Vec::from_iter((!self.table_parts.is_empty()).then(|| {
                    let mut e = El::new("tableParts").with_attr("count", self.table_parts.len());
                    self.table_parts.iter().for_each(|id| e.push(El::new("tablePart").with_attr("r:id", id)));
                    e
                })),
            ),
        ]
    }
}

// ------------------------------------------------------------------------------- comments

element! {
    /// `<comment>`: a note on a cell.
    pub struct Comment {
        author_id: u32 = "authorId",
        guid: String = "guid",
        shape_id: u32 = "shapeId",
    }
    + {
        at: CellRef,
        text: StringItem,
        /// `commentPr`, as written.
        props: Option<El>,
    }
}

/// The comments part of a worksheet (`xl/commentsN.xml`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comments {
    pub authors: Vec<String>,
    pub comments: Vec<Comment>,
    /// The whole part, for what is not lifted out (extensions).
    pub root: El,
}

impl Comments {
    /// Comments with no readable `ref` are dropped.
    pub fn read(root: El) -> Comments {
        let authors = root.child("authors").map(|a| a.children("author").map(|a| a.text().into_owned()).collect()).unwrap_or_default();
        let comments = root
            .child("commentList")
            .map(|l| {
                l.children("comment")
                    .filter_map(|c| {
                        Some(Comment {
                            at: CellRef::parse(c.attr("ref")?)?,
                            text: c.child("text").map(StringItem::read).unwrap_or_default(),
                            props: c.child("commentPr").cloned(),
                            ..Comment::read_attrs(c)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Comments { authors, comments, root }
    }

    /// The author of `c`.
    pub fn author(&self, c: &Comment) -> Option<&str> {
        self.authors.get(c.author_id? as usize).map(String::as_str)
    }

    pub fn to_xml(&self) -> Vec<u8> {
        let root = if self.root.name.is_empty() { El::new("comments") } else { self.root.clone() };
        let mut authors = El::new("authors");
        self.authors.iter().for_each(|a| authors.push(El::new("author").with_text(a.as_str())));
        let mut list = El::new("commentList");
        for c in &self.comments {
            let mut e = El::new("comment").with_attr("ref", c.at.a1());
            c.write_attrs(&mut e);
            e.push(c.text.to_el("text"));
            if let Some(p) = &c.props {
                e.push(p.clone());
            }
            list.push(e);
        }
        let mut out =
            crate::write::assemble(&root, &["authors", "commentList", "extLst"], vec![("authors", Some(authors)), ("commentList", Some(list))]);
        crate::ns::declare(&mut out);
        let mut w = craft_xml::Writer::document();
        w.tree(&out);
        w.into_bytes()
    }
}

/// How a note's box is drawn, from its legacy VML shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteShape {
    pub at: CellRef,
    /// Shown all the time (`<x:Visible/>`), not only on hover.
    pub visible: bool,
    /// `x:Anchor`: left column, offset px, top row, offset px, right column, offset px, bottom
    /// row, offset px.
    pub anchor: Option<[u32; 8]>,
    /// The shape's width and height in points, from its style.
    pub size_pt: Option<(f64, f64)>,
}

impl NoteShape {
    /// The note shapes of a VML drawing read with [`crate::ns::CANONICAL`]. Shapes may sit in
    /// groups; other shapes (form controls) are skipped.
    pub fn read_all(vml: &El) -> Vec<NoteShape> {
        let mut out = Vec::new();
        for shape in vml.descendants().filter(|e| e.name == "v:shape") {
            let Some(cd) = shape.child("x:ClientData").filter(|cd| cd.attr("ObjectType") == Some("Note")) else { continue };
            let n = |k: &str| cd.child(k).and_then(|v| v.text().trim().parse::<u32>().ok());
            let (Some(row), Some(col)) = (n("x:Row"), n("x:Column")) else { continue };
            if row >= MAX_ROWS || col >= MAX_COLS {
                continue;
            }
            let anchor = cd.child("x:Anchor").and_then(|a| {
                let v: Vec<u32> = a.text().split(',').map(|p| p.trim().parse::<u32>().ok()).collect::<Option<_>>()?;
                <[u32; 8]>::try_from(v).ok()
            });
            let style = shape.attr("style").unwrap_or("");
            let pt = |key: &str| {
                style
                    .split(';')
                    .filter_map(|d| d.split_once(':'))
                    .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
                    .and_then(|(_, v)| v.trim().strip_suffix("pt")?.trim().parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0 && *n < 1e6))
            };
            let size_pt = pt("width").zip(pt("height"));
            out.push(NoteShape { at: CellRef::new(row, col), visible: cd.child("x:Visible").is_some(), anchor, size_pt });
        }
        out
    }
}

/// A legacy VML drawing that shows `comments` as notes, Excel's way, with shape ids from block
/// `block` (`o:idmap`: ids `1024 * block + 1` on). A note with a shape in `shapes` keeps its box
/// and visibility; the others get Excel's default box, two columns right of the cell.
pub fn notes_vml(comments: &[Comment], shapes: &[NoteShape], block: u32) -> Vec<u8> {
    let mut s = format!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"{block}\"/></o:shapelayout><v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>"
    );
    for (i, c) in comments.iter().enumerate() {
        let shape = shapes.iter().find(|s| s.at == c.at);
        let id = u64::from(block) * 1024 + 1 + i as u64;
        let visible = shape.is_some_and(|s| s.visible);
        let clamp = |v: i64, max: u32| v.clamp(0, i64::from(max) - 1);
        let (row, col) = (i64::from(c.at.row), i64::from(c.at.col));
        let anchor = shape.and_then(|s| s.anchor).unwrap_or_else(|| {
            let a = [clamp(col + 1, MAX_COLS), 15, clamp(row, MAX_ROWS), 2, clamp(col + 3, MAX_COLS), 15, clamp(row + 4, MAX_ROWS), 16];
            a.map(|v| u32::try_from(v).unwrap_or(0))
        });
        let (w, h) = shape.and_then(|s| s.size_pt).unwrap_or((108.0, 59.25));
        let anchor = anchor.map(|v| v.to_string()).join(", ");
        s.push_str(&format!(
            "<v:shape id=\"_x0000_s{id}\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:59.25pt;margin-top:1.5pt;width:{}pt;height:{}pt;z-index:{};visibility:{}\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\"><v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{anchor}</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{}</x:Row><x:Column>{}</x:Column>{}</x:ClientData></v:shape>",
            num(w),
            num(h),
            i + 1,
            if visible { "visible" } else { "hidden" },
            c.at.row,
            c.at.col,
            if visible { "<x:Visible/>" } else { "" }
        ));
    }
    s.push_str("</xml>");
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(xml: &str) -> El {
        craft_xml::Options::canonical(&crate::ns::CANONICAL).parse(xml.as_bytes()).unwrap_or_default()
    }

    #[test]
    fn rules_with_scales_bars_and_icons_write_back() {
        let x = r#"<conditionalFormatting xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" sqref="A1:A9 C1"><cfRule type="colorScale" priority="3"><colorScale><cfvo type="min"/><cfvo type="percentile" val="50"/><cfvo type="max"/><color rgb="FFF8696B"/><color theme="4" tint="0.5"/><color rgb="FF63BE7B"/></colorScale></cfRule><cfRule type="dataBar" priority="2"><dataBar showValue="0"><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{B025F937-C7B1-47D3-B67F-A62EFF666E3E}"><x14:id>{1}</x14:id></ext></extLst></cfRule><cfRule type="iconSet" priority="1"><iconSet iconSet="3Arrows" reverse="1"><cfvo type="percent" val="0"/><cfvo type="percent" val="33" gte="0"/></iconSet></cfRule><cfRule type="top10" dxfId="1" priority="4" rank="5" percent="1" bottom="1" stopIfTrue="1"/></conditionalFormatting>"#;
        let cf = ConditionalFormatting::read(&read(x));
        assert_eq!(cf.sqref.len(), 2);
        let [scale, bar, icons, top] = &cf.rules[..] else { panic!("{:?}", cf.rules) };
        let scale = scale.color_scale.as_ref().unwrap();
        assert_eq!((scale.cfvos[1].kind.as_deref(), scale.cfvos[1].val.as_deref(), scale.colors[1].tint), (Some("percentile"), Some("50"), 0.5));
        assert_eq!(bar.data_bar.as_ref().map(|d| d.show_value), Some(Some(false)));
        assert!(bar.ext.as_ref().is_some_and(|e| e.find("x14:id").is_some()));
        assert_eq!(
            icons.icon_set.as_ref().map(|i| (i.icon_set.as_deref(), i.reverse, i.cfvos[1].gte)),
            Some((Some("3Arrows"), Some(true), Some(false)))
        );
        assert_eq!((top.rank, top.percent, top.bottom, top.stop_if_true, top.dxf_id), (Some(5), Some(true), Some(true), Some(true), Some(1)));
        let mut written = cf.to_el();
        crate::ns::declare(&mut written);
        assert_eq!(ConditionalFormatting::read(&read(&written.to_xml())), cf);
    }

    #[test]
    fn extension_validations_read_into_the_same_type() {
        let x = r#"<x14:dataValidation xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" type="list" allowBlank="1"><x14:formula1><xm:f>Other!$A$1:$A$3</xm:f></x14:formula1><xm:sqref>G1:G4</xm:sqref></x14:dataValidation>"#;
        let v = DataValidation::read(&read(x));
        assert_eq!(
            (v.kind.as_deref(), v.formula1.as_deref(), v.sqref.iter().map(range_ref).collect::<Vec<_>>()),
            (Some("list"), Some("Other!$A$1:$A$3"), vec!["G1:G4".to_string()])
        );
        assert_eq!(
            v.to_el().to_xml(),
            r#"<dataValidation type="list" allowBlank="1" sqref="G1:G4"><formula1>Other!$A$1:$A$3</formula1></dataValidation>"#
        );
    }

    #[test]
    fn references() {
        for r in ["A1", "B2:C9", "A:C", "3:5", "A1:XFD1048576"] {
            assert_eq!(range_ref(&Range::parse(r).unwrap()), r);
        }
        assert_eq!(parse_sqref(" A1  B2:C3 bad 4:4").len(), 3);
    }
}

// ------------------------------------------------------------------------------- tables

element! {
    /// `<tableColumn>`.
    pub struct TableColumn {
        id: u32 = "id",
        unique_name: String = "uniqueName",
        name: String = "name",
        /// `none`, `sum`, `min`, `max`, `average`, `count`, `countNums`, `stdDev`, `var`, `custom`.
        totals_row_function: String = "totalsRowFunction",
        totals_row_label: String = "totalsRowLabel",
        query_table_field_id: u32 = "queryTableFieldId",
        header_row_dxf_id: u32 = "headerRowDxfId",
        data_dxf_id: u32 = "dataDxfId",
        totals_row_dxf_id: u32 = "totalsRowDxfId",
        header_row_cell_style: String = "headerRowCellStyle",
        data_cell_style: String = "dataCellStyle",
        totals_row_cell_style: String = "totalsRowCellStyle",
    }
    + {
        /// `calculatedColumnFormula`, as written, and whether it is an array formula.
        calculated_formula: Option<(String, bool)>,
        /// `totalsRowFormula` (for `custom`), and whether it is an array formula.
        totals_row_formula: Option<(String, bool)>,
        /// `xmlColumnPr` and `extLst`, as written.
        other: Vec<El>,
    }
}

element! {
    /// `<tableStyleInfo>`.
    pub struct TableStyleInfo {
        name: String = "name",
        show_first_column: bool = "showFirstColumn",
        show_last_column: bool = "showLastColumn",
        show_row_stripes: bool = "showRowStripes",
        show_column_stripes: bool = "showColumnStripes",
    }
}

element! {
    /// A table (`<table>`, 18.5): a named range with a header row, columns and a style.
    pub struct Table {
        id: u32 = "id",
        name: String = "name",
        display_name: String = "displayName",
        comment: String = "comment",
        /// `worksheet`, `xml`, `queryTable`.
        table_type: String = "tableType",
        header_row_count: u32 = "headerRowCount",
        insert_row: bool = "insertRow",
        insert_row_shift: bool = "insertRowShift",
        totals_row_count: u32 = "totalsRowCount",
        totals_row_shown: bool = "totalsRowShown",
        published: bool = "published",
        header_row_dxf_id: u32 = "headerRowDxfId",
        data_dxf_id: u32 = "dataDxfId",
        totals_row_dxf_id: u32 = "totalsRowDxfId",
        header_row_border_dxf_id: u32 = "headerRowBorderDxfId",
        table_border_dxf_id: u32 = "tableBorderDxfId",
        totals_row_border_dxf_id: u32 = "totalsRowBorderDxfId",
        header_row_cell_style: String = "headerRowCellStyle",
        data_cell_style: String = "dataCellStyle",
        totals_row_cell_style: String = "totalsRowCellStyle",
        connection_id: u32 = "connectionId",
    }
    + {
        range: Range,
        /// `autoFilter` and `sortState`, as written (they may hold their own extensions).
        auto_filter: Option<El>,
        sort_state: Option<El>,
        columns: Vec<TableColumn>,
        style: Option<TableStyleInfo>,
        ext: Option<El>,
    }
}

impl Table {
    /// A table part read with [`crate::ns::CANONICAL`]. `None` without a readable `ref`.
    pub fn read(t: &El) -> Option<Table> {
        let formula = |c: &El, n: &str| c.child(n).map(|f| (f.text().into_owned(), f.flag("array", false)));
        let columns = t
            .child("tableColumns")
            .map(|cs| {
                cs.children("tableColumn")
                    .map(|c| TableColumn {
                        calculated_formula: formula(c, "calculatedColumnFormula"),
                        totals_row_formula: formula(c, "totalsRowFormula"),
                        other: kids(c, &["calculatedColumnFormula", "totalsRowFormula"]),
                        ..TableColumn::read_attrs(c)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Table {
            range: Range::parse(t.attr("ref")?)?,
            auto_filter: t.child("autoFilter").cloned(),
            sort_state: t.child("sortState").cloned(),
            columns,
            style: t.child("tableStyleInfo").map(TableStyleInfo::read_attrs),
            ext: t.child("extLst").cloned(),
            ..Table::read_attrs(t)
        })
    }

    /// The table part.
    pub fn to_xml(&self) -> Vec<u8> {
        let mut e = El::new("table");
        self.write_attrs(&mut e);
        e.attrs.insert(0, ("ref".into(), self.range.a1()));
        // `id`, `name` and `displayName` lead, as Excel writes them.
        e.attrs.sort_by_key(|(k, _)| match k.as_str() {
            "id" => 0,
            "name" => 1,
            "displayName" => 2,
            "ref" => 3,
            _ => 4,
        });
        for x in [&self.auto_filter, &self.sort_state].into_iter().flatten() {
            e.push(x.clone());
        }
        let mut cols = El::new("tableColumns").with_attr("count", self.columns.len());
        for c in &self.columns {
            let mut ce = El::new("tableColumn");
            c.write_attrs(&mut ce);
            for (n, f) in [("calculatedColumnFormula", &c.calculated_formula), ("totalsRowFormula", &c.totals_row_formula)] {
                if let Some((text, array)) = f {
                    let fe = El::new(n).with_text(text.as_str());
                    ce.push(if *array { fe.with_attr("array", 1) } else { fe });
                }
            }
            ce.children.extend(c.other.iter().cloned().map(Node::El));
            cols.push(ce);
        }
        e.push(cols);
        if let Some(s) = &self.style {
            let mut se = El::new("tableStyleInfo");
            s.write_attrs(&mut se);
            e.push(se);
        }
        if let Some(x) = &self.ext {
            e.push(x.clone());
        }
        crate::ns::declare(&mut e);
        let mut w = craft_xml::Writer::document();
        w.tree(&e);
        w.into_bytes()
    }
}
