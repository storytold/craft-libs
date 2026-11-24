//! `Table` and `MetadataTable`, as `List` with columns.

pub(crate) mod metadata;
mod sort;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub(crate) mod view;
mod words;

pub use metadata::{Checked, FieldKey, MetadataGroup, MetadataRow, MetadataTable, MetadataTableOutput};
pub use sort::sort_rows;
pub use view::{Table, TableStyle};
