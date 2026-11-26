//! `OpeningCard`: the layout of its rows and its Cancel.

use craft_ui::shell::OpeningCard;
use craft_ui::testing::{Fixture, Target, trial};
use std::cell::Cell;
use std::rc::Rc;

const LONG: &str = "a7rv-test_2024_02_17_shot_30_3705_with_an_even_longer_suffix_that_never_ends.psb";

/// A card for `name` in a narrow room, and whether its Cancel was picked.
fn card(name: &'static str) -> (Fixture, Rc<Cell<bool>>) {
    let cancelled = Rc::new(Cell::new(false));
    let seen = Rc::clone(&cancelled);
    let mut fixture = Fixture::new(move |ui| {
        let out = OpeningCard::new(name).message("Reading layers…").fraction(Some(0.4)).cancel("Cancel").show(ui);
        if out.cancelled {
            seen.set(true);
        }
    });
    fixture.room(egui::Vec2::new(280.0, 400.0));
    fixture.settle();
    (fixture, cancelled)
}

fn title(fixture: &Fixture, name: &str) -> Target {
    fixture.by_label(&format!("Opening {name}…")).expect("the title is in the tree under its whole text")
}

/// A long file name is elided on one line, so the percentage row and Cancel stay inside the card, and a click on
/// Cancel reports it.
#[trial]
fn the_opening_card_keeps_a_long_name_on_one_line_and_cancel_inside() {
    let (short, _) = card("short.psd");
    let one_line = title(&short, "short.psd").rect.height();

    let (mut fixture, cancelled) = card(LONG);
    let title = title(&fixture, LONG);
    assert!(title.rect.height() <= one_line + 1.0, "the long title is one line: {:?} against {one_line}", title.rect);
    let cancel = fixture.by_label("Cancel").expect("Cancel shows");
    assert!(cancel.rect.top() >= title.rect.bottom(), "Cancel is under the title: {:?} / {:?}", cancel.rect, title.rect);
    assert!(
        cancel.rect.left() >= title.rect.left() && cancel.rect.right() <= title.rect.right() + 0.5,
        "Cancel is inside the card's width, which the elided title fills: {:?} / {:?}",
        cancel.rect,
        title.rect
    );
    assert!(fixture.by_label("40%").is_some(), "the percentage shows at the start of the last row");
    assert!(!cancelled.get(), "nothing was cancelled yet");
    fixture.click(&cancel);
    assert!(cancelled.get(), "a click on Cancel reports it");
}
