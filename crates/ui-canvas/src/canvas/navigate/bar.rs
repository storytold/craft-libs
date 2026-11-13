//! [`NavigateBar`]: the options of the Zoom, Rotate View and Hand tools, as members of the app's options bar.

use super::NavigationOptions;
use crate::buttons::Button;
use crate::canvas::{CanvasWords, ViewCommands};
use crate::choice::{Checkbox, Segmented};
use crate::contract::{Bind, BodyCx, Entry, Gap, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::display::Divider;
use crate::form::Field;
use crate::layout_components::Row;
use crate::runtime::env::{Env, UnitKind};
use crate::text::NumberField;
use craft_behavior::canvas::zoom;
use craft_behavior::geometry::crop::normalized_angle;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvents, IconId, Mixed, ZoomLadder};
use egui::{Response, Ui};

/// What the navigate bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NavigateBarOutput {
    /// The strip's response.
    pub response: Response,
    /// The view changes of this pass from the Zoom and Angle fields, for a binding that does not write through.
    pub view: EditEvents<CanvasView>,
}

impl Outcome for NavigateBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The members of one tool's bar.
enum Members<'a> {
    Zoom { view: Bind<'a, CanvasView>, document: Size, options: &'a mut NavigationOptions, out: Bind<'a, bool> },
    RotateView { view: Bind<'a, CanvasView> },
    Hand { options: &'a mut NavigationOptions },
}

/// The options of a navigation tool (Zoom, Rotate View, Hand), shown as children of the app's options bar, as in
/// Photoshop.
///
/// The members:
///
/// - **Zoom:** Zoom In and Zoom Out as icon segments over the tool's direction, the Zoom field, then Resize Windows to
///   Fit and Scrubby Zoom, then 100%, Fit Screen and Fill Screen.
/// - **Rotate View:** the Angle field, then Reset View.
/// - **Hand:** Scroll All Windows, then 100%, Fit Screen and Fill Screen.
///
/// The fields edit the bound view: Zoom in percent, its range the document's zoom range widened to include the current
/// zoom, its arrows stepping the zoom ladder, the centre kept; Angle in degrees from -180 to 180. The buttons are the
/// [`ViewCommands`]: they act on the canvas that holds the key focus (else the one that held it last), which applies
/// them with its document and eases them under Animated Zoom; their enabled state comes from that canvas. The check
/// boxes write into the [`NavigationOptions`].
#[must_use = "show the bar's children with `show`"]
#[derive(crate::contract::Pattern)]
pub struct NavigateBar<'a> {
    members: Members<'a>,
}

impl<'a> NavigateBar<'a> {
    /// The Zoom tool's options over `view`, a document of `document` units, the user's `options` and the tool's
    /// direction `out` (Zoom Out picked).
    pub fn zoom(view: Bind<'a, CanvasView>, document: Size, options: &'a mut NavigationOptions, out: Bind<'a, bool>) -> Self {
        NavigateBar { members: Members::Zoom { view, document, options, out } }
    }

    /// The Rotate View tool's options over `view`.
    pub fn rotate_view(view: Bind<'a, CanvasView>) -> Self {
        NavigateBar { members: Members::RotateView { view } }
    }

    /// The Hand tool's options over the user's `options`.
    pub fn hand(options: &'a mut NavigationOptions) -> Self {
        NavigateBar { members: Members::Hand { options } }
    }

    /// Shows the bar's children.
    pub fn show(self, ui: &mut Ui) -> NavigateBarOutput {
        let NavigateBar { members } = self;
        let env = Env::of(ui);
        let words = env.words::<CanvasWords>();
        let units = env.units();
        let pixels_per_point = env.scale.0;
        let mut events = EditEvents::new();
        let row = Row::new().gap(Gap::Item);
        let shown = match members {
            Members::Zoom { mut view, document, options, mut out } => row.show(ui, |ui, cx| {
                let directions = [
                    Entry::new(false, words.navigate_zoom_in.clone()).icon(IconId::new("zoom_in")),
                    Entry::new(true, words.navigate_zoom_out.clone()).icon(IconId::new("zoom_out")),
                ];
                Segmented::new(words.navigate_direction.clone(), out.reborrowed(), &directions).show(ui);
                if let Some(Mixed::Same(current)) = view.reborrow().read(cx) {
                    let device = zoom::device(&current, pixels_per_point);
                    let ladder = |percent: f64, dir: i32| ZoomLadder::step(percent / 100.0, dir, document) * 100.0;
                    let field = NumberField::new(words.navigate_zoom.clone(), Bind::given(Some(device * 100.0)))
                        .codec(units.zoom_codec())
                        .range(ZoomLadder::percent_range(device, document))
                        .stepper(ladder)
                        .show(ui);
                    let at = |percent: Option<f64>| {
                        percent.map_or(current, |percent| zoom::with_device(&current, percent / 100.0, document, pixels_per_point))
                    };
                    events = write(&mut view, cx, field.edit.events.clone().map(at));
                }
                Divider::vertical().show(ui);
                Checkbox::new(Bind::bound(&mut options.zoom_resizes_windows)).label(words.navigate_resize_windows.clone()).show(ui);
                Checkbox::new(Bind::bound(&mut options.scrubby_zoom)).label(words.navigate_scrubby_zoom.clone()).show(ui);
                Divider::vertical().show(ui);
                view_buttons(ui, &words);
            }),
            Members::RotateView { mut view } => row.show(ui, |ui, cx| {
                if let Some(Mixed::Same(current)) = view.reborrow().read(cx) {
                    let angle = Field::new(words.navigate_angle.clone()).show(ui, |ui, _| {
                        NumberField::new(words.navigate_angle.clone(), Bind::given(Some(current.rotation)))
                            .unit_kind(UnitKind::Degrees)
                            .range(-180.0..=180.0)
                            .show(ui)
                    });
                    if let Some(field) = angle.inner {
                        let at = |angle: Option<f64>| {
                            let mut turned = current;
                            turned.rotation = angle.map_or(current.rotation, normalized_angle);
                            turned
                        };
                        events = write(&mut view, cx, field.edit.events.clone().map(at));
                    }
                }
                Button::command(ViewCommands::ResetRotation).label(words.navigate_reset_view.clone()).show(ui);
            }),
            Members::Hand { options } => row.show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut options.scroll_all_windows)).label(words.navigate_scroll_all.clone()).show(ui);
                Divider::vertical().show(ui);
                view_buttons(ui, &words);
            }),
        };
        NavigateBarOutput { response: shown.output, view: events }
    }
}

/// Writes the field's view `edits` through `view` and returns them.
fn write(view: &mut Bind<'_, CanvasView>, cx: &mut BodyCx<'_>, edits: EditEvents<CanvasView>) -> EditEvents<CanvasView> {
    for edit in edits.iter() {
        view.reborrow().write(cx, edit);
    }
    edits
}

/// 100%, Fit Screen and Fill Screen: the view commands the Zoom and Hand bars share.
fn view_buttons(ui: &mut Ui, words: &CanvasWords) {
    Button::command(ViewCommands::ActualPixels).show(ui);
    Button::command(ViewCommands::FitOnScreen).label(words.navigate_fit_screen.clone()).show(ui);
    Button::command(ViewCommands::FillScreen).label(words.navigate_fill_screen.clone()).show(ui);
}
