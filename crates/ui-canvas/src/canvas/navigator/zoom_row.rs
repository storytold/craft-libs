//! `NavigatorZoomRow`: the Navigator's percent field, Zoom Out, slider and Zoom In.

use crate::buttons::IconButton;
use crate::canvas::{CanvasWords, ViewCommands};
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Row;
use crate::runtime::env::Env;
use crate::text::{InvalidZoom, ZoomField};
use crate::value_drag::{Slider, SliderScale};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvent, EditEvents, IconId, Mixed, ZoomScheme};
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
    /// The zoom field's refused entry of this pass, for the app's alert; `None` when the field took its input.
    pub invalid_zoom: Option<InvalidZoom>,
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
/// - The field is the canvas view's zoom field: percent, the document's range widened to the current zoom, ladder
///   steps. A typed value applies without animation; Escape in the field restores the zoom from before the edit.
/// - Zoom Out and Zoom In are the [`ViewCommands::ZoomOut`] and [`ViewCommands::ZoomIn`] commands: label, chord, enabled
///   state and action come from the active canvas, which steps its view and reports the ease under Animated Zoom in
///   [`CanvasOutput`](crate::canvas::CanvasOutput).
/// - The slider is logarithmic over the same range, so equal distances are equal ratios and it always holds the
///   current zoom. It follows the interface direction like every slider, so in right-to-left the value grows toward
///   the left, where Zoom In sits.
/// - Every zoom goes through [`ZoomScheme::clamp`] and keeps the view's centre.
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
        let scheme = env.view().get::<ZoomScheme>();
        let zoom_label = words.navigator_zoom.clone();

        let mut events = EditEvents::new();
        let mut invalid_zoom = None;
        let shown = Row::new().show(ui, |ui, cx| {
            let Some(Mixed::Same(current)) = view.reborrow().read(cx) else { return };
            let device = zoom::device(&current, pixels_per_point);
            let at = |device: f64| zoom::with_device(&current, device, document, pixels_per_point, &scheme);
            let mut edits: Vec<EditEvent<CanvasView>> = Vec::new();

            let field = ZoomField::new(zoom_label.clone(), Some(current), document).stepper(true).show(ui);
            invalid_zoom = field.invalid_zoom;
            edits.extend(field.view);

            IconButton::command(IconId::new("minus"), ViewCommands::ZoomOut).show(ui);

            let low = scheme.min(document).min(device);
            let high = scheme.max_device().max(device);
            let slider = Slider::new(Bind::given(device), low..=high)
                .scale(SliderScale::Log)
                .label(zoom_label.clone())
                .value_text(units.zoom_percent(scheme.percent(device)))
                .show(ui);
            edits.extend(slider.events.clone().map(at));

            IconButton::command(IconId::new("plus"), ViewCommands::ZoomIn).show(ui);

            for edit in &edits {
                view.reborrow().write(cx, edit);
            }
            events = edits.into_iter().collect();
        });
        NavigatorZoomRowOutput { response: shown.output, view: events, invalid_zoom }
    }
}
