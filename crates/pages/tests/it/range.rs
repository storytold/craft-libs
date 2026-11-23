//! The page range field's menu of the app's page choices.

use craft_pages::ui::PageRangeField;
use craft_ui::contract::Entry;
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

/// The menu button lists the app's entries and a pick reports their pages; a field given no entries has no button.
#[trial]
fn the_menu_lists_the_apps_entries_and_a_field_without_entries_has_no_button() {
    let picked: Rc<RefCell<Option<Vec<usize>>>> = Rc::default();
    let seen = picked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let menu = [Entry::new(vec![0, 2, 4], "Odd pages only"), Entry::new(vec![1, 3], "Even pages only")];
        if let Some(pages) = PageRangeField::new("Pages", &[], 5, 0).menu(&menu).show(ui).picked {
            *seen.borrow_mut() = Some(pages);
        }
    });
    fixture.settle();
    let button = fixture.by_label("Select pages").expect("the field has a menu button");
    fixture.click(&button).settle();
    let odd = fixture.by_label("Odd pages only").expect("the menu lists the app's entries");
    assert!(fixture.by_label("Even pages only").is_some(), "every entry is listed");
    fixture.click(&odd).settle();
    assert_eq!(*picked.borrow(), Some(vec![0, 2, 4]), "the pick reports the entry's pages");

    let mut bare = Fixture::new(|ui| {
        let _ = PageRangeField::new("Pages", &[], 5, 0).show(ui);
    });
    bare.settle();
    assert!(bare.by_label("Select pages").is_none(), "no entries, no button");
}
