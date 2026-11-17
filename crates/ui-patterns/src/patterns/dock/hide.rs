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
    /// Floating windows show: the shell shows them, or it hides the panels and the view keeps what the host names.
    pub(super) floating: bool,
}

/// `layout` with the host's `hide`, the shell's panel visibility and the frame of `maximized` applied. Protected
/// panels always stay, so a canvas shows while the dock is hidden. While the shell hides the panels and keeps the
/// tools (Shift+Tab), the panels the host's `HidePanels::Except` names stay, docked or floating, and every other panel
/// hides; while the shell hides the dock (Tab, body-only mode), every panel hides. Size edits made on the view go back
/// through `FrameView::write_back`.
pub(super) fn shown<'a, P: Clone + Eq + Hash>(
    layout: &'a Layout<P>,
    maximized: Option<&P>,
    hide: HidePanels<'_, P>,
    shell: Option<&ShellCx>,
    protected: &dyn Fn(&P) -> bool,
) -> Shown<'a, P> {
    let (hide, floating) = resolved(hide, shell);
    Shown { view: maximize(layout, maximized, hide, protected), floating }
}

/// The panels the area hides, the host's `hide` with the shell's panel visibility applied, and whether floating windows
/// show (see [`shown`]).
pub(super) fn resolved<'h, P>(hide: HidePanels<'h, P>, shell: Option<&ShellCx>) -> (HidePanels<'h, P>, bool) {
    match shell {
        None => (hide, true),
        Some(shell) if shell.panels_hidden() => (if let HidePanels::Except(keep) = hide { HidePanels::Except(keep) } else { HidePanels::All }, true),
        Some(shell) => (if shell.dock_visible() { hide } else { HidePanels::All }, shell.floating_visible()),
    }
}
