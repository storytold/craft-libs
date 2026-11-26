//! The window skeleton as env data: the window key, the body mode, the layout of a shell pass and `ShellCx`, which
//! carries them to the slots and the body.

use super::{AppValues, EnvValue};
use crate::runtime::egui::pass;
use crate::runtime::frame::{PerViewport, Stamped, Stored};
use egui::Rect;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// The stable key of a slot.
pub type SlotKey = &'static str;

/// The key of a window. It equals the placement `SaveKey` of the window, so two windows of one app need different keys.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WindowKey(String);

impl WindowKey {
    /// The first window, `"main"`.
    pub fn main() -> Self {
        Self::new("main")
    }

    /// A key from any string.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for WindowKey {
    fn from(key: &str) -> Self {
        Self::new(key)
    }
}

/// What the window shows besides its body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyMode {
    /// The slots show as their `visible` flags say.
    #[default]
    Normal,
    /// No slot shows, floating windows and document tabs hide, and the backdrop is black. A reveal shows the slots built
    /// to show in a reveal.
    Only,
}

/// An edge of the window whose slots a pointer at the edge reveals in body-only mode. The bottom edge and the centre
/// never reveal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RevealEdge {
    /// The top edge.
    Top,
    /// The start edge.
    Start,
    /// The end edge.
    End,
}

impl RevealEdge {
    /// The position of the edge in the reveal flags.
    const fn index(self) -> usize {
        match self {
            RevealEdge::Top => 0,
            RevealEdge::Start => 1,
            RevealEdge::End => 2,
        }
    }
}

/// What one shell pass laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellRegions {
    /// The body's rect: the dock host's rect.
    pub body: Rect,
    /// The rects of the shown slots, in the order drawn.
    pub slots: Vec<(SlotKey, Rect)>,
}

impl ShellRegions {
    /// A layout with `body` and no slots.
    pub fn new(body: Rect) -> Self {
        ShellRegions { body, slots: Vec::new() }
    }

    /// The layout with the shown slot `key` at `rect`, after the slots already listed.
    pub fn with_slot(mut self, key: SlotKey, rect: Rect) -> Self {
        self.slots.push((key, rect));
        self
    }
}

/// What the window skeleton shows on this pass, read only.
///
/// The shell sets the value in the env before any slot container runs, so a slot body reads it with `BodyCx::shell`
/// in the same pass. [`ShellCx::detached`] and [`ShellCx::put`] set it the same way for tests and the gallery.
/// Cloning is cheap: the value is shared.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellCx {
    inner: Arc<Inner>,
}

#[derive(Clone, Debug, PartialEq)]
struct Inner {
    window: WindowKey,
    mode: BodyMode,
    tab_reveal: bool,
    /// The edge reveal flags, indexed by `RevealEdge::index`.
    edges: [bool; 3],
    tabs: bool,
    panels_hidden: bool,
    regions: ShellRegions,
    float_bounds: Rect,
    region: Option<Rect>,
    /// The key of the shell's revealed rects: the same in every pass of the shell, so a replayed view registers its
    /// rects for the shell that shows it.
    reveals: egui::Id,
    dialog: DialogRecord,
    page_band: Option<PageBandRoom>,
}

/// The room of the page band while a full-window page shows: the title-bar-high strip at the window's top and the room
/// the window's ends take in it.
#[derive(Clone, Copy, Debug, PartialEq, crate::contract::MemoProps)]
#[non_exhaustive]
pub struct PageBandRoom {
    /// The band's rect.
    pub rect: Rect,
    /// The width the window's start end takes (the traffic lights' inset or the caption buttons), from the band's
    /// reading start.
    pub start: f32,
    /// The width the window's end end takes, from the band's reading end.
    pub end: f32,
}

impl PageBandRoom {
    /// The band at `rect`, the window's start end taking `start` of it and its end end `end`.
    pub const fn new(rect: Rect, start: f32, end: f32) -> Self {
        PageBandRoom { rect, start, end }
    }
}

/// The rects that content registered on the running pass of a viewport for the edge it was revealed from, each with
/// the key of its shell.
#[derive(Default)]
struct RevealedRects(Vec<(egui::Id, RevealEdge, Rect)>);

impl Stored for RevealedRects {}

fn with_revealed<R>(ctx: &egui::Context, f: impl FnOnce(&mut RevealedRects) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Stamped<RevealedRects>>::with_pass(ctx, viewport, pass::pass_nr(ctx, viewport), |rects, _| rects.0.clear(), f)
}

crate::stays!(RevealEdge);

/// Registers `rect` for `edge` with the shell keyed `reveals`, on the running pass. A replayed view registers it again.
fn note_reveal(ctx: &egui::Context, reveals: egui::Id, edge: RevealEdge, rect: Rect) {
    crate::runtime::replay::journal(ctx, || (reveals, edge, rect), |ctx, &(reveals, edge, rect)| note_reveal(ctx, reveals, edge, rect));
    with_revealed(ctx, |rects| rects.0.push((reveals, edge, rect)));
}

/// How many dialogs, modals and windows the body showed on this pass. The record is shared by the clones of one
/// `ShellCx`.
#[derive(Clone, Debug, Default)]
struct DialogRecord(Arc<AtomicU32>);

impl PartialEq for DialogRecord {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl ShellCx {
    /// The skeleton of `window` in `mode` with the layout of this pass. No reveal is on, the document tabs show as the
    /// mode allows and floating frames may go over the body.
    pub fn new(window: WindowKey, mode: BodyMode, regions: ShellRegions) -> Self {
        let float_bounds = regions.body;
        let reveals = crate::runtime::memory::global("shell reveals", &window);
        let inner = Inner {
            window,
            mode,
            tab_reveal: false,
            edges: [false; 3],
            tabs: true,
            panels_hidden: false,
            regions,
            float_bounds,
            region: None,
            reveals,
            dialog: DialogRecord::default(),
            page_band: None,
        };
        ShellCx { inner: Arc::new(inner) }
    }

    /// A skeleton with no slots around `body`, for tests and the gallery without an `AppShell`.
    pub fn detached(window: WindowKey, mode: BodyMode, body: Rect) -> Self {
        Self::new(window, mode, ShellRegions::new(body))
    }

    /// Whether the host shows the document tabs; they show only in [`BodyMode::Normal`].
    pub fn with_tabs(mut self, tabs: bool) -> Self {
        Arc::make_mut(&mut self.inner).tabs = tabs;
        self
    }

    /// The panels are hidden while the tools and bars stay (Shift+Tab), or shown.
    pub fn with_panels_hidden(mut self, hidden: bool) -> Self {
        Arc::make_mut(&mut self.inner).panels_hidden = hidden;
        self
    }

    /// The Tab reveal is on or off.
    pub fn with_tab_reveal(mut self, on: bool) -> Self {
        Arc::make_mut(&mut self.inner).tab_reveal = on;
        self
    }

    /// The edge reveal of `edge` is on or off.
    pub fn with_edge_reveal(mut self, edge: RevealEdge, on: bool) -> Self {
        Arc::make_mut(&mut self.inner).edges[edge.index()] = on;
        self
    }

    /// Where floating frames may go: the body rect plus the area under bars that overlay the body.
    pub fn with_float_bounds(mut self, bounds: Rect) -> Self {
        Arc::make_mut(&mut self.inner).float_bounds = bounds;
        self
    }

    /// The screen rect of the region this shell fills inside another window (`AppShell::embedded`). Modal layers opened
    /// in the shell are contained in it.
    pub fn with_region(mut self, region: Rect) -> Self {
        Arc::make_mut(&mut self.inner).region = Some(region);
        self
    }

    /// The region an embedded shell fills; `None` for a window's own shell.
    pub fn region(&self) -> Option<Rect> {
        self.inner.region
    }

    /// Sets this value in `values`, where `BodyCx::shell` reads it. The shell calls it on the values of the scope that
    /// holds its slots and body.
    pub fn put(&self, values: &mut AppValues) {
        values.set(ShellEntry(Some(self.clone())));
    }

    /// The window's key; components in slots key their ids and memory by it.
    pub fn window(&self) -> &WindowKey {
        &self.inner.window
    }

    /// The body mode of this pass.
    pub fn body_mode(&self) -> BodyMode {
        self.inner.mode
    }

    /// Is the slot with this key shown on this pass?
    pub fn slot_visible(&self, key: SlotKey) -> bool {
        self.inner.regions.slots.iter().any(|(shown, _)| *shown == key)
    }

    /// Whether the panels are hidden while the tools and bars stay (Shift+Tab in normal mode).
    pub fn panels_hidden(&self) -> bool {
        self.inner.panels_hidden
    }

    /// Whether the dock at `edge` shows: in normal mode and during the Tab reveal unless the panels are hidden, and the
    /// `Start` and `End` docks also while that edge's reveal is on, in body-only mode or while the panels are hidden.
    pub fn dock_visible(&self, edge: super::PaneEdge) -> bool {
        let inner = &*self.inner;
        let base = !inner.panels_hidden && (inner.mode == BodyMode::Normal || inner.tab_reveal);
        base || match edge {
            super::PaneEdge::Start => self.edge_revealed(RevealEdge::Start),
            super::PaneEdge::End => self.edge_revealed(RevealEdge::End),
            _ => false,
        }
    }

    /// Whether floating frames show: in normal mode; in body-only mode only during the Tab reveal; never while the
    /// panels are hidden.
    pub fn floating_visible(&self) -> bool {
        !self.inner.panels_hidden && (self.inner.mode == BodyMode::Normal || self.inner.tab_reveal)
    }

    /// The same skeleton with its revealed rects registered under `id`, which stays the same across passes. Without
    /// it they are keyed by the window; `AppShell` passes its own id, so an embedded shell keeps its rects apart from
    /// the shell of the window around it.
    #[must_use]
    pub fn with_reveals_key(mut self, id: egui::Id) -> Self {
        Arc::make_mut(&mut self.inner).reveals = id;
        self
    }

    /// Registers `rect`, a revealed panel the body drew on this pass, as part of the revealed `edge`: the edge reveal
    /// lasts while the pointer is over it, as it does over a revealed slot. Call it in the pass that draws the panel; a
    /// memoized view that replays registers it again. A measuring run registers nothing.
    pub fn reveal_rect(&self, ctx: &egui::Context, edge: RevealEdge, rect: Rect) {
        if crate::runtime::measure::effect(ctx, "shell::reveal_rect") {
            note_reveal(ctx, self.inner.reveals, edge, rect);
        }
    }

    /// The rects registered with [`reveal_rect`](Self::reveal_rect) on this pass, in registration order.
    pub fn revealed_rects(&self, ctx: &egui::Context) -> Vec<(RevealEdge, Rect)> {
        let reveals = self.inner.reveals;
        with_revealed(ctx, |rects| rects.0.iter().filter(|(shell, _, _)| *shell == reveals).map(|&(_, edge, rect)| (edge, rect)).collect())
    }

    /// Records that a container in this shell shows as a dialog, a modal, a full-window layer or an OS window. Every
    /// container mount records itself here, measuring runs included.
    #[doc(hidden)]
    pub fn note_dialog(&self) {
        self.inner.dialog.0.fetch_add(1, Ordering::Relaxed);
    }

    /// How many times a container in this shell showed on this pass as a dialog (a layer with a dialog's focus scope,
    /// modal or not), a full-window layer or an OS window: open, or closed and still fading out. Menus, list boxes,
    /// popovers and tooltips do not count. A host reads it after the body ran, or before and after a part of the body
    /// to learn whether that part showed one. A measure of the body records open containers only (it runs no fades), so
    /// a host that reads it there knows before the first paint.
    pub fn dialogs_shown(&self) -> u32 {
        self.inner.dialog.0.load(Ordering::Relaxed)
    }

    /// Whether the document tabs show: the host's flag in normal mode, never in body-only mode.
    pub fn tabs_visible(&self) -> bool {
        self.inner.mode == BodyMode::Normal && self.inner.tabs
    }

    /// Are slots shown by a reveal (Tab or edge) on this pass? Only body-only mode reveals with Tab; an edge reveals in
    /// body-only mode and while the panels are hidden.
    pub fn revealed(&self) -> bool {
        let inner = &*self.inner;
        let body_only = inner.mode == BodyMode::Only;
        (body_only && inner.tab_reveal) || ((body_only || inner.panels_hidden) && inner.edges.iter().any(|on| *on))
    }

    /// Is the edge reveal on for `edge`? False while the panels show in normal mode, and during the Tab reveal.
    pub fn edge_revealed(&self, edge: RevealEdge) -> bool {
        let inner = &*self.inner;
        (inner.mode == BodyMode::Only || inner.panels_hidden) && !inner.tab_reveal && inner.edges[edge.index()]
    }

    /// The body rect and the shown slots of this pass.
    pub fn regions(&self) -> &ShellRegions {
        &self.inner.regions
    }

    /// Where floating frames may go: the body rect plus the area under bars that overlay the body.
    pub fn float_bounds(&self) -> Rect {
        self.inner.float_bounds
    }

    /// The page band's room while a full-window page shows, else `None`.
    pub fn page_band(&self) -> Option<PageBandRoom> {
        self.inner.page_band
    }

    /// The page band's room while a full-window page shows; `None` when no page shows or the window is full screen.
    pub fn with_page_band(mut self, room: Option<PageBandRoom>) -> Self {
        Arc::make_mut(&mut self.inner).page_band = room;
        self
    }
}

/// The env entry that carries the shell to its slots and body. A context with no shell holds `None`.
#[derive(Clone, PartialEq)]
#[doc(hidden)]
pub struct ShellEntry(#[doc(hidden)] pub Option<ShellCx>);

/// The shell's input fields: the window, the body's mode, the reveal and visibility flags, the regions, the float
/// bounds and the room of a full-window page's band. The registries the body writes into (revealed rects, the dialog count) are its output, not an input; the
/// revealed rects' key is fixed for the shell's life.
impl crate::contract::MemoProps for ShellEntry {
    fn key(&self, key: &mut crate::contract::MemoKey) {
        let Some(shell) = &self.0 else { return key.write_u32(0) };
        let Inner { window, mode, tab_reveal, edges, tabs, panels_hidden, regions, float_bounds, region, page_band, reveals: _, dialog: _ } =
            &*shell.inner;
        key.write_u32(1);
        key.feed_hash(window);
        key.feed_hash(&std::mem::discriminant(mode));
        (*tab_reveal, *edges, *tabs, *panels_hidden).key(key);
        regions.body.key(key);
        key.write_u32(regions.slots.len() as u32);
        for (slot, rect) in &regions.slots {
            slot.key(key);
            rect.key(key);
        }
        float_bounds.key(key);
        region.key(key);
        page_band.key(key);
    }
}

impl EnvValue for ShellEntry {
    fn root() -> Self {
        ShellEntry(None)
    }
}
