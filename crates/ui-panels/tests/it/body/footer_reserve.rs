//! A content-sized layer reserves its footer under a body that fills.

use craft_behavior::{DisplayOrder, ItemSource};
use craft_ui::contract::{Bind, ShowCollection as _, ShowContainer as _};
use craft_ui::contract::{Scroll, Show as _};
use craft_ui::dialog::Dialog;
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::testing::{Fixture, trial};
use egui::Rect;
use std::cell::Cell;
use std::rc::Rc;

const ROWS: usize = 80;

struct Rows;

impl DisplayOrder<usize> for Rows {
    fn len(&self) -> usize {
        ROWS
    }

    fn key(&self, at: usize) -> usize {
        at
    }
}

impl ItemSource<usize> for Rows {}

#[trial]
fn a_dialog_with_a_filling_body_keeps_its_footer_below_the_body_and_inside_the_screen() {
    let body = Rc::new(Cell::new(Rect::NOTHING));
    let seen = body.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut open = true;
        Dialog::new("Pick")
            .collection_body()
            .opened(Bind::bound(&mut open))
            .buttons([FooterButton::new("Apply", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
            .show(ui, |ui, _| {
                let row = Row::new().fill().show(ui, |ui, _| {
                    List::new("Rows").show_items(ui, &Rows, &mut |key, _| ListRow::new(format!("Row {key}")));
                });
                seen.set(row.output.rect);
            });
    });
    fixture.settle();
    let screen = fixture.ctx().content_rect();
    let body = body.get();
    for label in ["Apply", "Cancel"] {
        let button = fixture.by_label(label).expect("the footer button is shown");
        assert!(screen.contains_rect(button.rect), "{label} is inside the screen: {:?} in {screen:?}", button.rect);
        assert!(craft_ui::testing::relations::above(body, button.rect), "{label} is below the body: {:?} under {body:?}", button.rect);
    }
}

/// photocraft#16326: a dialog whose body holds a scrolling stack that fills, under a line of other content, keeps to
/// its share of the screen with the footer as its last row: the body leaves the footer its height instead of taking
/// the whole room and pushing the footer past it.
#[trial]
fn a_dialog_with_a_filling_scroll_body_keeps_to_its_room_with_the_footer_last() {
    let mut fixture = Fixture::new(move |ui| {
        let mut open = true;
        Dialog::new("Pick").opened(Bind::bound(&mut open)).buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |ui, _| {
            Text::new("Pick a row").show(ui);
            Stack::new().scroll(Scroll::vertical()).fill().show(ui, |ui, _| {
                for at in 0..ROWS {
                    Text::new(format!("Row {at}")).show(ui);
                }
            });
        });
    });
    fixture.room(egui::Vec2::new(1200.0, 700.0)).settle();
    let screen = fixture.ctx().content_rect();
    let dialog = fixture.by_role(craft_ui::contract::Role::DIALOG).first().expect("the dialog shows").rect;
    let ok = fixture.by_label("OK").expect("the footer's OK").rect;
    assert!(screen.contains_rect(dialog), "the dialog is inside the screen: {dialog:?} in {screen:?}");
    assert!(dialog.height() <= 0.9 * screen.height() + 1.0, "the dialog keeps to its share of the screen: {dialog:?} in {screen:?}");
    assert!(dialog.bottom() - ok.bottom() < 20.0, "OK is the dialog's last row: {ok:?} in {dialog:?}");
}
