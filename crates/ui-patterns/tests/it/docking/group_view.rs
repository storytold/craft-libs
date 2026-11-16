//! A group the host takes with `group_view` lays out in the group's rect, not in the room the dock area has left.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx};
use craft_ui::docking::{DockConfig, DockPanels, GroupAt, PanelSpec};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// Panel 0 is a canvas the host draws in its group's place; it records the group's rect and the `Ui` it was given.
struct Panels {
    seen: Rc<Cell<Option<(Rect, Rect, Rect)>>>,
}

impl DockPanels<usize> for Panels {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == 0 { spec.canvas() } else { spec }
    }

    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&0) {
            return None;
        }
        match at {
            GroupAt::Measure(_) => Some(0.0),
            GroupAt::Show(rect) => {
                self.seen.set(Some((rect, ui.max_rect(), ui.clip_rect())));
                Some(rect.height())
            }
        }
    }
}

#[trial]
fn a_group_view_lays_out_and_clips_to_its_group_rect() {
    let seen = Rc::new(Cell::new(None));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1], 0));
    let mut workspace = Workspace::new(Layout::new(Some(root), Vec::new()));
    let mut panels = Panels { seen: seen.clone() };
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let (rect, room, clip) = seen.get().expect("the canvas group was shown");
    assert!(rect.width() < 1000.0, "the canvas group takes part of the area: {rect:?}");
    assert!(rect.contains_rect(room), "the view's room {room:?} lies in its group's rect {rect:?}");
    assert!(rect.contains_rect(clip), "the view's clip {clip:?} lies in its group's rect {rect:?}");
}
