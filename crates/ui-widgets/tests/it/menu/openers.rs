//! A menu with several openers: a context menu over a popover.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::popover::Popover;
use craft_ui::testing::{Fixture, pointer, trial};
use std::cell::Cell;
use std::rc::Rc;

fn menu_rect(fixture: &Fixture) -> Option<egui::Rect> {
    fixture.by_role(Role::MENU).first().map(|menu| menu.rect)
}

/// A context menu opened over the popover it was opened from takes the click on its row: the row runs and the menu
/// closes, and the popover stays open below it.
#[trial]
fn a_menu_opened_from_a_popover_takes_a_click_on_its_row() {
    let ran = Rc::new(Cell::new(false));
    let seen = ran.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Brush").show(ui);
        let mut open = true;
        Popover::new("Brush options", Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
            let target = Button::new("Target").show(ui);
            Button::new("Reset").show(ui);
            Button::new("Swap").show(ui);
            Menu::context("Target menu", &target).show(ui, |ui, _| {
                if MenuItem::new("Delete").show(ui).ran {
                    seen.set(true);
                }
            });
        });
    });
    fixture.settle();
    let at = fixture.by_label("Target").expect("the target has a node").center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    let delete = fixture.by_label("Delete").expect("a secondary click on the target opens the menu");
    let popover = fixture.by_role(Role::DIALOG).first().map(|popover| popover.rect).expect("the popover is open");
    let over = popover.intersect(delete.rect);
    assert!(over.is_positive(), "the row lies over the popover");
    let at = over.center();
    assert!(popover.contains(at) && delete.rect.contains(at), "the click point is inside both the popover and the row");
    fixture.click_at(at);
    fixture.settle();
    assert!(ran.get(), "the click ran the row");
    assert!(menu_rect(&fixture).is_none(), "the menu closed");
    assert!(fixture.by_label("Reset").is_some(), "the popover stays open");
}
