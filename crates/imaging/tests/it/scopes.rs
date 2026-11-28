//! The scope panel and its parts: rows per view, the plot's levels and range, the three refresh paths, the clipping
//! buttons, the tone zones and the commands.

use craft_behavior::EditEvent;
use craft_imaging::ui::{ToneZone, ToneZones, ZoneScale, ZoneValues};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, trial};
use egui::vec2;
use std::cell::RefCell;
use std::rc::Rc;

/// The zone values the rig starts from: each zone at its index, in -100 to 100.
fn start_values() -> ZoneValues {
    ZoneValues::new(vec![0.0, 1.0, 2.0, 3.0, 4.0])
}

/// A fixture of the tone zones alone over a 200 by 100 plot at the origin, over values that start at
/// [`start_values`] and stay there, and the edits they report.
fn zone_rig() -> (Fixture, Rc<RefCell<Vec<EditEvent<ZoneValues>>>>) {
    let edits = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&edits);
    let mut fixture = Fixture::new(move |ui| {
        let zones = ToneZone::CAMERA_RAW;
        let scales = zones.map(|_| ZoneScale::new(-100.0..=100.0, 200.0, 1.0, 0.0));
        let texts: Vec<Label> = ["0", "1", "2", "3", "4"].map(Label::from).to_vec();
        let rect = ui.max_rect();
        let out = ToneZones::new("Tone zones", &zones, &scales, &texts).value(Bind::given(start_values())).show_at(ui, rect);
        seen.borrow_mut().extend(out.events);
    });
    fixture.settle();
    (fixture, edits)
}

#[trial]
fn tone_zone_drag_is_one_gesture() {
    let (mut fixture, edits) = zone_rig();
    let zone = fixture.by_label("Shadows").expect("a slider per zone");
    let by = zone.rect.width() * 0.5;
    fixture.drag(&zone, vec2(by, 0.0)).settle();
    let events = edits.borrow();
    let begins = events.iter().filter(|event| matches!(event, EditEvent::Begin { .. } | EditEvent::Set { .. })).count();
    assert_eq!(begins, 1, "one gesture per drag: {events:?}");
    let start = start_values();
    let only_shadows = |values: &ZoneValues| (0..5).all(|at| at == 1 || values.0[at] == start.0[at]);
    assert!(events.iter().filter_map(|event| event.value()).all(only_shadows), "the press locks the zone: {events:?}");
    let last = events.iter().rev().find_map(|event| event.value()).expect("the drag reports values");
    assert!(last.0[1] > start.0[1], "a drag to the right raises Shadows from its value at the press: {events:?}");
}
