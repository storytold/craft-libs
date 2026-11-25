//! `ReorderableSections`: the section menu runs the commands of the section it was opened on, reconciling the stored
//! arrangement with the registered sections writes nothing until the user changes it, headers show counts and buttons,
//! and Reset restores the usual arrangement.

use craft_behavior::IconId;
use craft_ui::contract::{Bind, Role};
use craft_ui::sections::{ReorderableSections, SectionButton, SectionEvent, SectionKey, SectionLayout, SectionSpec};
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

/// A sidebar whose Albums header has a button and a count, Folders a count and Keywords no rows and an empty hint;
/// the events it reports go to `events`.
fn counted_sidebar(layout: Rc<RefCell<SectionLayout>>, events: Rc<RefCell<Vec<SectionEvent>>>) -> Fixture {
    let specs = vec![
        SectionSpec::new("albums", "Albums").count(2).button(SectionButton::new("albumNew", IconId::new("plus"), "Create Album")),
        SectionSpec::new("folders", "Folders").count(4),
        SectionSpec::new("keywords", "Keywords").count(0).empty_hint("No keywords yet"),
    ];
    Fixture::new(move |ui| {
        let mut layout = layout.borrow_mut();
        let out = ReorderableSections::new("Sidebar", Bind::bound(&mut *layout), &specs).show(ui, |ui, key, _| {
            if key.as_str() != "keywords" {
                ui.label("body");
            }
        });
        events.borrow_mut().extend(out.events);
    })
}

#[trial]
fn a_folded_section_shows_its_count_and_an_empty_one_its_hint_and_a_header_button_press_neither_folds_nor_moves() {
    let mut start = SectionLayout::new(IDS.iter().map(|id| key(id)));
    start.set_folded(&key("folders"), true);
    let layout = Rc::new(RefCell::new(start.clone()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut fixture = counted_sidebar(Rc::clone(&layout), Rc::clone(&events));
    fixture.settle();
    let texts: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    assert!(texts.iter().any(|text| text == "4"), "the folded section shows its row count: {texts:?}");
    assert!(!texts.iter().any(|text| text == "2"), "an open section shows no count: {texts:?}");
    assert!(texts.iter().any(|text| text == "No keywords yet"), "an open section with no rows shows its hint: {texts:?}");
    let button = fixture.by_label("Create Album").expect("the Albums header shows its button");
    fixture.click(&button);
    fixture.settle();
    let pressed = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, SectionEvent::Button { key, id, .. } if key.as_str() == "albums" && *id == "albumNew"))
        .count();
    assert_eq!(pressed, 1, "the press is reported once, as the Albums button");
    assert_eq!(*layout.borrow(), start, "the press neither folded nor moved a section");
}

#[trial]
fn reset_in_the_section_menu_puts_a_moved_section_back() {
    let usual = SectionLayout::new(IDS.iter().map(|id| key(id)));
    let layout = Rc::new(RefCell::new(usual.clone()));
    let mut fixture = sidebar(Rc::clone(&layout));
    fixture.settle();
    let menu_row = |fixture: &mut Fixture, label: &str| {
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.key(Key::F10, Modifiers::SHIFT);
        fixture.settle();
        let row = fixture.by_label(label).unwrap_or_else(|| panic!("the section menu lists {label}"));
        fixture.click(&row);
        fixture.settle();
    };
    menu_row(&mut fixture, "Move Section Down");
    assert_ne!(*layout.borrow(), usual, "Move Section Down moved the first section");
    menu_row(&mut fixture, "Reset Sections");
    assert_eq!(*layout.borrow(), usual, "Reset Sections restored the usual arrangement");
}
