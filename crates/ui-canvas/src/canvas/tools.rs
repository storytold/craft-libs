//! The view tools (Hand, Zoom, Rotate View) and the view commands.
//!
//! A view tool never writes the app's view. It computes the view the gesture asks for from the canvas's
//! [`CanvasSpace`](super::CanvasSpace) and sets it with [`ToolContext::set_view`](super::ToolContext::set_view):
//! [`ToolHost::run`](super::ToolHost::run) reports it in [`ToolHostResponse::view`](super::ToolHostResponse::view) and
//! the app copies it into its own view (through a [`ZoomAnimation`](super::ZoomAnimation) when the response says the
//! change eases).

use super::navigate::{FlickPan, NavigationOptions, pan};
use super::rotate::ViewTurn;
use super::zoom_motion::{HoldZoom, ZoomDrag};
use super::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use crate::primitives::Draw;
use craft_behavior::canvas::zoom;
use craft_behavior::geometry::crop::normalized_angle;
use craft_behavior::kurbo::{Rect, Size, Vec2};
use craft_behavior::{CancelReason, CanvasView, Cursor, PaintedCursor, ZoomLadder};
use craft_keymap::Mods;
use egui::Pos2;

/// The rotation step of the Rotate View commands, in degrees.
const ROTATE_STEP_DEG: f64 = 15.0;

/// The room Fit Artboard on Screen leaves around the board, in points: at the sides, and above and below (the board's
/// name sits above it).
const ARTBOARD_MARGIN: Vec2 = Vec2::new(30.0, 40.0); // design: allow[literal-size/vec2](view-fit room in screen points, not a look)

/// The room Fit Layers on Screen leaves around the layers' bounds, in points.
const LAYERS_MARGIN: Vec2 = Vec2::new(20.0, 20.0); // design: allow[literal-size/vec2](view-fit room in screen points, not a look)

/// Photoshop's default Screen Resolution (Preferences, Units & Rulers), in pixels per inch.
const SCREEN_PPI: f64 = 72.0;

/// What the view commands act on: the document's size and the facts some commands need. The app gives it to
/// [`Canvas::document`](super::Canvas::document); a command whose fact is absent is disabled
/// ([`ViewCommands::available`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ViewTarget {
    /// The document's size, in document units.
    pub document: Size,
    /// The artboard Fit Artboard on Screen shows (the active layer's board, else the last one), in document units.
    pub artboard: Option<Rect>,
    /// The bounds Fit Layer(s) on Screen shows (the selected layers' content), in document units.
    pub layers: Option<Rect>,
    /// The document's resolution in pixels per inch, for Print Size.
    pub ppi: Option<f64>,
    /// The screen resolution Print Size shows the document at, in pixels per inch (Preferences, Units & Rulers).
    /// Default: 72.
    pub screen_ppi: f64,
}

impl ViewTarget {
    /// A document of `size` with no artboard, no layer bounds and no resolution.
    pub fn new(document: Size) -> Self {
        Self { document, artboard: None, layers: None, ppi: None, screen_ppi: SCREEN_PPI }
    }

    /// The artboard Fit Artboard on Screen shows.
    #[must_use]
    pub fn artboard(mut self, artboard: Rect) -> Self {
        self.artboard = Some(artboard);
        self
    }

    /// The bounds Fit Layer(s) on Screen shows.
    #[must_use]
    pub fn layers(mut self, layers: Rect) -> Self {
        self.layers = Some(layers);
        self
    }

    /// The document's resolution, for Print Size.
    #[must_use]
    pub fn ppi(mut self, ppi: f64) -> Self {
        self.ppi = Some(ppi);
        self
    }

    /// The screen resolution Print Size shows the document at.
    #[must_use]
    pub fn screen_ppi(mut self, screen_ppi: f64) -> Self {
        self.screen_ppi = screen_ppi;
        self
    }

    /// Print Size's device zoom: the screen resolution over the document's.
    fn print_zoom(&self) -> Option<f64> {
        self.ppi.filter(|ppi| ppi.is_finite() && *ppi > 0.0).map(|ppi| self.screen_ppi / ppi).filter(|zoom| zoom.is_finite() && *zoom > 0.0)
    }
}

impl From<Size> for ViewTarget {
    fn from(document: Size) -> Self {
        Self::new(document)
    }
}

/// A rect with area, or none.
fn with_area(rect: Option<Rect>) -> Option<Rect> {
    rect.map(|rect| rect.abs()).filter(|rect| rect.width() > 0.0 && rect.height() > 0.0)
}

/// The view commands: the keyboard path of the view tools, of the navigation menu and of the navigation bar's buttons.
/// [`apply`](Self::apply) is the one place that says what each does to a view, and [`available`](Self::available) when
/// each can run. The commands are scoped: a canvas container and a full-window dialog can be instances of the
/// `canvasView` scope, and a fired command goes to the instance that holds the key focus, else the one that held it
/// last, so a chord acts on the dialog's view while the dialog is open. The zoom commands also fire inside a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "canvasView", scope = "canvasView")]
#[non_exhaustive]
pub enum ViewCommands {
    /// Zoom in to the next step of the zoom ladder about the view's centre.
    #[command(label = "Zoom In", modal = "in_dialogs", default = "Cmd+=", default = "Cmd++")]
    ZoomIn,
    /// Zoom out to the previous step of the zoom ladder about the view's centre.
    #[command(label = "Zoom Out", modal = "in_dialogs", default = "Cmd+-")]
    ZoomOut,
    /// Show the whole document.
    #[command(label = "Fit on Screen", modal = "in_dialogs", default = "Cmd+0")]
    FitOnScreen,
    /// Zoom until the document covers the canvas.
    #[command(label = "Fill Screen")]
    FillScreen,
    /// Show the document at 100 percent: one document pixel per device pixel, about the view's centre.
    #[command(label = "100%", modal = "in_dialogs", default = "Cmd+1")]
    ActualPixels,
    /// Show the document at 200 percent about the view's centre.
    #[command(label = "200%")]
    TwoHundredPercent,
    /// Show the document at its print size: the screen resolution over the document's, about the view's centre.
    /// Disabled without the document's resolution.
    #[command(label = "Print Size")]
    PrintSize,
    /// Show the whole artboard, centred, with room for its name. Disabled without an artboard.
    #[command(label = "Fit Artboard on Screen")]
    FitArtboard,
    /// Show the selected layers' bounds, centred. Disabled without them.
    #[command(label = "Fit Layer(s) on Screen")]
    FitLayers,
    /// Turn the view clockwise by 15 degrees.
    #[command(label = "Rotate View Clockwise")]
    RotateClockwise,
    /// Turn the view counterclockwise by 15 degrees.
    #[command(label = "Rotate View Counterclockwise")]
    RotateCounterclockwise,
    /// Turn the view back to upright.
    #[command(label = "Reset Rotation")]
    ResetRotation,
}

impl ViewCommands {
    /// Every view command, in menu order.
    pub const ALL: [ViewCommands; 12] = [
        ViewCommands::ZoomIn,
        ViewCommands::ZoomOut,
        ViewCommands::FitOnScreen,
        ViewCommands::FillScreen,
        ViewCommands::ActualPixels,
        ViewCommands::TwoHundredPercent,
        ViewCommands::PrintSize,
        ViewCommands::FitArtboard,
        ViewCommands::FitLayers,
        ViewCommands::RotateClockwise,
        ViewCommands::RotateCounterclockwise,
        ViewCommands::ResetRotation,
    ];

    /// Applies the command to `view` for `target` at `pixels_per_point`. Returns whether the view changed: a command
    /// whose fact `target` lacks changes nothing.
    pub fn apply(self, view: &mut CanvasView, target: &ViewTarget, pixels_per_point: f32) -> bool {
        let before = *view;
        let size = target.document;
        let zoom_about_centre = |view: &mut CanvasView, device: f64| {
            zoom::about(view, view.rect.center(), ZoomLadder::clamp(device, size), false, pixels_per_point);
        };
        match self {
            ViewCommands::ZoomIn => {
                zoom::step(view, 1, size, pixels_per_point);
            }
            ViewCommands::ZoomOut => {
                zoom::step(view, -1, size, pixels_per_point);
            }
            // Photoshop fits the image to the view, with no margin.
            ViewCommands::FitOnScreen => zoom::fit(view, size, pixels_per_point, 0.0),
            ViewCommands::FillScreen => zoom::fill(view, size, pixels_per_point),
            ViewCommands::ActualPixels => zoom_about_centre(view, 1.0),
            ViewCommands::TwoHundredPercent => zoom_about_centre(view, 2.0),
            ViewCommands::PrintSize => {
                if let Some(device) = target.print_zoom() {
                    zoom_about_centre(view, device);
                }
            }
            ViewCommands::FitArtboard => {
                if let Some(board) = with_area(target.artboard) {
                    zoom::fit_rect(view, board, size, pixels_per_point, ARTBOARD_MARGIN);
                }
            }
            ViewCommands::FitLayers => {
                if let Some(bounds) = with_area(target.layers) {
                    zoom::fit_rect(view, bounds, size, pixels_per_point, LAYERS_MARGIN);
                }
            }
            ViewCommands::RotateClockwise => view.rotation = normalized_angle(view.rotation + ROTATE_STEP_DEG),
            ViewCommands::RotateCounterclockwise => view.rotation = normalized_angle(view.rotation - ROTATE_STEP_DEG),
            ViewCommands::ResetRotation => view.rotation = 0.0,
        }
        *view != before
    }

    /// Whether the command can run on `view` for `target`: Zoom In and Zoom Out stop at the ends of the zoom range,
    /// Print Size needs the document's resolution, Fit Artboard an artboard and Fit Layers the layers' bounds.
    pub fn available(self, view: &CanvasView, target: &ViewTarget, pixels_per_point: f32) -> bool {
        match self {
            ViewCommands::ZoomIn | ViewCommands::ZoomOut => self.apply(&mut view.clone(), target, pixels_per_point),
            ViewCommands::PrintSize => target.print_zoom().is_some(),
            ViewCommands::FitArtboard => with_area(target.artboard).is_some(),
            ViewCommands::FitLayers => with_area(target.layers).is_some(),
            _ => true,
        }
    }

    /// Whether the command changes the zoom or the centre, so it eases under Animated Zoom; the rotate commands only
    /// turn the view.
    pub fn zooms(self) -> bool {
        !matches!(self, ViewCommands::RotateClockwise | ViewCommands::RotateCounterclockwise | ViewCommands::ResetRotation)
    }
}

/// The Hand tool: a drag pans the view with the document under the pointer; a release while still moving glides on
/// with [`FlickPan`] when Flick Panning is on; Escape during the drag puts the view back. It does not autoscroll.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HandTool {
    /// The user's navigation choices (Flick Panning).
    pub options: NavigationOptions,
    /// The view and pointer when the drag began.
    start: Option<(CanvasView, Pos2)>,
    /// The pointer at the previous drag pass: a drag step is the flick's input.
    last: Pos2,
    flick: FlickPan,
}

impl HandTool {
    /// A Hand tool under `options`.
    pub fn new(options: NavigationOptions) -> Self {
        Self { options, start: None, last: Pos2::ZERO, flick: FlickPan::default() }
    }

    /// Ends a glide: the app's clamp of the view the tool set (Overscroll off) moved it, so the view hit a scroll
    /// limit.
    pub fn stop_glide(&mut self) {
        self.flick.stop();
    }

    /// Whether a flick glides on.
    pub(crate) fn is_gliding(&self) -> bool {
        self.flick.gliding()
    }

    /// The view after this pass's step of a glide at `now` from `view`.
    fn glide(&mut self, view: &CanvasView, now: f64) -> Option<CanvasView> {
        let step = self.flick.advance(now)?;
        let mut next = *view;
        pan(&mut next, step);
        Some(next)
    }
}

impl CanvasTool for HandTool {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        if self.start.is_some() || !self.flick.gliding() {
            return;
        }
        let Some(space) = cx.space() else { return };
        // The glide steps from the view it set, not the shown one, so steps under half a device pixel add up.
        if let Some(view) = self.glide(&space.target(), cx.time) {
            cx.set_view(view);
        }
        if self.flick.gliding() {
            cx.request_repaint();
        }
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.flick.stop();
        self.start = cx.space().map(|space| (space.view, pointer.pos));
        self.last = pointer.pos;
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some((view, from)) = self.start else { return };
        let delta = pointer.pos - self.last;
        self.last = pointer.pos;
        let mut next = view;
        pan(&mut next, pointer.pos - from);
        cx.set_view(next);
        self.flick.drag(pointer.time, delta);
        if delta != egui::Vec2::ZERO {
            cx.request_repaint();
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some((view, from)) = self.start.take() else { return };
        let mut next = view;
        pan(&mut next, pointer.pos - from);
        // Letting go while the pointer still moves starts the glide, whose first step is this pass's.
        self.flick.release(pointer.time, self.options.flick_panning);
        if let Some(view) = self.glide(&next, pointer.time) {
            next = view;
        }
        if self.flick.gliding() {
            cx.request_repaint();
        }
        cx.set_view(next);
    }

    fn cancel(&mut self, _reason: CancelReason, cx: &mut ToolContext<'_>) {
        if let Some((view, _)) = self.start.take() {
            cx.set_view(view);
        }
        self.flick.stop();
    }

    fn key(&mut self, _key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        self.flick.stop();
        false
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        PaintedCursor::Hand.into()
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the arrow keys and the page keys scroll the view")
    }
}

/// The Zoom tool: a click steps the ladder about the clicked point (Alt: out; Zoom Clicked Point to Center centres
/// it), a press held still zooms continuously ([`HoldZoom`], Animated Zoom only), a drag scrubs or draws a zoom
/// rectangle ([`ZoomDrag`], Scrubby Zoom). The cursor is the magnifier, with a minus while Alt is held or `out` is
/// set (Zoom Out held with Alt+Space). At a zoom limit the view stays put. The tool reads the document's size from
/// the canvas's [`CanvasSpace`](super::CanvasSpace) (the size given to [`Canvas::document`](super::Canvas::document));
/// on a canvas given no document it does nothing.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ZoomTool {
    /// Zoom out instead of in (Alt+Space held); Alt flips it.
    pub out: bool,
    /// The navigation choices the tool reads.
    pub options: NavigationOptions,
    hold: Option<HoldZoom>,
    drag: Option<ZoomDrag>,
    /// The view, time and screen point of the press in progress.
    press: Option<(CanvasView, f64, Pos2)>,
    /// The pointer at the last drag sample.
    last: Pos2,
    /// The host called `drag` in this press: the pointer moved past the arena's slop.
    dragged: bool,
}

impl ZoomTool {
    /// A Zoom tool under `options`.
    pub fn new(options: NavigationOptions) -> Self {
        Self { out: false, options, hold: None, drag: None, press: None, last: Pos2::ZERO, dragged: false }
    }

    /// One pass of a press in progress, with the pointer at `at`: the hold zooms once past its delay (and then the
    /// drag has nothing to do), else the drag scrubs or moves the rectangle's corner.
    fn advance(&mut self, at: Pos2, mods: Mods, time: f64, cx: &mut ToolContext<'_>) {
        let (Some(drag), Some(space)) = (self.drag.as_mut(), cx.space()) else { return };
        let Some(size) = space.document() else { return };
        let ppp = space.pixels_per_point;
        let out = self.out != mods.alt;
        let mut view = space.view;
        let before = view;
        let mut waiting = false;
        let held = match self.hold.as_mut() {
            Some(hold) => {
                let held = hold.advance(&mut view, time, out, size, ppp);
                // The press asks for passes while it waits out the delay and while it zooms.
                waiting = held || hold.waiting(time);
                held
            }
            None => false,
        };
        if !held {
            drag.drag(&mut view, at, size, ppp);
        }
        if waiting {
            cx.request_repaint();
        }
        if view != before {
            cx.set_view(view);
        }
    }
}

/// The view after a Zoom tool click at `screen`: one ladder rung in (or `out`) about the point, or centred on it with
/// Zoom Clicked Point to Center. `None` at a zoom limit.
fn click_step(view: &CanvasView, screen: Pos2, out: bool, size: Size, options: &NavigationOptions, pixels_per_point: f32) -> Option<CanvasView> {
    let current = zoom::device(view, pixels_per_point);
    let next = ZoomLadder::step(current, if out { -1 } else { 1 }, size);
    if !next.is_finite() || (next / current - 1.0).abs() <= 1e-9 {
        return None;
    }
    let mut stepped = *view;
    zoom::about(&mut stepped, screen, next, options.zoom_clicked_point_to_center, pixels_per_point);
    Some(stepped)
}

impl CanvasTool for ZoomTool {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        // The hold advances without pointer movement, so a press held perfectly still keeps zooming.
        if self.press.is_some() {
            let (at, mods, time) = (self.last, cx.mods, cx.time);
            self.advance(at, mods, time, cx);
        }
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space().filter(|space| space.document().is_some()) else {
            return ToolAction::Claimed;
        };
        let view = space.view;
        self.press = Some((view, pointer.time, pointer.pos));
        self.last = pointer.pos;
        self.dragged = false;
        self.hold = self.options.animated_zoom.then(|| HoldZoom::press(pointer.pos, view.to_doc(pointer.pos), pointer.time));
        self.drag = Some(ZoomDrag::begin(&view, pointer.pos, self.options.scrubby_zoom, space.pixels_per_point));
        if self.hold.is_some() {
            cx.request_repaint();
        }
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.last = pointer.pos;
        self.dragged = true;
        // A hold that has not started zooming is a drag now.
        if self.hold.as_ref().is_some_and(|hold| !hold.zooming()) {
            self.hold = None;
        }
        self.advance(pointer.pos, pointer.mods, pointer.time, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (Some((start, _, anchor)), Some(drag), Some(space)) = (self.press.take(), self.drag.take(), cx.space()) else { return };
        let hold = self.hold.take();
        let Some(size) = space.document() else { return };
        let ppp = space.pixels_per_point;
        let out = self.out != pointer.mods.alt;
        // A hold that zoomed has already changed the view; the release is no click.
        if hold.is_some_and(|hold| hold.zooming()) {
            return;
        }
        if !self.dragged {
            // A click steps from the view the press found.
            match click_step(&start, anchor, out, size, &self.options, ppp) {
                Some(next) if self.options.animated_zoom => {
                    let anchored = (!self.options.zoom_clicked_point_to_center).then(|| (start.to_doc(anchor), anchor));
                    cx.ease_view(next, anchored);
                }
                Some(next) => cx.set_view(next),
                None => cx.set_view(start),
            }
            return;
        }
        let mut view = space.view;
        let mut drag = drag;
        drag.drag(&mut view, pointer.pos, size, ppp);
        drag.release(&mut view, out, size, ppp, cx.slop);
        cx.set_view(view);
    }

    fn cancel(&mut self, _reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.hold = None;
        self.drag = None;
        if let Some((view, _, _)) = self.press.take() {
            cx.set_view(view);
        }
    }

    /// The zoom rectangle while drawn.
    fn paint(&self, _draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let Some(rect) = self.drag.as_ref().and_then(ZoomDrag::rect) else { return };
        let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        cx.marching_ants(&corners, true);
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        PaintedCursor::zoom(self.out != cx.mods.alt).into()
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Zoom In and Zoom Out commands; the zoom rectangle has no numeric form")
    }
}

/// The Rotate View tool: a drag turns the view about the canvas centre ([`ViewTurn`]); a double click or Escape (with
/// no drag running) resets the angle to 0.
#[derive(Clone, Debug, Default)]
pub struct RotateViewTool {
    drag: Option<(ViewTurn, CanvasView)>,
}

impl CanvasTool for RotateViewTool {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.drag = cx.space().map(|space| (ViewTurn::begin(&space.view, pointer.pos), space.view));
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some((turn, start)) = &self.drag else { return };
        let mut view = *start;
        view.rotation = turn.drag(pointer.pos, pointer.mods.shift, start.flip_x);
        cx.set_view(view);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some((turn, start)) = self.drag.take() else { return };
        let mut view = start;
        view.rotation = if pointer.clicks >= 2 { 0.0 } else { turn.drag(pointer.pos, pointer.mods.shift, start.flip_x) };
        cx.set_view(view);
    }

    /// Escape puts the angle back to 0 (not during a drag: that cancels it).
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let Some(space) = cx.space() else { return false };
        if !matches!(key, ToolKey::Cancel) || self.drag.is_some() || space.view.rotation == 0.0 {
            return false;
        }
        let mut view = space.view;
        view.rotation = 0.0;
        cx.set_view(view);
        true
    }

    fn cancel(&mut self, _reason: CancelReason, cx: &mut ToolContext<'_>) {
        if let Some((_, view)) = self.drag.take() {
            cx.set_view(view);
        }
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        PaintedCursor::Hand.into()
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Rotate View commands")
    }
}
