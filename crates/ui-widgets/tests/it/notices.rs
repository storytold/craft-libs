//! Notices: the live region of an announced notice.

use craft_theme::variant::Tone;
use craft_ui::contract::{Politeness, Props as _, Show as _};
use craft_ui::notices::Notice;
use craft_ui::testing::{Fixture, trial};
use egui::Id;
use std::cell::Cell;
use std::rc::Rc;

/// An announced notice is a live region whose name follows its message, so a message that changes while it shows is
/// announced again.
#[trial]
fn an_announced_notice_is_a_live_region_that_follows_its_message() {
    let message = Rc::new(Cell::new("No matching commands"));
    let id = Rc::new(Cell::new(Id::NULL));
    let (shown, seen) = (message.clone(), id.clone());
    let mut fixture = Fixture::new(move |ui| {
        seen.set(Notice::new(Tone::Neutral, shown.get()).announce(Politeness::Polite).id_salt("status").show(ui).response.id);
    });
    fixture.settle();
    assert_eq!(fixture.tree().live(id.get()), Some(Politeness::Polite), "the notice is a polite live region");
    assert!(fixture.by_label("No matching commands").is_some());
    message.set("No matching layers");
    fixture.settle();
    assert_eq!(fixture.tree().live(id.get()), Some(Politeness::Polite));
    assert!(fixture.by_label("No matching layers").is_some(), "the region's name is the new message");
}
