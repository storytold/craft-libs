//! About's Contributors tab: the grab bag of name links and the Table view.

use craft_ui::contract::Role;
use craft_ui::patterns::form_dialogs::{AboutDialog, AboutInfo, AboutTab, Contributor, Credits};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

static INFO: LazyLock<AboutInfo> = LazyLock::new(|| AboutInfo::new("PhotoCraft", "An image editor.", "1.0"));
static CREDITS: LazyLock<Credits> = LazyLock::new(|| {
    Credits::new(
        vec![
            Contributor::new("alice").profile("https://github.com/alice").counts(1, 12),
            Contributor::new("bob").profile("https://github.com/bob").counts(3, 40),
        ],
        Vec::new(),
        52,
    )
});

#[trial]
fn a_click_on_a_name_in_the_grab_bag_reports_its_profile() {
    let links = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&links);
    let mut fixture = Fixture::new(move |ui| {
        let shown = AboutDialog::new(&INFO, &CREDITS).tab(AboutTab::Contributors).show(ui);
        seen.borrow_mut().extend(shown.link);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert!(fixture.by_role(Role::TABLE).is_empty(), "the tab opens on the grab bag");
    let bob = fixture.by_label("@bob").expect("the grab bag lists bob");
    fixture.click(&bob).idle(4);
    assert_eq!(links.borrow().as_slice(), ["https://github.com/bob".to_owned()], "one click reports bob's profile");
    let table = fixture.by_label("Table").expect("the View control offers Table");
    fixture.click(&table).idle(10);
    assert!(!fixture.by_role(Role::TABLE).is_empty(), "Table shows the contributors as a table");
}
