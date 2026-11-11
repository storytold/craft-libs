//! `AppShell`: the window skeleton. An ordered stack of top bars starting with the title bar, bottom bars, start and end
//! rails, and the body in what is left. Each bar and rail is a container on `Mount::Pane`, so F6 pane focus and the
//! name of each region come from the pipeline.

mod dialogs;
#[cfg(feature = "testing")]
mod stories;

pub use self::dialogs::{ShellDialogsCx, WindowDialogs};
use super::canvas_shell::slots as keys;
use super::regions::{Claim, SlotSize, layout};
use super::reveal::{Aim, Reveal, Tracking};
use super::screen_mode::{ScreenMode, ScreenModeCommands, ScreenModeFollow};
use super::words::ShellWords;
use crate::contract::overlay::Resize;
use crate::contract::{
    Bind, BodyCx, Common, Component, Container, ContainerProps, Cx, Mount, NoValue, Outcome, PaneEdge, Props as _, Remember as _, Role, ShellPane,
    ShowContainer as _,
};
use crate::notices::ToastStack;
use crate::runtime::commands::{CommandAccess as _, CommandSet as _};
use crate::runtime::egui::pass;
use crate::runtime::env::{AppValues, BodyMode, Env, Label, RevealEdge, ShellCx, ShellEntry, ShellRegions, WindowKey};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::frame::{note_shell, take_chrome_toggles};
use crate::runtime::overlay::{OpenFilter, any_open};
use crate::runtime::pointer;
use crate::shell::Slot;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind, UiTokens};
use craft_behavior::{Metrics, Sense};
use egui::{Rect, Response, Ui};
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
    shell: Option<ShellCx>,
}

impl ShellRegion {
    /// The whole content of the window: its edges resize the window where the app draws the frame. It paints no
    /// surface of its own: the parts the shell places paint theirs, and a gap between them (a dock's gutter) shows the
    /// window's clear colour.
    pub fn window() -> Self {
        let props = ContainerProps::new().mount(Mount::window(Resize::GRIPS)).command_scope(ScreenModeCommands::SCOPE);
        ShellRegion { common: Common::new(), props, shell: None }
    }

    /// The content of a shell shown inside a region of another window: it fills its room, answers the screen-mode
    /// commands of its own scope and never frames or resizes the OS window.
    pub fn embedded() -> Self {
        let props =
            ContainerProps::new().sizing(crate::contract::Sizing::Fill).surface(SurfaceKind::Background).command_scope(ScreenModeCommands::SCOPE);
        ShellRegion { common: Common::new(), props, shell: None }
    }

    /// An inline region that fills its room and sets `shell` for everything inside it.
    pub fn scope(shell: ShellCx) -> Self {
        let props = ContainerProps::new().sizing(crate::contract::Sizing::Fill).command_scope(crate::runtime::commands::CommandScope::APP);
        ShellRegion { common: Common::new(), props, shell: Some(shell) }
    }

    /// A pane of the window; it fills the room its edge gives it.
    pub fn pane(pane: ShellPane) -> Self {
        let props = ContainerProps::new().mount(Mount::Pane(pane)).focus(FocusPolicy::PANE);
        ShellRegion { common: Common::new(), props, shell: None }
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
        Sense::empty()
    }

    fn content_env(&self, _: &Cx<'_, Self>, values: &mut AppValues) {
        if let Some(shell) = &self.shell {
            shell.put(values);
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

    /// The slot's extent along its edge: its size, or the default of its edge, with a rail's columns side by side.
    fn extent(&self, tokens: &UiTokens) -> f32 {
        let size = self.slot.slot_size().unwrap_or(match self.edge {
            PaneEdge::Top if self.title => SlotSize::TitleBar,
            PaneEdge::Top => SlotSize::OptionsBar,
            PaneEdge::Bottom => SlotSize::StatusBar,
            _ => SlotSize::RailWidth,
        });
        let columns = if matches!(self.edge, PaneEdge::Start | PaneEdge::End) { self.slot.column_count() } else { 1 };
        size.resolve(tokens) * columns as f32
    }
}

/// What the shell keeps between passes, in the root container's slot.
#[derive(Clone, Debug, Default)]
struct ShellState {
    reveal: Reveal,
    screen: ScreenModeFollow,
}

/// What a pass of the shell leaves.
#[derive(Clone, Debug)]
struct Parts {
    regions: ShellRegions,
    revealed: bool,
    panels_hidden: bool,
    mode: ScreenMode,
    dialog: bool,
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
    /// For an embedded shell: its body or its dialog host showed a container on this pass as a dialog, a modal, a
    /// full-window layer or an OS window, open or still fading out (`ShellCx::dialog_shown`), its own or one of an
    /// embedded shell inside it. Menus, popovers and tooltips do not count. A host that keeps such content closed until
    /// asked reads it from a measure of the body under a `ShellCx` of its own, before the first paint, and here for
    /// what the measure cannot reach: the body of an embedded shell inside, which runs in real runs only.
    pub dialog: bool,
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
///   [`tools`](Self::tools) and [`status`](Self::status) add the slots of a document editor under the keys of
///   [`slots`](super::slots), named by `ShellWords`; each takes the host's `visible` flag, passed every frame.
/// - **Shell data.** Slots and the body read the skeleton with `BodyCx::shell()`.
/// - **App state.** `S` is the state the slots and the body edit; [`show`](Self::show) hands each of them `&mut S` in
///   turn, so a title bar, a rail and a body that edit one document share it without interior mutability. An app with
///   no shared state uses `S = ()`.
/// - **Dialogs.** The window owns one dialog host, shared by every body in it (`ShellDialogsCx::dialogs`). The shell runs it
///   after the body in each pass, so dialogs show over the window.
/// - **Embedded.** [`embedded`](Self::embedded) shows a shell inside a region of another window. Its screen modes
///   hide and reveal its slots in that region and never ask the OS window for anything.
#[must_use = "show the shell with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(AppShell<'static, ()>))]
pub struct AppShell<'a, S> {
    window: WindowKey,
    bare: bool,
    embedded: bool,
    /// The screen mode. Without a binding the shell keeps it, starting at `Standard`.
    #[prop]
    screen_mode: Bind<'a, ScreenMode>,
    /// Whether the Tab reveal and the edge reveal work in body-only mode. Off for a presentation.
    #[prop]
    allow_reveal: bool,
    /// Whether the pointer at the top, start or end edge reveals the hidden bars in body-only mode.
    #[prop]
    edge_reveal: bool,
    /// The host's flag for the document tabs, read back through `ShellCx::tabs_visible`.
    #[prop]
    document_tabs: bool,
    /// The name of the body region; the translated "Document" without it.
    #[prop(into)]
    body_label: Option<Label>,
    title: Option<Placed<'a, S>>,
    slots: Vec<Placed<'a, S>>,
    /// The body, shown in what the slots leave.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>, &mut S)")]
    body: Option<SlotBody<'a, S>>,
}

impl<'a, S> AppShell<'a, S> {
    /// A shell for `window` with no slots.
    pub fn new(window: WindowKey) -> Self {
        AppShell {
            window,
            bare: false,
            embedded: false,
            screen_mode: Bind::kept(),
            allow_reveal: true,
            edge_reveal: true,
            document_tabs: true,
            body_label: None,
            title: None,
            slots: Vec::new(),
            body: None,
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
        self.title = Some(Placed { slot, edge: PaneEdge::Top, title: true, named: None, add: Box::new(add) });
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
        self.slots.push(Placed { slot, edge: PaneEdge::Top, title: false, named: Some(|words| words.options.clone()), add: Box::new(add) });
        self
    }

    /// The tool rail, a start slot keyed `"tools"` that reveals in body-only mode.
    pub fn tools(mut self, visible: bool, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        let slot = Slot::new(keys::TOOLS).visible(visible).reveal();
        self.slots.push(Placed { slot, edge: PaneEdge::Start, title: false, named: Some(|words| words.tools.clone()), add: Box::new(add) });
        self
    }

    /// The status bar, a bottom slot keyed `"status"` that never reveals.
    pub fn status(mut self, visible: bool, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        let slot = Slot::new(keys::STATUS).visible(visible);
        self.slots.push(Placed { slot, edge: PaneEdge::Bottom, title: false, named: Some(|words| words.status.clone()), add: Box::new(add) });
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

    /// A slot at the reading end, inside the ones added before it.
    pub fn end(self, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.place(PaneEdge::End, slot, add)
    }

    fn place(mut self, edge: PaneEdge, slot: Slot, add: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut S) + 'a) -> Self {
        self.slots.push(Placed { slot, edge, title: false, named: None, add: Box::new(add) });
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
        let AppShell { window, bare, embedded, mut screen_mode, allow_reveal, edge_reveal, document_tabs, body_label, title, slots, mut body } = self;
        let env = Env::of(ui);
        let outer = env.get::<ShellEntry>().0;
        let words = env.words::<ShellWords>();
        let tokens = env.tokens.clone();
        let body_label = body_label.unwrap_or_else(|| words.document.clone());
        let mut slots: Vec<Placed<'a, S>> = title.into_iter().chain(slots).collect();
        slots.sort_by_key(Placed::rank);

        let root = if embedded { ShellRegion::embedded() } else { ShellRegion::window() }.id_salt(window.as_str()).label(words.window.clone());
        let shell_body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let ctx = ui.ctx().clone();
            let viewport = pass::current_viewport(&ctx);
            let mut kept = cx.keep::<ShellState>().clone();

            let name: Arc<str> = words.screen_mode.as_str().into();
            let (mode, changed) = kept.screen.follow(&ctx, cx, screen_mode.reborrow(), name, !embedded);
            if changed {
                kept.reveal.clear();
            }

            let host_only = mode.body_only();
            if let Some(root) = cx.command_chain().innermost(ScreenModeCommands::SCOPE.name()) {
                note_shell(&ctx, viewport, &window, root);
            }
            kept.reveal.apply(&take_chrome_toggles(&ctx, viewport, &window), host_only, allow_reveal);
            let only = kept.reveal.only(host_only);
            let pointer = pointer::frame(&ctx);
            let frozen = !pointer.buttons.is_empty() || any_open(&ctx, OpenFilter::Keys);
            let rtl = cx.env.direction().is_rtl();
            kept.reveal.track(Tracking::new(only, allow_reveal, edge_reveal), Aim::new(pointer.pos, frozen, cx.rect, rtl));

            // Every shown slot's rect and the body's, before any slot runs. Slots that a reveal shows in body-only mode
            // cover the body.
            let visible: Vec<bool> = slots.iter().map(|placed| !bare && kept.reveal.shows(&placed.slot, placed.edge, only, allow_reveal)).collect();
            // A pass with no title bar has no move band: the OS moves the window from nowhere in it.
            if !embedded && !slots.iter().zip(&visible).any(|(placed, shown)| placed.title && *shown) {
                super::title_bar::set_window_movable(&ctx, false);
            }
            let claims: Vec<Claim> = slots
                .iter()
                .zip(&visible)
                .filter(|(_, shown)| **shown)
                .map(|(placed, _)| Claim { key: placed.slot.key(), edge: placed.edge, extent: placed.extent(&tokens), overlay: only })
                .collect();
            let regions = layout(cx.rect, &claims, rtl);

            let tabs = mode.tabs_visible() && document_tabs;
            let mut shell = ShellCx::new(window.clone(), if only { BodyMode::Only } else { BodyMode::Normal }, regions.clone())
                .with_tabs(tabs)
                .with_tab_reveal(kept.reveal.tab())
                .with_panels_hidden(kept.reveal.panels_hidden());
            // The room an embedded shell shows: its rect less what a scroll or a parent clips. Its dialogs are contained in
            // it.
            let region = cx.rect.intersect(ui.clip_rect());
            // An embedded shell scrolled or clipped out of sight shows nothing in this pass: its region has no room for
            // its body or its dialogs.
            let unseen = embedded && !region.is_positive();
            if embedded {
                shell = shell.with_region(region);
            }
            for edge in [RevealEdge::Top, RevealEdge::Start, RevealEdge::End] {
                shell = shell.with_edge_reveal(edge, kept.reveal.edge(edge));
            }
            let panels_hidden = kept.reveal.panels_hidden();

            let rect_of = |key| regions.slots.iter().find(|(shown, _)| *shown == key).map_or(Rect::ZERO, |(_, rect)| *rect);
            let scope = shell.clone();
            ShellRegion::scope(shell).id_salt("shell").show(ui, |ui, _| {
                if unseen {
                    return;
                }
                let mut show_slots = |ui: &mut Ui, covering: bool, app: &mut S| {
                    for (placed, shown) in slots.iter_mut().zip(&visible) {
                        if !*shown || only != covering {
                            continue;
                        }
                        let extent = placed.extent(&tokens);
                        let Placed { slot, edge, named, add, title, .. } = placed;
                        let pane = ShellPane::new(slot.key()).edge(*edge).extent(extent).at(rect_of(slot.key()));
                        let mut region = ShellRegion::pane(pane).id_salt(("slot", slot.key()));
                        // A bar is as high as its content and the slot as high as its token, so the slot paints the bar's
                        // surface under it, and the bar sits at the slot's centre line: no band of the backdrop shows
                        // between the bar and the body.
                        let centred = !*title && matches!(edge, PaneEdge::Top | PaneEdge::Bottom);
                        if centred {
                            region = region.surface(SurfaceKind::Chrome);
                        }
                        if let Some(label) = slot.name().cloned().or_else(|| named.map(|name| name(&words))) {
                            region = region.label(label);
                        }
                        region.show(ui, |ui, bcx| {
                            if centred {
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| add(ui, bcx, app));
                            } else {
                                add(ui, bcx, app);
                            }
                        });
                    }
                };
                show_slots(ui, false, app);
                // Only a body-only shell paints behind its body (the scrim). Otherwise the body paints what it shows, and the gaps
                // between its parts (the dock's gutters) are the window's own clear colour.
                let mut center = ShellRegion::pane(ShellPane::new("body").at(regions.body)).label(body_label.clone());
                if only {
                    center = center.surface(SurfaceKind::Scrim);
                }
                center.show(ui, |ui, bcx| {
                    if let Some(SlotBody(body)) = body.as_mut() {
                        body(ui, bcx, app);
                    }
                });
                show_slots(ui, true, app);
                if let Some(dialogs) = WindowDialogs::existing(ui.ctx(), &window) {
                    dialogs.run(ui);
                }
            });
            // What shows here counts for the shell around this one too, so a host reads it from its own shell.
            let opened = scope.dialog_shown();
            if opened && let Some(outer) = &outer {
                outer.note_dialog();
            }
            let dialog = embedded && opened;

            ToastStack::show(ui);

            let mut edges: Vec<(RevealEdge, Rect)> = regions
                .slots
                .iter()
                .filter_map(|(key, rect)| {
                    slots.iter().find(|placed| placed.slot.key() == *key).and_then(|placed| placed.edge.reveal_edge()).map(|edge| (edge, *rect))
                })
                .collect();
            edges.extend(scope.revealed_rects());
            kept.reveal.record(edges);
            let revealed = only && !regions.slots.is_empty();
            *cx.keep::<ShellState>() = kept;
            Parts { regions, revealed, panels_hidden, mode, dialog }
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
                dialog: parts.dialog,
            },
            None => AppShellOutput {
                response,
                regions: ShellRegions::new(Rect::ZERO),
                revealed: false,
                panels_hidden: false,
                screen_mode: ScreenMode::default(),
                dialog: false,
            },
        }
    }
}
