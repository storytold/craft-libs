//! Inspected regions: a region records its own components, hit areas and focus stops, names a node by its label, a
//! reader shown before it gets the record of the same pass, and the inspector numbers every stop.

use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, CommandSet, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::InspectOverlay;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::commands::CommandRef;
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

/// The inspected region's rect and its record, as the latest real pass showed them.
type Drawn = (Rect, Arc<Recorded>);

/// Each Tab-order badge shows whole inside the drawing, in both directions: a badge on a stop at the region's edge
/// moves in until it fits, and on an axis where it already fits it stays on its stop's leading top corner.
#[trial]
fn every_badge_shows_whole_inside_the_region() {
    use craft_ui::runtime::env::Direction;
    use egui::{Shape, vec2};

    for direction in [Direction::Ltr, Direction::Rtl] {
        let key = Id::NULL.with("trial.inspect.badges");
        let seen: Rc<RefCell<Option<Drawn>>> = Rc::new(RefCell::new(None));
        let out = Rc::clone(&seen);
        let mut snap = true;
        let mut fixture = Fixture::new(move |ui| {
            let (shown, recorded) = inspect::record(ui, key, |ui| {
                Stack::new().show(ui, |ui, _| {
                    Button::new("Apply").show(ui);
                    Checkbox::new(Bind::bound(&mut snap)).label("Snap to grid").show(ui);
                })
            });
            if let Some(recorded) = recorded {
                InspectOverlay::new(Arc::clone(&recorded)).show_at(ui, shown.output.rect);
                *out.borrow_mut() = Some((shown.output.rect, recorded));
            }
        });
        fixture.room(vec2(400.0, 300.0)).direction(direction);
        fixture.ctx().disable_accesskit();
        fixture.settle();
        let shapes = fixture.frame_shapes();
        let seen = seen.borrow();
        let (region, recorded) = seen.as_ref().expect("the region ran");
        let mut stops: Vec<(usize, usize)> =
            recorded.elements.iter().enumerate().filter_map(|(index, element)| Some((element.stop?, index))).collect();
        stops.sort_unstable();
        assert!(stops.len() >= 2, "{direction:?}: the region has stops: {stops:?}");
        for (number, index) in stops {
            let text = fixture.texts().iter().find(|text| text.text == number.to_string()).cloned().expect("the badge's number is painted");
            // The badge's disc and ring, centred on its number; a circle's stroke lies outside its radius.
            let reach = shapes
                .iter()
                .filter_map(|shape| match shape {
                    Shape::Circle(circle) if circle.center.distance(text.rect.center()) < 1.0 => Some(circle.radius + circle.stroke.width),
                    _ => None,
                })
                .fold(0.0_f32, f32::max);
            assert!(reach > 0.0, "{direction:?}: badge {number} has a disc round its number");
            let badge = Rect::from_center_size(text.rect.center(), vec2(2.0 * reach, 2.0 * reach));
            assert!(region.expand(0.05).contains_rect(badge), "{direction:?}: badge {number} at {badge:?} shows whole in the region {region:?}");
            assert!(region.expand(0.5).contains_rect(text.rect), "{direction:?}: badge {number}'s number at {:?} shows whole", text.rect);
            // Where the stop's corner leaves room for the whole badge, the badge stays on it.
            let stop = recorded.stop_rect(index).expect("the stop is recorded");
            let corner = if direction == Direction::Rtl { stop.right_top() } else { stop.left_top() };
            let room = region.shrink(reach);
            if (room.min.x..=room.max.x).contains(&corner.x) {
                assert!((badge.center().x - corner.x).abs() < 1.0, "{direction:?}: badge {number} stays on its stop across");
            }
            if (room.min.y..=room.max.y).contains(&corner.y) {
                assert!((badge.center().y - corner.y).abs() < 1.0, "{direction:?}: badge {number} stays on its stop down");
            }
        }
    }
}

/// The commands the command trial's buttons are bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "inspectTrial")]
#[non_exhaustive]
enum Bound {
    #[command(label = "Apply", modal = "always")]
    Apply,
    #[command(label = "Revert", modal = "always")]
    Revert,
}

#[trial]
fn an_element_records_the_command_it_is_bound_to_with_accesskit_off() {
    let key = Id::NULL.with("trial.inspect.command");
    let seen: Rc<RefCell<Option<Arc<Recorded>>>> = Rc::new(RefCell::new(None));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (_, recorded) = inspect::record(ui, key, |ui| {
            Stack::new().show(ui, |ui, _| {
                Button::command(Bound::Apply).show(ui);
                Button::command(Bound::Revert).show(ui);
                Button::new("Plain").show(ui);
            })
        });
        if recorded.is_some() {
            *out.borrow_mut() = recorded;
        }
    });
    // AccessKit off: no node is built, and the record still carries each control's command.
    fixture.ctx().disable_accesskit();
    fixture.frame(Vec::new());
    let seen = seen.borrow();
    let recorded = seen.as_deref().expect("the region ran");
    let buttons: Vec<&inspect::Element> =
        recorded.elements.iter().filter(|element| element.part.is_none() && element.component == "Button").collect();
    assert_eq!(buttons.len(), 3, "the three buttons are recorded: {buttons:?}");
    let commands: Vec<Option<CommandRef>> = buttons.iter().map(|element| element.command.clone()).collect();
    assert_eq!(
        commands,
        vec![Some(CommandRef::from(Bound::Apply)), Some(CommandRef::from(Bound::Revert)), None],
        "each bound button records its own command, the plain one none"
    );
    // The rect a badge is placed at is the bound control's own.
    for (index, element) in recorded.elements.iter().enumerate().filter(|(_, element)| element.command.is_some()) {
        let rect = recorded.stop_rect(index).expect("the element is recorded");
        assert!(rect.is_positive() && element.rect.expand(0.5).contains_rect(rect), "{:?} lies on its control {:?}", rect, element.rect);
    }
}
