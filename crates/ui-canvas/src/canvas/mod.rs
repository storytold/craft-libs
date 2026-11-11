//! The `Canvas` container with its tool host, pasteboard, split views, rulers, navigator, cursors and raster display.
//!
//! `Canvas` is an arena root: the leaf components its body shows with `show_at(canvas_rect)` (canvas controls,
//! such as `CropOverlay`) join its arena, and a press away from all of their parts reaches the active tool through
//! the canvas's background part. A canvas given a view ([`Canvas::view`]) owns what happens to it in a pass: it
//! sets the view's rect to its own, fits a view never shown, folds in the wheel, a middle-button pan and the view
//! commands routed to it ([`ViewCommands`]), keeps the document in view with Overscroll off, eases a zoom under
//! Animated Zoom, publishes the view it shows as a [`CanvasSpace`] to its body, and reports the view the app copies.

mod crop;
mod frame_step;
// The full-window dialog family; `dialog` re-exports it.
pub(crate) mod full_window;
pub(crate) mod full_window_parts;
mod march;
mod readout;
mod space;
#[cfg(feature = "testing")]
mod stories;
#[cfg(feature = "testing")]
pub(crate) mod testing;
mod words;

mod tool;
mod tool_groups;
mod tool_host;

pub use crate::canvas_data::{Guide, GuideColor, GuideDrag, GuideKey, GuideLine, NavigatorOptions, NavigatorSettings, ProxyColor};
pub use crop::{CropOverlay, CropOverlayOutput, CropShield, CropStyle, ShieldColor};
pub use frame_step::{FRAME_STEP_MAX, frame_step};
pub(crate) use march::{MARCH_SECONDS, MARCH_TICK, march_shift};
pub use readout::readout;
pub use space::{CanvasSpace, RulerText, canvas_space};
pub use words::CanvasWords;

#[doc(hidden)]
pub use tool::tool_probe;
pub use tool::{CanvasTool, NoTool, ToolAccess, ToolAction, ToolAnchor, ToolClaims, ToolContext, ToolKey, ToolPointer, ViewEase};
pub use tool_groups::{PickStep, tool_pick};
pub use tool_host::{ToolHost, ToolHostResponse, ToolInput, ToolReadout};

mod raster_source;
pub use raster_source::{PixelRect, RasterSource};

mod pasteboard;
mod split;
pub use pasteboard::{PageBorder, PageFrame, Pasteboard, PasteboardColor, PasteboardOutput, PasteboardStyle};
pub use split::{SplitAt, SplitLayout, SplitPane, SplitStyle, SplitViews, SplitViewsOutput};
pub use words::SplitWords;

mod raster;
pub use raster::{DetailPlan, RasterDisplay, RasterDisplayOutput, RasterStyle, snap_to_texels};

// l4d-space
mod snap;
pub(crate) use rulers::LABEL_STEP_MIN;
pub use rulers::{NUDGE, NUDGE_FAR};
pub use snap::{Snap, canvas_snap};
pub(crate) use space::context_menu;
pub use space::{BackgroundPress, CanvasRulers, context_click};
// End of l4d-space.

// canvas-navigate-a
mod momentary;
mod navigate;
mod rotate;
mod tools;
mod zoom_motion;

pub use momentary::{Momentary, MomentaryCommands, MomentaryTool};
pub use navigate::{Autoscroll, FlickPan, NavigationOptions, PageKey, TouchpadGlide, WheelAction, WheelInput, WheelNavigation, pan};
pub use rotate::{RotateView, RotateViewOutput, RotateViewStyle, ViewTurn};
pub use tools::{HandTool, RotateViewTool, ViewCommands, ZoomTool};
pub use zoom_motion::{HoldZoom, ZoomAnimation, ZoomDrag};

mod rulers;
mod scrollbars;
pub use rulers::{INCH_EIGHTHS, INCH_SIXTEENTHS, RulerCommands, RulerMargins, RulerPart, RulerTick, Rulers, RulersOutput, StepLadder, TickRule};
pub use scrollbars::{CanvasScrollbars, CanvasScrollbarsOutput};

// canvas-navigator-a
mod navigator;
pub use navigator::{
    Navigator, NavigatorCommands, NavigatorOutput, NavigatorPaint, NavigatorPreview, NavigatorZoomRow, NavigatorZoomRowOutput, preview_rect,
    proxy_quad, view_center_for, zoom_to_document_rect,
};
// End of canvas-navigator-a.
// l4e-navprev
pub use navigator::NavigatorStyle;
// End of l4e-navprev.
// The Navigator panel pattern; `patterns::canvas` re-exports it.
#[doc(hidden)]
pub mod navigator_panel;

use crate::contract::{
    Bind, Common, Component, Container, ContainerProps, Cx, Focus as _, NoValue, Outcome, PartsCx, Pointer as _, Props as _, Role, Sizing,
};
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::runtime::motion::{Animate as _, spec};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{
    CanvasTransform, CanvasView, EditEvent, HitLevel, HitPriority, Metrics, Mixed, PartKey, PointerButton, PressPhase, PressState, Sense,
};
use egui::{Pos2, Rect, Response, Vec2};
use zoom_motion::{Steer, ViewMotion};

const BACKGROUND: PartKey = PartKey::new("background");

/// The view commands a canvas publishes the state of.
const VIEW_COMMANDS: [ViewCommands; 8] = [
    ViewCommands::ZoomIn,
    ViewCommands::ZoomOut,
    ViewCommands::FitOnScreen,
    ViewCommands::FillScreen,
    ViewCommands::ActualPixels,
    ViewCommands::RotateClockwise,
    ViewCommands::RotateCounterclockwise,
    ViewCommands::ResetRotation,
];

/// Why the background part is pointer-only: the keyboard path of the running tool is its command form
/// ([`CanvasTool::command_form`]), the one owner of that fact.
const TOOL_PATH: &str = "the running tool's command form";

/// How a tool is reached without a pointer (8.3, owner question 15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolPath {
    /// The tool creates or selects geometry: a command form takes the same numbers. The text names the command
    /// ("the Crop To... command").
    Command(&'static str),
    /// The tool is freehand (brush, pencil, eraser, smudge, clone): pointer only. The text is the reason: it names
    /// the command that does the same job, or says there is none.
    Freehand(&'static str),
}

/// The style of a canvas: its surface comes from `ContainerProps`.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CanvasStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// How far the rulers' strips push the view's rect in from the canvas's top-left corner, while they show.
    rulers: Vec2,
}

/// A canvas: a pasteboard the caller's canvas controls and the active tool share.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Canvas<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// The view the canvas shows its document through, as a binding: the canvas writes the view of its pass through
    /// it (`Bind::bound` holds it in the app's variable, `Bind::kept` in the canvas's own memory). A given view
    /// (`Bind::given`, or a plain [`CanvasView`]) is only read: the app copies what a tool changed from
    /// [`ToolHostResponse::view`] (or [`CanvasOutput::view`] without a tool host) every pass.
    ///
    /// The canvas sets the view's rect to its own rect, inside the rulers while they show ([`show_rulers`](Self::show_rulers)),
    /// so the body and the reported view carry this pass's rect. A view whose rect has no area (`CanvasView::new(Rect::NOTHING, ..)`,
    /// a document never shown) opens fitted: Fit on Screen in the first pass the canvas has area and a document. A
    /// canvas with no area (a minimized window) leaves the view as it is, so a restore does not refit.
    ///
    /// The body reads the view as a [`CanvasSpace`] with [`canvas_space`]: the view on screen, part way through a
    /// zoom that eases. Its controls register parts in the canvas root's screen space through
    /// `CanvasView::to_screen`. The view sets no body transform: a body of components that keep canvas-space geometry
    /// (a node graph) also sets `transform(view.layer_transform())`, which exists for an unrotated, unmirrored,
    /// square view only.
    #[prop(into)]
    view: Option<Bind<'a, CanvasView>>,
    /// The view the binding held when the pass began.
    given: Option<CanvasView>,
    /// The document the view shows, in document units. With a view and a document, the canvas fits a view never
    /// shown, keeps the wheel's zoom inside the document's zoom range, keeps the document in view with Overscroll off,
    /// and applies the view commands routed to it ([`ViewCommands`]); without either, the view commands are disabled
    /// for it.
    #[prop(quantity = "document units")]
    document: Option<Size>,
    /// The user's navigation options: how the wheel and a pinch move the view ([`WheelNavigation`]), whether a zoom
    /// eases (Animated Zoom) and whether the view may leave the document (Overscroll).
    #[prop]
    navigation: NavigationOptions,
    /// Whether the rulers show along the top and left edges (View > Rulers): the view lies inside them.
    #[prop]
    show_rulers: bool,
    /// The rulers' units and zero point, published to the body in its [`CanvasSpace`].
    rulers: Option<CanvasRulers>,
    /// The snapping of the coordinates the pointer sets on this canvas (View > Snap To): one function that the rulers
    /// snap their zero point with, the guides snap a guide with and the tool host hands to the tool
    /// ([`ToolContext::snap`]). Without it nothing snaps.
    #[prop]
    snap: Option<Snap>,
    /// The view of this pass, after the canvas folded in its input.
    settled: Option<Settled>,
}

/// A canvas's view in one pass.
#[derive(Clone, Copy, Debug)]
struct Settled {
    /// The view the app copies.
    target: CanvasView,
    /// The view on screen: the target, or a view part way to it while a zoom eases.
    shown: CanvasView,
}

impl<'a> Canvas<'a> {
    /// A canvas named `label`. The keyboard path of its tool is the tool's own
    /// ([`CanvasTool::command_form`](CanvasTool::command_form)).
    pub fn new(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new().surface(SurfaceKind::Canvas).arena(true).sizing(Sizing::Fill).command_scope(ViewCommands::SCOPE);
        Canvas {
            common: Common::new(),
            props,
            view: None,
            given: None,
            document: None,
            navigation: NavigationOptions::default(),
            show_rulers: false,
            rulers: None,
            snap: None,
            settled: None,
        }
        .label(label)
        .ltr_axis()
    }

    /// Leaves the view commands to the instance around the canvas that owns its view (a full-window dialog's): the
    /// canvas is no instance of them, so the chords reach that instance while the canvas has the focus.
    #[must_use]
    pub(crate) fn outer_view_commands(mut self) -> Self {
        self.props.command_scope = None;
        self
    }

    /// Publishes the view commands' state for this canvas and applies the ones routed to it, in order, to `view`:
    /// the view they ask for, and whether they zoom.
    fn view_commands(&self, cx: &mut Cx<'_, Self>, view: Option<CanvasView>) -> Option<(CanvasView, bool)> {
        self.props.command_scope?;
        let pixels_per_point = cx.env.scale().0;
        let target = view.zip(self.document);
        for command in VIEW_COMMANDS {
            // Zoom In and Zoom Out are disabled at the ends of the zoom range.
            let enabled = target.is_some_and(|(mut view, document)| match command {
                ViewCommands::ZoomIn | ViewCommands::ZoomOut => command.apply(&mut view, document, pixels_per_point),
                _ => true,
            });
            cx.publish(command, CommandState::new().enabled(enabled));
        }
        let (view, document) = target?;
        let (mut next, mut zoomed) = (view, false);
        while let Some((command, _)) = cx.take_command::<ViewCommands>() {
            command.apply(&mut next, document, pixels_per_point);
            zoomed |= !matches!(command, ViewCommands::RotateClockwise | ViewCommands::RotateCounterclockwise | ViewCommands::ResetRotation);
        }
        (next != view).then_some((next, zoomed))
    }

    /// The rect the view lies in: the canvas's own, inside the rulers while they show.
    fn view_rect(&self, rect: Rect, style: &CanvasStyle) -> Rect {
        if self.show_rulers { Rect::from_min_max(rect.min + style.rulers, rect.max) } else { rect }
    }

    /// The view of this pass from the app's `given` view: this pass's rect, the first fit, the wheel, a middle-button
    /// pan, the view commands, the Overscroll clamp, and the zoom ease.
    fn settle(&self, cx: &mut Cx<'_, Self>, given: CanvasView) -> Settled {
        let ctx = cx.core.ctx.clone();
        let id = cx.core.id;
        let mut motion = ViewMotion::take(&ctx, id);
        let pixels_per_point = cx.env.scale().0;
        let size = self.document.unwrap_or(Size::ZERO);
        let mut view = given;
        let area = self.view_rect(cx.content_rect, cx.style);
        if area.width() > 0.0 && area.height() > 0.0 {
            let never_shown = !(view.rect.width() > 0.0 && view.rect.height() > 0.0);
            view.rect = area;
            if never_shown && let Some(document) = self.document {
                ViewCommands::FitOnScreen.apply(&mut view, document, pixels_per_point);
            }
        }
        let mut jumped = false;
        if let Some(wheel) = cx.take_wheel()
            && let Some(action) = motion.wheel.read(&wheel, &self.navigation)
        {
            let at = cx.response.hover_pos().unwrap_or_else(|| view.rect.center());
            jumped |= WheelNavigation::apply(&mut view, action, at, size, pixels_per_point);
        }
        if let Some(delta) = Self::background_press(cx).1 {
            pan(&mut view, delta);
            jumped = true;
        }
        // Page Up and Down, Home and End scroll the view as in Photoshop; the arrows are the tool's nudges.
        while let Some((step, mods)) = cx.intents().take_move() {
            if let (Some(document), Some(key)) = (self.document, PageKey::of(step, mods, area.size())) {
                jumped |= key.apply(&mut view, document);
            }
        }
        // A pan or a scroll takes over from an ease, here or in a tool.
        let mut steer = motion.steer.take();
        if jumped {
            steer = Some(Steer::Stop);
        }
        let before = view;
        if let Some((next, zoomed)) = self.view_commands(cx, Some(view)) {
            view = next;
            if zoomed && self.navigation.animated_zoom {
                // A zoom that keeps the centre eases about it; one that moves the centre (Fit on Screen) eases freely.
                let anchor = (next.center == before.center).then_some((before.center, before.rect.center()));
                steer = Some(Steer::Ease { from: before, anchor });
            }
        }
        let target = clamped(view, self.document, &self.navigation);
        let mut progress = cx.anim("zoom", motion.animation.target(), spec::MOVE);
        match steer {
            Some(Steer::Ease { mut from, anchor }) => {
                from.rect = target.rect;
                motion.animation.begin(&from, anchor, progress);
                progress = cx.anim("zoom", motion.animation.target(), spec::MOVE);
            }
            Some(Steer::Stop) => motion.animation.stop(),
            None => {}
        }
        let shown = motion.animation.shown(&target, progress);
        motion.restore(&ctx, id);
        Settled { target, shown }
    }

    /// Measures the document's axes in `x` and `y`, counted from `zero` (document units). The rulers, the guides and
    /// the measures of the body read the units and the zero from the canvas's [`CanvasSpace`], so the app states them
    /// once. A canvas that publishes none measures in pixels from the document's origin. Whether the strips show, and
    /// so where the view lies, is [`show_rulers`](Self::show_rulers).
    #[must_use]
    pub fn rulers(mut self, x: RulerText, y: RulerText, zero: Point) -> Self {
        self.rulers = Some(CanvasRulers::new(x, y, zero));
        self
    }

    fn space(&self, pixels_per_point: f32, background: BackgroundPress) -> Option<CanvasSpace> {
        let view = self.settled.map(|settled| settled.shown).or(self.given);
        view.map(|view| CanvasSpace::new(view, pixels_per_point).with_document(self.document).with_rulers(self.rulers).with_background(background))
    }

    /// The press on the background part as the canvas reports it, and the pan of a middle-button drag: a middle press
    /// pans the view and is never a tool press.
    fn background_press(cx: &Cx<'_, Self>) -> (PressState, Option<Vec2>) {
        let press = cx.part(BACKGROUND).press;
        let middle = press.phase.start().is_some_and(|start| start.button == PointerButton::Middle);
        match press.phase {
            PressPhase::DragStart { start, pos, .. } if middle => (PressState::IDLE, Some(pos - start.origin)),
            PressPhase::Drag { delta, .. } if middle => (PressState::IDLE, Some(delta)),
            _ if middle => (PressState::IDLE, None),
            _ => (press, None),
        }
    }

    /// Shows the body zoomed and moved by `transform`, so controls and members keep canvas-space geometry.
    #[must_use]
    pub fn transform(mut self, transform: CanvasTransform) -> Self {
        self.props = self.props.transform(transform);
        self
    }
}

/// What a canvas reports: its response and the press the tool gets.
#[derive(Debug)]
#[non_exhaustive]
pub struct CanvasOutput {
    /// The canvas's response.
    pub response: Response,
    /// The press on the background part: a press no canvas control claimed. A middle-button press pans the view and
    /// is never a tool press, so this is idle while one runs.
    pub press: PressState,
    /// The view on screen and the interface scale, for a canvas given a view. [`ToolHost::run`] hands it to the tool.
    pub space: Option<CanvasSpace>,
    /// The canvas's view of this pass, when it differs from the view the app gave: this pass's rect, the first fit,
    /// the wheel, a middle-button pan, the view commands and the Overscroll clamp. A zoom that eases reports its
    /// target; the canvas shows the way there itself. [`ToolHost::run`] carries it in [`ToolHostResponse::view`]
    /// when no tool set a view, so an app with a tool host copies that one view.
    pub view: Option<CanvasView>,
    pub(crate) snap: Option<Snap>,
    pub(crate) document: Option<Size>,
    pub(crate) navigation: NavigationOptions,
}

impl CanvasOutput {
    /// `view` as the canvas keeps it: inside the document's edges with Overscroll off. The tool host passes a tool's
    /// view through it.
    pub(crate) fn clamped(&self, view: CanvasView) -> CanvasView {
        clamped(view, self.document, &self.navigation)
    }

    /// Where a press on the background was released without a drag. A tool that creates geometry opens its command
    /// form at this point.
    pub fn click_at(&self) -> Option<Pos2> {
        match self.press.phase {
            PressPhase::Release { pos, click: true, .. } => Some(pos),
            _ => None,
        }
    }
}

impl Outcome for CanvasOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl Component for Canvas<'_> {
    const ROLE: Role = Role::CANVAS;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves();
    type Style = CanvasStyle;
    type Output = CanvasOutput;

    fn style(cx: &StyleCx<'_>, _: CanvasStyleKey) -> CanvasStyle {
        // The rulers' own layout says where the view lies inside them.
        CanvasStyle { rulers: Rulers::content_rect(cx, Rect::ZERO).min.to_vec2(), ..CanvasStyle::default() }
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL
    }

    /// The view of this pass, kept for the body and `finish`.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // The canvas is no value component: it reads and writes its view's binding itself, as a container does.
        self.given = self.view.as_mut().and_then(|view| view.reborrow().read(cx)).and_then(|held| held.same().copied());
        self.settled = if self.view.is_some() {
            // A view never shown has no rect: the canvas fits it in the first pass it has area and a document.
            let given = self.given.unwrap_or_else(|| CanvasView::new(Rect::NOTHING, Point::ZERO, 1.0));
            let settled = self.settle(cx, given);
            if Some(settled.target) != self.given
                && let Some(view) = self.view.as_mut()
            {
                let event = EditEvent::set(Mixed::Same(given), settled.target, cx.label_arc(), None);
                let _ = view.reborrow().write(cx, &event);
            }
            Some(settled)
        } else {
            self.view_commands(cx, None);
            None
        };
    }

    /// The canvas's [`CanvasSpace`] for its body; a canvas without a view hides an outer canvas's.
    fn content_env(&self, cx: &Cx<'_, Self>, values: &mut crate::runtime::env::AppValues) {
        space::publish(values, self.space(cx.env.scale().0, BackgroundPress::of(&Self::background_press(cx).0)));
        snap::publish(values, self.snap.clone());
    }

    /// The background part the active tool gets. It sits in the canvas root's space, the parent layer's. A press on it
    /// focuses the canvas, so the keys the tool reads reach it.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(BACKGROUND, rect)
            .priority(HitPriority::new(HitLevel::Background, 0))
            .sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS)
            .pointer_only(TOOL_PATH);
    }

    fn finish(self, cx: Cx<'_, Self>) -> CanvasOutput {
        let press = Self::background_press(&cx).0;
        let space = self.space(cx.env.scale().0, BackgroundPress::of(&press));
        let view = self.settled.map(|settled| settled.target).filter(|target| Some(*target) != self.given);
        CanvasOutput { response: cx.response.clone(), press, space, view, snap: self.snap, document: self.document, navigation: self.navigation }
    }
}

impl Container for Canvas<'_> {}

/// `view` inside the document's edges with Overscroll off: the one clamp of a canvas's view, the canvas's own and a
/// tool's alike. A view with no rect yet, or a canvas without a document, is left as it is.
fn clamped(mut view: CanvasView, document: Option<Size>, navigation: &NavigationOptions) -> CanvasView {
    if let Some(document) = document
        && !navigation.overscroll
        && view.rect.width() > 0.0
        && view.rect.height() > 0.0
    {
        CanvasScrollbars::clamp(&mut view, DocRect::from_origin_size(Point::ZERO, document));
    }
    view
}
