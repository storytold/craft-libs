//! `Table` and `MetadataTable`, as `List` with columns.

mod metadata;
mod sort;
#[cfg(feature = "testing")]
mod stories;
mod view;
mod words;

pub use metadata::{Checked, FieldKey, MetadataGroup, MetadataRow, MetadataTable, MetadataTableOutput};
pub use sort::sort_rows;
pub use view::{Table, TableStyle};
