//! A panel shows no header unless it is titled: in a dock the tab names it, and its title only names the group for
//! assistive technology.

use craft_ui::contract::Role;
use craft_ui::sections::Panel;
use craft_ui::testing::{Fixture, trial};

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
