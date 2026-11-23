//! Live preferences apply an edit without waiting for the window to close.

use craft_ui::contract::Bind;
use craft_ui::patterns::preferences::{
    PageSpec, PrefValue, PreferencesDraft, PreferencesMode, PreferencesModel, PreferencesState, PreferencesWindow, RowKind, RowSpec,
};
use craft_ui::testing::{Fixture, trial};
use egui::Ui;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Model {
    draft: bool,
    saved: bool,
    applies: usize,
}

impl PreferencesModel for Model {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        vec![PageSpec::new("general", "General", vec![RowSpec::new("snap", "Snap", RowKind::Bool(self.draft))])]
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
        self.applies += 1;
        Ok(())
    }
    fn cancel(&mut self) {
        self.draft = self.saved;
    }
}

#[trial]
fn an_edit_applies_before_ok() {
    let model = Rc::new(RefCell::new(Model::default()));
    let shown = model.clone();
    let mut state = PreferencesState::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = PreferencesWindow::new(Bind::bound(&mut state)).mode(PreferencesMode::Live).embedded().show(ui, &mut *shown.borrow_mut());
    });
    fixture.settle();
    let snap = fixture.by_label("Snap").expect("the setting");
    fixture.click(&snap).settle();
    assert!(model.borrow().saved, "the change applies while the window remains open");
    assert_eq!(model.borrow().applies, 1);
    let ok = fixture.by_label("OK").expect("the close button");
    fixture.click(&ok).settle();
    assert_eq!(model.borrow().applies, 1, "OK only closes");
}

#[derive(Default)]
struct CustomModel {
    draft: u8,
    saved: u8,
}

impl PreferencesModel for CustomModel {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        vec![PageSpec::new("general", "General", vec![RowSpec::new("custom", "", RowKind::Custom)])]
    }
    fn set(&mut self, _: &str, _: PrefValue) {}
    fn custom_row(&mut self, ui: &mut Ui, _: &str) -> bool {
        let before = self.draft;
        if ui.button("Invalid").clicked() {
            self.draft = 1;
        }
        if ui.button("Correct").clicked() {
            self.draft = 2;
        }
        self.draft != before
    }
    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn reset_page(&mut self, _: &str) {}
}

impl PreferencesDraft for CustomModel {
    fn differs(&self) -> bool {
        self.draft != self.saved
    }
    fn apply(&mut self) -> Result<(), String> {
        if self.draft == 1 {
            return Err("Invalid value".into());
        }
        self.saved = self.draft;
        Ok(())
    }
    fn cancel(&mut self) {}
}

#[trial]
fn corrected_custom_row_applies_after_an_error() {
    let model = Rc::new(RefCell::new(CustomModel::default()));
    let shown = model.clone();
    let mut state = PreferencesState::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = PreferencesWindow::new(Bind::bound(&mut state)).mode(PreferencesMode::Live).embedded().show(ui, &mut *shown.borrow_mut());
    });
    fixture.settle();
    let invalid = fixture.by_label("Invalid").expect("invalid choice");
    fixture.click(&invalid).settle();
    assert_eq!(model.borrow().saved, 0);
    let correct = fixture.by_label("Correct").expect("correct choice");
    fixture.click(&correct).settle();
    assert_eq!(model.borrow().saved, 2, "a correction applies without closing the window");
}
