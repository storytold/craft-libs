//! The Tabs dialog's focus: it opens on the default interval, and Set moves into the new stop.

use craft_text::ui::tabs_dialog::TabStopsDialog;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

/// Which of `labels` holds the focus.
fn focused<'a>(fixture: &Fixture, labels: &[&'a str]) -> Option<&'a str> {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    labels.iter().copied().find(|label| fixture.by_label(label).is_some_and(|target| target.node == focused))
}

#[trial]
fn the_dialog_opens_on_the_default_interval_and_set_moves_into_the_new_stop() {
    let dialog = Rc::new(RefCell::new(TabStopsDialog::new(Vec::new(), 36.0)));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.idle(30);
    let labels = ["Default tab stops:", "Tab stop position:", "Set"];
    assert_eq!(focused(&fixture, &labels), Some("Default tab stops:"), "the dialog opens on its first field");
    let set = fixture.by_label("Set").expect("Set is shown");
    fixture.click(&set).idle(4);
    assert_eq!(focused(&fixture, &labels), Some("Tab stop position:"), "Set moves the focus into the new stop's position");
}
