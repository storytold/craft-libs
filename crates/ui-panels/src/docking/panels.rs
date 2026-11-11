//! `DockPanels`: what a dock asks of the host for each panel.

use super::flyout::GroupAt;
use super::spec::PanelSpec;
use crate::contract::BodyCx;
use crate::menu::PanelRow;
use craft_behavior::dock::Permissions;
use egui::Ui;

/// The host's panels. A dock calls, in one pass and for each group it shows:
///
/// 1. `group_view`, with the group's panels and where the group goes. When it returns a height the host took the group
///    itself: the dock shows no tab strip and no panel body for that group, and calls neither `group_header` nor `body`
///    for it. A flyout measures its group before it shows it, so a host view runs at `GroupAt::Measure` too.
/// 2. Otherwise `group_header` once, inside the group's header region before its tab strip, then `spec` for each of
///    its panels, then `body` for the panels whose bodies are shown. `panel_rows` runs when a panel's menu opens.
///
/// A panel's `spec` is read every pass, so a host may change a label or a limit between passes. The dock's actions
/// name a group by one of its panels.
pub trait DockPanels<P> {
    /// What the dock reads of `panel`.
    fn spec(&self, panel: &P) -> PanelSpec;

    /// What may happen to `panel`: whether it moves, floats, closes and takes tabs and splits (the default allows
    /// everything). A canvas is `Permissions::PROTECTED`. A locked workspace narrows these itself.
    fn permissions(&self, _panel: &P) -> Permissions {
        Permissions::ALL
    }

    /// Shows `panel`'s body in `ui`.
    fn body(&mut self, panel: &P, ui: &mut Ui, cx: &mut BodyCx<'_>);

    /// Takes the group holding `panels`, whose tab `active` is selected, in place of its tab strip and body: at
    /// `GroupAt::Show(rect)` it shows its view in `rect`, at `GroupAt::Measure(width)` it measures that view at `width`
    /// the way it shows it, and it returns the height the view takes (at `Show`, the rect's). `None` (the default)
    /// leaves the group to the dock, at both. A canvas group uses the panels to tell which document it holds and draws
    /// it at the group's rect.
    fn group_view(&mut self, _panels: &[P], _active: usize, _ui: &mut Ui, _at: GroupAt, _cx: &mut BodyCx<'_>) -> Option<f32> {
        None
    }

    /// Shows the host's controls in the header of the group holding `panels`, whose tab `active` is selected,
    /// before its tab strip (the default shows none). It does not run for a group whose `group_view` took it.
    fn group_header(&mut self, _panels: &[P], _active: usize, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// The host's rows of `panel`'s menu, shown before the dock's own rows (the default has none): command rows,
    /// which run by themselves.
    fn panel_rows(&mut self, _panel: &P) -> Vec<PanelRow> {
        Vec::new()
    }
}
