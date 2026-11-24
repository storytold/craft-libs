//! A panel shows no header unless it is titled: in a dock the tab names it, and its title only names the group for
//! assistive technology.

use craft_ui::contract::{Role, ShowContainer as _};
use craft_ui::sections::Panel;
use craft_ui::testing::{Fixture, trial};

#[trial]
fn title_note_remains_accessible_when_the_row_is_narrow() {
    use craft_ui::contract::Show as _;
    use craft_ui::sections::TitleRow;
    let mut fixture = Fixture::new(|ui| {
        ui.set_max_width(100.0);
        TitleRow::new("Properties").note("A long description of the selection").show(ui);
    });
    fixture.settle();
    let heading = fixture.by_label("Properties").expect("title node");
    assert!(fixture.tree().description_of(heading.node).is_some_and(|description| description.contains("A long description of the selection")));
    assert!(fixture.texts().iter().any(|run| run.text.contains('·')), "the note is painted even when it is cut");
}

fn panel(titled: bool) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let panel = Panel::new("Brush settings");
        let panel = if titled { panel.titled() } else { panel };
        panel.show(ui, |ui, _| {
            ui.label("body");
        });
    });
    fixture.settle();
    fixture
}

#[trial]
fn a_panel_paints_its_title_only_when_titled_and_is_named_by_it_either_way() {
    let plain = panel(false);
    assert!(!plain.texts().iter().any(|run| run.text == "Brush settings"), "an untitled panel paints no title");
    assert!(plain.by_label("Brush settings").is_some(), "the title still names the panel");
    assert!(plain.by_role(Role::HEADING).is_empty(), "an untitled panel has no title row");
    let titled = panel(true);
    assert!(titled.texts().iter().any(|run| run.text == "Brush settings"), "a titled panel paints its title in a header");
}

#[trial]
fn panel_routes_unused_arrow_from_button_to_its_collection() {
    use craft_behavior::keyed;
    use craft_ui::buttons::Button;
    use craft_ui::contract::{CollectionEvent, Props as _, Show as _, ShowCollection as _};
    use craft_ui::list::{List, ListRow};
    use craft_ui::runtime::focus::{FocusTarget, request};
    use egui::{Id, Key, Modifiers};
    use std::cell::RefCell;
    use std::rc::Rc;
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&events);
    let list_id = Id::unique("panel fallback list");
    let button_id = Id::unique("panel button");
    let mut fixture = Fixture::new(move |ui| {
        Panel::new("Region").fallback_collection(list_id).show(ui, |ui, _| {
            Button::new("Action").id(button_id).show(ui);
            let keys = [0_usize, 1, 2];
            let source = keyed(&keys, |key: &usize| *key);
            let out = List::new("Items").id(list_id).natural_height(true).show_items(ui, &source, &mut |key, _| ListRow::new(format!("Item {key}")));
            seen.borrow_mut().extend(out.events);
        });
    });
    fixture.settle();
    request(fixture.ctx(), FocusTarget::Id(button_id));
    fixture.settle();
    fixture.key(Key::ArrowDown, Modifiers::NONE).settle();
    assert!(events.borrow().iter().any(|event| matches!(event, CollectionEvent::LeadMoved { .. })), "the list receives the move");
}
