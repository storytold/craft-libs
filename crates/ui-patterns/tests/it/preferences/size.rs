//! The size the preferences window opens at.

use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::preferences::{
    MIN_SIZE, PageSpec, PrefValue, PreferencesDraft, PreferencesModel, PreferencesState, PreferencesWindow, RowKind, RowSpec,
};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Ui, Vec2, vec2};

struct Model;

impl PreferencesModel for Model {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        ["General", "Type"].into_iter().map(|title| PageSpec::new(title, title, vec![RowSpec::new(title, title, RowKind::Bool(false))])).collect()
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

/// The bounds of the window opened with `size`, or at its own size without it, in a room larger than either.
fn opened(size: Option<Vec2>) -> Rect {
    let mut state = PreferencesState::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut window = PreferencesWindow::new(Bind::bound(&mut state)).embedded();
        if let Some(size) = size {
            window = window.size(size);
        }
        let _ = window.show(ui, &mut Model);
    });
    fixture.room(vec2(1600.0, 1200.0)).settle();
    fixture.by_role(Role::DIALOG).first().map(|dialog| dialog.rect).unwrap_or_else(|| panic!("the preferences window"))
}

#[trial]
fn the_window_opens_at_the_size_the_host_gives() {
    let own = opened(None);
    let asked = (own.size() + MIN_SIZE) / 2.0;
    let smaller = opened(Some(asked));
    assert!(smaller.width() < own.width() && smaller.height() < own.height(), "{smaller:?} is not smaller than {own:?}");
    let larger = opened(Some(own.size() + vec2(100.0, 100.0)));
    assert!(larger.width() > own.width() && larger.height() > own.height(), "{larger:?} is not larger than {own:?}");
}

#[trial]
fn a_size_that_is_not_finite_is_ignored() {
    let endless = opened(Some(vec2(f32::INFINITY, f32::NAN)));
    assert_eq!(endless.size(), opened(None).size(), "a size that is not finite opens the window at its own size");
}
