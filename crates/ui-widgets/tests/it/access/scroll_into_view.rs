//! A screen reader's Scroll Into View shows its node and leaves keyboard focus where it was; a Focus request moves it.

use craft_behavior::{Selection, keyed};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowCollection as _};
use craft_ui::list::{List, ListRow};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Key, Modifiers, Rect, Vec2};

#[trial]
fn scroll_into_view_moves_no_focus() {
    let mut fixture = Fixture::new(|ui| {
        Button::new("First").show(ui);
        Button::new("Second").show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    let focused = fixture.focused_stop();
    assert!(focused.is_some(), "Tab focuses the first button");
    let second = fixture.by_label("Second").expect("the second button");
    fixture.access(AccessActionKind::ScrollIntoView, &second).settle();
    assert_eq!(fixture.focused_stop(), focused, "Scroll Into View moved no focus");
    fixture.access(AccessActionKind::Focus, &second).settle();
    assert_ne!(fixture.focused_stop(), focused, "a Focus request moves it");
}

/// Whether `view` shows all of `row`: a row out of view may report bounds cut to nothing at the view's edge.
fn shows(view: Rect, row: Rect) -> bool {
    row.height() > 1.0 && view.expand(0.5).contains_rect(row)
}

/// A screen reader's Scroll Into View on a list row scrolls the list's own body until the row shows, as the old egui
/// list's scroll area did for any widget in it; on a row already in view it moves nothing. Row 150 is the selection's
/// anchor, so the list builds it, and reports its node, while it is scrolled out.
#[trial]
fn scroll_into_view_on_a_list_row_scrolls_the_list_to_it() {
    let list_id = Id::new("rows");
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..200).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let list = List::new("Rows").id(list_id).selection(Bind::given(Selection::single(150_usize)));
        list.show_items(ui, &source, &mut |key, _| ListRow::new(format!("Row {key}")));
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let view = fixture.node_of(list_id).expect("the list's node").rect;
    let row = fixture.by_label("Row 150").expect("row 150's node, kept as the anchor");
    assert!(!shows(view, row.rect), "row 150 starts out of view: {:?} in {view:?}", row.rect);
    fixture.access(AccessActionKind::ScrollIntoView, &row).settle();
    let shown = fixture.by_label("Row 150").expect("row 150's node").rect;
    assert!(shows(view, shown), "Scroll Into View brought row 150 into the list's view: {shown:?} in {view:?}");
    let near = fixture.by_label("Row 148").expect("row 148, in view beside row 150");
    assert!(shows(view, near.rect), "row 148 shows: {:?} in {view:?}", near.rect);
    fixture.access(AccessActionKind::ScrollIntoView, &near).settle();
    let after = fixture.by_label("Row 150").expect("row 150's node").rect;
    assert!((after.min - shown.min).length() <= 0.5, "Scroll Into View on a row in view moved nothing: {after:?}, was {shown:?}");
}
