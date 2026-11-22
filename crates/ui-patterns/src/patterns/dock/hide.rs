//! The hidden-panel prune of a dock area: what the host hides, the shell's panel visibility and Maximize Frame
//! decide which panels the area shows. The area derives the pruned layout under a [`PruneKey`] and keeps it while the
//! key holds.

use super::area::ModelKey;
use crate::docking::{HidePanels, PanelKey};
use crate::runtime::env::ShellCx;

/// What the pruned layout is a function of: the workspace, the specs and the lock (the protected panels), what is
/// hidden after the shell's visibility, whether floating windows show and the maximized panel.
#[derive(crate::contract::MemoProps)]
pub(super) struct PruneKey<'h> {
    pub(super) model: ModelKey,
    pub(super) hide: HidePanels<'h>,
    pub(super) floating: bool,
    pub(super) maximized: Option<PanelKey>,
}

/// The panels the area hides, the host's `hide` with the shell's panel visibility applied, and whether floating windows
/// show. Protected panels always stay, so a canvas shows while the dock is hidden. While the shell hides the panels and
/// keeps the tools (Shift+Tab), the panels the host's `HidePanels::Except` names stay, docked or floating, and every
/// other panel hides; while the shell hides the dock (Tab, body-only mode), every panel hides. Size edits made on the
/// pruned view go back through `FrameView::write_back`.
pub(super) fn resolved<'h>(hide: HidePanels<'h>, shell: Option<&ShellCx>) -> (HidePanels<'h>, bool) {
    match shell {
        None => (hide, true),
        Some(shell) if shell.panels_hidden() => (if let HidePanels::Except(keep) = hide { HidePanels::Except(keep) } else { HidePanels::All }, true),
        Some(shell) => (if shell.dock_visible() { hide } else { HidePanels::All }, shell.floating_visible()),
    }
}
