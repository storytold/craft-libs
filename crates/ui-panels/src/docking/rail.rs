//! What a rail icon does for a panel and whether it shows lit, from the dock layout.

use craft_behavior::dock::{Action, Layout, Node, PanelPlace, PanelState};
use std::hash::Hash;

/// Whether the group, window or accordion section of `panel` is minimized. An accordion entry is minimized while it
/// is closed. A panel the layout does not hold is not.
pub(crate) fn minimized<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P) -> bool {
    let Some(place) = layout.place(panel) else { return false };
    match &place {
        PanelPlace::Node(_) => match layout.node_of(&place) {
            Some(Node::Stack { entries, .. }) => entries.iter().any(|entry| &entry.panel == panel && !entry.open),
            _ => false,
        },
        PanelPlace::Floating { window, .. } => layout.floating.get(*window).is_some_and(|window| window.minimized),
        _ => layout.group(&place).is_some_and(|group| group.minimized),
    }
}

/// The actions a click on the rail icon of `panel` takes: minimize a docked and shown panel, restore and activate a
/// minimized one, close a floating one's window, open a hidden one.
pub fn rail_actions<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P) -> Vec<Action<P>> {
    match layout.place(panel) {
        None => vec![Action::open(panel.clone(), None)],
        Some(PanelPlace::Floating { .. }) => vec![Action::close_window(panel.clone())],
        Some(_) if minimized(layout, panel) => vec![Action::set_minimized(panel.clone(), false), Action::activate(panel.clone())],
        Some(_) => vec![Action::set_minimized(panel.clone(), true)],
    }
}

/// Whether the rail icon of `panel` shows lit: its group is in the layout and not minimized.
pub fn rail_lit<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P) -> bool {
    layout.panel_state(panel) != PanelState::Hidden && !minimized(layout, panel)
}
