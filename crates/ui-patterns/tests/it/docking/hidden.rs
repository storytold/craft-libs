//! What a dock area shows while the shell hides the panels and keeps the tools (Shift+Tab) or hides the dock (Tab).

use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, HidePanels, PanelKey, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::env::{BodyMode, RevealEdge, ShellCx, ShellEntry, WindowKey};
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

const CANVAS: usize = 1;
/// Docked, and named by the host's `HidePanels::Except`.
const TOOLBAR: usize = 2;
const LAYERS: usize = 3;
/// Floating, and named by the host's `HidePanels::Except`.
const PALETTE: usize = 4;
const NAVIGATOR: usize = 5;

/// Records the panels whose bodies ran.
struct Panels {
    shown: Rc<RefCell<BTreeSet<usize>>>,
}

/// Panels named by number; the canvas is protected.
#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn permissions(&self, panel: &usize) -> Permissions {
        if *panel == CANVAS { Permissions::PROTECTED } else { Permissions::ALL }
    }
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, panel: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {
        self.shown.borrow_mut().insert(*panel);
    }
}

/// The toolbar in a start dock, the canvas, Layers in an end dock, and two floating windows.
fn layout() -> Layout<usize> {
    let start = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![TOOLBAR], 0)])]);
    let end = Node::dock(DockEdge::End, vec![Column::new(vec![Group::new(vec![LAYERS], 0)])]);
    let root = Node::split(Axis::X, SplitSize::Ratio(0.2), start, Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![CANVAS], 0), end));
    let window =
        |panel: usize, x: f32| Floating::single(panel as u64, vec![panel], 0, Rect::from_min_size(Pos2::new(x, 60.0), Vec2::new(120.0, 120.0)));
    Layout::new(Some(root), vec![window(PALETTE, 90.0), window(NAVIGATOR, 220.0)])
}

/// The panels the area shows under `shell`, with the host keeping the toolbar and the palette when `keeps`.
fn shown_under(shell: ShellCx, keeps: bool) -> BTreeSet<usize> {
    let shown: Rc<RefCell<BTreeSet<usize>>> = Rc::default();
    let seen = Rc::clone(&shown);
    let mut fixture = Fixture::new(move |ui| {
        seen.borrow_mut().clear();
        let mut workspace = Arc::new(Workspace::new(layout()));
        let mut panels = Panels { shown: Rc::clone(&seen) };
        let specs = Tracked::new(Specs);
        let keep = Tracked::new(BTreeSet::from([PanelKey::of(&TOOLBAR), PanelKey::of(&PALETTE)]));
        let mut area = DockArea::new("Hidden dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default());
        if keeps {
            area = area.hide(HidePanels::Except(&keep));
        }
        area.show(ui);
    });
    fixture.root_value(ShellEntry(Some(shell))).settle();
    shown.take()
}

#[trial]
fn shift_tab_keeps_the_panels_the_host_names_docked_and_floating_and_tab_hides_them_all() {
    let room = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
    let normal = ShellCx::detached(WindowKey::main(), BodyMode::Normal, room);
    let all = BTreeSet::from([CANVAS, TOOLBAR, LAYERS, PALETTE, NAVIGATOR]);
    assert_eq!(shown_under(normal.clone(), false), all, "every panel shows while nothing hides them");
    let shift_tab = normal.with_panels_hidden(true);
    assert_eq!(
        shown_under(shift_tab.clone(), true),
        BTreeSet::from([CANVAS, TOOLBAR, PALETTE]),
        "Shift+Tab keeps the canvas and what the host names, docked and floating"
    );
    assert_eq!(shown_under(shift_tab, false), BTreeSet::from([CANVAS]), "Shift+Tab with nothing named keeps the canvas only");
    let tab = ShellCx::detached(WindowKey::main(), BodyMode::Only, room);
    assert_eq!(shown_under(tab, true), BTreeSet::from([CANVAS]), "Tab keeps the canvas only, whatever the host names");
}

#[trial]
fn a_revealed_edge_shows_its_dock_alone_under_tab_and_shift_tab() {
    let room = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
    let tab = ShellCx::detached(WindowKey::main(), BodyMode::Only, room);
    let shift_tab = ShellCx::detached(WindowKey::main(), BodyMode::Normal, room).with_panels_hidden(true);
    for (keys, hiding) in [("Tab", tab), ("Shift+Tab", shift_tab)] {
        let under = |edge: Option<RevealEdge>| match edge {
            Some(edge) => shown_under(hiding.clone().with_edge_reveal(edge, true), false),
            None => shown_under(hiding.clone(), false),
        };
        assert_eq!(under(Some(RevealEdge::Start)), BTreeSet::from([CANVAS, TOOLBAR]), "{keys}: the start edge shows the start dock alone");
        assert_eq!(under(Some(RevealEdge::End)), BTreeSet::from([CANVAS, LAYERS]), "{keys}: the end edge shows the end dock alone");
        assert_eq!(under(None), BTreeSet::from([CANVAS]), "{keys}: with no edge revealed no dock shows");
    }
}
