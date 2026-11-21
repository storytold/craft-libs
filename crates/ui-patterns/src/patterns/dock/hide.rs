//! The hidden-panel prune of a dock area: what the host hides, the shell's panel visibility and Maximize Frame
//! decide which panels the area shows.

use crate::docking::HidePanels;
use crate::docking::frame::{FrameView, maximize};
use crate::runtime::env::{PaneEdge, ShellCx};
use craft_behavior::dock::{DockEdge, Layout, Node, PanelPlace, node_at};
use std::hash::Hash;

/// The layout as the area shows it, and whether its floating windows show.
pub(super) struct Shown<'a, P: Clone> {
    /// The pruned layout with its write-back map.
    pub(super) view: FrameView<'a, P>,
    /// Floating windows show: the shell shows them, or it hides the panels and the view keeps what the host names.
    pub(super) floating: bool,
}

/// `layout` with the host's `hide`, the shell's panel visibility and the frame of `maximized` applied. Protected
/// panels always stay, so a canvas shows while the dock is hidden. The shell decides per dock (see [`resolved`]): a
/// dock that shows keeps the host's `hide`; while the shell hides the panels and keeps the tools (Shift+Tab), the
/// panels the host's `HidePanels::Except` names stay, docked or floating, and every other panel of a hidden dock hides;
/// while the shell hides the dock (Tab, body-only mode), every panel of it hides. Size edits made on the view go back
/// through `FrameView::write_back`.
pub(super) fn shown<'a, P: Clone + Eq + Hash>(
    layout: &'a Layout<P>,
    maximized: Option<&P>,
    hide: HidePanels<'_, P>,
    shell: Option<&ShellCx>,
    protected: &dyn Fn(&P) -> bool,
) -> Shown<'a, P> {
    let prune = Prune::new(layout, hide, shell);
    Shown { view: maximize(layout, maximized, prune.hide(), protected), floating: prune.floating }
}

/// The host's `hide` with the shell's per-dock visibility applied over a layout. `maximize` takes one `HidePanels` for
/// the whole layout, so a shell that hides a dock turns it into a per-panel keep.
pub(super) struct Prune<'a, P> {
    hide: HidePanels<'a, P>,
    keep: Option<Box<dyn Fn(&P) -> bool + 'a>>,
    /// Floating windows show.
    pub(super) floating: bool,
}

impl<'a, P: Clone + Eq + Hash> Prune<'a, P> {
    /// The prune of `layout` under `shell`. When every dock shows the host's `hide` goes through unchanged, so
    /// `maximize` can skip the prune.
    pub(super) fn new(layout: &'a Layout<P>, hide: HidePanels<'a, P>, shell: Option<&ShellCx>) -> Self {
        let floating = resolved(hide, shell, None).1;
        let Some(shell) = shell.filter(|shell| !shell.dock_visible(PaneEdge::Top)) else {
            return Prune { hide, keep: None, floating };
        };
        let shell = shell.clone();
        let keep = move |panel: &P| match resolved(hide, Some(&shell), dock_side(layout, panel)).0 {
            HidePanels::None => true,
            HidePanels::All => false,
            HidePanels::Except(keep) => keep(panel),
        };
        Prune { hide, keep: Some(Box::new(keep)), floating }
    }

    /// The panels the area hides.
    pub(super) fn hide(&self) -> HidePanels<'_, P> {
        match &self.keep {
            Some(keep) => HidePanels::Except(&**keep),
            None => self.hide,
        }
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

/// The panels the area hides in the dock at `side` (`None`: panels outside every dock), the host's `hide` with the
/// shell's panel visibility applied, and whether floating windows show. A dock whose edge shows keeps the host's
/// `hide`; a hidden one keeps the host's `Except` while the shell hides only the panels (Shift+Tab) and nothing under
/// Tab.
pub(super) fn resolved<'h, P>(hide: HidePanels<'h, P>, shell: Option<&ShellCx>, side: Option<DockEdge>) -> (HidePanels<'h, P>, bool) {
    let Some(shell) = shell else { return (hide, true) };
    let edge = match side {
        Some(DockEdge::Start) => PaneEdge::Start,
        Some(DockEdge::End) => PaneEdge::End,
        _ => PaneEdge::Top,
    };
    let floating = shell.panels_hidden() || shell.floating_visible();
    let hide = match hide {
        _ if shell.dock_visible(edge) => hide,
        HidePanels::Except(_) if shell.panels_hidden() => hide,
        _ => HidePanels::All,
    };
    (hide, floating)
}
