//! The label scope and the merge rule.

use craft_ui::contract::{FieldLabel, Props as _, Show as _, ShowContainer as _};
use craft_ui::testing::access::{AccessTree, NodeKey, begin_label_scope};
use craft_ui::testing::memory::global;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use egui::{Id, Rect};
use std::cell::RefCell;

fn field_label() -> FieldLabel {
    FieldLabel::new(global("access.label", "name"), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(40.0, 10.0)), "Name")
}

/// One frame of a container with a side label around `controls` probes.
fn field(frames: &mut Frames, controls: usize, keeps_name: bool) -> (Id, Vec<Id>, AccessTree) {
    let container = global("access.field", "scope");
    let mut ids = Vec::new();
    frames.frame(vec![], |ui| {
        ids.clear();
        Panel::new().id(container).show(ui, |ui, _| {
            // The container's body opens the scope; its report step closes it and patches the nodes registered in it.
            begin_label_scope(ui.ctx(), container, field_label());
            for at in 0..controls {
                let seen = RefCell::new(Seen::default());
                Probe::new(&seen).label(format!("Control {at}")).keeps_name(keeps_name).show(ui);
                ids.extend(seen.borrow().id);
            }
        });
    });
    (container, ids, frames.tree())
}

#[trial]
fn several_controls_make_the_container_labelled() {
    let mut frames = Frames::new();
    let (container, ids, tree) = field(&mut frames, 2, false);
    assert_eq!(tree.labelled_by(container), [NodeKey::of(field_label().id)]);
    assert!(ids.iter().all(|id| tree.labelled_by(*id).is_empty()), "the controls keep their own names");
}
