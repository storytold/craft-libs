//! Notices: the live region of an announced notice and a card's bullet lines.

use craft_theme::variant::Tone;
use craft_ui::contract::{Politeness, Props as _, Show as _};
use craft_ui::notices::{Notice, NoticeLayout};
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

/// A card with ten bullet lines shows the first eight, then one line that counts the two left out.
#[trial]
fn a_card_shows_eight_bullet_lines_and_counts_the_rest() {
    let mut fixture = Fixture::new(move |ui| {
        let lines: Vec<String> = (1..=10).map(|n| format!("Warning {n}")).collect();
        let _ = Notice::new(Tone::Warn, "").title("Opened with 10 warnings").layout(NoticeLayout::Card).lines(lines).show(ui);
    });
    fixture.settle();
    let painted = |text: &str| fixture.texts().iter().any(|painted| painted.text == text);
    assert!((1..=8).all(|n| painted(&format!("Warning {n}"))), "the first eight lines show");
    assert!(!painted("Warning 9") && !painted("Warning 10"), "the rest are left out");
    assert!(painted("\u{2026}and 2 more"), "one line counts them");
    assert_eq!(fixture.texts().iter().filter(|painted| painted.text == "\u{2022}").count(), 8, "each shown line has a bullet");
}
