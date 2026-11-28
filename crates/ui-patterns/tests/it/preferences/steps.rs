//! Previous and Next in the preferences window's footer.

use craft_ui::contract::Bind;
use craft_ui::patterns::preferences::{
    PageSpec, PrefValue, PreferencesDraft, PreferencesModel, PreferencesState, PreferencesWindow, RowKind, RowSpec,
};
use craft_ui::testing::{Fixture, trial};
use egui::Ui;
use std::cell::RefCell;
use std::rc::Rc;

struct Model;

impl PreferencesModel for Model {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        ["General", "Type", "Units"]
            .into_iter()
            .map(|title| PageSpec::new(title, title, vec![RowSpec::new(title, title, RowKind::Bool(false))]))
            .collect()
    }
    fn set(&mut self, _: &str, _: PrefValue) {}
    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn reset_page(&mut self, _: &str) {}
}

impl PreferencesDraft for Model {
    fn differs(&self) -> bool {
        false
    }
    fn apply(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn cancel(&mut self) {}
}

/// A window with Previous and Next, and the page it shows after each pass.
fn window() -> (Fixture, Rc<RefCell<Option<String>>>) {
    let page = Rc::new(RefCell::new(None));
    let seen = page.clone();
    let mut state = PreferencesState::default();
    let fixture = Fixture::new(move |ui| {
        let shown = PreferencesWindow::new(Bind::bound(&mut state)).page_steps().embedded().show(ui, &mut Model);
        *seen.borrow_mut() = shown.page;
    });
    (fixture, page)
}

/// Whether the footer button `label` is disabled.
fn disabled(fixture: &Fixture, label: &str) -> bool {
    let button = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} button"));
    fixture.tree().is_disabled_key(button.node)
}

#[trial]
fn next_and_previous_step_through_the_pages() {
    let (mut fixture, page) = window();
    fixture.settle();
    assert_eq!(page.borrow().as_deref(), Some("General"));
    assert!(disabled(&fixture, "Previous"), "the first page has no page before it");
    let next = fixture.by_label("Next").expect("the Next button");
    fixture.click(&next).settle();
    assert_eq!(page.borrow().as_deref(), Some("Type"), "Next shows the next page");
    let next = fixture.by_label("Next").expect("the Next button");
    fixture.click(&next).settle();
    assert_eq!(page.borrow().as_deref(), Some("Units"));
    assert!(disabled(&fixture, "Next"), "the last page has no page after it");
    let previous = fixture.by_label("Previous").expect("the Previous button");
    fixture.click(&previous).settle();
    assert_eq!(page.borrow().as_deref(), Some("Type"), "Previous shows the page before");
}

#[trial]
fn the_steps_follow_the_pages_the_search_lists() {
    let (mut fixture, page) = window();
    fixture.settle();
    let search = fixture.by_label("Search settings").expect("the search field");
    fixture.click(&search).settle();
    // Lists General and Units, not Type.
    fixture.type_text("n").settle();
    assert_eq!(page.borrow().as_deref(), Some("General"));
    let next = fixture.by_label("Next").expect("the Next button");
    fixture.click(&next).settle();
    assert_eq!(page.borrow().as_deref(), Some("Units"), "Next passes over the page the search hides");
    assert!(disabled(&fixture, "Next"), "Units is the last listed page");
}
