//! The classic preferences window: OK, Cancel, Prev and Next in a column beside the page, Prev and Next stepping
//! through the listed pages, OK and Cancel as before.

use craft_ui::contract::Bind;
use craft_ui::patterns::preferences::{
    GroupSpec, PageSpec, PrefValue, PreferencesDraft, PreferencesLayout, PreferencesModel, PreferencesState, PreferencesWindow, RowKind, RowSpec,
};
use craft_ui::testing::{Fixture, trial};
use egui::Ui;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Model {
    draft: bool,
    saved: bool,
}

impl PreferencesModel for Model {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        vec![
            PageSpec::new("general", "General", vec![RowSpec::new("snap", "Snap", RowKind::Bool(self.draft))]),
            PageSpec::new("tools", "Tools", Vec::new())
                .group(GroupSpec::new(Some("Options"), vec![RowSpec::new("tips", "Show Tips", RowKind::Bool(true))])),
            PageSpec::new("type", "Type", vec![RowSpec::new("fonts", "Font Previews", RowKind::Bool(true))]),
        ]
    }
    fn set(&mut self, _: &str, value: PrefValue) {
        if let PrefValue::Bool(value) = value {
            self.draft = value;
        }
    }
    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn reset_page(&mut self, _: &str) {
        self.draft = false;
    }
}

impl PreferencesDraft for Model {
    fn differs(&self) -> bool {
        self.draft != self.saved
    }
    fn apply(&mut self) -> Result<(), String> {
        self.saved = self.draft;
        Ok(())
    }
    fn cancel(&mut self) {
        self.draft = self.saved;
    }
}

/// A classic window over a fresh model, opened on `page`, with the state it binds.
fn classic(page: &str) -> (Fixture, Rc<RefCell<Model>>, Rc<RefCell<PreferencesState>>) {
    let model = Rc::new(RefCell::new(Model::default()));
    let state = Rc::new(RefCell::new(PreferencesState::default()));
    state.borrow_mut().selected = page.to_string();
    let (shown, bound) = (model.clone(), state.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut state = bound.borrow_mut();
        let _ = PreferencesWindow::new(Bind::bound(&mut *state)).layout(PreferencesLayout::Classic).embedded().show(ui, &mut *shown.borrow_mut());
    });
    fixture.settle();
    (fixture, model, state)
}

fn press(fixture: &mut Fixture, label: &str) {
    let button = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} button"));
    fixture.click(&button).settle();
}

#[trial]
fn next_and_prev_step_through_the_pages_and_wrap() {
    let (mut fixture, _, state) = classic("general");
    press(&mut fixture, "Next");
    assert_eq!(state.borrow().selected, "tools");
    press(&mut fixture, "Next");
    press(&mut fixture, "Next");
    assert_eq!(state.borrow().selected, "general", "Next on the last page shows the first");
    press(&mut fixture, "Prev");
    assert_eq!(state.borrow().selected, "type", "Prev on the first page shows the last");
    assert!(fixture.by_label("Font Previews").is_some(), "the page Prev picked is shown");
}

#[trial]
fn the_buttons_stand_in_a_column_beside_the_page_with_no_apply() {
    let (fixture, _, _) = classic("tools");
    assert!(fixture.by_label("Apply").is_none(), "the classic window steps through pages instead of applying in place");
    let ok = fixture.by_label("OK").expect("OK").rect;
    let cancel = fixture.by_label("Cancel").expect("Cancel").rect;
    let prev = fixture.by_label("Prev").expect("Prev").rect;
    let next = fixture.by_label("Next").expect("Next").rect;
    let row = fixture.by_label("Show Tips").expect("the row").rect;
    for (name, button) in [("Cancel", cancel), ("Prev", prev), ("Next", next)] {
        assert!((button.left() - ok.left()).abs() < 1.0, "{name} stands under OK: {button:?} vs {ok:?}");
    }
    assert!(ok.top() < cancel.top() && cancel.top() < prev.top() && prev.top() < next.top(), "OK, Cancel, Prev, Next top to bottom");
    assert!(ok.left() > row.right(), "the buttons stand beside the page, not under it");
}

#[trial]
fn ok_applies_and_cancel_drops_the_draft() {
    let (mut fixture, model, _) = classic("general");
    press(&mut fixture, "Snap");
    press(&mut fixture, "Cancel");
    assert!(!model.borrow().saved && !model.borrow().draft, "Cancel drops the edit");

    let (mut fixture, model, _) = classic("general");
    press(&mut fixture, "Snap");
    press(&mut fixture, "OK");
    assert!(model.borrow().saved, "OK applies the edit");
}
