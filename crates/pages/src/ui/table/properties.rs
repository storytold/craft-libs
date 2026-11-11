//! `TableProperties`: InDesign's Table panel (and the Table section of Properties): rows and columns, row height (At
//! Least or Exactly) and column width, Distribute Rows Evenly, cell text alignment and insets, merge and unmerge,
//! insert and delete rows and columns, the table's direction, alternating row fills, header and footer rows. Every
//! change goes out as one [`TableChange`]; the table stays the app's.

use craft_behavior::{Mixed, Value};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::Field;
use craft_ui::layout_components::Stack;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::sections::GroupBox;
use craft_ui::text::NumberField;
use egui::{Response, Ui};

/// Points in an inch: the unit lengths are kept in.
const POINTS_PER_INCH: f64 = 72.0;

craft_ui::words! {
    /// The panel's words (DesignCraft's strings).
    #[non_exhaustive]
    pub struct TablePropertiesWords {
        rows: "tableProperties.rows" = "Rows";
        columns: "tableProperties.columns" = "Columns";
        row_height_mode: "tableProperties.rowHeightMode" = "Row Height";
        /// The row height's length field (the select above it is "Row Height").
        row_height: "tableProperties.rowHeight" = "Height";
        at_least: "tableProperties.atLeast" = "At Least";
        exactly: "tableProperties.exactly" = "Exactly";
        column_width: "tableProperties.columnWidth" = "Column Width";
        distribute_rows: "tableProperties.distributeRows" = "Distribute Rows Evenly";
        cell_text: "tableProperties.cellText" = "Cell Text";
        cell_align: "tableProperties.cellAlign" = "Vertical Alignment";
        align_top: "tableProperties.alignTop" = "Top";
        align_center: "tableProperties.alignCenter" = "Center";
        align_bottom: "tableProperties.alignBottom" = "Bottom";
        align_justify: "tableProperties.alignJustify" = "Justify";
        inset_top: "tableProperties.insetTop" = "Top";
        inset_left: "tableProperties.insetLeft" = "Left";
        inset_bottom: "tableProperties.insetBottom" = "Bottom";
        inset_right: "tableProperties.insetRight" = "Right";
        merge: "tableProperties.merge" = "Merge Cells";
        unmerge: "tableProperties.unmerge" = "Unmerge";
        insert_row: "tableProperties.insertRow" = "Insert Row";
        insert_column: "tableProperties.insertColumn" = "Insert Column";
        delete_row: "tableProperties.deleteRow" = "Delete Row";
        delete_column: "tableProperties.deleteColumn" = "Delete Column";
        direction: "tableProperties.direction" = "Direction";
        left_to_right: "tableProperties.leftToRight" = "Left to Right";
        right_to_left: "tableProperties.rightToLeft" = "Right to Left";
        alternating_fills: "tableProperties.alternatingFills" = "Alternating Row Fills";
        repeat_header: "tableProperties.repeatHeader" = "Repeat Header Rows";
        header_rows: "tableProperties.headerRows" = "Header Rows";
        footer_rows: "tableProperties.footerRows" = "Footer Rows";
    }
}

/// How a row's height is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RowHeight {
    /// The row is at least this tall.
    #[default]
    AtLeast,
    /// The row is exactly this tall.
    Exactly,
}

/// Cell text's vertical justification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CellAlign {
    /// At the top.
    #[default]
    Top,
    /// In the middle.
    Center,
    /// At the bottom.
    Bottom,
    /// Spread over the cell's height.
    Justify,
}

/// Which way a cell's text runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextDirection {
    /// Left to right.
    #[default]
    LeftToRight,
    /// Right to left.
    RightToLeft,
}

/// The values at the cell selection; `None` where the selected cells differ (the field shows
/// mixed).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TableValues {
    /// Number of rows in the table.
    pub rows: usize,
    /// Number of columns in the table.
    pub columns: usize,
    /// How row heights are set.
    pub row_height_mode: Option<RowHeight>,
    /// Points.
    pub row_height: Option<f64>,
    /// Column width in points.
    pub column_width: Option<f64>,
    /// The cell text's vertical justification.
    pub cell_align: Option<CellAlign>,
    /// Top, left, bottom, right, points.
    pub insets: [Option<f64>; 4],
    /// The table's direction.
    pub direction: TextDirection,
    /// Alternating row fills are on.
    pub alternating_fills: bool,
    /// Header rows repeat on every page.
    pub repeat_header: bool,
    /// Number of header rows.
    pub header_rows: usize,
    /// Number of footer rows.
    pub footer_rows: usize,
    /// More than one cell is selected (Merge Cells is enabled).
    pub several_cells: bool,
    /// The selection holds a merged cell (Unmerge is enabled).
    pub merged: bool,
}

/// One change the user made.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TableChange {
    /// Set the number of rows.
    Rows(usize),
    /// Set the number of columns.
    Columns(usize),
    /// Set how row heights are set.
    RowHeightMode(RowHeight),
    /// Row height in points.
    RowHeight(f64),
    /// Column width in points.
    ColumnWidth(f64),
    /// Make the selected rows equally tall.
    DistributeRows,
    /// Set the cell text's vertical justification.
    CellAlign(CellAlign),
    /// Inset `index` (top, left, bottom, right) in points.
    Inset(usize, f64),
    /// Merge the selected cells.
    Merge,
    /// Split the selected merged cell.
    Unmerge,
    /// Insert a row.
    InsertRow,
    /// Insert a column.
    InsertColumn,
    /// Delete a row.
    DeleteRow,
    /// Delete a column.
    DeleteColumn,
    /// Set the table's direction.
    Direction(TextDirection),
    /// Turn alternating row fills on or off.
    AlternatingFills(bool),
    /// Turn repeating header rows on or off.
    RepeatHeader(bool),
    /// Number of header rows.
    HeaderRows(usize),
    /// Number of footer rows.
    FooterRows(usize),
}

/// What the panel reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct TablePropertiesOutput {
    /// The panel's response.
    pub response: Response,
    /// The change the user made this pass: a value a field proposed, committed or set, or a button pressed. A field's
    /// scrub or typing run reports a change on every step; the app applies each to the table.
    pub change: Option<TableChange>,
}

impl Outcome for TablePropertiesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// InDesign's Table panel body (look InDesign; behaviour Photoshop's panel fields, then DesignCraft).
///
/// - Counts are `NumberField`s and lengths `NumberField`s in the document's unit, empty where [`TableValues`] has
///   `None`; row height mode, cell alignment and direction are `Select`s; Merge, Unmerge, Insert and Delete are
///   `Button`s, enabled from `several_cells` and `merged`.
/// - The number fields have no upper bound (counts from 1, or 0 for header and footer rows; lengths from 0), as
///   DesignCraft's table panel; the app refuses what its table can't take.
/// - Keys: Tab moves through the fields in reading order; each control's own keys.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct TableProperties<'a> {
    values: &'a TableValues,
    unit: UnitKind,
}

impl<'a> TableProperties<'a> {
    /// A panel showing `values`, with lengths in `unit`, a length unit.
    pub fn new(values: &'a TableValues, unit: UnitKind) -> Self {
        TableProperties { values, unit }
    }

    /// `points` in `unit`.
    pub fn in_unit(points: f64, unit: UnitKind) -> f64 {
        unit.per_inch(POINTS_PER_INCH).map_or(points, |per_inch| points / POINTS_PER_INCH * per_inch)
    }

    /// `value` of `unit` in points.
    pub fn in_points(value: f64, unit: UnitKind) -> f64 {
        unit.per_inch(POINTS_PER_INCH).map_or(value, |per_inch| value / per_inch * POINTS_PER_INCH)
    }

    // designcraft: crates/ui-egui/src/panels/table.rs:11 `show`.
    /// Shows the panel and returns the change the user made.
    pub fn show(self, ui: &mut Ui) -> TablePropertiesOutput {
        let words = Env::of(ui).words::<TablePropertiesWords>();
        let (v, unit) = (self.values, self.unit);
        let mut change = None;
        let shown = Stack::new().show(ui, |ui, _| {
            let mut set = |next: Option<TableChange>| change = change.take().or(next);
            set(count(ui, &words.rows, v.rows, 1, TableChange::Rows));
            set(count(ui, &words.columns, v.columns, 1, TableChange::Columns));
            let modes = [Entry::new(RowHeight::AtLeast, words.at_least.clone()), Entry::new(RowHeight::Exactly, words.exactly.clone())];
            set(choose(ui, &words.row_height_mode, v.row_height_mode, &modes, TableChange::RowHeightMode));
            set(length(ui, &words.row_height, v.row_height, unit, TableChange::RowHeight));
            set(length(ui, &words.column_width, v.column_width, unit, TableChange::ColumnWidth));
            set(button(ui, &words.distribute_rows, true, TableChange::DistributeRows));
            GroupBox::new(words.cell_text.clone(), None).show(ui, |ui, _| {
                let aligns = [
                    Entry::new(CellAlign::Top, words.align_top.clone()),
                    Entry::new(CellAlign::Center, words.align_center.clone()),
                    Entry::new(CellAlign::Bottom, words.align_bottom.clone()),
                    Entry::new(CellAlign::Justify, words.align_justify.clone()),
                ];
                set(choose(ui, &words.cell_align, v.cell_align, &aligns, TableChange::CellAlign));
                let insets = [&words.inset_top, &words.inset_left, &words.inset_bottom, &words.inset_right];
                for (index, caption) in insets.into_iter().enumerate() {
                    set(length(ui, caption, v.insets[index], unit, move |points| TableChange::Inset(index, points)));
                }
            });
            set(button(ui, &words.merge, v.several_cells, TableChange::Merge));
            set(button(ui, &words.unmerge, v.merged, TableChange::Unmerge));
            set(button(ui, &words.insert_row, true, TableChange::InsertRow));
            set(button(ui, &words.insert_column, true, TableChange::InsertColumn));
            set(button(ui, &words.delete_row, true, TableChange::DeleteRow));
            set(button(ui, &words.delete_column, true, TableChange::DeleteColumn));
            let directions = [
                Entry::new(TextDirection::LeftToRight, words.left_to_right.clone()),
                Entry::new(TextDirection::RightToLeft, words.right_to_left.clone()),
            ];
            set(choose(ui, &words.direction, Some(v.direction), &directions, TableChange::Direction));
            set(check(ui, &words.alternating_fills, v.alternating_fills, TableChange::AlternatingFills));
            set(check(ui, &words.repeat_header, v.repeat_header, TableChange::RepeatHeader));
            set(count(ui, &words.header_rows, v.header_rows, 0, TableChange::HeaderRows));
            set(count(ui, &words.footer_rows, v.footer_rows, 0, TableChange::FooterRows));
        });
        TablePropertiesOutput { response: shown.output, change }
    }
}

/// A whole-number field from `least`, labelled `caption`.
fn count(ui: &mut Ui, caption: &Label, value: usize, least: usize, change: fn(usize) -> TableChange) -> Option<TableChange> {
    let mut picked = None;
    Field::new(caption.clone()).show(ui, |ui, _| {
        let edit = NumberField::new(caption.clone(), Bind::given(Some(value as f64))).range(least as f64..=f64::INFINITY).step(1.0).show(ui);
        picked = edit.value().copied().flatten().map(|n| change(n.round().max(least as f64) as usize));
    });
    picked
}

/// A length field: `points` shown in the document's `unit`, empty while the selected cells differ.
fn length(ui: &mut Ui, caption: &Label, points: Option<f64>, unit: UnitKind, change: impl Fn(f64) -> TableChange) -> Option<TableChange> {
    let mut picked = None;
    Field::new(caption.clone()).show(ui, |ui, _| {
        let shown = points.map(|p| TableProperties::in_unit(p, unit));
        let edit = NumberField::new(caption.clone(), Bind::given(shown)).unit_kind(unit).range(0.0..=f64::INFINITY).show(ui);
        picked = edit.value().copied().flatten().map(|value| change(TableProperties::in_points(value, unit)));
    });
    picked
}

/// A select over `entries`; mixed while the selected cells differ.
fn choose<T: Value + Copy>(
    ui: &mut Ui,
    caption: &Label,
    value: Option<T>,
    entries: &[Entry<T>],
    change: fn(T) -> TableChange,
) -> Option<TableChange> {
    let mut picked = None;
    Field::new(caption.clone()).show(ui, |ui, _| {
        let shown = value.map_or(Mixed::Mixed, Mixed::Same);
        picked = Select::new(caption.clone(), Bind::given(shown), entries).show(ui).value().copied().map(change);
    });
    picked
}

fn check(ui: &mut Ui, caption: &Label, value: bool, change: fn(bool) -> TableChange) -> Option<TableChange> {
    Checkbox::new(Bind::given(value)).label(caption.clone()).show(ui).value().copied().map(change)
}

fn button(ui: &mut Ui, caption: &Label, enabled: bool, change: TableChange) -> Option<TableChange> {
    clicked(&Button::new(caption.clone()).enabled(enabled).show(ui)).then_some(change)
}
