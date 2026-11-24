//! The hidden-panel prune of a dock area: what the host hides, the shell's panel visibility and Maximize Frame
//! decide which panels the area shows. The area derives the pruned layout under a [`PruneKey`] and keeps it while the
//! key holds.

use super::area::ModelKey;
use crate::contract::Tracked;
use crate::docking::frame::{FrameView, maximize};
use crate::docking::{HidePanels, PanelKey};
use crate::runtime::env::{PaneEdge, ShellCx};
use craft_behavior::dock::{DockEdge, Layout, Node, PanelPlace, node_at};
use std::collections::BTreeSet;
use std::hash::Hash;

/// What the pruned layout is a function of: the workspace, the specs and the lock (the protected panels), what the
/// host hides, which docks the shell shows, whether floating windows show and the maximized panel.
#[derive(crate::contract::MemoProps)]
pub(super) struct PruneKey<'h> {
    model: ModelKey,
    hide: HidePanels<'h>,
    shows: Option<Shows>,
    /// Floating windows show: the shell shows them, or it hides the panels and the view keeps what the host names.
    pub(super) floating: bool,
    maximized: Option<PanelKey>,
}

impl<'h> PruneKey<'h> {
    /// The key of the prune of the workspace `model` with the host's `hide` under `shell`.
    pub(super) fn new(model: ModelKey, hide: HidePanels<'h>, shell: Option<&ShellCx>, maximized: Option<PanelKey>) -> Self {
        let floating = shell.is_none_or(|shell| shell.panels_hidden() || shell.floating_visible());
        PruneKey { model, hide, shows: shell.map(Shows::of), floating, maximized }
    }
}

/// Which docks the shell shows, as the prune reads it.
#[derive(Clone, Copy, crate::contract::MemoProps)]
struct Shows {
    /// The shell hides the panels and keeps the tools (Shift+Tab).
    panels_hidden: bool,
    /// The dock at the start edge shows.
    start: bool,
    /// The dock at the end edge shows.
    end: bool,
    /// The panels outside the start and end docks show.
    other: bool,
}

impl Shows {
    fn of(shell: &ShellCx) -> Self {
        Shows {
            panels_hidden: shell.panels_hidden(),
            start: shell.dock_visible(PaneEdge::Start),
            end: shell.dock_visible(PaneEdge::End),
            other: shell.dock_visible(PaneEdge::Top),
        }
    }

    /// The panels hidden in the dock at `side` (`None`: panels outside every dock). A dock that shows keeps the host's
    /// `hide`; a hidden one keeps the host's `Except` while the shell hides only the panels (Shift+Tab), and nothing
    /// under Tab.
    fn resolved(self, hide: HidePanels<'_>, side: Option<DockEdge>) -> HidePanels<'_> {
        let shows = match side {
            Some(DockEdge::Start) => self.start,
            Some(DockEdge::End) => self.end,
            _ => self.other,
        };
        match hide {
            _ if shows => hide,
            HidePanels::Except(_) if self.panels_hidden => hide,
            _ => HidePanels::All,
        }
    }
}

/// `layout` with the host's `hide`, the shell's panel visibility and the frame of `maximized` applied. Protected
/// panels always stay, so a canvas shows while the dock is hidden. The shell decides per dock (see
/// [`Shows::resolved`]): while it hides the panels and keeps the tools (Shift+Tab), the panels the host's
/// `HidePanels::Except` names stay, docked or floating, and every other panel of a hidden dock hides; while it hides the
/// dock (Tab, body-only mode), every panel of a hidden dock hides. Size edits made on the view go back through
/// `FrameView::write_back`.
pub(super) fn shown<'a, P: Clone + Eq + Hash>(
    layout: &'a Layout<P>,
    maximized: Option<&P>,
    hide: HidePanels<'_>,
    shell: Option<&ShellCx>,
    protected: &dyn Fn(&P) -> bool,
) -> FrameView<'a, P> {
    maximize(layout, maximized, Prune::new(layout, hide, shell).hide(), protected)
}

/// The host's `hide` with the shell's per-dock visibility applied over a layout. `maximize` and `Placing` take one
/// `HidePanels` for the whole layout, so while the shell shows some docks and hides others the prune holds the set of
/// panels it keeps.
pub(super) struct Prune<'h> {
    hide: HidePanels<'h>,
    keep: Option<Tracked<BTreeSet<PanelKey>>>,
}

impl<'h> Prune<'h> {
    /// The prune of `layout` under `shell`. When every dock resolves alike the resolved `hide` goes through unchanged,
    /// so `maximize` can skip the prune.
    pub(super) fn new<P: Clone + Eq + Hash>(layout: &Layout<P>, hide: HidePanels<'h>, shell: Option<&ShellCx>) -> Self {
        let Some(shows) = shell.map(Shows::of) else { return Prune { hide, keep: None } };
        if shows.start == shows.other && shows.end == shows.other {
            return Prune { hide: shows.resolved(hide, None), keep: None };
        }
        let keep = layout
            .panels()
            .into_iter()
            .filter(|panel| match shows.resolved(hide, dock_side(layout, *panel)) {
                HidePanels::All => false,
                HidePanels::Except(keep) => keep.contains(&PanelKey::of(*panel)),
                // `None`, and a way of hiding this code does not know, hide nothing.
                _ => true,
            })
            .map(PanelKey::of)
            .collect();
        Prune { hide, keep: Some(Tracked::new(keep)) }
    }

    /// The panels the area hides.
    pub(super) fn hide(&self) -> HidePanels<'_> {
        self.keep.as_ref().map_or(self.hide, HidePanels::Except)
    }
}

/// The side of the dock `panel` sits in; `None` for a panel outside every dock.
fn dock_side<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P) -> Option<DockEdge> {
    let PanelPlace::Dock { path, .. } = layout.place(panel)? else { return None };
    match layout.root.as_ref().and_then(|root| node_at(root, &path))? {
        Node::Dock { side, .. } => Some(*side),
        _ => None,
    }
}
