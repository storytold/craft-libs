//! A panel whose body is the app's own egui code (`PanelSpec::host_body`) lays out with the style of the `Ui` the dock
//! was shown in; any other body keeps the group's spacing.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, trial};
use egui::{Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The app's own egui panel; the other is a plain panel.
const HOSTED: usize = 0;

#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == HOSTED { spec.host_body() } else { spec }
    }
}

/// Records the item spacing each body's `Ui` has, by panel.
struct Panels {
    seen: Rc<RefCell<[Option<Vec2>; 2]>>,
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        self.seen.borrow_mut()[*panel] = Some(ui.spacing().item_spacing);
    }
}

#[trial]
fn a_host_body_lays_out_with_the_style_outside_the_dock_and_other_bodies_keep_the_groups() {
    let seen = Rc::new(RefCell::new([None; 2]));
    let outside = Rc::new(RefCell::new(None));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (Panels { seen: seen.clone() }, Tracked::new(Specs));
    let outer = outside.clone();
    let mut fixture = Fixture::new(move |ui| {
        *outer.borrow_mut() = Some(ui.spacing().item_spacing);
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let outside = outside.borrow().expect("the dock was shown");
    let [hosted, plain] = *seen.borrow();
    assert_eq!(hosted, Some(outside), "the app's own egui body lays out as it does outside the dock");
    let plain = plain.expect("the plain body was shown");
    assert_ne!(plain, outside, "a framework body keeps the group's spacing");
}
