//! `NavigatorZoomRow`: the Navigator's percent field, Zoom Out, slider and Zoom In.

use crate::buttons::IconButton;
use crate::canvas::{CanvasWords, ViewCommands};
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Row;
use crate::runtime::env::Env;
use crate::text::NumberField;
use crate::value_drag::{Slider, SliderScale};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvent, EditEvents, IconId, Mixed, ZoomLadder};
use egui::{Response, Ui};

/// What the zoom row reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NavigatorZoomRowOutput {
    /// The row's response.
    pub response: Response,
    /// The view changes of this pass, for a binding that does not write through: a typed percent or a slider drag as
    /// its gesture.
    pub view: EditEvents<CanvasView>,
}

impl Outcome for NavigatorZoomRowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Navigator's zoom row: a percent field, Zoom Out, a slider over the whole zoom range and Zoom In, in reading
/// order.
///
/// - The field shows the zoom in percent, its range the document's zoom range widened to include the current zoom, so
///   the field never pulls the zoom back. A typed value applies without animation; Escape in the field restores the
///   zoom from before the edit.
/// - The field shows up to two decimals, and its arrows and wheel step the zoom ladder.
/// - Zoom Out and Zoom In are the [`ViewCommands::ZoomOut`] and [`ViewCommands::ZoomIn`] commands: label, chord, enabled
///   state and action come from the active canvas, which steps its view and reports the ease under Animated Zoom in
///   [`CanvasOutput`](crate::canvas::CanvasOutput).
/// - The slider is logarithmic over the same range, so equal distances are equal ratios and it always holds the
///   current zoom. It follows the interface direction like every slider, so in right-to-left the value grows toward
///   the left, where Zoom In sits.
/// - Every zoom goes through [`ZoomLadder::clamp`] and keeps the view's centre.
/// - A disabled container around the row disables its field, buttons and slider.
#[must_use = "show the zoom row with `show`"]
#[derive(crate::contract::Pattern)]
pub struct NavigatorZoomRow<'a> {
    view: Bind<'a, CanvasView>,
    document: Size,
}

impl<'a> NavigatorZoomRow<'a> {
    /// A zoom row that edits `view`, over a document of `document` units.
    pub fn new(view: Bind<'a, CanvasView>, document: Size) -> Self {
        NavigatorZoomRow { view, document }
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> NavigatorZoomRowOutput {
        let NavigatorZoomRow { mut view, document } = self;
        let env = Env::of(ui);
        let words = env.words::<CanvasWords>();
        let units = env.units();
        let pixels_per_point = env.scale.0;
        let zoom_label = words.navigator_zoom.clone();

        let mut events = EditEvents::new();
        let shown = Row::new().show(ui, |ui, cx| {
            let Some(Mixed::Same(current)) = view.reborrow().read(cx) else { return };
            let device = zoom::device(&current, pixels_per_point);
            let range = ZoomLadder::percent_range(device, document);
            let at = |device: f64| zoom::with_device(&current, device, document, pixels_per_point);
            let mut edits: Vec<EditEvent<CanvasView>> = Vec::new();

            let ladder = |percent: f64, dir: i32| ZoomLadder::step(percent / 100.0, dir, document) * 100.0;
            let field = NumberField::new(zoom_label.clone(), Bind::given(Some(device * 100.0)))
                .codec(units.zoom_codec())
                .range(range.clone())
                .stepper(ladder)
                .show(ui);
            edits.extend(field.edit.events.clone().map(|percent| percent.map_or(current, |percent| at(percent / 100.0))));

            IconButton::command(IconId::new("minus"), ViewCommands::ZoomOut).show(ui);

            let low = ZoomLadder::min(document).min(device);
            let high = ZoomLadder::MAX.max(device);
            let slider = Slider::new(Bind::given(device), low..=high)
                .scale(SliderScale::Log)
                .label(zoom_label.clone())
                .value_text(units.zoom_percent(device * 100.0))
                .show(ui);
            edits.extend(slider.events.clone().map(at));

            IconButton::command(IconId::new("plus"), ViewCommands::ZoomIn).show(ui);

            for edit in &edits {
                view.reborrow().write(cx, edit);
            }
            events = edits.into_iter().collect();
        });
        NavigatorZoomRowOutput { response: shown.output, view: events }
    }
}
