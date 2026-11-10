//! The generic object tree the model is built from: [`RawTree`] (the walk, in memory) of [`RawObject`]s, with
//! the bulky point lists of brush strokes as [`PackedList`]s.

mod builder;
mod packed;
mod tree;

pub use builder::RawTree;
pub use packed::{PackedLayout, PackedList, PackedRow};
pub use tree::{RawObject, RawValue};
