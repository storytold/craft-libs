//! `AppShell`: the window skeleton. An ordered stack of top bars starting with the title bar, bottom bars, start and end
//! rails, and the body in what is left. Each bar and rail is a container on `Mount::Pane`, so F6 pane focus and the
//! name of each region come from the pipeline.

mod dialogs;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use self::dialogs::{ShellDialogsCx, WindowDialogs};
use super::canvas_shell::slots as keys;
use super::options_place::{FloatingBar, OptionsBarDrag, OptionsBarPlace, PlaceTarget};
use super::regions::{Claim, SlotSize, layout};
use super::reveal::{Aim, Reveal, Tracking};
use super::screen_mode::{ScreenMode, ScreenModeCommands, ScreenModeFollow, ScreenModeSet};
use super::title_bar::PageBand;
use super::words::ShellWords;
use crate::contract::overlay::{PopupRegion, Resize};
use crate::contract::{
    Bind, BodyCx, BodyLayout, Common, Component, Container, ContainerProps, Cx, DropZone, Mount, NoValue, Outcome, Overlay, PaneEdge, Props as _,
    Remember as _, Role, ShellPane, ShowContainer as _, Sizing,
};
use crate::layout_components::Stack;
use crate::notices::ToastStack;
use crate::runtime::commands::{CommandAccess as _, CommandScope, CommandSet as _};
use crate::runtime::drag::zone_target;
use crate::runtime::egui::pass;
use crate::runtime::env::{AppValues, BodyMode, Env, Label, RevealEdge, ShellCx, ShellEntry, ShellRegions, SlotKey, WindowKey};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::frame::{note_shell, shell_root, take_chrome_toggles};
use crate::runtime::memory::{self, sub};
use crate::runtime::overlay::{OpenFilter, any_open};
use crate::runtime::pointer;
use crate::runtime::windows::{WindowAction, WindowHost};
use crate::shell::Slot;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind, UiTokens};
use craft_behavior::{EditEvent, Metrics, Mixed, Sense};
use egui::{Context, Rect, Response, Ui, ViewportId, pos2};
use std::sync::Arc;

/// The closure that fills a region.
pub(super) type RegionFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// The closure that fills a region, as a `func` prop.
pub(super) struct RegionBody<'a>(pub(super) RegionFn<'a>);

impl<'a> RegionBody<'a> {
    /// The body that runs `body`.
    pub(super) fn new(body: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        RegionBody(Box::new(body))
    }
}

/// The closure that fills a slot or the body of an `AppShell`; it gets the app state the shell is shown with.
type SlotFn<'a, S> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a>;

/// The closure that fills the body of an `AppShell`, as a `func` prop.
struct SlotBody<'a, S>(SlotFn<'a, S>);

impl<'a, S> SlotBody<'a, S> {
    /// The body that runs `body`.
    fn new(body: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        SlotBody(Box::new(body))
    }
}

/// The style of a shell region: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ShellRegionStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a shell region reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ShellRegionOutput {
    /// The region's response; its rect is the region's place in the window.
    pub response: Response,
}

impl Outcome for ShellRegionOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A region of the window skeleton: the root window's content, the scope that carries the shell to its slots, or one
/// pane (a bar, a rail or the body).
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct ShellRegion {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// The shell the scope carries to its slots, and the rect popups opened inside it are contained in.
    shell: Option<(ShellCx, Option<Rect>)>,
    /// Sets the shell's screen modes for everything inside the region, beside its shell: a `ScreenModeButton` in a
    /// slot reads them with `Env::get`.
    #[prop(name = "with_mode_set", vis = "")]
    mode_set: Option<ScreenModeSet>,
    /// Whether the region senses a press on its own surface, where no child or part takes it (`Sense::SURFACE`).
    #[prop(flag)]
    senses_surface: bool,
}

impl ShellRegion {
    /// The whole content of the window: its edges resize the window where the app draws the frame. It paints the
    /// background surface under everything the shell places, so a gap between parts (a dock's gutter, the room beside
    /// the document tabs) shows the background, never the platform's clear colour.
    pub fn window() -> Self {
        let props =
            ContainerProps::new().mount(Mount::window(Resize::GRIPS)).surface(SurfaceKind::Background).command_scope(ScreenModeCommands::SCOPE);
        ShellRegion { common: Common::new(), props, shell: None, mode_set: None, senses_surface: false }
    }

    /// The content of a shell shown inside a region of another window: it fills its room, answers the screen-mode
    /// commands of its own scope and never frames or resizes the OS window.
    pub fn embedded() -> Self {
        let props =
            ContainerProps::new().sizing(crate::contract::Sizing::Fill).surface(SurfaceKind::Background).command_scope(ScreenModeCommands::SCOPE);
        ShellRegion { common: Common::new(), props, shell: None, mode_set: None, senses_surface: false }
    }

    /// An inline region that fills its room and sets `shell` for everything inside it.
    pub fn scope(shell: ShellCx) -> Self {
        let props = ContainerProps::new().sizing(crate::contract::Sizing::Fill).command_scope(crate::runtime::commands::CommandScope::APP);
        ShellRegion { common: Common::new(), props, shell: Some((shell, None)), mode_set: None, senses_surface: false }
    }

    /// The scope's popups, and the popups opened from them, stay inside `rect`.
    fn popups(mut self, rect: Rect) -> Self {
        if let Some((_, popups)) = &mut self.shell {
            *popups = Some(rect);
        }
        self
    }

    /// A pane of the window; it fills the room its edge gives it. A top or bottom pane lays its content along its edge
    /// on one line at the middle of its height, so a bar in it fills that height and centres its members there; a
    /// start, end or centre pane stacks its content.
    pub fn pane(pane: ShellPane) -> Self {
        let layout = if matches!(pane.edge, PaneEdge::Top | PaneEdge::Bottom) { BodyLayout::row() } else { BodyLayout::stack() };
        let props = ContainerProps::new().mount(Mount::Pane(pane)).focus(FocusPolicy::PANE).layout(layout);
        ShellRegion { common: Common::new(), props, shell: None, mode_set: None, senses_surface: false }
    }

    /// A bar or rail pane at an edge of the window: it paints the chrome surface over the whole pane and a divider
    /// along its inner edge. A bar or rail inside that is smaller than the pane leaves no band of the backdrop, and the
    /// pane meets the body at the rule.
    pub fn chrome(pane: ShellPane) -> Self {
        let mut region = Self::pane(pane);
        region.props = region.props.surface(SurfaceKind::Chrome).edge_rule(true);
        region
    }
}

impl Component for ShellRegion {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ShellRegionStyle;
    type Output = ShellRegionOutput;

    fn style(_: &StyleCx<'_>, _: ShellRegionStyleKey) -> ShellRegionStyle {
        ShellRegionStyle::default()
    }

    fn sense(&self) -> Sense {
        if self.senses_surface { Sense::SURFACE } else { Sense::empty() }
    }

    fn content_env(&self, _: &Cx<'_, Self>, values: &mut AppValues) {
        if let Some((shell, popups)) = &self.shell {
            shell.put(values);
            values.set(PopupRegion::new(*popups));
        }
        if let Some(set) = self.mode_set {
            values.set(set);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ShellRegionOutput {
        ShellRegionOutput { response: cx.response.clone() }
    }
}

impl Container for ShellRegion {}

/// One bar or rail with the edge it sits on and the closure that fills it.
struct Placed<'a, S> {
    slot: Slot,
    edge: PaneEdge,
    /// The size of a slot that sets none: the title bar for the title, else the edge's default.
    title: bool,
    /// The options bar: it follows the place, and it is the options bar's height at every edge.
    options: bool,
    /// The slot's name in the shell's words, for a slot of the canvas preset.
    named: Option<fn(&ShellWords) -> Label>,
    add: SlotFn<'a, S>,
}

impl<S> Placed<'_, S> {
    /// The order slots claim space in: the title and top bars, the bottom bars, the start rails, the end rails.
    fn rank(&self) -> u8 {
        match self.edge {
            PaneEdge::Top => 0,
            PaneEdge::Bottom => 1,
            PaneEdge::Start => 2,
            _ => 3,
        }
    }

    /// The default size of the slot's edge: the title bar for the title, else the options bar, the status bar or the
    /// rail width.
    fn edge_size(&self) -> SlotSize {
        match self.edge {
            PaneEdge::Top if self.title => SlotSize::TitleBar,
            PaneEdge::Top => SlotSize::OptionsBar,
            PaneEdge::Bottom if self.options => SlotSize::OptionsBar,
            PaneEdge::Bottom => SlotSize::StatusBar,
            _ => SlotSize::RailWidth,
        }
    }

    /// Whether the slot is as large as its content ([`SlotSize::Content`]).
    fn sized_by_content(&self) -> bool {
        self.slot.slot_size() == Some(SlotSize::Content)
    }

    /// Whether the slot's extent runs along x: a start or end slot.
    fn along_x(&self) -> bool {
        matches!(self.edge, PaneEdge::Start | PaneEdge::End)
    }

    /// The slot's extent along its edge: its size, or the default of its edge. A slot sized by its content takes the
    /// extent `measured` holds for it, the default of its edge until it has one.
    fn extent(&self, tokens: &UiTokens, measured: &[(SlotKey, f32)]) -> f32 {
        match self.slot.slot_size() {
            Some(SlotSize::Content) => measured
                .iter()
                .find(|(key, _)| *key == self.slot.key())
                .map(|(_, extent)| *extent)
                .unwrap_or_else(|| self.edge_size().resolve(tokens)),
            Some(size) => size.resolve(tokens),
            None => self.edge_size().resolve(tokens),
        }
    }
}

/// How far a content slot's measure may move before the shell lays the window out again: less than a pixel at any
/// scale.
const MEASURE_SLACK: f32 = 0.5;

/// Writes the extents content slots took this pass (`now`, the last of a key winning) into `kept`, and says whether any
/// moved by more than [`MEASURE_SLACK`] from the one it was laid out at, or is new. A slot not shown this pass keeps
/// its extent.
fn remeasure(kept: &mut Vec<(SlotKey, f32)>, now: &[(SlotKey, f32)]) -> bool {
    let mut moved = false;
    for &(key, took) in now {
        match kept.iter_mut().find(|(kept_key, _)| *kept_key == key) {
            Some((_, extent)) => {
                if (*extent - took).abs() > MEASURE_SLACK {
                    *extent = took;
                    moved = true;
                }
            }
            None => {
                kept.push((key, took));
                moved = true;
            }
        }
    }
    moved
}

/// Whether `slot` covers the body instead of taking room from it: every slot of a body-only pass but the screen bar,
/// which Focus and Immersive lay out above the body.
fn covers(slot: &Slot, only: bool) -> bool {
    only && !slot.is_screen_bar()
}

/// What the shell keeps between passes, in the root container's slot.
#[derive(Clone, Debug, Default)]
struct ShellState {
    reveal: Reveal,
    screen: ScreenModeFollow,
    /// The extent each slot sized by its content ([`SlotSize::Content`]) took when it last showed, by key.
    measured: Vec<(SlotKey, f32)>,
}

/// What a pass of the shell leaves.
#[derive(Clone, Debug)]
struct Parts {
    regions: ShellRegions,
    revealed: bool,
    panels_hidden: bool,
    mode: ScreenMode,
    dropped: bool,
}

/// What [`AppShell::show`] drew.
#[derive(Debug)]
#[non_exhaustive]
pub struct AppShellOutput {
    /// The response of the window's root region.
    pub response: Response,
    /// The body's rect and the rects of the shown slots, in the order drawn.
    pub regions: ShellRegions,
    /// Slots are shown by a reveal on this pass (Tab or edge).
    pub revealed: bool,
    /// The panels are hidden while the tools and bars stay (Shift+Tab). A body that hosts panels reads it with
    /// `ShellCx::panels_hidden`.
    pub panels_hidden: bool,
    /// The screen mode after this pass.
    pub screen_mode: ScreenMode,
    /// Whether the drop-down slot ([`Slot::drops`]) shows after this pass. A host that binds `dropped` with
    /// `Bind::given` writes it back, so a press in the body or a mode change that cleared it holds.
    pub dropped: bool,
}

/// What a window hides, as the last pass of its shell left it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChromeState {
    /// The user hid every panel, tool and bar.
    pub hidden: bool,
    /// The panels are hidden while the tools and bars stay.
    pub panels_hidden: bool,
    /// The edge whose slots a reveal shows, when one is on.
    pub revealed: Option<RevealEdge>,
}

/// The hiding of the window `viewport` shows, as the last pass of its shell left it, read from the shell's one kept
/// reveal. It serves code that runs outside the shell's pass and has no [`AppShellOutput`]: the window commands'
/// handlers and the menu check refresh. A viewport with no shell reads `ChromeState::default()`.
pub fn chrome_state(ctx: &Context, viewport: ViewportId) -> ChromeState {
    shell_root(ctx, viewport).map_or_else(ChromeState::default, |root| {
        // A container keeps its session memory under `sub(id, "body")`, where the shell's body keeps its state.
        memory::with_session::<ShellState, _>(ctx, sub(root, "body"), |state| ChromeState::of(&state.reveal))
    })
}

impl ChromeState {
    /// What `reveal` says: the one source of a pass's output and of [`chrome_state`].
    fn of(reveal: &Reveal) -> Self {
        ChromeState { hidden: reveal.hidden(), panels_hidden: reveal.panels_hidden(), revealed: reveal.revealed_edge() }
    }
}

impl Outcome for AppShellOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The window skeleton.
///
/// - **Order.** Slots claim space in this order, which is also the F6 order: the title, then the `top` slots in call
///   order from the window's top edge inward; the `bottom` slots from the bottom edge inward; then `start`, then `end`;
///   then the body in what is left. Start and end swap sides in a right-to-left window; top and bottom do not. A theme or
///   size never changes the order.
/// - **Extent.** A slot is as high (top, bottom) or wide (start, end) as its [`SlotSize`] says, times its columns for a
///   rail. Without a size it takes the default of its edge. The shell lays out every shown slot and the body before any
///   slot runs, so `BodyCx::shell()` holds this pass's rects.
/// - **Screen mode.** The `screen_mode` binding holds the window's mode. The window's `screenMode.*` commands
///   (F and Shift+F cycle) change it, and a change to or from full screen is carried out through `WindowHost::act`.
///   In `FullScreen` no slot shows but those built with [`Slot::reveal`], during a reveal.
/// - **Reveal.** In body-only mode Tab (or the `window.toggle_panels` command) shows the revealing slots until the next
///   Tab; the pointer at the top, start or end edge shows that edge's slots, unless a button is held or a popup is
///   open or `edge_reveal` is off. Revealed slots cover the body and leave its rect as it is. A screen-mode change ends
///   a reveal. The shell takes its own window's Tab toggles from the frame state in the pass that records them: in
///   normal mode Tab hides every slot and panel and shows them again.
/// - **Canvas preset.** [`title_bar_if`](Self::title_bar_if), [`options_bar`](Self::options_bar),
///   and [`status`](Self::status) add the slots of a document editor under the keys of [`slots`](super::slots),
///   named by `ShellWords`; the toolbar is a panel of the dock, not a slot; each takes the host's `visible` flag, passed every frame.
/// - **Shell data.** Slots and the body read the skeleton with `BodyCx::shell()`.
/// - **App state.** `S` is the state the slots and the body edit; [`show`](Self::show) hands each of them `&mut S` in
///   turn, so a title bar, a rail and a body that edit one document share it without interior mutability. An app with
///   no shared state uses `S = ()`.
/// - **Dialogs.** The window owns one dialog host, shared by every body in it (`ShellDialogsCx::dialogs`). The shell runs it
///   after the body in each pass, so dialogs show over the window.
/// - **Embedded.** [`embedded`](Self::embedded) shows a shell inside a region of another window. Its screen modes
///   hide and reveal its slots in that region and never ask the OS window for anything. With
///   [`content_height`](Self::content_height) it is as tall as its body's content and its top and bottom slots.
#[must_use = "show the shell with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(AppShell<'static, ()>))]
pub struct AppShell<'a, S> {
    window: WindowKey,
    bare: bool,
    embedded: bool,
    /// An embedded shell as wide as the room of the flow it is shown in and as tall as its body's content and its top
    /// and bottom slots, from the measure its body keeps while its inputs hold; a body that fills takes at most this
    /// room. Shown with [`show`](Self::show), it takes that room in the flow.
    #[prop(name = "content_height", quantity = "room the host's flow offers, in points")]
    content_room: Option<f32>,
    /// The screen mode. Without a binding the shell keeps it, starting at `Standard`.
    #[prop]
    screen_mode: Bind<'a, ScreenMode>,
    /// Which screen modes the F key cycles through. The shell publishes it to its regions.
    #[prop]
    mode_set: ScreenModeSet,
    /// Whether the drop-down slot of a Focus window is shown. A body press edge or a mode change clears it.
    #[prop]
    dropped: Bind<'a, bool>,
    /// The bottom alignment of notices in the body; without one they use its trailing bottom corner.
    #[prop]
    toast_alignment: Option<egui::Align2>,
    /// Whether the Tab reveal and the edge reveal work in body-only mode. Off for a presentation.
    #[prop]
    allow_reveal: bool,
    /// Whether the pointer at the top, start or end edge reveals the hidden bars in body-only mode.
    #[prop]
    edge_reveal: bool,
    /// The host's flag for the document tabs, read back through `ShellCx::tabs_visible`.
    #[prop]
    document_tabs: bool,
    /// Whether the title slot mirrors with the window's macOS traffic lights when AppKit lays them out from the right
    /// end (`MacInset::lights_right`: a right-to-left system language, whatever the app's language): the slot's flow
    /// runs from the right, so what starts the title bar sits beside the lights and its document tabs read from the
    /// right, their overflow at the other end. Text keeps the interface's direction. Off, the bar keeps its order and
    /// only the lights' room moves to the right end.
    #[prop]
    title_follows_lights: bool,
    /// The window's title: the shell sends `WindowAction::SetTitle` every pass and the window host drops a title equal
    /// to the last one it set. Not in an embedded shell.
    #[prop(into)]
    window_title: Option<String>,
    /// The name of the body region; the translated "Document" without it.
    #[prop(into)]
    body_label: Option<Label>,
    title: Option<Placed<'a, S>>,
    slots: Vec<Placed<'a, S>>,
    /// The options bar follows the bound place: the top or the bottom slot, or, while floating, a movable modeless layer over
    /// the body that takes no room. A drag of a [`BarGrip`](super::BarGrip) writes the place on the release. Without the
    /// call the bar is the top slot, as [`options_bar`](Self::options_bar) shows it.
    #[prop(name = "options_place")]
    place: Bind<'a, OptionsBarPlace>,
    /// The body, shown in what the slots leave.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>, &mut S)")]
    body: Option<SlotBody<'a, S>>,
    /// The full-window page and its command scope, set while it shows.
    page: Option<(CommandScope, SlotBody<'a, S>)>,
}

impl<'a, S> AppShell<'a, S> {
    /// A shell for `window` with no slots.
    pub fn new(window: WindowKey) -> Self {
        AppShell {
            window,
            bare: false,
            embedded: false,
            content_room: None,
            screen_mode: Bind::kept(),
            mode_set: ScreenModeSet::default(),
            dropped: Bind::kept(),
            toast_alignment: None,
            allow_reveal: true,
            edge_reveal: true,
            document_tabs: true,
            title_follows_lights: false,
            window_title: None,
            body_label: None,
            title: None,
            slots: Vec::new(),
            place: Bind::kept(),
            body: None,
            page: None,
        }
    }

    /// A window with no chrome (a torn-off document window): only a body, with the same window scope. Slots added to
    /// it are not shown.
    pub fn bare(window: WindowKey) -> Self {
        AppShell { bare: true, ..Self::new(window) }
    }

    /// A shell shown inside a region of another window (a story cell, a preview). Its screen modes act inside its
    /// region: full screen hides and reveals its slots there and never asks the OS window for anything. `window` keys
    /// its state, its Tab toggle and its dialog host, so two embedded shells need two keys.
    pub fn embedded(window: WindowKey) -> Self {
        AppShell { embedded: true, ..Self::new(window) }
    }

    /// The title bar slot, the first top slot, keyed `"title"`: visible, and revealing in body-only mode. The title bar
    /// types fill it.
    pub fn title_bar(self, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.title(Slot::new(keys::TITLE).reveal(), add)
    }

    /// The title slot with a slot of your own, for a title that follows the host's menu bar flag.
    pub fn title(mut self, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.title = Some(Placed { slot, edge: PaneEdge::Top, title: true, options: false, named: None, add: Box::new(add) });
        self
    }

    /// The title bar slot with the host's menu bar flag: `visible` shows it, and it reveals in body-only mode only when
    /// it is on.
    pub fn title_bar_if(self, visible: bool, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.title(Slot::new(keys::TITLE).visible(visible).reveal_if(visible), add)
    }

    /// The options bar, a top slot keyed `"options"` that reveals in body-only mode.
    pub fn options_bar(mut self, visible: bool, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        let slot = Slot::new(keys::OPTIONS).visible(visible).reveal();
        self.slots.push(Placed {
            slot,
            edge: PaneEdge::Top,
            title: false,
            options: true,
            named: Some(|words| words.options.clone()),
            add: Box::new(add),
        });
        self
    }

    /// The status bar, a bottom slot keyed `"status"` that never reveals.
    pub fn status(mut self, visible: bool, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        let slot = Slot::new(keys::STATUS).visible(visible);
        self.slots.push(Placed {
            slot,
            edge: PaneEdge::Bottom,
            title: false,
            options: false,
            named: Some(|words| words.status.clone()),
            add: Box::new(add),
        });
        self
    }

    /// A top slot, below the ones added before it.
    pub fn top(self, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.place(PaneEdge::Top, slot, add)
    }

    /// A bottom slot, above the ones added before it.
    pub fn bottom(self, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.place(PaneEdge::Bottom, slot, add)
    }

    /// A slot at the reading start, inside the ones added before it.
    pub fn start(self, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.place(PaneEdge::Start, slot, add)
    }

    /// A page that, while `shown`, fills the window in place of the slots and the body (a backstage, a print page). A
    /// move band with the window's ends (the traffic lights' room or the caption buttons) lies over its top; the page
    /// reads the room they take with `ShellCx::page_band` and keeps its controls clear of the band. The page's region is
    /// the instance of `scope`; dialogs run over it, and with nothing focused the page takes keys beside the window's,
    /// so `scope`'s commands get their default keys.
    pub fn page(mut self, shown: bool, scope: CommandScope, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.page = shown.then(|| (scope, SlotBody::new(add)));
        self
    }

    /// A slot at the reading end, inside the ones added before it.
    pub fn end(self, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.place(PaneEdge::End, slot, add)
    }

    fn place(mut self, edge: PaneEdge, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.slots.push(Placed { slot, edge, title: false, options: false, named: None, add: Box::new(add) });
        self
    }

    /// Lays out the slots, then the body in what is left, runs the window's dialogs over them and shows the toasts of
    /// the window. Each slot and the body get `app`, one after the other: the slots that do not cover the body, then the
    /// body, then the slots a reveal lays over it.
    pub fn show(self, ui: &mut Ui, app: &mut S) -> AppShellOutput {
        self.show_in(ui, None, app)
    }

    /// Shows the shell at `rect` and takes that rect's room in the flow of `ui`, as [`show`](Self::show) does
    /// otherwise: for an embedded shell whose host sized it (a story preview sized to its story).
    pub fn show_at(self, ui: &mut Ui, rect: Rect, app: &mut S) -> AppShellOutput {
        self.show_in(ui, Some(rect), app)
    }

    fn show_in(self, ui: &mut Ui, at: Option<Rect>, app: &mut S) -> AppShellOutput {
        let AppShell {
            window,
            bare,
            embedded,
            content_room,
            mut screen_mode,
            mut place,
            mode_set,
            mut dropped,
            toast_alignment,
            allow_reveal,
            edge_reveal,
            document_tabs,
            title_follows_lights,
            window_title,
            body_label,
            title,
            slots,
            mut body,
            mut page,
        } = self;
        let embedded = embedded || content_room.is_some();
        if let Some(title) = window_title
            && !embedded
            && !ui.is_sizing_pass()
        {
            // The window host skips a title equal to the one it last set for the viewport.
            let ctx = ui.ctx();
            WindowHost::of(ctx).act(pass::current_viewport(ctx), WindowAction::SetTitle(title));
        }
        let env = Env::of(ui);
        let outer = env.get::<ShellEntry>().0;
        let words = env.words::<ShellWords>();
        let tokens = env.tokens.clone();
        let body_label = body_label.unwrap_or_else(|| words.document.clone());
        let held = match place.reborrow().get() {
            Some(Mixed::Same(held)) => held,
            _ => OptionsBarPlace::default(),
        };
        let floating = held.floating;
        let mut slots: Vec<Placed<'a, S>> = title.into_iter().chain(slots).collect();
        for placed in slots.iter_mut().filter(|placed| placed.options) {
            placed.edge = if held.bottom { PaneEdge::Bottom } else { PaneEdge::Top };
        }
        slots.sort_by_key(Placed::rank);

        let mut root = if embedded { ShellRegion::embedded() } else { ShellRegion::window() }.id_salt(window.as_str()).label(words.window.clone());
        if let Some(room) = content_room {
            root.props = root.props.room_height(room);
            root = root.sizing(Sizing::FillWidth);
        }
        let shell_body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let ctx = ui.ctx().clone();
            let viewport = pass::current_viewport(&ctx);
            let mut kept = cx.keep::<ShellState>().clone();

            let name: Arc<str> = words.screen_mode.as_str().into();
            let (mode, changed) = kept.screen.follow(&ctx, cx, screen_mode.reborrow(), Arc::clone(&name), !embedded, mode_set);
            if changed {
                kept.reveal.clear();
            }
            let dropped_before = matches!(dropped.reborrow().read(cx), Some(Mixed::Same(true)));
            // A mode change folds the drop-down slot away, whoever changed the mode.
            let mut is_dropped = dropped_before && !changed;

            let host_only = mode.body_only();
            if let Some(root) = cx.command_chain().innermost(ScreenModeCommands::SCOPE.name()) {
                note_shell(&ctx, viewport, &window, root);
            }
            // A page takes no part in the reveal: the toggles are taken, so they are not replayed once it closes.
            let paged = page.is_some();
            let toggles = take_chrome_toggles(&ctx, viewport, &window);
            if !paged {
                kept.reveal.apply(&toggles, host_only, allow_reveal);
            }
            let only = kept.reveal.only(host_only);
            let pointer = pointer::frame(&ctx);
            let frozen = !pointer.buttons.is_empty() || any_open(&ctx, OpenFilter::Keys);
            let rtl = cx.env.direction().is_rtl();
            if !paged {
                kept.reveal.track(Tracking::new(only, allow_reveal, edge_reveal), Aim::new(pointer.pos, frozen, cx.rect, rtl));
            }

            // Every shown slot's rect and the body's, before any slot runs. Slots that a reveal shows in body-only mode
            // cover the body; the screen bar takes its room from it.
            let plan = |is_dropped: bool| {
                let visible: Vec<bool> = slots
                    .iter()
                    .map(|placed| !bare && !paged && kept.reveal.shows(&placed.slot, placed.edge, only, allow_reveal, mode, is_dropped))
                    .collect();
                let claims: Vec<Claim> = slots
                    .iter()
                    .zip(&visible)
                    // A floating options bar is a layer over the body and takes no room.
                    .filter(|(placed, shown)| **shown && !(floating && placed.options))
                    .map(|(placed, _)| Claim {
                        key: placed.slot.key(),
                        edge: placed.edge,
                        extent: placed.extent(&tokens, &kept.measured),
                        overlay: covers(&placed.slot, only),
                    })
                    .collect();
                let regions = layout(cx.rect, &claims, rtl);
                (visible, claims, regions)
            };
            let (mut visible, mut claims, mut regions) = plan(is_dropped);
            // A press in the body folds the drop-down slot away; a press in the slot itself, which lies over the body,
            // is the slot's own. Only the pass that receives the press sees it: a rerun pass of the frame has none.
            if is_dropped
                && !paged
                && let Some(press) = pointer.press
                && regions.body.contains(press.origin)
                && !slots
                    .iter()
                    .zip(&visible)
                    .filter(|(placed, shown)| **shown && placed.slot.drops_down())
                    .any(|(placed, _)| regions.slots.iter().any(|(key, rect)| *key == placed.slot.key() && rect.contains(press.origin)))
            {
                is_dropped = false;
                (visible, claims, regions) = plan(is_dropped);
            }
            if is_dropped != dropped_before {
                let event = EditEvent::set(Mixed::Same(dropped_before), is_dropped, Arc::from(""), None);
                let _ = dropped.reborrow().write(cx, &event);
            }
            let dropped_shown = slots.iter().zip(&visible).any(|(placed, shown)| *shown && placed.slot.drops_down());

            // A pass with no title bar has no move band: the OS moves the window from nowhere in it. A page's band is
            // the move band of its pass, and a modal over the page keeps it through the window's caption regions.
            if !embedded && !paged && !slots.iter().zip(&visible).any(|(placed, shown)| placed.title && *shown) {
                super::title_bar::set_window_movable(&ctx, false);
            }

            let chrome = ChromeState::of(&kept.reveal);
            let tabs = mode.tabs_visible() && document_tabs;
            // The title slot reads from the right while the lights sit there; a right-to-left window already does.
            let mirror_title =
                title_follows_lights && !embedded && !rtl && matches!(WindowHost::of(&ctx).frame(), super::Frame::Inset(inset) if inset.lights_right);
            let mut shell = ShellCx::new(window.clone(), if only { BodyMode::Only } else { BodyMode::Normal }, regions.clone())
                .with_tabs(tabs)
                .with_tab_reveal(kept.reveal.tab())
                .with_panels_hidden(chrome.panels_hidden)
                .with_reveals_key(sub(cx.id, "reveals"));
            // The room an embedded shell shows: its rect less what a scroll or a parent clips. Its dialogs are contained in
            // it.
            let region = cx.rect.intersect(ui.clip_rect());
            // An embedded shell scrolled or clipped out of sight shows nothing in this pass: its region has no room for
            // its body or its dialogs. A measuring run measures it wherever it is.
            let unseen = embedded && !region.is_positive();
            if embedded {
                shell = shell.with_region(region);
            }
            for edge in [RevealEdge::Top, RevealEdge::Start, RevealEdge::End] {
                shell = shell.with_edge_reveal(edge, kept.reveal.edge(edge));
            }
            if paged {
                shell = shell.with_page_band(super::title_bar::page_band_room(cx, &tokens, cx.rect, !embedded));
            }
            // Popups stay above every bottom slot the pass shows, the covering ones included: they are painted there.
            let shell_rect = if embedded { region } else { cx.rect };
            let floor = claims
                .iter()
                .filter(|claim| claim.edge == PaneEdge::Bottom)
                .filter_map(|claim| regions.slots.iter().find(|(key, _)| *key == claim.key).map(|(_, rect)| rect.top()))
                .fold(shell_rect.bottom(), f32::min);
            let popups = Rect::from_min_max(shell_rect.min, pos2(shell_rect.right(), floor.max(shell_rect.top())));
            let bar_extent = slots.iter().find(|placed| placed.options).map(|placed| placed.extent(&tokens, &kept.measured));
            let mut landed = None;
            let mut floated: Vec<(RevealEdge, Rect)> = Vec::new();

            let rect_of = |key| regions.slots.iter().find(|(shown, _)| *shown == key).map_or(Rect::ZERO, |(_, rect)| *rect);
            // The room a content slot can grow into: from its outer edge to the far edge of what the slots before it
            // leave. Its content runs there, so it measures at its own extent, never at the extent it was laid out at.
            let window_rect = cx.rect;
            let grow_room = |key: SlotKey| {
                let Some(at) = claims.iter().position(|claim| claim.key == key) else { return Rect::ZERO };
                let mut claimed = claims[..=at].to_vec();
                claimed[at].extent = f32::INFINITY;
                layout(window_rect, &claimed, rtl).slots.get(at).map_or(Rect::ZERO, |(_, rect)| *rect)
            };
            // The extents the content slots were laid out at, and what their content took in this pass.
            let measured_before = kept.measured.clone();
            let mut measured_now: Vec<(SlotKey, f32)> = Vec::new();
            let scope = shell.clone();
            let mut scope_region = ShellRegion::scope(shell).with_mode_set(mode_set).id_salt("shell").popups(popups);
            if content_room.is_some() {
                scope_region = scope_region.sizing(Sizing::FillWidth);
            }
            scope_region.show(ui, |ui, _| {
                if unseen && !ui.is_sizing_pass() {
                    return;
                }
                if let Some((scope, SlotBody(page))) = page.as_mut() {
                    let pane = ShellPane::new("page");
                    let pane = if content_room.is_some() { pane.at_content_extent(regions.body) } else { pane.at(regions.body) };
                    let mut region = ShellRegion::pane(pane).label(body_label.clone());
                    region.props = region.props.command_scope(*scope);
                    let shown = region.show(ui, |ui, bcx| page(ui, bcx, app));
                    // With nothing focused the page takes keys beside the window's, as a modal's root does; the window's
                    // shell record stays.
                    // PLAN: assumed `commands::note_page_root(ctx: &Context, viewport: ViewportId, id: Id)` (chrome-w6)
                    crate::runtime::commands::note_page_root(&ctx, viewport, shown.output.response.id);
                    PageBand::new().show_over_page(ui);
                    if let Some(dialogs) = WindowDialogs::existing(ui.ctx(), &window) {
                        dialogs.run(ui);
                    }
                    return;
                }
                let mut show_slots = |ui: &mut Ui, covering: bool, app: &mut S| {
                    for (placed, shown) in slots.iter_mut().zip(&visible) {
                        let floats = floating && placed.options;
                        // A floating bar is a layer over the body: it shows with the covering slots in every mode.
                        if !*shown || if floats { !covering } else { covers(&placed.slot, only) != covering } {
                            continue;
                        }
                        let mirrored = placed.title && mirror_title;
                        let by_content = placed.sized_by_content();
                        let along_x = placed.along_x();
                        let extent = placed.extent(&tokens, &measured_before);
                        let Placed { slot, edge, named, add, .. } = placed;
                        let label = slot.name().cloned().or_else(|| named.map(|name| name(&words)));
                        if floats {
                            let corner = held.corner(regions.body, tokens.space.gap.section);
                            let overlay = Overlay::floating().modeless().bounds(regions.body).placed_at(corner).body_scroll(false);
                            let layer = FloatingBar::new(overlay, label).id_salt(("slot", slot.key())).show(ui, |ui, bcx| add(ui, bcx, app));
                            floated.extend(edge.reveal_edge().map(|edge| (edge, layer.output.response.rect)));
                            continue;
                        }
                        // A slot sized by its content is as large as its content from its outer edge, in the room it
                        // can grow into; any other slot takes its extent and fills its rect.
                        let pane = ShellPane::new(slot.key()).edge(*edge);
                        let pane =
                            if by_content { pane.at_content_extent(grow_room(slot.key())) } else { pane.extent(extent).at(rect_of(slot.key())) };
                        // Every slot is a chrome pane: it paints the surface under its bar or rail and the rule toward
                        // the body.
                        let mut region = ShellRegion::chrome(pane).id_salt(("slot", slot.key()));
                        // A slot sized by its content stacks it from the slot's outer edge: a top pane's own layout
                        // centres one line in its height, which would lay a strip and the row under it side by side.
                        if by_content {
                            region.props = region.props.layout(BodyLayout::stack());
                        }
                        if let Some(label) = label {
                            region = region.label(label);
                        }
                        let key = slot.key();
                        // The slot runs every pass; the app memoizes inside it.
                        if !by_content {
                            region.show(ui, |ui, bcx| in_lights_flow(ui, mirrored, |ui| add(ui, bcx, app)));
                            continue;
                        }
                        // The slot's extent along its edge is the pane's: its content's, with what the pane keeps
                        // around its body (its rule). A top or bottom slot's content fills its width and is as tall
                        // as it is; a rail's is as wide as it is. Only the slot's own real run records it: a run in a
                        // sizing pass (the scope's measuring run, which alone runs for a shell out of sight) is not
                        // what shows.
                        let shown = region.show(ui, |ui, bcx| {
                            let stack = if along_x { Stack::new() } else { Stack::new().fill_width() };
                            stack.show(ui, |ui, _| in_lights_flow(ui, mirrored, |ui| add(ui, bcx, app)));
                            !ui.is_sizing_pass()
                        });
                        let pane = shown.output.response.rect;
                        let took = if along_x { pane.width() } else { pane.height() };
                        if shown.inner == Some(true) && took.is_finite() && took > 0.0 {
                            measured_now.push((key, took));
                        }
                    }
                };
                show_slots(ui, false, app);
                // Behind its body a shell paints the scrim where the user hid the chrome of a mode that shows it, else the
                // mode's surround (Focus's black). Otherwise the body paints what it shows, and the gaps between its parts
                // (the dock's gutters) show the window's background; Immersive's body fills the room with its own canvas.
                let pane = ShellPane::new("body");
                let pane = if content_room.is_some() { pane.at_content_extent(regions.body) } else { pane.at(regions.body) };
                let mut center = ShellRegion::pane(pane).label(body_label.clone());
                let surround = if only && !host_only { Some(SurfaceKind::Scrim) } else { mode.surround() };
                if let Some(surface) = surround {
                    center = center.surface(surface);
                }
                let shown = center.show(ui, |ui, bcx| {
                    if let Some(SlotBody(body)) = body.as_mut() {
                        body(ui, bcx, app);
                    }
                });
                // A shell as tall as its content reaches past its body by the bottom slots, which the layout puts there.
                if content_room.is_some() {
                    let bottom: f32 = claims.iter().filter(|claim| claim.edge == PaneEdge::Bottom && !claim.overlay).map(|claim| claim.extent).sum();
                    crate::runtime::egui::alloc::reach_down_to(ui, shown.output.response.rect.max.y + bottom);
                }
                if let Some(bar) = bar_extent.filter(|_| !bare) {
                    let target = PlaceTarget { body: regions.body, bar, held };
                    let zone = DropZone::new(ui.layer_id(), regions.body);
                    landed = zone_target::<OptionsBarDrag, _>(ui, ui.scope_id().with("options_place"), zone, &target);
                }
                show_slots(ui, true, app);
                if let Some(dialogs) = WindowDialogs::existing(ui.ctx(), &window) {
                    dialogs.run(ui);
                }
            });
            // The place is written only from a taken drop; a drop that leaves it as it is writes nothing.
            if let Some(landing) = landed.filter(|landing| landing.place != held) {
                let event = EditEvent::set(Mixed::Same(held), landing.place, Arc::from(words.move_bar.as_str()), None);
                let _ = place.reborrow().write(cx, &event);
            }
            // A content slot whose content took another extent than it was laid out at lays the window out again, so the
            // slot and the body under it take their places in this frame: the slot never clips its content, nor leaves
            // a band of its surface under it. Only a real run records: a measuring run's extent is not what shows.
            let real = !measured_now.is_empty() && !ui.is_sizing_pass() && crate::runtime::measure::effect(&ctx, "shell::slot_extent");
            if real && remeasure(&mut kept.measured, &measured_now) {
                if crate::runtime::frame::discardable(&ctx) {
                    crate::runtime::frame::discard_pass(&ctx, "craft_ui: a content slot's extent changed");
                } else {
                    crate::runtime::frame::request_repaint(&ctx);
                }
            }
            // What shows here counts for the shell around this one too, so a host reads it from its own shell.
            if scope.dialogs_shown() > 0
                && !ui.is_sizing_pass()
                && let Some(outer) = &outer
            {
                outer.note_dialog();
            }

            // The window's toasts sit in its body; an embedded shell's in the part of it in view.
            let body = if embedded { regions.body.intersect(region) } else { regions.body };
            ToastStack::show_at(ui, &window, body, toast_alignment);

            let mut edges: Vec<(RevealEdge, Rect)> = regions
                .slots
                .iter()
                .filter_map(|(key, rect)| {
                    slots.iter().find(|placed| placed.slot.key() == *key).and_then(|placed| placed.edge.reveal_edge()).map(|edge| (edge, *rect))
                })
                .collect();
            edges.extend(scope.revealed_rects(ui.ctx()));
            edges.extend(floated);
            kept.reveal.record(edges);
            // The screen bar and the drop-down slot show by the mode, never by a reveal.
            let revealed =
                only && slots.iter().zip(&visible).any(|(placed, shown)| *shown && !placed.slot.is_screen_bar() && !placed.slot.drops_down());
            *cx.keep::<ShellState>() = kept;
            Parts { regions, revealed, panels_hidden: chrome.panels_hidden, mode, dropped: dropped_shown }
        };
        let shown = match at {
            Some(rect) => root.show_at_taking_room(ui, rect, shell_body),
            None => root.show(ui, shell_body),
        };
        let response = shown.output.response;
        match shown.inner {
            Some(parts) => AppShellOutput {
                response,
                regions: parts.regions,
                revealed: parts.revealed,
                panels_hidden: parts.panels_hidden,
                screen_mode: parts.mode,
                dropped: parts.dropped,
            },
            None => AppShellOutput {
                response,
                regions: ShellRegions::new(Rect::ZERO),
                revealed: false,
                panels_hidden: false,
                screen_mode: ScreenMode::default(),
                dropped: false,
            },
        }
    }
}

/// Runs `body` with its flow from the right when `mirrored` (the title slot following traffic lights at the right end),
/// else as it is. Only the flow turns: text keeps the interface's direction.
fn in_lights_flow<R>(ui: &mut Ui, mirrored: bool, body: impl FnOnce(&mut Ui) -> R) -> R {
    if !mirrored {
        return body(ui);
    }
    // The slot's room is the bar's own: a bar that fills takes it all, whatever the flow of the body around it.
    let room = ui.max_rect();
    crate::runtime::env::with_env(
        ui,
        |overrides| overrides.flow = Some(craft_i18n::Direction::Rtl),
        |ui| crate::runtime::env::in_room(ui, room, body),
    )
}
