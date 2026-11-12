//! Submenus: a row opens its submenu as soon as the pointer is on it, and an open submenu stays open while the
//! pointer heads for it across other rows.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, pointer, trial};

/// A menu with two submenu rows, Recent and Templates, the first with a submenu taller than the rows below it.
fn open_menu() -> Fixture {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Main").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("New").show(ui);
            Menu::submenu_row(ui, "Recent", |ui, _| {
                for name in ["Report.txt", "Notes.txt", "Plan.txt", "Budget.txt", "Draft.txt"] {
                    MenuItem::new(name).show(ui);
                }
            });
            Menu::submenu_row(ui, "Templates", |ui, _| {
                MenuItem::new("Blank").show(ui);
            });
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener);
    fixture.settle();
    fixture
}

#[trial]
fn a_submenu_opens_as_soon_as_the_pointer_is_on_its_row() {
    let mut fixture = open_menu();
    let recent = fixture.by_label("Recent").expect("the menu is open").center();
    fixture.frame(vec![pointer::moved(recent)]).idle(1);
    assert!(fixture.by_label("Report.txt").is_some(), "the submenu opens without the pointer resting on its row");
}

#[trial]
fn an_open_submenu_stays_open_while_the_pointer_heads_for_it_across_another_row() {
    let mut fixture = open_menu();
    let recent = fixture.by_label("Recent").expect("the menu is open").center();
    fixture.frame(vec![pointer::moved(recent)]).idle(1);
    let submenu = fixture.by_label("Report.txt").expect("the submenu is open").rect;
    let templates = fixture.by_label("Templates").expect("the second submenu row").rect;
    // A point on Templates, near the submenu's edge and level with its rows: the pointer heads for the submenu.
    let crossing = egui::pos2(templates.max.x - 2.0, templates.center().y);
    assert!(crossing.y < submenu.max.y, "the submenu reaches below the crossing point");
    fixture.frame(vec![pointer::moved(crossing)]).idle(1);
    assert!(fixture.by_label("Report.txt").is_some(), "the submenu the pointer heads for stays open");
    assert!(fixture.by_label("Blank").is_none(), "the crossed row does not open its submenu");
    fixture.advance(0.4).idle(2);
    assert!(fixture.by_label("Blank").is_some(), "the crossed row takes over once the pointer rests on it");
}
