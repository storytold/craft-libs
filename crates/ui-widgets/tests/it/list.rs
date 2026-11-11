//! List rows: the rename draft, the row menu and the trailing parts a row reveals.

use craft_behavior::{EditEvent, Mixed, keyed};
use craft_ui::contract::{CollectionEvent, Props as _, ShowCollection as _, request_rename};
use craft_ui::list::{List, ListRow, RowEvent};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const NAMES: [&str; 3] = ["Artboard 1", "Artboard 2", "Artboard 3"];

/// The list's id, which a rename request names.
fn list_id() -> Id {
    Id::NULL.with("rows")
}

/// A list of the three rows whose rename draft is `draft`, and what it reported.
fn rig(draft: fn(&usize) -> Option<String>) -> (Fixture, Rc<RefCell<Vec<CollectionEvent<usize, RowEvent>>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let list = List::new("Artboards").id(list_id()).rename(true).rename_draft(draft);
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    (fixture, seen)
}

/// The text the rename field of `row` opens with.
fn opened_with(draft: fn(&usize) -> Option<String>, row: usize) -> Option<String> {
    let (mut fixture, seen) = rig(draft);
    request_rename(fixture.ctx(), list_id(), &row);
    fixture.settle();
    seen.borrow().iter().find_map(|event| match event {
        CollectionEvent::Rename(EditEvent::Begin { before: Mixed::Same(rename), .. }) => Some(rename.name.clone()),
        _ => None,
    })
}

/// A rename draft replaces the row's text in the field it opens; without one the field holds the row's name.
#[trial]
fn a_rename_draft_opens_the_field_with_it_and_without_one_the_field_holds_the_name() {
    let empty_for_first: fn(&usize) -> Option<String> = |row| (*row == 0).then(String::new);
    assert_eq!(opened_with(empty_for_first, 0).as_deref(), Some(""), "the draft opens the field empty");
    assert_eq!(opened_with(empty_for_first, 1).as_deref(), Some(NAMES[1]), "a row without a draft opens with its name");
}

/// The shapes drawn wholly inside a small square around `at`: an icon's, not a row fill's.
fn drawn_at(shapes: &[egui::Shape], at: egui::Pos2) -> usize {
    let square = egui::Rect::from_center_size(at, Vec2::splat(24.0));
    shapes.iter().filter(|shape| square.contains_rect(shape.visual_bounding_rect())).count()
}

/// A list's row menu button shows on the selected row and on the hovered one, not on another; a click on it asks for
/// that row's menu.
#[trial]
fn the_row_menu_button_shows_on_the_hovered_and_selected_rows_and_asks_for_the_row_menu() {
    use craft_behavior::Selection;
    use craft_ui::contract::Bind;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let list = List::new("Artboards").selection(Bind::given(Selection::single(1))).row_menu();
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let button = |fixture: &Fixture, row: usize| fixture.by_label(&format!("More ({})", NAMES[row])).expect("each row has its menu button").center();
    let (first, second, third) = (button(&fixture, 0), button(&fixture, 1), button(&fixture, 2));
    let rest = fixture.frame_shapes();
    assert!(drawn_at(&rest, second) > 0, "the selected row shows its button");
    assert_eq!(drawn_at(&rest, first), 0, "a row neither hovered nor selected shows none");
    fixture.frame(vec![craft_ui::testing::pointer::moved(third)]);
    let hovered = fixture.frame_shapes();
    assert!(drawn_at(&hovered, third) > 0, "the hovered row shows its button");
    assert_eq!(drawn_at(&hovered, first), 0, "another row still shows none");
    seen.borrow_mut().clear();
    fixture.click_at(first).settle();
    assert!(
        seen.borrow().iter().any(|event| matches!(event, CollectionEvent::ContextMenu(request) if request.key == 0)),
        "a click on the button asks for its row's menu"
    );
}
