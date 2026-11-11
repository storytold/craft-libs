//! `WindowMenuRows`: the Window menu's check row for every panel of an app.

use super::DockPanels;
use crate::contract::Show as _;
use crate::menu::MenuItem;
use craft_behavior::dock::{Action, Layout, PanelState};
use egui::Ui;
use std::hash::Hash;

/// One check row per panel in the order the app gives, for a Window menu. A row is checked while its panel is in the
/// layout (closed panels are not), and its label and icon come from [`DockPanels::spec`]. A pick returns the action
/// that shows the panel (open a closed one, bring a panel behind another tab to the front) or closes it (a panel
/// showing); the app applies it through the same path as the actions of a `DockArea`.
#[derive(crate::contract::Pattern)]
#[pattern(probe(WindowMenuRows<'static, u32>))]
pub struct WindowMenuRows<'a, P> {
    layout: &'a Layout<P>,
    panels: &'a dyn DockPanels<P>,
    order: &'a [P],
}

/// What the rows report.
#[derive(Debug)]
#[non_exhaustive]
pub struct WindowMenuRowsOutput<P> {
    /// What the picked row asks for.
    pub action: Option<Action<P>>,
}

impl<'a, P: Clone + Eq + Hash> WindowMenuRows<'a, P> {
    /// Rows for `order` (every panel the app has, open or not), checked from `layout`, labelled by `panels`.
    pub fn new(layout: &'a Layout<P>, panels: &'a dyn DockPanels<P>, order: &'a [P]) -> Self {
        WindowMenuRows { layout, panels, order }
    }

    /// Shows the rows in the menu body `ui`.
    pub fn show(self, ui: &mut Ui) -> WindowMenuRowsOutput<P> {
        let mut action = None;
        for panel in self.order {
            let spec = self.panels.spec(panel);
            let state = self.layout.panel_state(panel);
            let mut row = MenuItem::new(spec.label).check(state != PanelState::Hidden);
            if let Some(icon) = spec.icon {
                row = row.icon(icon);
            }
            if row.show(ui).ran {
                action = Some(match state {
                    PanelState::Hidden => Action::open(panel.clone(), None),
                    PanelState::Open => Action::activate(panel.clone()),
                    _ => Action::close(panel.clone()),
                });
            }
        }
        WindowMenuRowsOutput { action }
    }
}
