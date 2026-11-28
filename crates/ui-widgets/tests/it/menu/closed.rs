//! A closed menu runs nothing: its opener runs, and the menu's run starts when it opens.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, last_pass_runs, trial};
use egui::{Key, Modifiers};

fn menu_runs(fixture: &Fixture) -> u32 {
    last_pass_runs(fixture.ctx()).iter().filter(|(ty, _)| ty.ends_with("::Menu<'_>") || ty.ends_with("::Menu")).map(|(_, runs)| runs).sum()
}

/// A settled pass with the menu closed runs no `Menu`; opened from its button it runs, and after Escape and its exit
/// fade it runs no more.
#[trial]
fn a_closed_menu_runs_nothing_until_it_opens() {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Actions").show(ui);
        Menu::new("Actions menu").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("Rename").show(ui);
        });
    });
    fixture.settle().idle(1);
    assert_eq!(menu_runs(&fixture), 0, "a closed menu ran: {:?}", last_pass_runs(fixture.ctx()));
    let at = fixture.by_label("Actions").expect("the opener has a node").center();
    fixture.click_at(at).settle();
    assert!(fixture.by_label("Rename").is_some(), "a click on the opener opens the menu");
    fixture.idle(1);
    assert!(menu_runs(&fixture) > 0, "an open menu runs");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(fixture.by_label("Rename").is_none(), "Escape closes the menu");
    // The exit fade runs the menu; the pass after it ends runs none.
    let faded = (0..120).find(|_| {
        fixture.idle(1);
        menu_runs(&fixture) == 0
    });
    assert!(faded.is_some(), "a menu closed by Escape still runs after its exit fade");
}
