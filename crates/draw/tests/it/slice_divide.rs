//! The Divide Slices dialog's value field: a size shows its pixel unit, a count none.

use craft_draw::ui::slice::{DivideBy, DivideSlices, DivideSlicesDialog};
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Fixture, trial};

#[trial]
fn a_count_shows_no_unit_and_a_size_shows_pixels() {
    let mut dialog = DivideSlicesDialog::new(DivideSlices::new(Some(DivideBy::Count(4)), Some(DivideBy::Size(64))));
    let mut fixture = Fixture::new(move |ui| {
        let _ = dialog.show(ui, &DialogStack::default());
    });
    fixture.idle(4);
    let units = fixture.texts().iter().filter(|run| run.text == "px").count();
    assert_eq!(units, 1, "only the size field shows a unit");
}
