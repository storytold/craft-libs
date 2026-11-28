//! The mock editor's dock: the toolbar is a stepped column and the Navigator a collapsed one.

use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, trial};
use craft_ui_gallery::mock::{EditorMock, MockState};
use egui::Vec2;
use std::cell::RefCell;
use std::rc::Rc;

fn editor() -> Fixture {
    let state = Rc::new(RefCell::new(MockState::default()));
    let mut fixture = Fixture::new(move |ui| {
        let mut state = state.borrow_mut();
        EditorMock::new("Editor", &mut state).show(ui);
    });
    fixture.room(Vec2::new(1000.0, 700.0)).settle();
    fixture
}

/// The toolbar is one tool column of the dock, not a shell rail: its buttons stand in one column that ends above the
/// foot of the window, and the Navigator shows as an icon of a collapsed column.
#[trial]
fn the_mock_toolbar_is_a_stepped_dock_column_and_the_navigator_an_icon() {
    let fixture = editor();
    let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown")).rect;
    let (first, last) = (rect("Move"), rect("Clone Stamp"));
    assert!((first.left() - last.left()).abs() < 1.0, "the tools stand in one column: {first:?} and {last:?}");
    assert!(last.bottom() < fixture.shown().bottom() - 20.0, "the column ends above the foot of the window: {last:?}");
    let tree = fixture.tree();
    let navigator = fixture.by_role(Role::BUTTON).into_iter().any(|button| tree.name_of(button.node) == Some("Navigator"));
    assert!(navigator, "the Navigator shows as an icon of its collapsed column");
}
