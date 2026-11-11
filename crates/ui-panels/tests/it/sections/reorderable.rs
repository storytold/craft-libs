//! `ReorderableSections`: the section menu runs the commands of the section it was opened on, and reconciling the
//! stored arrangement with the registered sections writes nothing until the user changes it.

use craft_ui::contract::{Bind, Role};
use craft_ui::sections::{ReorderableSections, SectionKey, SectionLayout, SectionSpec};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

const IDS: [&str; 3] = ["albums", "folders", "keywords"];

fn key(id: &str) -> SectionKey {
    SectionKey::new(id)
}

fn sidebar(layout: Rc<RefCell<SectionLayout>>) -> Fixture {
    let specs: Vec<SectionSpec> = IDS.iter().map(|id| SectionSpec::new(*id, *id)).collect();
    Fixture::new(move |ui| {
        let mut layout = layout.borrow_mut();
        let _ = ReorderableSections::new("Sidebar", Bind::bound(&mut *layout), &specs).show(ui, |ui, _, _| {
            ui.label("body");
        });
    })
}

#[trial]
fn a_menu_row_runs_on_the_section_the_menu_opened_on_after_focus_moves_into_the_menu() {
    let layout = Rc::new(RefCell::new(SectionLayout::new(IDS.iter().map(|id| key(id)))));
    let mut fixture = sidebar(Rc::clone(&layout));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::F10, Modifiers::SHIFT);
    fixture.settle();
    assert!(!fixture.by_role(Role::MENU).is_empty(), "Shift+F10 on the first section opens its menu");
    let fold = fixture.by_label("Fold Section").expect("the menu lists the section's Fold command");
    fixture.click(&fold);
    fixture.settle();
    let layout = layout.borrow();
    assert!(layout.is_folded(&key("albums")), "the command folded the section the menu opened on");
    assert!(!layout.is_folded(&key("folders")) && !layout.is_folded(&key("keywords")), "no other section folded");
}

#[trial]
fn showing_an_arrangement_that_needs_reconciling_writes_nothing_until_the_user_changes_it() {
    let layout = Rc::new(RefCell::new(SectionLayout::new([key("albums")])));
    let stored = layout.borrow().clone();
    let mut fixture = sidebar(Rc::clone(&layout));
    fixture.idle(4);
    assert_eq!(*layout.borrow(), stored, "passes without a user event leave the stored arrangement as it was");
}
