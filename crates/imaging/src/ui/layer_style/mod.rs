//! The Layer Style dialog body ([`LayerStyleDialog`]): the effect list, the Styles page, the Blending Options page with
//! its [`BlendIfBar`]s, and the effect pages drawn from the [`EffectSpec`](crate::layer_style::EffectSpec) tables, with
//! [`ContourField`]s for contours.
//!
//! The dialog reads a [`LayerStyleDraft`](crate::layer_style::LayerStyleDraft) (the model in [`crate::layer_style`],
//! without egui). The host owns the dialog frame, the draft, the engine and the preview; the dialog reports the edits and
//! asks for colours, presets and contour files. The edit's value is the [`LayerStyle`](crate::layer_style::LayerStyle)
//! alone; the page, the Preview check and the Blend If channel are reported beside it ([`LayerStyleOutput::page`] and the
//! other two), so the host's undo never moves them.
//!
//! ```ignore
//! use craft_imaging::layer_style::LayerStyleDraft;
//! use craft_imaging::ui::layer_style::LayerStyleDialog;
//!
//! let out = LayerStyleDialog::new("Layer Style", &draft, &channels).blend_modes(&modes).show(ui);
//! out.apply_view(&mut draft); // the page, Preview and Blend If channel are the draft's view state
//! if let Some(value) = out.edit.value() {
//!     draft.style = value.clone();
//! }
//! if let Some(request) = out.pick_color {
//!     // Open the colour picker; its answer: LayerStyleDialog::set_color(&mut draft, &request.key, colour).
//! }
//! ```

mod blend_if;
mod blending;
mod contour;
mod dialog;
mod labels;
mod list;
mod page;
#[cfg(feature = "screen-testing")]
mod stories;
mod styles;

pub use blend_if::{BlendIfBar, BlendIfWords};
pub use contour::{
    ContourAction, ContourEditorDialog, ContourField, ContourFieldOutput, ContourKey, ContourRequest, ContourWords, contour_rows, from_curve,
    to_curve,
};
pub use dialog::{LayerStyleDialog, LayerStyleOutput};
pub use labels::LayerStyleWords;
pub use list::LayerStyleCommand;
pub use styles::StylePresets;

pub use crate::layer_style::LayerStylePage;
