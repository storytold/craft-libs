//! `WindowMenuRows`: the Window menu's check row for every panel of an app.

use super::SpecSource;
use crate::contract::Show as _;
use crate::menu::MenuItem;
use craft_behavior::dock::{Action, Layout, PanelState};
use egui::Ui;
use std::hash::Hash;

/// One check row per panel in the order the app gives, for a Window menu. A row is checked while its panel is in the
/// layout (closed panels are not), and its label and icon come from the panels' specs ([`PanelSpecs::spec`](super::PanelSpecs::spec)). A pick returns the action
/// that shows the panel (open a closed one, bring a panel behind another tab to the front) or closes it (a panel
/// showing); the app applies it through the same path as the actions of a `DockArea`. A Hidden pick's `Action::open`
/// restores the panel's last place through `Workspace::apply_all`.
///
/// A panel in a column collapsed to icons ([`Layout::opens_in_flyout`]) shows in its flyout: its row is checked
/// while its flyout is open ([`flyout`](WindowMenuRows::flyout)), and a pick opens the flyout, or closes it when it is
/// open, through [`WindowMenuRowsOutput::flyout`], which the app writes to the binding it gives `DockArea::flyout`.
#[derive(crate::contract::Pattern)]
#[pattern(probe(WindowMenuRows<'static, u32>))]
pub struct WindowMenuRows<'a, P> {
    layout: &'a Layout<P>,
    specs: &'a dyn SpecSource<P>,
    order: &'a [P],
    flyout: Option<&'a P>,
}

/// What the rows report.
#[derive(Debug)]
#[non_exhaustive]
pub struct WindowMenuRowsOutput<P> {
    /// What the picked row asks for.
    pub action: Option<Action<P>>,
    /// The open flyout the picked row asks for, when its panel is in a column collapsed to icons: that panel, or
    /// `None` to close the flyout it had open.
    pub flyout: Option<Option<P>>,
}

impl<'a, P: Clone + Eq + Hash> WindowMenuRows<'a, P> {
    /// Rows for `order` (every panel the app has, open or not), checked from `layout`, labelled by `specs`, the
    /// host's spec model the dock shows ([`DockArea::new`](crate::docking)).
    pub fn new(layout: &'a Layout<P>, specs: &'a dyn SpecSource<P>, order: &'a [P]) -> Self {
        WindowMenuRows { layout, specs, order, flyout: None }
    }

    /// The panel whose flyout is open: the value of the binding the app gives `DockArea::flyout`.
    #[must_use]
    pub fn flyout(mut self, open: Option<&'a P>) -> Self {
        self.flyout = open;
        self
    }

    /// Shows the rows in the menu body `ui`.
    pub fn show(self, ui: &mut Ui) -> WindowMenuRowsOutput<P> {
        let (mut action, mut flyout) = (None, None);
        for panel in self.order {
            let spec = self.specs.spec(panel);
            let state = self.layout.panel_state(panel);
            let in_flyout = self.layout.opens_in_flyout(panel);
            let open = self.flyout == Some(panel);
            let mut row = MenuItem::new(spec.label).check(if in_flyout { open } else { state != PanelState::Hidden });
            if let Some(icon) = spec.icon {
                row = row.icon(icon);
            }
            if !row.show(ui).ran {
                continue;
            }
            if in_flyout {
                flyout = Some((!open).then(|| panel.clone()));
            } else {
                action = Some(match state {
                    PanelState::Hidden => Action::open(panel.clone(), None),
                    PanelState::Open => Action::activate(panel.clone()),
                    _ => Action::close(panel.clone()),
                });
            }
        }
        WindowMenuRowsOutput { action, flyout }
    }
}
