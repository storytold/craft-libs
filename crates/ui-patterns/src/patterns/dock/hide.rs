//! The hidden-panel prune of a dock area: what the host hides, the shell's panel visibility and Maximize Frame
//! decide which panels the area shows.

use crate::docking::HidePanels;
use crate::docking::frame::{FrameView, maximize};
use crate::runtime::env::ShellCx;
use craft_behavior::dock::Layout;
use std::hash::Hash;

/// The layout as the area shows it, and whether its floating windows show.
pub(super) struct Shown<'a, P: Clone> {
    /// The pruned layout with its write-back map.
    pub(super) view: FrameView<'a, P>,
    /// Floating windows show: the shell shows them and the panels are not hidden.
    pub(super) floating: bool,
}

/// `layout` with the host's `hide`, the shell's panel visibility and the frame of `maximized` applied. Protected
/// panels always stay, so a canvas shows while the dock is hidden. Size edits made on the view go back through
/// `FrameView::write_back`.
pub(super) fn shown<'a, P: Clone + Eq + Hash>(
    layout: &'a Layout<P>,
    maximized: Option<&P>,
    hide: HidePanels<'_, P>,
    shell: Option<&ShellCx>,
    protected: &dyn Fn(&P) -> bool,
) -> Shown<'a, P> {
    let dock_hidden = shell.is_some_and(|shell| !shell.dock_visible());
    let hide = if dock_hidden { HidePanels::All } else { hide };
    let floating = shell.is_none_or(|shell| shell.floating_visible());
    Shown { view: maximize(layout, maximized, hide, protected), floating }
}
