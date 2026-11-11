//! A content-sized layer reserves its footer under a body that fills.

use craft_behavior::{DisplayOrder, ItemSource};
use craft_ui::contract::{Bind, ShowCollection as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::layout_components::Row;
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
