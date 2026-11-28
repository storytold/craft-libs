//! The `Canvas` container with its tool host, pasteboard, split views, rulers, navigator, cursors and raster display.
//!
//! `Canvas` is an arena root: the leaf components its body shows with `show_at(canvas_rect)` (canvas controls,
//! such as `CropOverlay`) join its arena, and a press away from all of their parts reaches the active tool through
//! the canvas's background part. A canvas given a view ([`Canvas::view`]) owns what happens to it in a pass: it
//! sets the view's rect to its own, fits a view never shown, folds in the wheel (eased under the eased wheel, gliding
//! on after a touchpad lift), a middle-button pan or latched scroll and the view commands routed to it
//! ([`ViewCommands`]), keeps the document in view with Overscroll off, eases a zoom under Animated Zoom, publishes the
//! view it shows as a [`CanvasSpace`] to its body, and reports the view the app copies.

pub(crate) mod crop;
// The full-window dialog family; `dialog` re-exports it.
pub(crate) mod full_window;
pub(crate) mod full_window_parts;
mod march;
mod readout;
mod space;
#[cfg(feature = "suite")]
pub(crate) mod stories;
#[cfg(feature = "testing")]
pub(crate) mod testing;
mod words;

mod tool;
mod tool_groups;
mod tool_host;

pub use crate::canvas_data::{Guide, GuideColor, GuideDrag, GuideKey, GuideLine, NavigatorOptions, NavigatorSettings, ProxyColor};
pub use craft_behavior::{FRAME_STEP_MAX, frame_step};
pub use crop::{CropApply, CropOutside, CropOverlay, CropOverlayOutput, CropScope, CropShield, CropStyle, ShieldColor, paint_outside};
pub(crate) use march::{MARCH_SECONDS, MARCH_TICK, march_shift};
pub use readout::{ShowReadouts, hud, readout};
pub use space::{CanvasSpace, RulerText, canvas_space, pixel_aligned, ruler_guide, with_canvas_space};
pub use words::CanvasWords;

#[doc(hidden)]
pub use tool::tool_probe;
pub use tool::{CanvasTool, NoTool, ToolAccess, ToolAction, ToolAnchor, ToolClaims, ToolContext, ToolKey, ToolPointer, ToolRegion, ViewEase};
pub use tool_groups::{PickStep, tool_pick};
pub use tool_host::{ContextRequest, ContextSource, StandInKey, ToolHost, ToolHostResponse, ToolInput, ToolReadout, ViewPan, tool_pass};

mod raster_source;
pub use raster_source::{PixelRect, RasterSource};

pub(crate) mod pasteboard;
pub(crate) mod split;
pub use pasteboard::{ArtboardEdge, PageBorder, PageFill, PageFrame, Pasteboard, PasteboardColor, PasteboardOutput, PasteboardStyle};
pub use split::{PanePress, SplitAt, SplitLayout, SplitPane, SplitPaneAt, SplitStyle, SplitViews, SplitViewsOutput, TabDrop, TabDropTarget};
pub use words::SplitWords;

mod app_paint;
pub(crate) mod raster;
pub use app_paint::AppPaint;
pub use raster::{DetailPlan, Extent, RasterDisplay, RasterDisplayOutput, RasterStyle, snap_to_texels};

// l4d-space
mod opacity_keys;
mod snap;
pub use opacity_keys::OpacityDigit;
pub use rulers::{NUDGE, NUDGE_FAR};
pub use snap::{GestureSnap, Snap, canvas_snap, snap_suspended, snap_tolerance};
pub(crate) use space::context_menu;
pub use space::{BackgroundPress, BodyKeys, CanvasRulers, context_click};
// End of l4d-space.

// canvas-navigate-a
mod momentary;
pub(crate) mod navigate;
pub(crate) mod rotate;
mod tools;
mod zoom_motion;

pub use momentary::{Momentary, MomentaryCommands, MomentaryShift, MomentaryTool};
pub use navigate::{
    Autoscroll, FlickPan, MiddleAutoscrollFeel, MiddleButton, NavigateBar, NavigateBarOutput, NavigationOptions, PageKey, WheelAction, WheelInput,
    WheelNavigation, WheelScheme, pan,
};
pub use rotate::{RotateView, RotateViewOutput, RotateViewStyle, ViewTurn};
pub use tools::{HandTool, LayerBounds, RotateViewTool, ViewCommands, ViewRequest, ViewTarget, ZoomTool};
pub use zoom_motion::{HoldZoom, ZoomAnimation, ZoomDrag, easing_view};

pub(crate) mod rulers;
pub(crate) mod scrollbars;
pub use rulers::{
    CENTIMETRE_LABEL_STEP_MIN, INCH_EIGHTHS, INCH_SIXTEENTHS, LABEL_STEP_MIN, RulerCommands, RulerMargins, RulerPart, RulerTick, Rulers,
    RulersOutput, StepLadder, TickRule,
};
pub use scrollbars::{CanvasScrollbars, CanvasScrollbarsOutput};

// canvas-navigator-a
pub(crate) mod navigator;
pub use navigator::{
    Navigator, NavigatorCommands, NavigatorOutput, NavigatorPreview, NavigatorZoomRow, NavigatorZoomRowOutput, preview_rect, proxy_quad,
    view_center_for, zoom_to_document_rect,
};
// End of canvas-navigator-a.
// l4e-navprev
pub use navigator::NavigatorStyle;
// End of l4e-navprev.
// The Navigator panel pattern; `patterns::canvas` re-exports it.
#[doc(hidden)]
pub mod navigator_panel;

use crate::contract::{
    Bind, Common, Component, Container, ContainerProps, Cx, EditorHandle, Focus as _, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _,
    Role, Sizing,
};
use crate::primitives::{EditorPurpose, ImeStamp};
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::{EditSet, KeyUse, NudgeMods};
use crate::runtime::frame;
use crate::runtime::lock_keys;
use crate::runtime::measure;
use crate::runtime::motion::{Animate as _, spec};
use crate::style::{ComponentStyle, Stateful, StyleColor, StyleCx, SurfaceKind, SurfaceLook};
use craft_behavior::geometry::crop::normalized_angle;
use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{
    CanvasTransform, CanvasView, CursorIcon, CursorPriority, EditEvent, EditorInput, HitLevel, HitPriority, HitShape, Input, Intent, Metrics, Mixed,
    MultiTap, PartKey, PointerButton, PressPhase, PressState, Sense, Wheel, WheelNotch, WheelSource, WheelUnit, ZoomScheme,
};
use egui::{Context, Pos2, Rect, Response, Vec2};
use navigate::MiddleAutoscroll;
use std::sync::Arc;
use zoom_motion::{Steer, ViewMotion};

pub(crate) const BACKGROUND: PartKey = PartKey::new("background");

/// The editor part a canvas with [`Canvas::editor`] registers.
const EDITOR: PartKey = PartKey::new("editor");
/// The key the canvas shows its rulers under.
const RULERS: &str = "rulers";

/// The areas the active tool claims over the canvas controls ([`Canvas::tool_regions`]): `TOOL_REGION.at(i)` is the
/// region at `i`.
const TOOL_REGION: PartKey = PartKey::new("toolRegion");

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

/// The style of a canvas.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CanvasStyle {
    /// The canvas surface. It paints in the paint step, before the canvas reserves its picture slot, so the picture
    /// lies on the surface and under every canvas control of the body.
    #[style(surface)]
    surface: Option<SurfaceLook>,
    #[style(metrics)]
    metrics: Metrics,
    /// How far the rulers' strips push the view's rect in from the canvas's top-left corner, while they show.
    rulers: Vec2,
    /// The middle-button autoscroll mark's fill and ink: the overlay handle colours, so it reads on any page.
    mark: Option<(StyleColor, StyleColor)>,
}

/// A canvas: a pasteboard the caller's canvas controls and the active tool share.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Canvas<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// The view the canvas shows its document through ([`view`](Self::view)).
    view: Option<Bind<'a, CanvasView>>,
    /// The view the binding held when the pass began.
    given: Option<CanvasView>,
    /// The document the view shows: its size in document units (a `Size` is enough), and the artboard and resolution
    /// some view commands need ([`ViewTarget`]). With a view and a document, the canvas fits a view
    /// never shown, keeps the wheel's zoom inside the document's zoom range, keeps the document in view with Overscroll
    /// off, and applies the view commands routed to it ([`ViewCommands`]); without either, the view commands are
    /// disabled for it, and so is each command whose fact the target lacks.
    #[prop(into)]
    document: Option<ViewTarget>,
    /// The bounds Fit Layers on Screen shows, asked only in the pass that runs the command. Without it Fit Layers is
    /// disabled for this canvas.
    #[prop]
    layer_bounds: Option<LayerBounds<'a>>,
    /// View commands the app asks of this canvas's view (a menu row or an action step it runs, a new document's fit):
    /// the canvas runs them in this pass, in order and before the ones fired to it, as it runs those. A
    /// [`ViewRequest::Command`] eases as a fired command does; a [`ViewRequest::Place`] that moves the view makes the
    /// pass's view jump. A measuring run runs none. The app asks for them until a pass that is not a measuring run
    /// showed the canvas, then drops them.
    #[prop]
    queued: &'a [ViewRequest],
    /// The user's navigation options: how the wheel and a pinch move the view ([`WheelNavigation`]), whether a zoom
    /// eases (Animated Zoom) and whether the view may leave the document (Overscroll).
    #[prop]
    navigation: NavigationOptions,
    /// The rulers along the top and left edges (View > Rulers), which the canvas shows itself: the view lies inside
    /// them ([`with_rulers`](Self::with_rulers)).
    strips: Option<Rulers<'a>>,
    /// Whether the strips show; the rulers' own output of this pass is [`CanvasOutput::rulers`].
    strips_shown: bool,
    /// What the rulers reported in this pass, from the paint step.
    ruler_output: Option<RulersOutput>,
    /// Whether the canvas leaves Page Up, Page Down, Home, End and the arrows to its body: it applies no page keys,
    /// which its body reads in [`CanvasSpace::keys`], and the arrows reach the body's tool host and session controls
    /// first, then the body with [`CanvasSpace::take_nudge`], in the same pass. For a body that pages its document
    /// itself (a page view).
    #[prop]
    keys_to_body: bool,
    /// The arrow modifiers the body acts on besides the plain and Shift steps every tool and control reads
    /// (`NudgeMods::STEPS`): with nothing in the window focused, those arrows are the canvas's alone, and an arrow with
    /// modifiers neither lists stays the app's shortcut (`KeyUse::nudge_mods`). A page view lists the Cmd and Ctrl
    /// arrows that turn its pages.
    #[prop]
    body_nudges: NudgeMods,
    /// The canvas registers one editor part of this purpose: the part's widget is the component's focus holder and the
    /// input-method owner, and keys arrive as editor input ([`ToolPass::editor_input`]); [`keys_to_body`](Self::keys_to_body)
    /// does not apply. The body reports its editor state through [`ToolPass::editor`].
    #[prop]
    editor: Option<EditorPurpose>,
    /// Edit commands the canvas's editor part cannot act on right now (for instance Cut and Copy with nothing
    /// selected, or Undo and Redo with no editor-local history). Applied to [`KeyUse::inactive`] while
    /// [`editor`](Self::editor) is set: Cut, Copy, Paste and Select All stay the editor's and resolve disabled; Undo
    /// and Redo fall through to the app command's state. Ignored without [`editor`](Self::editor).
    #[prop]
    inactive_edits: EditSet,
    /// While the editor part has the keys, chords with the platform's command modifier and function keys still run the
    /// app's commands, whatever its typing rule ([`KeyUse::command_keys`]): a type tool whose app runs its commands
    /// while typing. The editor keeps its text editing chords. Ignored without [`editor`](Self::editor).
    #[prop]
    editor_command_keys: bool,
    /// The editor part's inputs of this pass, taken once in `update`: `tool_pass` is built from a shared `Cx`.
    editor_input: Arc<[EditorInput]>,
    /// The rulers' units and zero point, published to the body in its [`CanvasSpace`].
    rulers: Option<CanvasRulers>,
    /// The zero point of the rulers given to `with_rulers`, which the space publishes in `rulers`.
    ruler_zero: Point,
    /// The snapping of the coordinates the pointer sets on this canvas (View > Snap To), as data over the document's
    /// targets: the rulers snap their zero point with it, the guides snap a guide with it and the tool host hands it to
    /// the tool ([`ToolContext::snap`]). The canvas sets its tolerance each pass from the view it shows. Without it
    /// nothing snaps.
    #[prop]
    snap: Option<Snap>,
    /// The pasteboard colour, set once here; the pasteboard and the crop shield read it from the space.
    #[prop]
    pasteboard: PasteboardColor,
    /// Preferences > Cursors > Other Cursors is Precise. Caps Lock toggles it; the result is published in the
    /// [`CanvasSpace`] ([`CanvasSpace::precise`]), where the tool and the body's controls read it.
    #[prop]
    precise_cursors: bool,
    /// The view of this pass, after the canvas folded in its input.
    settled: Option<Settled>,
    /// The Menu key or Shift+F10 reached the focused canvas this pass.
    context_key: bool,
    /// The text of the paste the editor part takes this pass, taken in `update` with its inputs.
    pasted: Option<Arc<str>>,
    /// The ease the app asked for from the view it drew ([`ease_from`](Self::ease_from)).
    ease: Option<Steer>,
    /// The app's tool moves the view with the pointer this pass (an image-moving crop slides the view by fractional
    /// pointer distances under a frame that stays put): the shown view stays as given, not [`pixel_aligned`], so the
    /// picture and the frame do not jitter by part of a pixel as the pointer moves. P-15: the tool host passes the
    /// framework `CropTool`'s image mode.
    #[prop(name = "view_follows_pointer")]
    follows_pointer: bool,
    /// The areas where the active tool takes a press over every canvas control this pass, from
    /// [`CanvasTool::regions`], which the app asks before it shows the canvas. A press there is the canvas's background
    /// press, the one the tool host and the body's controls read. The canvas maps them to the screen through the view
    /// given this pass: while the canvas moves the view itself in a pass (the wheel, a middle-button pan, a zoom
    /// ease), the shapes the next press meets lag that pass's motion. Without a given view nothing is claimed.
    #[prop]
    tool_regions: Vec<ToolRegion>,
    /// The slot the body's paint reserved for the picture, published in the [`CanvasSpace`]; none in a measuring pass.
    picture: Option<crate::primitives::PaintSlot>,
}

/// How a pass's view commands move the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandMotion {
    /// No command zoomed or placed it (a turn).
    Keep,
    /// A command zoomed it: the zoom eases under Animated Zoom.
    Ease,
    /// The app placed it: the view jumps.
    Jump,
}

/// A canvas's view in one pass.
#[derive(Clone, Copy, Debug)]
struct Settled {
    /// The view the app copies.
    target: CanvasView,
    /// The view on screen: the target, or a view part way to it while a zoom eases.
    shown: CanvasView,
    /// A zoom eases: `shown` is part way to `target`.
    easing: bool,
    /// The keys the canvas leaves to its body.
    keys: BodyKeys,
    /// The wheel share the canvas took this pass, handed on to its body.
    wheel: Option<Wheel>,
    /// A middle-button autoscroll or a held middle pan runs.
    auto_scrolling: bool,
    /// The mark of a running middle-button autoscroll, in screen points.
    mark: Option<Pos2>,
    /// A press that stopped an autoscroll is down: it reaches no tool.
    swallowing: bool,
    /// The screen point a wheel or pinch zoom of this pass kept still ([`CanvasSpace::zoom_anchor`]).
    zoom_anchor: Option<Pos2>,
}

impl<'a> Canvas<'a> {
    /// A canvas named `label`. The keyboard path of its tool is the tool's own
    /// ([`CanvasTool::command_form`]).
    pub fn new(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new().arena(true).sizing(Sizing::Fill).command_scope(ViewCommands::SCOPE);
        Canvas {
            common: Common::new(),
            props,
            view: None,
            given: None,
            document: None,
            layer_bounds: None,
            queued: &[],
            navigation: NavigationOptions::default(),
            strips: None,
            strips_shown: false,
            ruler_output: None,
            keys_to_body: false,
            body_nudges: NudgeMods::NONE,
            editor: None,
            inactive_edits: EditSet::NONE,
            editor_command_keys: false,
            editor_input: Arc::from([]),
            pasteboard: PasteboardColor::Default,
            precise_cursors: false,
            rulers: None,
            ruler_zero: Point::ZERO,
            snap: None,
            settled: None,
            context_key: false,
            pasted: None,
            ease: None,
            follows_pointer: false,
            tool_regions: Vec::new(),
            picture: None,
        }
        .label(label)
        .ltr_axis()
    }

    /// The view the canvas shows its document through, as a binding: the canvas writes the view of its pass through
    /// it (`Bind::bound` holds it in the app's variable, `Bind::kept` in the canvas's own memory). A given view
    /// (`Bind::given`, or a plain [`CanvasView`]) is only read: the app copies what a tool changed from
    /// [`ToolHostResponse::view`] (or [`CanvasOutput::view`] without a tool host) every pass.
    ///
    /// The canvas sets the view's rect to its own rect, inside the rulers while they show ([`with_rulers`](Self::with_rulers)),
    /// so the body and the reported view carry this pass's rect. A view whose rect has no area (`CanvasView::new(Rect::NOTHING, ..)`,
    /// a document never shown) opens fitted: Fit on Screen in the first pass the canvas has area and a document. A
    /// canvas with no area (a minimized window) leaves the view as it is, so a restore does not refit.
    ///
    /// The body reads the view as a [`CanvasSpace`] with [`canvas_space`]: the view on screen, part way through a
    /// zoom that eases. Its controls register parts in the canvas root's screen space through
    /// `CanvasView::to_screen`. The view sets no body transform: a body of components that keep canvas-space geometry
    /// (a node graph) also sets `transform(view.layer_transform())`, which exists for an unrotated, unmirrored,
    /// square view only.
    ///
    /// A canvas with a view is its body's scroll owner: the page keys a control focused in it leaves scroll the view
    /// as the canvas's own do, and focus arriving at a control off the view brings the control into view.
    #[must_use]
    pub fn view(mut self, view: impl Into<Bind<'a, CanvasView>>) -> Self {
        self.view = Some(view.into());
        self.props.scroll_owner = true;
        self
    }

    /// Eases from `from`, the view the app drew last, to the view given this pass, as the canvas eases its own zoom
    /// commands: an app's zoom step made outside the canvas (a menu row, a Zoom tool click before the app's tools run
    /// on the tool host). `anchor` keeps a document point under a screen point along the way. With Animated Zoom off,
    /// or when the wheel or a pan moves the view in the same pass, the view jumps.
    #[must_use]
    pub fn ease_from(mut self, from: CanvasView, anchor: Option<(Point, Pos2)>) -> Self {
        self.ease = Some(Steer::Ease { from, anchor });
        self
    }

    /// The app moved the view itself since its last pass (a pan of its own): an ease running stops at the view given
    /// this pass, as for a pan of the canvas's own.
    #[must_use]
    pub fn jumped(mut self) -> Self {
        self.ease = Some(Steer::Stop);
        self
    }

    /// Leaves the view commands to the instance around the canvas that owns its view (a full-window dialog's): the
    /// canvas is no instance of them, so the chords reach that instance while the canvas has the focus.
    #[must_use]
    pub(crate) fn outer_view_commands(mut self) -> Self {
        self.props.command_scope = None;
        self
    }

    /// Publishes the view commands' state for this canvas and applies the ones the app queued, then the ones routed to
    /// it, in order, to `view`: the view they ask for, and how the pass moves to it.
    fn view_commands(&self, cx: &mut Cx<'_, Self>, view: Option<CanvasView>) -> Option<(CanvasView, CommandMotion)> {
        self.props.command_scope?;
        let pixels_per_point = cx.env.scale().0;
        let scheme = Env::of_root(&cx.core.ctx).view().get::<ZoomScheme>();
        let target = view.zip(self.document);
        for command in ViewCommands::ALL {
            let enabled = target.is_some_and(|(view, document)| command.available(&view, &document, self.layer_bounds, pixels_per_point, &scheme));
            cx.publish(command, CommandState::new().enabled(enabled));
        }
        let (view, document) = target?;
        let (mut next, mut zoomed, mut placed) = (view, false, false);
        let queued = if measure::active(&cx.core.ctx) { &[][..] } else { self.queued };
        for request in queued {
            let command = request.command();
            let changed = command.apply(&mut next, &document, self.layer_bounds, pixels_per_point, &scheme);
            match request {
                ViewRequest::Command(_) => zoomed |= changed && command.zooms(),
                ViewRequest::Place(_) => placed |= changed,
            }
        }
        while let Some((command, _)) = cx.take_command::<ViewCommands>() {
            zoomed |= command.apply(&mut next, &document, self.layer_bounds, pixels_per_point, &scheme) && command.zooms();
        }
        let motion = if placed {
            CommandMotion::Jump
        } else if zoomed {
            CommandMotion::Ease
        } else {
            CommandMotion::Keep
        };
        (next != view).then_some((next, motion))
    }

    /// The document's size, from the target the app gave.
    fn document_size(&self) -> Option<Size> {
        self.document.map(|target| target.document)
    }

    /// The view of this pass from the app's `given` view: this pass's rect, the first fit, the wheel with its ease and
    /// the touchpad glide, a middle-button pan or autoscroll, the view commands, the Overscroll clamp, and the zoom
    /// ease.
    fn settle(&self, cx: &mut Cx<'_, Self>, given: CanvasView) -> Settled {
        let ctx = cx.core.ctx.clone();
        let id = cx.core.id;
        let mut motion = ViewMotion::take(&ctx, id);
        let pixels_per_point = cx.env.scale().0;
        let size = self.document_size().unwrap_or(Size::ZERO);
        let scheme = Env::of_root(&ctx).view().get::<ZoomScheme>();
        let mut view = given;
        let area = cx.content_rect;
        if area.width() > 0.0 && area.height() > 0.0 {
            let never_shown = !(view.rect.width() > 0.0 && view.rect.height() > 0.0);
            view.rect = area;
            if never_shown && let Some(document) = self.document {
                ViewCommands::FitOnScreen.apply(&mut view, &document, None, pixels_per_point, &scheme);
            }
        }
        let at_start = view;
        // The pass clock. A later pass of the same frame reads the same time, so the glide and the autoscroll move the
        // view only in the pass that receives the frame's time.
        let now = frame::clock(&ctx);
        let (press, on_region) = self.pressed(cx);
        // A press, a zoom, a key move and a view command end the touchpad glide and the eased wheel's owed motion.
        let mut halt = matches!(press.phase, PressPhase::Down { .. });
        // A press, a key move and a view command also drop the eased wheel zoom's owed part; its own zoom does not.
        let mut stop_zoom_ease = halt;
        let mut jumped = false;
        let mut line_pan = Vec2::ZERO;
        // The screen point a wheel or pinch zoom of this pass kept still, before the clamp below can move the centre.
        let mut zoom_anchor = None;
        let wheel = cx.take_wheel();
        let wheeled = wheel.is_some();
        if let Some(wheel) = &wheel {
            let read = self.read_wheel(cx, &mut motion, &mut view, wheel, now);
            line_pan = read.line_pan;
            jumped |= read.jumped;
            zoom_anchor = read.zoom_anchor.or(zoom_anchor);
            halt |= read.zoomed;
        }
        let paid = pay_wheel_zoom(&ctx, &mut motion, &mut view, size, pixels_per_point, &scheme, now);
        jumped |= paid.jumped;
        zoom_anchor = paid.zoom_anchor.or(zoom_anchor);
        halt |= paid.zoomed;
        // Every pass feeds the ease, the line pan or nothing, so owed motion keeps paying; the service takes the time
        // from its own clock, so a later pass of the same frame pays nothing.
        if self.navigation.eased_wheel && !halt {
            let step = cx.ease_wheel(line_pan);
            if step != Vec2::ZERO {
                pan(&mut view, step);
                jumped = true;
            }
            if step != Vec2::ZERO || line_pan != Vec2::ZERO {
                frame::request_repaint(&ctx);
            }
        }
        let before_glide = view;
        let glide = if halt { Vec2::ZERO } else { motion.glide.step(now) };
        if glide != Vec2::ZERO {
            pan(&mut view, glide);
            jumped = true;
        }
        if motion.glide.gliding() {
            frame::request_repaint(&ctx);
        }
        // Under Autoscroll the middle button's press on the background (no tool region holds it) is the autoscroll's;
        // elsewhere a middle drag pans the view 1:1.
        let middle = press.phase.start().is_some_and(|start| start.button == PointerButton::Middle);
        let held_pan = middle && self.middle_pans(on_region) && press.phase.is_drag();
        if held_pan {
            cx.cursor(CursorIcon::Grabbing, CursorPriority::Drag);
        }
        let (_, drag) = self.background_press(cx);
        if let Some(delta) = drag {
            pan(&mut view, delta);
            jumped = true;
        }
        let mut swallowing = false;
        if !measure::active(&ctx) {
            // An autoscroll whose canvas missed a frame (another tab showed) ends, so it never comes back with the canvas.
            let frame_nr = frame::running(&ctx).map_or(0, |pass| pass.frame_nr);
            let away = motion.scrolled_in.is_some_and(|last| frame_nr > last.saturating_add(1));
            let (moved, swallowed) = self.autoscroll(cx, &mut motion.autoscroll, &mut view, size, wheeled || away);
            jumped |= moved;
            swallowing = swallowed;
            motion.scrolled_in = motion.autoscroll.mark().map(|_| frame_nr);
        }
        let auto_scrolling = motion.autoscroll.mark().is_some() || held_pan;
        // As its body's scroll owner the canvas shows a control focus arrives at, and applies the page keys a focused
        // control leaves as its own.
        if let Some(by) = cx.take_reveal(area) {
            pan(&mut view, -by);
            jumped = true;
        }
        // Page Up and Down, Home and End scroll the view as in Photoshop; the arrows are the tool's nudges. A canvas
        // that leaves its keys to its body hands it the page keys; the arrows stay routed, for the body's tool host
        // first and then the body (`CanvasSpace::take_nudge`). With an editor part the keys are its input: they
        // reached the body as editor input before this step, so nothing is left to hand over.
        let to_body = self.keys_to_body && self.editor.is_none();
        let mut keys = BodyKeys::default();
        let mut moves = Vec::new();
        while let Some(mv) = cx.intents().take_move() {
            moves.push(mv);
        }
        moves.extend(cx.take_scroll_moves());
        halt |= !moves.is_empty();
        stop_zoom_ease |= !moves.is_empty();
        for (step, mods) in moves {
            if to_body {
                keys.page = Some((step, mods));
            } else if let (Some(document), Some(key)) = (self.document_size(), PageKey::of(step, mods, area.size())) {
                jumped |= key.apply(&mut view, document);
            }
        }
        // A pan or a scroll takes over from an ease, here or in a tool.
        let mut steer = motion.steer.take();
        if let Some(ease) = self.ease {
            steer = Some(ease);
        }
        // With Animated Zoom off no ease runs, whether the app passed one this pass or one was running.
        if jumped || !self.navigation.animated_zoom {
            steer = Some(Steer::Stop);
        }
        let before = view;
        if let Some((next, motion)) = self.view_commands(cx, Some(view)) {
            // A view command's zoom keeps its own point (the centre), not the wheel's.
            if next.zoom != view.zoom {
                zoom_anchor = None;
            }
            view = next;
            halt = true;
            stop_zoom_ease = true;
            match motion {
                CommandMotion::Ease if self.navigation.animated_zoom => {
                    // A zoom that keeps the centre eases about it; one that moves the centre (Fit on Screen) eases
                    // freely.
                    let anchor = (next.center == before.center).then_some((before.center, before.rect.center()));
                    steer = Some(Steer::Ease { from: before, anchor });
                }
                CommandMotion::Jump => steer = Some(Steer::Stop),
                _ => {}
            }
        }
        let target = clamped(view, self.document_size(), &self.navigation);
        if glide != Vec2::ZERO {
            motion.glide.moved(target != before_glide);
        }
        if halt {
            motion.glide.stop();
            cx.stop_wheel_ease();
        }
        motion.wheel_zoom.stop_if(stop_zoom_ease);
        // A view that moves during a wheel gesture marks it moved, so a body that turns pages at the edge does not turn
        // one after a pan or a go-to.
        if target != at_start {
            cx.mark_wheel_moved();
        }
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
        let easing = motion.animation.running();
        let mark = motion.autoscroll.mark();
        motion.restore(&ctx, id);
        Settled { target, shown, easing, keys, wheel, auto_scrolling, mark, swallowing, zoom_anchor }
    }

    /// One pass's wheel on `view`: a pan the eased wheel pays out later, or a pan, zoom or turn applied now, and the
    /// touchpad glide's reading of it.
    fn read_wheel(&self, cx: &mut Cx<'_, Self>, motion: &mut ViewMotion, view: &mut CanvasView, wheel: &Wheel, now: f64) -> WheelRead {
        let pixels_per_point = cx.env.scale().0;
        let size = self.document_size().unwrap_or(Size::ZERO);
        let (line, os) = (self.navigation.wheel_line, cx.env.shell().os);
        let mut read = WheelRead::default();
        if let Some(action) = motion.wheel.read(wheel, line, os, &self.navigation) {
            match action {
                // The eased wheel hands a line's pan to the wheel service, which pays it out over the passes after.
                WheelAction::Pan(delta) if self.navigation.eased_wheel && wheel.unit == WheelUnit::Line => read.line_pan = delta,
                // The eased wheel zoom owes a notch's zoom, which `settle` pays over this pass and the ones after.
                WheelAction::Zoom(factor) if self.navigation.eased_wheel_zoom && eases_zoom(wheel) => {
                    let at = wheel.at.unwrap_or_else(|| view.rect.center());
                    let frame_dt = f64::from(cx.core.ctx.input(|input| input.stable_dt));
                    motion.wheel_zoom.feed(factor, at, now, frame_dt);
                    read.zoomed = true;
                }
                action => {
                    read.zoomed = !matches!(action, WheelAction::Pan(_));
                    // The pointer the wheel came at: the canvas's own widget is not hovered where an app widget in
                    // its body covers it.
                    let at = wheel.at.unwrap_or_else(|| view.rect.center());
                    let (anchor, zoom) = (WheelNavigation::zoom_anchor(view, action, at, size), view.zoom);
                    let scheme = Env::of_root(&cx.core.ctx).view().get::<ZoomScheme>();
                    read.jumped |= WheelNavigation::apply(view, action, at, size, pixels_per_point, &scheme);
                    if view.zoom != zoom {
                        read.zoom_anchor = anchor;
                    }
                }
            }
        }
        // A trackpad rotation or a twist of two fingers turns the view; a mirrored view turns the other way.
        if let Some(degrees) = WheelNavigation::rotation(&WheelInput::of(wheel, line, os), &self.navigation) {
            view.rotation = normalized_angle(view.rotation + ViewTurn::screen_delta(degrees, view.flip_x));
            read.jumped = true;
            read.zoomed = true;
        }
        motion.glide.read(wheel, now, !read.zoomed && self.navigation.touchpad_glide);
        read
    }

    /// One pass of the middle button's autoscroll over `view`: a middle press on the background starts it under
    /// [`MiddleButton::AutoScroll`] (away from the tool regions, and the press focuses the canvas); while it runs each
    /// pass scrolls toward the pointer (the other way under Invert Middle) and asks for the next pass; any input ends
    /// it, and the pass that ends it applies no step. A drag while the button is held scrolls the same way, and its
    /// release ends the run; a click leaves it running. `ended` is whether a wheel or a zoom reached the canvas this
    /// pass, or the canvas missed a frame. Returns whether the view moved, and whether a press that stopped the run is down (it and its
    /// release belong to the autoscroll, not to the tool).
    ///
    /// Every end signal is read in the pass that gets the input (a rerun pass of a frame reads no key and no press,
    /// and shares the frame's clock, so it steps zero): `keyed` for any key (`runtime/keys/input.rs`), the background
    /// press or the frame's press for a press, egui's window focus (winit `Focused`, the web's blur) and a pointer
    /// position of `None` (winit `CursorLeft`, egui `PointerGone`) for leaving the window.
    fn autoscroll(&self, cx: &mut Cx<'_, Self>, run: &mut MiddleAutoscroll, view: &mut CanvasView, size: Size, ended: bool) -> (bool, bool) {
        let ctx = cx.core.ctx.clone();
        let input = crate::runtime::pointer::frame(&ctx);
        let (press, on_region) = self.pressed(cx);
        let phase = press.phase;
        let down = matches!(phase, PressPhase::Down { .. });
        let swallowing = run.swallowing();
        if swallowing && matches!(phase, PressPhase::Release { .. } | PressPhase::Cancel { .. }) {
            run.stop_swallowing();
        }
        let moved = 'pass: {
            if run.mark().is_some() {
                let keyed = crate::runtime::keys::keyed(&ctx);
                // The window's blur reaches the pass as its event (`PassPointer::focus_lost`) before egui's focus flag
                // follows it, and a pointer gone from the window as its event: either ends the run in that pass.
                let left = crate::runtime::pointer::with_pass_pointer(&ctx, |pointer| {
                    pointer.focus_lost || pointer.events.iter().any(|event| matches!(event, craft_behavior::PointerEvent::Gone))
                });
                let feel = self.navigation.middle_autoscroll;
                // The reader's run lives while the canvas holds the focus: a modal over it or another control taking
                // the keys ends it once the button is up.
                let canvas_left = feel == MiddleAutoscrollFeel::Reader && !run.held() && !cx.is_focused();
                let unfocused = left || !ctx.input(|i| i.focused) || canvas_left;
                let off = self.navigation.middle_button != MiddleButton::AutoScroll;
                if keyed || ended || down || input.press.is_some() || unfocused || input.pos.is_none() || off {
                    // Escape only stops the scroll; any other key also does its own job.
                    if keyed {
                        let _ = cx.intents().take(Intent::Cancel);
                    }
                    run.end();
                    if down {
                        run.swallow_press();
                    }
                    break 'pass false;
                }
                if phase.start().is_some_and(|start| start.button == PointerButton::Middle) {
                    match phase {
                        PressPhase::DragStart { .. } | PressPhase::Drag { .. } => run.drag(),
                        PressPhase::Release { .. } | PressPhase::Cancel { .. } => run.release(feel),
                        _ => {}
                    }
                }
            } else if !self.middle_pans(on_region)
                && let PressPhase::Down { start, .. } = phase
                && start.button == PointerButton::Middle
            {
                run.press(start.origin);
                // The autoscroll lives while the canvas holds the focus, and the focus service moves focus on primary
                // presses only: the middle press that starts it takes the focus itself.
                cx.focus_own();
            }
            let Some(pointer) = input.pos.filter(|_| run.mark().is_some()) else { break 'pass false };
            let step = run.step(pointer, frame::clock(&ctx), self.navigation.middle_autoscroll);
            let step = if self.navigation.invert_middle { -step } else { step };
            crate::runtime::cursor::request(&ctx, run.cursor(Some(pointer)).into(), CursorPriority::Drag);
            frame::request_repaint(&ctx);
            Autoscroll::scroll(view, step, size).is_some()
        };
        (moved, swallowing || run.swallowing())
    }

    /// Shows `rulers` along the top and left edges of the canvas, which the view lies inside (View > Rulers); none shows
    /// no strips. The canvas hands them its space and snap, and the body (guides) reads the guide a drag out of them
    /// places with [`ruler_guide`]. What they report comes back in [`CanvasOutput::rulers`].
    #[must_use]
    pub fn with_rulers(mut self, rulers: Option<Rulers<'a>>) -> Self {
        self.ruler_zero = rulers.as_ref().map_or(Point::ZERO, Rulers::zero);
        self.strips_shown = rulers.is_some();
        self.strips = rulers;
        self
    }

    /// Measures the document's axes in `x` and `y`, counted from the zero point of the [`Rulers`] given to
    /// [`with_rulers`](Self::with_rulers) (the document's origin without them). The rulers, the guides and the
    /// measures of the body read the units and the zero from the canvas's [`CanvasSpace`], so the app states the zero
    /// once. A canvas that publishes none measures in pixels from the document's origin. Whether the strips show, and
    /// so where the view lies, is [`with_rulers`](Self::with_rulers).
    #[must_use]
    pub fn rulers(mut self, x: RulerText, y: RulerText) -> Self {
        self.rulers = Some(CanvasRulers::new(x, y, Point::ZERO));
        self
    }

    /// The space the body and the tool see in this pass; `caps_lock` is whether Caps Lock is on in it.
    fn space(&self, id: egui::Id, pixels_per_point: f32, background: BackgroundPress, caps_lock: bool, scheme: ZoomScheme) -> Option<CanvasSpace> {
        let (shown, target, easing, keys, wheel, zoom_anchor) = match self.settled {
            Some(settled) => (settled.shown, settled.target, settled.easing, settled.keys, settled.wheel, settled.zoom_anchor),
            None => (self.given?, self.given?, false, BodyKeys::default(), None, None),
        };
        let mut space = CanvasSpace::new(target, pixels_per_point).with_follows_pointer(self.follows_pointer).with_zoom_anchor(zoom_anchor);
        space.set_view(target);
        if easing {
            space.ease_through(shown);
        }
        let space = space
            .with_keys(keys)
            .with_document(self.document_size())
            .with_rulers(self.rulers.map(|rulers| CanvasRulers::new(rulers.x, rulers.y, self.ruler_zero)))
            .with_pasteboard(self.pasteboard)
            .with_background(background)
            .with_precise(self.precise_cursors != caps_lock)
            .with_zoom_scheme(scheme)
            .with_wheel(wheel)
            .with_canvas(id);
        Some(match self.picture {
            Some(slot) => space.with_picture(slot),
            None => space,
        })
    }

    /// The snap of this pass: the app's with the tolerance of the view on screen and the OS the pass runs on.
    fn snapped(&self, os: craft_keymap::Platform) -> Option<Snap> {
        let view = self.settled.map(|settled| settled.shown).or(self.given)?;
        self.snap.clone().map(|snap| snap.at(&view, os))
    }

    /// The press on the background part, or on the tool region that holds it, and whether a tool region holds it.
    fn pressed(&self, cx: &Cx<'_, Self>) -> (PressState, bool) {
        let region = (0..self.tool_regions.len() as u64)
            .filter_map(|i| cx.part_if_declared(TOOL_REGION.at(i)))
            .map(|result| result.press)
            .find(|press| !matches!(press.phase, PressPhase::Idle));
        match region {
            Some(press) => (press, true),
            None => (cx.part(BACKGROUND).press, false),
        }
    }

    /// Whether a middle drag of a press on the background, or on a tool region when `on_region`, pans the view: under
    /// [`MiddleButton::AutoScroll`] a press away from the tool regions belongs to the autoscroll instead.
    fn middle_pans(&self, on_region: bool) -> bool {
        on_region || self.navigation.middle_button != MiddleButton::AutoScroll
    }

    /// The press on the background part, or on the tool region that holds it, as the canvas reports it, and the pan of a
    /// middle-button drag: a middle press pans the view ([`middle_pans`](Self::middle_pans)), or belongs to the
    /// autoscroll, and is never a tool press. Under Invert Middle the drag pans the other way, as the autoscroll does.
    fn background_press(&self, cx: &Cx<'_, Self>) -> (PressState, Option<Vec2>) {
        let (press, on_region) = self.pressed(cx);
        let middle = press.phase.start().is_some_and(|start| start.button == PointerButton::Middle);
        let pans = self.middle_pans(on_region);
        let sign = if self.navigation.invert_middle { -1.0 } else { 1.0 };
        match press.phase {
            PressPhase::DragStart { start, pos, .. } if middle && pans => (PressState::IDLE, Some((pos - start.origin) * sign)),
            PressPhase::Drag { delta, .. } if middle && pans => (PressState::IDLE, Some(delta * sign)),
            _ if middle => (PressState::IDLE, None),
            _ => (press, None),
        }
    }

    /// The tool host's input of this pass: settled in `update`, so the body and `finish` read the same.
    fn tool_pass(&self, cx: &Cx<'_, Self>) -> ToolPass {
        // A press that stopped an autoscroll is the autoscroll's until its release.
        let press = if self.settled.is_some_and(|settled| settled.swallowing) { PressState::IDLE } else { self.background_press(cx).0 };
        let caps_lock = lock_keys::caps_lock(&cx.response.ctx);
        let scheme = Env::of_root(&cx.response.ctx).view().get::<ZoomScheme>();
        let space = self
            .space(cx.response.id, cx.env.scale().0, BackgroundPress::of(&press, self.context_key), caps_lock, scheme)
            .map(|space| space.with_space_held(tool_host::space_held(&cx.response.ctx)));
        ToolPass {
            id: cx.response.id,
            rect: cx.response.rect,
            press,
            space,
            view: self.settled.map(|settled| settled.target).filter(|target| Some(*target) != self.given),
            snap: self.snapped(cx.env.shell().os),
            document: self.document_size(),
            navigation: self.navigation,
            editor_input: self.editor_input.clone(),
            editor: cx.editor_handle(),
            pasted: self.pasted.clone(),
        }
    }

    /// Shows the body zoomed and moved by `transform`, so controls and members keep canvas-space geometry.
    #[must_use]
    pub fn transform(mut self, transform: CanvasTransform) -> Self {
        self.props = self.props.transform(transform);
        self
    }
}

/// What a canvas reports: its response and the press the tool got.
#[derive(Debug)]
#[non_exhaustive]
pub struct CanvasOutput {
    /// The canvas's response.
    pub response: Response,
    /// The press on the background part: a press no canvas control claimed. A middle-button press pans the view and
    /// is never a tool press, so this is idle while one runs.
    pub press: PressState,
    /// The view on screen and the interface scale, for a canvas given a view.
    pub space: Option<CanvasSpace>,
    /// The canvas's view of this pass, when it differs from the view the app gave: this pass's rect, the first fit,
    /// the wheel, a middle-button pan, the view commands and the Overscroll clamp. A zoom that eases reports its
    /// target; the canvas shows the way there itself. [`ToolHost::run`] carries it in [`ToolHostResponse::view`]
    /// when no tool set a view, so an app with a tool host copies that one view.
    pub view: Option<CanvasView>,
    /// The multi-finger tap this pass ended, from the pointer pass; `None` when there was none.
    pub multi_tap: Option<MultiTap>,
    /// True while a middle-button autoscroll or a held middle pan runs this pass.
    pub auto_scrolling: bool,
    /// The mark of a running middle-button autoscroll, for captures.
    pub autoscroll: Option<Pos2>,
    /// What the rulers given to [`Canvas::with_rulers`] reported in this pass.
    pub rulers: Option<RulersOutput>,
}

impl CanvasOutput {
    /// Where a primary press on the background was released without a drag. A tool that creates geometry opens its
    /// command form at this point.
    pub fn click_at(&self) -> Option<Pos2> {
        match self.press.phase {
            PressPhase::Release { start, pos, click: true, .. } if start.button == PointerButton::Primary => Some(pos),
            _ => None,
        }
    }
}

/// What the tool host of a canvas runs on in one pass: the canvas's id and rect, the press its background part got,
/// its space, view, snapping and navigation. The canvas settles these before its body runs and publishes them to the
/// body ([`tool_pass`]), so the body runs the host after its canvas controls (whose parts take a press first) and
/// before its picture, and the picture shows what this pass's press and moves did. A measuring run publishes none:
/// the host never runs in one.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolPass {
    /// The canvas's id, which keys the host's state.
    pub id: egui::Id,
    /// The canvas's rect.
    pub rect: Rect,
    /// The press on the background part, as [`CanvasOutput::press`].
    pub press: PressState,
    /// The view on screen and the interface scale, for a canvas given a view. The host hands it to the tool.
    pub space: Option<CanvasSpace>,
    /// The canvas's view of this pass, when it differs from the view the app gave, as [`CanvasOutput::view`].
    pub view: Option<CanvasView>,
    pub(crate) snap: Option<Snap>,
    pub(crate) document: Option<Size>,
    pub(crate) navigation: NavigationOptions,
    /// The editor part's inputs of this pass, in the order they arrived; the body applies them to its own document.
    pub editor_input: Arc<[EditorInput]>,
    /// The editor part of this pass, through which the body reports its editor state; `None` without an editor.
    pub editor: Option<EditorHandle>,
    /// The text of the paste the editor part takes this pass (its [`EditorInput::Edit`] of
    /// [`EditCommand::Paste`](craft_behavior::EditCommand::Paste)), as the platform's paste event carried it: Ctrl+V's,
    /// or the one a Paste command asks the platform for. The framework takes that event out of egui's input, and the
    /// platform's read can succeed where the installed clipboard can't be opened. `None` in a pass without one.
    pub pasted: Option<Arc<str>>,
}

impl ToolPass {
    /// Notes the input-method receipt between the stamps `before` and `after` for the canvas's editor part; true when the
    /// stamps prove the engine applied it.
    /// A canvas without an editor part notes nothing and proves nothing.
    pub fn ime_receipt(&self, ctx: &Context, before: ImeStamp, after: ImeStamp) -> bool {
        self.editor.as_ref().is_some_and(|editor| editor.ime_receipt(ctx, before, after))
    }

    /// Notes an input-method application change (an edit the app made, not the input method) for the canvas's editor
    /// part. A canvas without an editor part notes nothing.
    pub fn ime_application_change(&self, ctx: &Context) {
        if let Some(editor) = &self.editor {
            editor.ime_application_change(ctx);
        }
    }

    /// Asks for the OS composition to be interrupted this pass because the body refused an input method's edit, as
    /// [`EditorHandle::ime_interrupt`]: a commit the body applied after the refusal keeps its `InputMethod` cause. A
    /// canvas without an editor part asks nothing.
    pub fn ime_interrupt(&self, ctx: &Context) {
        if let Some(editor) = &self.editor {
            editor.ime_interrupt(ctx);
        }
    }

    /// `view` as the canvas keeps it: inside the document's edges with Overscroll off. The tool host passes a tool's
    /// view through it.
    pub(crate) fn clamped(&self, view: CanvasView) -> CanvasView {
        clamped(view, self.document, &self.navigation)
    }

    /// Hides the pointer for this pass while it is over the canvas, over every other request, links and the tool's
    /// cursor included (`CursorPriority::Hidden`): a body that hides a still pointer (a full-screen presentation) asks
    /// in each pass the pointer should stay hidden, and stops asking when it moves. A measuring run publishes no pass,
    /// so only the real run asks.
    pub fn hide_pointer(&self, ui: &egui::Ui) {
        if crate::runtime::pointer::hover_point_in(ui.ctx(), self.rect).is_some() {
            let hidden = craft_behavior::Cursor::Painted(craft_behavior::PaintedCursor::Hidden);
            crate::runtime::cursor::request(ui.ctx(), hidden, CursorPriority::Hidden);
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
        let surface = SurfaceLook::new(Stateful::new(cx.color(cx.tokens.color.surface.canvas))).kind(SurfaceKind::Canvas);
        // The rulers' own layout says where the view lies inside them. The autoscroll mark takes the overlay handle
        // colours, not the window fill and text ink: the canvas has no window fill, and the overlay pair is the one
        // that reads on any page.
        let overlay = &cx.tokens.color.overlay;
        CanvasStyle {
            surface: Some(surface),
            rulers: Rulers::content_rect(cx, Rect::ZERO).min.to_vec2(),
            mark: Some((cx.color(overlay.handle_fill), cx.color(overlay.stroke))),
            ..CanvasStyle::default()
        }
    }

    /// With an editor part the canvas keeps an editor's keys: typed text, the arrows as moves and Delete reach the part
    /// as editor input, where the canvas's own keys would leave the text unrouted and make the arrows the tool's
    /// nudges. It still marks its window as showing a document.
    fn keys(&self) -> KeyUse {
        match self.editor {
            Some(_) => {
                let keys = KeyUse::of(Input::Editor).with_chrome_key(true).with_moves().with_inactive(self.inactive_edits);
                if self.editor_command_keys { keys.with_command_keys() } else { keys }
            }
            None => Self::KEYS.with_nudge_mods(NudgeMods::STEPS.union(self.body_nudges)),
        }
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL
    }

    fn wheel_notch(&self) -> WheelNotch {
        WheelNotch::Points(self.navigation.wheel_line)
    }

    /// The rulers' strips lie outside the view the body is clipped to.
    fn padding(&self, _rect: Rect, style: &CanvasStyle) -> craft_behavior::Insets {
        if self.strips_shown { craft_behavior::Insets::new(style.rulers.x, 0.0, style.rulers.y, 0.0) } else { style.metrics().padding }
    }

    /// Reserves the slot the picture paints into, under every canvas control of the body (`CanvasSpace::paint_picture`),
    /// and shows the rulers over the canvas's rect.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.picture = Some(ui.reserve_slot(cx.content_rect));
        if let Some(rulers) = self.strips.take() {
            let pass = self.tool_pass(cx);
            self.ruler_output = Some(ui.child(RULERS, cx.rect, rulers.on_canvas(pass.space, pass.snap)));
        }
    }

    /// The mark of a running middle-button autoscroll, over the body's picture and controls, clipped to the canvas.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(at), Some((fill, ink))) = (self.settled.and_then(|settled| settled.mark), cx.style.mark) else { return };
        ui.clipped(cx.content_rect, |ui| navigate::paint_mark(&ui.draw(), at, fill, ink));
    }

    /// The view of this pass, kept for the body and `finish`.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // Taken once: `Cx::editor_input` empties the pass's inputs, and the body reads them from the `ToolPass`.
        self.editor_input = cx.editor_input().collect::<Vec<_>>().into();
        self.pasted = match cx.clipboard().paste() {
            crate::runtime::pending::Pending::Ready(content) => content.text.clone(),
            _ => None,
        };
        // Left untaken: the canvas's generic context request is recorded as for a secondary click.
        self.context_key = cx.intents().untaken(&Intent::ContextMenu);
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

    /// The canvas's [`CanvasSpace`] and [`ToolPass`] for its body; a canvas without a view hides an outer canvas's
    /// space.
    fn content_env(&self, cx: &Cx<'_, Self>, values: &mut crate::runtime::env::AppValues) {
        let pass = self.tool_pass(cx);
        space::publish(values, pass.space);
        snap::publish(values, pass.snap.clone());
        space::publish_ruler_guide(values, self.ruler_output.as_ref().and_then(|output| output.guide));
        tool_host::publish(values, (!measure::active(&cx.response.ctx)).then_some(pass));
    }

    /// The background part the active tool gets. It sits in the canvas root's space, the parent layer's. A press on it
    /// focuses the canvas, so the keys the tool reads reach it. It has a context menu, so the press machine tells a
    /// secondary click from a secondary drag on it as it does for the primary button.
    ///
    /// The tool's regions are parts of the same kind, `over` the body, so they rank above every canvas control.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sense = Sense::CLICK | Sense::CONTEXT_CLICK | Sense::DRAG | Sense::TAKE_FOCUS;
        cx.part(BACKGROUND, rect).priority(HitPriority::new(HitLevel::Background, 0)).sense(sense).pointer_only(TOOL_PATH);
        // The editor part's widget is the canvas's focus holder, so the press that focuses the canvas focuses it.
        if let Some(purpose) = self.editor {
            cx.editor(EDITOR, rect, purpose);
        }
        if self.tool_regions.is_empty() {
            return;
        }
        // `parts` runs before `update` settles this pass's view: the regions map through the view given.
        let Some(view) = self.view.as_mut().and_then(|view| view.reborrow().get()).and_then(|held| held.same().copied()) else { return };
        if !view.rect.is_positive() {
            return;
        }
        for (i, region) in self.tool_regions.iter().enumerate() {
            let points: Vec<Pos2> = region.points.iter().map(|&p| view.to_screen(p)).collect();
            if points.len() < 3 {
                continue;
            }
            cx.part(TOOL_REGION.at(i as u64), HitShape::Polygon(points))
                .priority(HitPriority::new(HitLevel::Background, 0))
                .sense(sense)
                .pointer_only(TOOL_PATH)
                .over();
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> CanvasOutput {
        let pass = self.tool_pass(&cx);
        let multi_tap = cx.take_multi_tap();
        let autoscroll = self.settled.and_then(|settled| settled.mark);
        let auto_scrolling = self.settled.is_some_and(|settled| settled.auto_scrolling);
        CanvasOutput {
            response: cx.response.clone(),
            press: pass.press,
            space: pass.space,
            view: pass.view,
            multi_tap,
            auto_scrolling,
            autoscroll,
            rulers: self.ruler_output,
        }
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

/// Pays this pass's part of what the eased wheel zoom's notches owe (`NavigationOptions::eased_wheel_zoom`) on `view`,
/// and asks for the next pass while more is owed: whether it zoomed, whether the view moved and the point the zoom kept
/// still.
fn pay_wheel_zoom(
    ctx: &Context,
    motion: &mut ViewMotion,
    view: &mut CanvasView,
    size: Size,
    pixels_per_point: f32,
    scheme: &ZoomScheme,
    now: f64,
) -> WheelRead {
    let paid = motion.wheel_zoom.step(now).map(|(factor, at)| {
        let action = WheelAction::Zoom(factor);
        let (anchor, zoom) = (WheelNavigation::zoom_anchor(view, action, at, size), view.zoom);
        let jumped = WheelNavigation::apply(view, action, at, size, pixels_per_point, scheme);
        WheelRead { jumped, zoom_anchor: anchor.filter(|_| view.zoom != zoom), zoomed: true, ..WheelRead::default() }
    });
    if motion.wheel_zoom.owes() {
        frame::request_repaint(ctx);
    }
    paid.unwrap_or_default()
}

/// Whether the eased wheel zoom takes `wheel`'s zoom: a wheel notch under the zoom modifier (Cmd or Ctrl), whose
/// deltas the wheel service turns into the share's zoom and never into scroll. A pinch (no modifier), a trackpad's
/// steps and a step under 8 points of egui's wheel zoom speed (which egui leaves unsmoothed too) zoom at once.
fn eases_zoom(wheel: &Wheel) -> bool {
    /// The log of the factor of 8 points at egui's default wheel zoom speed (1/200 a point).
    const SMALLEST: f32 = 8.0 / 200.0;
    (wheel.mods.cmd || wheel.mods.ctrl) && wheel.source != WheelSource::Trackpad && wheel.zoom.ln().abs() >= SMALLEST
}

/// What a pass's wheel did to the view ([`Canvas::read_wheel`]).
#[derive(Default)]
struct WheelRead {
    /// A line's pan the eased wheel pays out over the passes after.
    line_pan: Vec2,
    /// Whether the view moved at once.
    jumped: bool,
    /// The screen point a zoom kept still.
    zoom_anchor: Option<Pos2>,
    /// Whether the wheel zoomed or turned the view.
    zoomed: bool,
}
