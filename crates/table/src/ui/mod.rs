//! The table widgets, for egui. The look follows Excel (GridCraft), the condition builder
//! LightCraft's smart-collection rules.
//!
//! - [`grid`] has the cell-selection model, [`LineIndex`], [`SheetGeometry`] and the [`CellGrid`].
//! - [`conditions`] has the [`ConditionBuilder`].
//! - [`outline`] has the [`OutlineBand`].
//! - [`fields`] has the [`FieldListPane`].
//! - [`chart`] has [`ChartPrimitive`] and [`paint_chart`].
//! - [`slicer`] has the [`Slicer`].
//! - [`refpick`] has the [`ReferencePicker`].
//! - [`stats`] has [`SelectionStats`] and the [`SelectionStatsReadout`].
//! - [`error_tag`] has the [`ErrorTag`].
//! - [`filter`] has the [`FilterButton`].
//!
//! The cell-selection model ([`CellAt`], [`CellRange`], [`CellSelection`], [`CellSpace`]) is craft-behavior's,
//! re-exported here.

pub mod chart;
pub mod conditions;
pub mod error_tag;
pub mod fields;
pub mod filter;
pub mod grid;
pub mod outline;
pub mod refpick;
pub mod slicer;
pub mod stats;

/// The registrations of the table widgets' stories and trials.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}

pub use chart::{ChartPrimitive, HAlign, VAlign, paint_chart};
pub use conditions::{
    Condition, ConditionBuilder, ConditionBuilderOutput, ConditionFields, ConditionGroup, ConditionItem, ConditionKind, ConditionValue,
    ConditionWords, Match,
};
pub use error_tag::{ErrorTag, ErrorTagOutput, ErrorTagStyle, ErrorTagWords};
pub use fields::{
    FieldArea, FieldGroup, FieldListEdit, FieldListPane, FieldListPaneOutput, FieldListWords, FieldMove, FieldSort, PivotField, ReportLayout, ShowAs,
    Summary, ValueSetting, ValuesRow,
};
pub use filter::{FilterButton, FilterButtonEvent, FilterButtonOutput, FilterButtonStyle, FilterButtonWords};
pub use grid::{
    CellAdvance, CellAt, CellDirection, CellEdit, CellGrid, CellGridStyle, CellGridWords, CellProblem, CellRange, CellSelection, CellSpace,
    ColumnFilter, GridEdit, GridLayout, GridMenuTarget, GridModel, GridOutput, GridRow, GridState, HEADER_MIN_ZOOM, LineIndex, Positions,
    SHEET_COLUMNS, SHEET_ROWS, SHEET_ZOOM, SheetGeometry, SheetMetrics, absolute_range_reference, absolute_reference, column_letters,
};
pub use outline::{OutlineAction, OutlineAxis, OutlineBand, OutlineBandOutput, OutlineBandStyle, OutlineBandWords, OutlineGroup};
pub use refpick::ReferencePicker;
pub use slicer::{Slicer, SlicerAction, SlicerOutput, SlicerWords};
pub use stats::{
    SelectionStats, SelectionStatsOutput, SelectionStatsReadout, SelectionStatsReadoutStyle, StatPart, StatsShown, StatsWords, stats_parts,
};
