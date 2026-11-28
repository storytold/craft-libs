//! The accessibility emitter: the label scope and merge rule, announcements, modal inertness, viewports.

pub(crate) mod scroll_into_view;
pub(crate) mod step_ends;

use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::form::{Field, Help};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;

/// A field's help line describes its control, as the label scope's help line does: a screen reader reads the hint
/// that sighted users see under the control.
#[trial]
fn a_field_help_line_describes_its_control() {
    const HINT: &str = "Up to 256 KiB";
    let mut fixture = Fixture::new(|ui| {
        let mut text = String::new();
        Field::new("Limit").help(Help::Text(HINT.into())).show(ui, |ui, _| TextField::new("Limit", Bind::bound(&mut text)).show(ui));
    });
    fixture.settle();
    let field = fixture.by_label("Limit").expect("the field's control");
    assert_eq!(fixture.tree().description_of(field.node), Some(HINT));
}
