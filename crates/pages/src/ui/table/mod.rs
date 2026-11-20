//! Document tables: the table parts of Word, InDesign and PowerPoint that sit in a page's text: the border edge
//! picker, the table properties panel, the Insert Table size grid and the selected cells' overlay. The spreadsheet
//! grid is craft-table's.

pub mod edges;
pub mod properties;
pub mod selection;
pub mod size_picker;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use edges::{BorderEdges, BorderPreset, EdgeKey, EdgeLook, EdgePicker, EdgePickerStyle, EdgePickerWords, SampleArea};
pub use properties::{CellAlign, RowHeight, TableChange, TableProperties, TablePropertiesOutput, TablePropertiesWords, TableValues, TextDirection};
pub use selection::{CellSelectionOverlay, CellSelectionStyle, TableGeometry};
pub use size_picker::{TableSizePicker, TableSizePickerOutput, TableSizePickerStyle, TableSizePickerWords};
