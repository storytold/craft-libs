//! A select's width: the field width by default, its widest entry when it is sized to its content.

use craft_ui::contract::{Bind, Entry, Props as _, Show as _, Sizing};
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

/// A select sized to its content is narrower than the field width a select takes by default, and still shows its
/// widest entry whole.
#[trial]
fn a_content_sized_select_fits_its_widest_entry() {
    let rects = Rc::new(Cell::new([egui::Rect::NOTHING; 2]));
    let seen = rects.clone();
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "One"), Entry::new(1u8, "Twelve")];
        let plain = Select::new("Plain", Bind::given(1u8), &entries).show(ui).response.rect;
        let fitted = Select::new("Fitted", Bind::given(1u8), &entries).sizing(Sizing::Content).show(ui).response.rect;
        seen.set([plain, fitted]);
    });
    fixture.settle();
    let [plain, fitted] = rects.get();
    assert!(fitted.width() < plain.width(), "the content-sized select is narrower than the default one: {fitted:?} {plain:?}");
    let shown = fixture.texts().iter().find(|run| run.text == "Twelve" && fitted.contains_rect(run.rect)).map(|run| run.elided);
    assert_eq!(shown, Some(false), "the content-sized select shows its widest entry whole inside its box");
}
