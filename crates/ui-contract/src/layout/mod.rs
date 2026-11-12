//! The layout services: `LabelColumn`, `ColumnGroup`, the `Grid` cursor, `measure_slot` and `arrange`.

pub(crate) mod arrange;
pub(crate) mod body;
pub(crate) mod column_group;
mod label;
mod slot;

pub use arrange::{Placed, arrange};
pub use body::{BodyLayout, RowAlign, ShrinkOrder, ShrinkUnit, shrink_widths};
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
pub(crate) use body::{
    InFlow, MainFill, close_first_line, counting_fills, fill_share, flow_fill_room, flow_region, flow_room, in_grid_content_column, leaf_fill_room,
    main_fill, open_first_lines, place_in_cell, report_least, report_shrink, row_alone_room, run_body, run_body_measured, shrink_cap, take_in_flow,
};
