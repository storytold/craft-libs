//! The Layers panel for Craft apps, over the app's own layer tree.
//!
//! The app implements `ui::tree::LayerTree`, a read-only view of its layers, and shows it with
//! `ui::panel::LayersPanel`, which reports what the user did as events in the order they
//! happened. The app applies them with its own commands and undo.

pub mod ui;

/// The stories and trials this crate registers, and where a host test finds the parts of a layer row that have no
/// access node of their own: a test finds the row by the layer's name and passes the row's node.
#[cfg(feature = "screen-testing")]
pub mod testing {
    use crate::ui::tree::{LayerInfo, LayerPart};
    use craft_ui::list::{ThumbTarget, TrailingPart};
    use craft_ui::testing::pointer::part_rect_of_node;
    use egui::accesskit::NodeId;
    use egui::{Context, Rect};

    craft_ui::testing_registered!();

    /// The rect of the thumbnail that shows `part` of the layer `info`, in the row whose node is `row`, as the row
    /// recorded it last pass; `None` when the row shows no such thumbnail. `row` is the layer's row, or for
    /// [`LayerPart::FilterMask`] the layer's Smart Filters row, whose thumbnail the filter mask is.
    pub fn thumb_rect(ctx: &Context, row: NodeId, info: &LayerInfo, part: LayerPart) -> Option<Rect> {
        let thumb = if part == LayerPart::FilterMask { ThumbTarget::First } else { info.thumb(part)? };
        part_rect_of_node(ctx, row, thumb.part())
    }

    /// The rect of the `fx` badge of the layer row whose node is `row`, as the row recorded it last pass: the badge
    /// is a drag handle, named in the row's description; `None` when the layer has no effects.
    pub fn fx_badge_rect(ctx: &Context, row: NodeId) -> Option<Rect> {
        part_rect_of_node(ctx, row, TrailingPart::new(crate::ui::panel::FX_BADGE).part())
    }
}
