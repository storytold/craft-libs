//! Notices: the live region of an announced notice, a card's bullet lines and a centred notice's hosted content.

use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::contract::{Politeness, Props as _, Show as _};
use craft_ui::notices::{Notice, NoticeAction, NoticeLayout};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Vec2};
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

/// A click on a notice's action or its dismiss button is reported once, also in a frame that runs twice: the pass
/// that takes the press reports it.
#[trial]
fn a_notice_reports_its_action_and_dismissal_in_a_frame_that_runs_twice() {
    let taken = Rc::new(Cell::new((0_usize, 0_usize)));
    let seen = taken.clone();
    let mut fixture = Fixture::new(move |ui| {
        let out = Notice::new(Tone::Info, "This document is secured.").dismiss().action(NoticeAction::new("Security settings")).show(ui);
        let (actions, dismissals) = seen.get();
        seen.set((actions + usize::from(out.action == Some(0)), dismissals + usize::from(out.dismissed)));
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs the frame twice");
        }
    });
    fixture.room(Vec2::new(600.0, 200.0));
    fixture.settle();
    let action = fixture.by_label("Security settings").expect("the action");
    fixture.click(&action).settle();
    assert_eq!(taken.get().0, 1, "the action is reported once");
    let dismiss = fixture.by_label("Dismiss").expect("the dismiss button");
    fixture.click(&dismiss).settle();
    assert_eq!(taken.get().1, 1, "the dismissal is reported once");
}

/// A centred notice hosts the app's own leaf under its actions: the leaf sits centred under them, and a click on it
/// reaches the app through the leaf's output.
#[trial]
fn a_centred_notice_hosts_content_under_its_actions() {
    let clicked = Rc::new(Cell::new(false));
    let seen = clicked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let seen = seen.clone();
        let _ = Notice::new(Tone::Neutral, "Create a new document or open an existing file.")
            .layout(NoticeLayout::Centred)
            .title("No document")
            .action(NoticeAction::new("New Document"))
            .content(Button::new("Browse Samples"), move |out| seen.set(seen.get() || out.clicks > 0))
            .show(ui);
    });
    fixture.room(Vec2::new(500.0, 400.0));
    fixture.settle();
    let action = fixture.by_label("New Document").expect("the action");
    let hosted = fixture.by_label("Browse Samples").expect("the hosted leaf has its node");
    assert!(hosted.rect.top() >= action.rect.bottom(), "the leaf sits under the actions");
    assert!((hosted.rect.center().x - action.rect.center().x).abs() < 1.0, "both are centred in the column");
    fixture.click(&hosted).settle();
    assert!(clicked.get(), "the click reaches the app");
}
