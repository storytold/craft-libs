//! The scope panel and its parts: rows per view, the plot's levels and range, the three refresh paths, the clipping
//! buttons, the tone zones and the commands.

use craft_behavior::EditEvent;
use craft_imaging::ui::{ScopeZoneEdit, ToneZone, ToneZones};
use craft_ui::contract::Show as _;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, trial};
use egui::vec2;
use std::cell::RefCell;
use std::rc::Rc;

/// A fixture of the tone zones alone over a 200 by 100 plot at the origin, and the edits they report.
fn zone_rig() -> (Fixture, Rc<RefCell<Vec<EditEvent<ScopeZoneEdit>>>>) {
    let edits = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&edits);
    let mut fixture = Fixture::new(move |ui| {
        let zones = ToneZone::CAMERA_RAW;
        let texts: Vec<Label> = ["0", "1", "2", "3", "4"].map(Label::from).to_vec();
        let rect = ui.max_rect();
        let out = ToneZones::new("Tone zones", &zones, &texts).show_at(ui, rect);
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
    assert!(events.iter().filter_map(|event| event.value()).all(|edit| edit.key == "shadows"), "the press locks the zone: {events:?}");
}
