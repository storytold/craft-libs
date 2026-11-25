//! The layout services: `LabelColumn`, `ColumnGroup`, the `Grid` cursor, `measure_slot` and `arrange`.

pub(crate) mod arrange;
pub(crate) mod body;
pub(crate) mod column_group;
mod label;
mod slot;

pub use arrange::{Placed, arrange};
pub use body::{BodyLayout, CellAlign, GridFit, RowAlign, ShrinkOrder, ShrinkUnit, empty_cell, levelled_unit, plan_levels, shrink_widths};
pub use column_group::ColumnGroup;
#[doc(hidden)]
pub use column_group::ambient_width;
#[doc(hidden)]
pub use column_group::report_beyond;
pub(crate) use column_group::{Columns, GLYPHS, LABELS, Reached, ambient_room};
pub use label::{control_baseline, place_label, record_control_baseline};
pub use slot::measure_slot;

pub(crate) use crate::runtime::env::Shrink;

#[doc(hidden)]
pub use body::place_in_flow;
#[cfg(feature = "testing")]
pub(crate) use body::run_body;
pub(crate) use body::{
    BodyLine, BracketTook, HeldKind, InFlow, ask_shortfall, at_flow_start, close_first_line, close_flow_start_in, count_fill_view, counting_fills,
    fill_share, fill_size, flow_fill_room, flow_region, flow_room, flow_start, grid_cell, held_kind, in_grid_content_column, leaf_fill_room,
    members_line, open_first_lines, open_view_fill, place_in_cell, report_bracket_control, report_held, report_least, report_shrink, row_alone_room,
    run_body_measured, shares_y, shrink_cap, take_bracket, take_cell, take_in_flow, take_shortfall, take_view_fill, view_room,
};
