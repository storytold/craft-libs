//! Inspected regions: a region records its own components, hit areas and focus stops, names a node by its label, a
//! reader shown before it gets the record of the same pass, and the inspector numbers every stop.

use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, CommandSet, MemoProps, MemoView, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::InspectOverlay;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::inspect::{self, Recorded};
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, trial};
use egui::{Id, Rect, Ui};
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

/// A generic component is recorded by its type's own name (`Select`, not its last generic argument), and a text
/// field's facts list the text AccessKit reads as its value.
#[trial]
fn a_region_names_generic_components_and_lists_text_values() {
    use craft_ui::contract::Entry;
    use craft_ui::pickers::Select;
    use craft_ui::text::TextField;
    let key = Id::NULL.with("trial.inspect.generic");
    let seen = Rc::new(RefCell::new(None));
    let out = Rc::clone(&seen);
    let (mut choice, mut text) = (1u8, String::from("abc"));
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two")];
        let (_, recorded) = inspect::record(ui, key, |ui| {
            Select::new("Pick", Bind::bound(&mut choice), &entries).show(ui);
            TextField::new("Name", Bind::bound(&mut text)).show(ui);
        });
        if recorded.is_some() {
            *out.borrow_mut() = recorded;
        }
    });
    fixture.settle();
    let seen = seen.borrow();
    let recorded = seen.as_deref().expect("the region ran");
    let names: Vec<&str> = recorded.elements.iter().map(|element| element.component).collect();
    assert!(names.contains(&"Select"), "the Select is recorded by its type's name: {names:?}");
    let field = recorded.elements.iter().find(|element| element.component == "TextField").expect("the field is recorded");
    let value = field.facts.as_ref().and_then(|facts| facts.value.as_deref());
    assert_eq!(value, Some("abc"), "the field's facts list its text as the value");
}

/// A view showing one button, as a panel shows its slots in a memoized grid.
#[derive(MemoProps)]
struct Grid {
    label: &'static str,
}

fn grid(props: &mut Grid, ui: &mut Ui) {
    Button::new(props.label).show(ui);
}

/// What an inspected region records of a button in a memoized view and of a button beside it.
fn grid_and_footer(recorded: &Recorded) -> (Option<(usize, &inspect::Element)>, Option<(usize, &inspect::Element)>) {
    let named = |name: &str| {
        recorded.elements.iter().enumerate().find(|(_, element)| {
            element.part.is_none() && element.component == "Button" && element.facts.as_ref().and_then(|facts| facts.name.as_deref()) == Some(name)
        })
    };
    (named("Slot"), named("Footer"))
}

#[trial]
fn a_region_keeps_the_components_of_a_memoized_view_in_the_passes_it_replays() {
    let key = Id::NULL.with("trial.inspect.memo");
    let seen: Rc<RefCell<Option<Arc<Recorded>>>> = Rc::new(RefCell::new(None));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (_, recorded) = inspect::record(ui, key, |ui| {
            Stack::new().show(ui, |ui, _| {
                MemoView::new("grid", Grid { label: "Slot" }, grid).show(ui);
                Button::new("Footer").show(ui);
            })
        });
        if recorded.is_some() {
            *out.borrow_mut() = recorded;
        }
    });
    let replayed =
        |fixture: &Fixture| last_frame_memo_runs(fixture.ctx()).iter().any(|(of, _, run)| of.contains("Grid") && *run == MemoRun::Replayed);
    let mut replays = 0;
    for frame in 0..8 {
        fixture.frame(Vec::new());
        replays += usize::from(replayed(&fixture));
        let seen = seen.borrow();
        let recorded = seen.as_deref().expect("the region ran");
        let (slot, footer) = grid_and_footer(recorded);
        let (slot_at, slot) = slot.unwrap_or_else(|| panic!("frame {frame}: the memoized view's button is recorded: {:?}", recorded.elements));
        let (footer_at, footer) = footer.unwrap_or_else(|| panic!("frame {frame}: the button beside the view is recorded"));
        assert!(slot_at < footer_at, "frame {frame}: the view's button is recorded before the one after it");
        assert_eq!(slot.depth, footer.depth, "frame {frame}: both sit in the same stack");
        assert!(slot.rect.is_positive() && slot.facts.as_ref().is_some_and(|facts| facts.interactive), "frame {frame}: {slot:?}");
        assert!(recorded.hits.iter().any(|hit| hit.owner == slot.id && hit.rect.is_positive()), "frame {frame}: the view's button has its hit area");
        assert!(slot.stop.is_some() && slot.stop < footer.stop, "frame {frame}: Tab order runs through both, the view's button first");
    }
    assert!(replays > 0, "the view replayed in some of the passes");
}
