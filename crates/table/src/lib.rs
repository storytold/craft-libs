//! Spreadsheet widgets: the cell grid and the tools around it (outline bands, field lists,
//! conditions, charts, slicers, reference picking, statistics, error tags, filter buttons).
//! UI-only: everything is in [`ui`], behind the `ui` feature.

#[cfg(feature = "ui")]
pub mod ui;
