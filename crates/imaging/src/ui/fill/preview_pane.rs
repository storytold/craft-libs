//! [`FillPreviewPane`]: Content-Aware Fill's Preview panel.

use egui::{Response, Ui};

use craft_behavior::kurbo::{Point, Rect, Size};
use craft_behavior::{CanvasView, ZoomLadder};
use craft_ui::canvas::{NavigationOptions, RasterSource, ViewCommands};
use craft_ui::contract::{Bind, Pattern, ShowContainer as _};
use craft_ui::dialog::{FullWindowCanvas, FullWindowStatus, FullWindowView, FullWindowZoomBar};
use craft_ui::layout_components::Stack;
use craft_ui::primitives::Checker;
use craft_ui::runtime::commands::CommandSet as _;
use craft_ui::runtime::pending::Pending;

use crate::ui::wide_angle::PreviewDone;

/// The window the preview was last framed on, and the zoom and centre that framing gave: the pane frames again
/// when the window moves, but never over the user's own zoom or pan.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreviewFraming(Option<Framed>);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Framed {
    frame: Rect,
    zoom: f64,
    center: Point,
}

/// The Preview panel's content: a zoom bar with the job's progress, the failure of a failed job, and the fill
/// result in its own view, under it.
///
/// - The view opens framed on the sampling window ([`frame`](Self::frame)) and keeps its own zoom and pan:
///   drag pans, the wheel zooms by `NavigationOptions`, the zoom bar steps.
/// - While the job is loading the zoom bar shows its progress; a failure shows its message
///   ("nothing to sample from: the sampling area has no pixels outside the fill area") above the view.
/// - The pane is an instance of the view commands of its own: the zoom chords act on it while it holds the key
///   focus, and on the workspace's canvas otherwise. The canvas takes no tool; its own Hand pans.
#[derive(Pattern)]
pub struct FillPreviewPane<'a> {
    view: &'a mut FullWindowView,
    framing: &'a mut PreviewFraming,
    status: &'a Pending<PreviewDone, String>,
    source: &'a dyn RasterSource,
    frame: Option<Rect>,
    navigation: NavigationOptions,
    checkerboard: Checker,
}

impl<'a> FillPreviewPane<'a> {
    /// The pane over `view` and `framing`, which it pans, zooms and frames, showing `source` and the state of the
    /// preview job `status`.
    pub fn new(
        view: &'a mut FullWindowView,
        framing: &'a mut PreviewFraming,
        status: &'a Pending<PreviewDone, String>,
        source: &'a dyn RasterSource,
    ) -> Self {
        Self { view, framing, status, source, frame: None, navigation: NavigationOptions::default(), checkerboard: Checker::default() }
    }

    /// The sampling window in document pixels: the view fits it until the user zooms.
    #[must_use]
    pub fn frame(mut self, frame: Option<Rect>) -> Self {
        self.frame = frame;
        self
    }

    /// The user's navigation options for the pane's wheel and drag; Photoshop's by default.
    #[must_use]
    pub fn navigation(mut self, navigation: NavigationOptions) -> Self {
        self.navigation = navigation;
        self
    }

    /// The user's transparency grid under the preview.
    #[must_use]
    pub fn checkerboard(mut self, checkerboard: Checker) -> Self {
        self.checkerboard = checkerboard;
        self
    }

    /// Draws the pane in the body of a non-scrolling side column, which gives it the column's room.
    pub fn show(self, ui: &mut Ui) -> Response {
        let Self { view, framing, status, source, frame, navigation, checkerboard } = self;
        frame_view(ui, view, framing, source, frame);
        Stack::new()
            .fill()
            .command_scope(ViewCommands::SCOPE)
            .show(ui, |ui, _| {
                let state = FullWindowStatus::of(status);
                FullWindowZoomBar::new(Bind::bound(&mut *view)).progress(state.progress()).show(ui);
                state.show(ui);
                FullWindowCanvas::new(Bind::bound(&mut *view), source).checkerboard(checkerboard).navigation(navigation).show(ui).response
            })
            .output
    }
}

/// Frames `view` on `frame` when the window moved since the last framing and the user has not moved the view
/// since. The room is the rect the canvas settled on in the last pass, so the first pass fits the whole picture.
fn frame_view(ui: &Ui, view: &mut FullWindowView, framing: &mut PreviewFraming, source: &dyn RasterSource, frame: Option<Rect>) {
    let size = source.size();
    let current = view.view();
    let room = current.rect;
    let Some(frame) = frame.filter(|_| size.width > 0 && size.height > 0 && room.width() > 1.0 && room.height() > 1.0) else { return };
    let untouched = framing.0.is_none_or(|framed| framed.zoom == current.zoom && framed.center == current.center);
    if !untouched || framing.0.is_some_and(|framed| framed.frame == frame) {
        return;
    }
    let ppp = ui.pixels_per_point();
    let device_room = |side: f32| f64::from(side) * f64::from(ppp);
    let fit = (device_room(room.width()) / frame.width().max(1.0)).min(device_room(room.height()) / frame.height().max(1.0));
    let device = ZoomLadder::clamp(fit, Size::new(f64::from(size.width), f64::from(size.height)));
    let center = Point::new((frame.x0 + frame.x1) / 2.0, (frame.y0 + frame.y1) / 2.0);
    let mut next = current;
    next.center = center;
    next.zoom = CanvasView::points_per_unit(device, ppp);
    *view = view.took(next);
    framing.0 = Some(Framed { frame, zoom: next.zoom, center });
}
