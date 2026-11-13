//! The zoom field of a canvas view: the Navigator's and the Zoom tool bar's percent field.

use crate::contract::{Bind, Props as _, Show as _};
use crate::runtime::env::{Env, Label, UnitKind};
use crate::text::NumberField;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvents, ZoomLadder};
use egui::Ui;

/// Shows the zoom field named `label` over `current`, a view of a document of `document` units, and returns the views
/// it asks for this pass.
///
/// - The field shows the device zoom in percent, with the percent sign and up to two decimals.
/// - Its range is the document's zoom range widened to include the current zoom, so the field never pulls the zoom
///   back.
/// - Its arrows, Page keys and wheel step the zoom ladder.
/// - A typed zoom goes through [`ZoomLadder::clamp`] and keeps the view's centre; clearing the field keeps the view.
pub(crate) fn show_zoom_field(ui: &mut Ui, label: Label, current: CanvasView, document: Size) -> EditEvents<CanvasView> {
    let env = Env::of(ui);
    let pixels_per_point = env.scale.0;
    let device = zoom::device(&current, pixels_per_point);
    let ladder = |percent: f64, dir: i32| ZoomLadder::step(percent / 100.0, dir, document) * 100.0;
    let field = NumberField::new(label, Bind::given(Some(device * 100.0)))
        .codec(env.units().zoom_codec())
        .unit_kind(UnitKind::Percent)
        .range(ZoomLadder::percent_range(device, document))
        .stepper(ladder)
        .show(ui);
    field
        .edit
        .events
        .clone()
        .map(|percent| percent.map_or(current, |percent| zoom::with_device(&current, percent / 100.0, document, pixels_per_point)))
}
