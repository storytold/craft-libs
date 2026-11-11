//! The label step (B2, 16.3): a side label's placement, label scope, scrub, double-click reset and reset dot. A
//! container with `ContainerProps.label` runs it in its body step and `PaintUi::labelled` runs it for one leaf
//! child, so the two line up and behave alike.

pub use crate::runtime::access::scope::FieldLabelRole;

mod leaf;
mod place;
mod props;
mod step;

#[doc(hidden)]
pub use leaf::colon_of;
pub(crate) use place::{control_after, label_gap};
pub use props::{CellWidth, FieldLayout, GlyphLabel, LabelAlign, LabelKind, Scrub, SideLabel};
pub(crate) use step::{LabelIn, LeafLabel, flow_leaf, leaf, run, show_leaf_in_flow};
