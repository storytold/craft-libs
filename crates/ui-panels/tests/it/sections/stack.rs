//! A section stack moves a section by a drag from its header's title, as from the rest of its header: the title only
//! clicks, so the drag falls through to the header's reorder band.

use craft_ui::contract::{Props as _, Role, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::Text;
use craft_ui::menu::MenuItem;
use craft_ui::sections::{DropAction, Section, SectionStack};
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Ui, vec2};
use std::cell::RefCell;
use std::rc::Rc;

const TITLES: [&str; 3] = ["Layers", "Effects", "Paths"];

/// The title row of the section titled `title`.
fn title_row(fixture: &Fixture, title: &str) -> Target {
    let tree = fixture.tree();
    fixture.by_role(Role::HEADING).into_iter().find(|row| tree.name_of(row.node) == Some(title)).expect("the section's title row")
}

/// A drag from a collapsible section's title, where a click folds it, moves the section in the stack, with a header
/// menu and in a drop slot as an app's library groups are; a click there still folds it.
#[trial]
fn a_drag_from_a_collapsible_section_title_moves_the_section() {
    let order = Rc::new(RefCell::new(vec![0_u8, 1, 2]));
    let held = Rc::clone(&order);
    let mut fixture = Fixture::new(move |ui| {
        let shown = held.borrow().clone();
        let stack = SectionStack::new(&shown);
        let rows = stack.rows();
        let moves = stack
            .show(ui, |ui, _| {
                rows.each(ui, |ui, key, slot| {
                    let title = TITLES[usize::from(*key)];
                    let section = |ui: &mut Ui| {
                        Section::new(title)
                            .in_stack(slot)
                            .menu(|ui, _| _ = MenuItem::new("Rename").show(ui))
                            .show(ui, |ui, _| {
                                for line in 0..6 {
                                    Text::new(format!("{title} line {line}")).show(ui);
                                }
                            })
                            .output
                    };
                    let wrapped = DropAction::new(|_: &String| true).sizing(Sizing::FillWidth).show(ui, |ui, _| section(ui));
                    wrapped.inner.unwrap_or_else(|| section(ui))
                });
            })
            .output
            .moves;
        let mut order = held.borrow_mut();
        for moved in moves.iter().filter_map(|edit| edit.value()) {
            if let Some(from) = order.iter().position(|key| *key == moved.key) {
                let key = order.remove(from);
                let to = moved.to.min(order.len());
                order.insert(to, key);
            }
        }
    });
    fixture.room(vec2(320.0, 480.0)).settle();
    let (layers, paths) = (title_row(&fixture, "Layers"), title_row(&fixture, "Paths"));
    fixture.drag_at(layers.center(), vec2(0.0, paths.rect.bottom() - layers.center().y));
    fixture.settle();
    assert_ne!(order.borrow().first(), Some(&0), "Layers left the top: {:?}", order.borrow());

    let tree = fixture.tree();
    let lead = fixture.by_role(Role::BUTTON).into_iter().find(|button| tree.name_of(button.node) == Some("Effects")).expect("the disclosure");
    assert_eq!(tree.expanded_of(lead.node), Some(true), "Effects starts open");
    let effects = title_row(&fixture, "Effects");
    fixture.click(&effects).settle();
    assert_eq!(fixture.tree().expanded_of(lead.node), Some(false), "a click on the title folds the section");
}
