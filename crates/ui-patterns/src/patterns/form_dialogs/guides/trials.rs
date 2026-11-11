//! Trials of the guide dialogs.

use super::{GuideLayoutDialog, GuideLayoutValues, GuideRect, GuideUnits};
use crate::contract::Bind;
use crate::runtime::env::UnitKind;
use crate::testing::{Fixture, trial};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[trial]
fn the_preview_check_box_under_the_side_buttons_edits_the_bound_values() {
    let values = Rc::new(RefCell::new(GuideLayoutValues::default()));
    let previewing = Rc::new(Cell::new(false));
    let (held, seen) = (Rc::clone(&values), Rc::clone(&previewing));
    let mut fixture = Fixture::new(move |ui| {
        let units = GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0);
        let document = GuideRect::new(0.0, 0.0, 3000.0, 2000.0);
        let mut values = held.borrow_mut();
        let shown = GuideLayoutDialog::new(units, document, Bind::bound(&mut *values)).opened(Bind::given(true)).show(ui);
        seen.set(shown.preview.is_some());
    });
    fixture.settle();
    assert!(values.borrow().preview && previewing.get(), "Preview starts on and the layout previews");
    let (Some(preview), Some(ok)) = (fixture.by_label("Preview"), fixture.by_label("OK")) else {
        panic!("the Preview check box and OK are shown");
    };
    assert!(preview.rect.min.y >= ok.rect.max.y && (preview.rect.min.x - ok.rect.min.x).abs() < 0.5, "Preview sits under the buttons");
    fixture.click(&preview);
    fixture.settle();
    assert!(!values.borrow().preview, "the click reached the bound values");
    assert!(!previewing.get(), "with Preview off the dialog previews nothing");
}
