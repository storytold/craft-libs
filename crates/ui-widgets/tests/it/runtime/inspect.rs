//! Inspected regions: a region records its own components, hit areas and focus stops, names a node by its label, a
//! reader shown before it gets the record of the same pass, and the inspector numbers every stop.

use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::InspectOverlay;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::inspect::{self, Recorded};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Rect};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

/// What the latest real pass saw.
struct Seen {
    early: Option<Arc<Recorded>>,
    recorded: Option<Arc<Recorded>>,
    region: Rect,
    other: Rect,
}

#[trial]
fn a_region_records_its_own_elements_and_numbers_every_stop() {
    let key = Id::NULL.with("trial.inspect");
    let seen = Rc::new(RefCell::new(Seen { early: None, recorded: None, region: Rect::NOTHING, other: Rect::NOTHING }));
    let out = Rc::clone(&seen);
    let (mut snap, mut size) = (true, 1u8);
    let mut fixture = Fixture::new(move |ui| {
        let early = inspect::recorded(ui.ctx(), key);
        let (shown, recorded) = inspect::record(ui, key, |ui| {
            Stack::new().show(ui, |ui, _| {
                Button::new("Apply").show(ui);
                Checkbox::new(Bind::bound(&mut snap)).label("Snap to grid").show(ui);
                RadioGroup::new("Size", Bind::bound(&mut size)).show(ui, |ui, _| {
                    Radio::new(1u8).label("Small").show(ui);
                    Radio::new(2u8).label("Medium").show(ui);
                    Radio::new(3u8).label("Large").show(ui);
                });
            })
        });
        let other = Stack::new().show(ui, |ui, _| {
            Button::new("Elsewhere").show(ui);
        });
        if let Some(recorded) = &recorded {
            InspectOverlay::new(Arc::clone(recorded)).show_at(ui, shown.output.rect);
        }
        if recorded.is_some() {
            *out.borrow_mut() = Seen { early, recorded, region: shown.output.rect, other: other.output.rect };
        }
    });
    // AccessKit off, as in an app no screen reader reads: the record does not depend on it.
    fixture.ctx().disable_accesskit();
    // One frame: the reader shown before the region finds its record within it.
    fixture.frame(Vec::new());
    let seen = seen.borrow();
    let recorded = seen.recorded.as_deref().expect("the region ran");
    assert_eq!(seen.early.as_deref(), Some(recorded), "a reader shown before the region gets this pass's record");

    let elements = &recorded.elements;
    let inside = seen.region.expand(0.5);
    for element in elements.iter().filter(|element| element.rect.is_positive()) {
        assert!(inside.contains_rect(element.rect), "{} at {:?} lies in the region {:?}", element.component, element.rect, seen.region);
        assert!(!element.rect.intersects(seen.other.shrink(0.5)), "{} meets the stack outside the region", element.component);
    }
    let ids: HashSet<Id> = elements.iter().map(|element| element.id).collect();
    assert!(recorded.hits.iter().all(|hit| ids.contains(&hit.owner)), "the region's hit areas are its own components'");
    let components = |name: &str| elements.iter().filter(|element| element.part.is_none() && element.component == name).count();
    assert_eq!(components("Button"), 1, "the button outside the region is not recorded");
    let checkbox =
        elements.iter().find(|element| element.part.is_none() && element.component == "Checkbox").and_then(|element| element.facts.as_ref());
    assert_eq!(checkbox.and_then(|facts| facts.name.as_deref()), Some("Snap to grid"), "a node is named by the label that labels it: {checkbox:?}");

    // Every radio has its own number, and the numbers run 1..n in show order.
    let stops: Vec<(usize, &str)> = elements.iter().filter_map(|element| Some((element.stop?, element.component))).collect();
    let numbers: Vec<usize> = stops.iter().map(|(number, _)| *number).collect();
    assert_eq!(numbers, (1..=stops.len()).collect::<Vec<_>>(), "{stops:?}");
    assert_eq!(stops.iter().filter(|(_, component)| *component == "Radio").count(), 3, "{stops:?}");
    assert!(stops.iter().any(|(_, component)| *component == "Button") && stops.iter().any(|(_, component)| *component == "Checkbox"), "{stops:?}");

    // The drawing numbers each stop.
    let painted: HashSet<String> = fixture.texts().iter().map(|text| text.text.clone()).collect();
    for number in 1..=stops.len() {
        assert!(painted.contains(&number.to_string()), "badge {number} is painted");
    }
}
