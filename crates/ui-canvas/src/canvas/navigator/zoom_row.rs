//! `NavigatorZoomRow`: the Navigator's percent field, Zoom Out, slider and Zoom In.

use crate::buttons::IconButton;
use crate::canvas::{CanvasWords, ViewCommands};
use crate::contract::{Bind, BodyCx, NoValue, Outcome, Props as _, Remember as _, Scroll, Show as _, ShowContainer as _, Sizing};
use crate::layout::place_in_flow;
use crate::layout_components::{Row, Stack};
use crate::runtime::env::Env;
use crate::runtime::focus::{self, FocusTarget};
use crate::style::StyleCx;
use crate::text::{InvalidZoom, ZoomField};
use crate::value_drag::{Slider, SliderScale};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvent, EditEvents, IconId, Mixed, Proposal, ZoomScheme};
use egui::{Id, Response, Ui, Vec2};

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
/// When the field and the other controls cannot share one line, the buttons and slider move below it. In a narrower
/// room those controls also stack, with the slider taking the available width.
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
        self.show_in(ui, false, false)
    }

    pub(in crate::canvas) fn show_in_panel(self, ui: &mut Ui, focus_field: bool) -> NavigatorZoomRowOutput {
        self.show_in(ui, focus_field, true)
    }

    fn show_in(self, ui: &mut Ui, focus_field: bool, panel: bool) -> NavigatorZoomRowOutput {
        let NavigatorZoomRow { mut view, document } = self;
        let env = Env::of(ui);
        let words = env.words::<CanvasWords>();
        let units = env.units();
        let pixels_per_point = env.scale.0;
        let scheme = env.view().get::<ZoomScheme>();
        let zoom_label = words.navigator_zoom.clone();
        let style = StyleCx::of_env(ui.ctx(), &env);
        let control = style.control();
        let thumb = style.glyph().max(control.icon);

        let mut events = EditEvents::new();
        let mut invalid_zoom = None;
        // Measure in the caller's cell, then contribute only the width the controls use.
        let stack = Stack::new();
        let stack = if panel { stack.scroll(Scroll::vertical().overlay()) } else { stack };
        let shown = stack.show(ui, |ui, cx| {
            let Some(Mixed::Same(current)) = view.reborrow().read(cx) else { return };
            let device = zoom::device(&current, pixels_per_point);
            let at = |device: f64| zoom::with_device(&current, device, document, pixels_per_point, &scheme);
            let mut edits: Vec<EditEvent<CanvasView>> = Vec::new();

            let [minus_id, plus_id, slider_id] = ["zoom-out", "zoom-in", "zoom-slider"].map(|name| cx.id.with(name));
            let zoom_out = || IconButton::command(IconId::new("minus"), ViewCommands::ZoomOut).id(minus_id);
            let zoom_in = || IconButton::command(IconId::new("plus"), ViewCommands::ZoomIn).id(plus_id);
            let low = scheme.min(document).min(device);
            let high = scheme.max_device().max(device);
            let slider = || {
                Slider::new(Bind::given(device), low..=high)
                    .id(slider_id)
                    .scale(SliderScale::Log)
                    .label(zoom_label.clone())
                    .value_text(units.zoom_percent(scheme.percent(device)))
                    .sizing(Sizing::FillWidth)
            };
            let clip = ui.clip_rect();
            let watch_focus = panel && !cx.memory.is_peek();
            // A resize can clip a control without moving focus. Reveal a previously visible stop once, rather
            // than pulling it back whenever a user scrolls it out of view.
            if watch_focus
                && let Some((room, id)) = *cx.memory::<Option<(Vec2, Id)>>()
                && (clip.width() < room.x || clip.height() < room.y)
                && focus::is_focused(ui.ctx(), id)
                && !ui.input(|input| input.pointer.any_pressed())
            {
                focus::request(ui.ctx(), FocusTarget::Id(id));
            }
            let shown = Row::new().wrap(true).show(ui, |ui, _| {
                let minus = zoom_out().measure_in(ui, Proposal::Min);
                let plus = zoom_in().measure_in(ui, Proposal::Min);
                let track = slider().measure_in(ui, Proposal::Ideal);
                let gap = env.tokens.space.gap.row;
                let buttons_width = minus.x + plus.x + 2.0 * gap;
                // A themed thumb can be as wide as the icon. Leave room past it for pointer travel.
                let track_min = control.height.max(thumb + 4.0);
                // Cap unbounded rooms at the natural track width. The wrapping row may also have its own line cap.
                let natural_width = control.number + gap + buttons_width + track.x.max(track_min);
                let minimum = track_min.max(minus.x).max(plus.x);
                let preferred = if Env::of(ui).view().measuring_least_width() { minimum } else { natural_width };
                let width = ui.available_width().min(preferred);
                let controls_width = buttons_width + track_min;
                let one_line = width >= control.number + gap + controls_width;
                let stack_controls = width < controls_width;
                let (field_width, group_width) = if one_line { (control.number, width - control.number - gap) } else { (width, width) };
                let group_height = if stack_controls { minus.y + track.y + plus.y + 2.0 * gap } else { minus.y.max(plus.y).max(track.y) };
                let field_rect = place_in_flow(ui, Vec2::new(field_width, control.height));
                let field = ZoomField::new(zoom_label.clone(), Some(current), document).stepper(true).show_at(ui, field_rect);
                if focus_field {
                    focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
                }
                invalid_zoom = field.invalid_zoom;
                edits.extend(field.view);

                let group_width = if one_line { group_width.min(ui.available_rect_before_wrap().width()) } else { group_width };
                let group_rect = place_in_flow(ui, Vec2::new(group_width, group_height));
                let show_controls = |ui: &mut Ui, _: &mut BodyCx<'_>| {
                    zoom_out().show(ui);
                    let slider = if stack_controls {
                        let rect = place_in_flow(ui, Vec2::new(group_width.max(track_min), track.y));
                        slider().show_at(ui, rect)
                    } else {
                        slider().show(ui)
                    };
                    zoom_in().show(ui);
                    slider.events.map(at)
                };
                let controls = if stack_controls {
                    let stack = Stack::new();
                    let stack = if group_width < minimum { stack.scroll(Scroll::horizontal().overlay()) } else { stack };
                    stack.show_at(ui, group_rect, show_controls)
                } else {
                    Row::new().fill_width().show_at(ui, group_rect, show_controls)
                };
                edits.extend(controls.inner.into_iter().flatten());
                field.response.id
            });
            if watch_focus && let Some(field_id) = shown.inner {
                *cx.memory::<Option<(Vec2, Id)>>() = [field_id, minus_id, slider_id, plus_id]
                    .into_iter()
                    .find(|&id| {
                        focus::is_focused(ui.ctx(), id) && ui.ctx().read_response(id).is_some_and(|response| clip.contains_rect(response.rect))
                    })
                    .map(|id| (clip.size(), id));
            }

            for edit in &edits {
                view.reborrow().write(cx, edit);
            }
            events = edits.into_iter().collect();
        });
        NavigatorZoomRowOutput { response: shown.output, view: events, invalid_zoom }
    }
}
