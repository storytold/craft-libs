//! A read-only row whose value has no break opportunity wraps inside the dialog: the dialog keeps to its share of the
//! window rather than growing to the value's one-line width.

use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog};
use craft_ui::testing::snapshot::layer_rects;
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

#[trial]
fn a_long_unbroken_read_only_value_wraps_inside_the_dialog() {
    let room = vec2(900.0, 700.0);
    let mut fixture = Fixture::new(|ui| {
        let mut fields = DialogFields::new();
        ParamDialog::new("long", "Document Properties", vec![Param::read_only_text("file", "File", "a".repeat(240))])
            .values(Bind::bound(&mut fields))
            .show(ui);
    });
    fixture.room(room);
    fixture.settle();
    let layers = layer_rects(fixture.ctx());
    let dialog = layers.first().expect("the dialog layer");
    assert!(dialog.left() >= 0.0 && dialog.right() <= room.x, "the dialog stays inside the window: {dialog:?}");
}

/// A number row with a unit shows the unit's symbol after the number, as the parameter says.
#[trial]
fn a_number_row_shows_its_unit_after_the_number() {
    use craft_ui::runtime::env::UnitKind;
    let mut fixture = Fixture::new(|ui| {
        let mut fields = DialogFields::new();
        let rows = vec![Param::number("width", "Width", 0.01, 0.001, 3).unit(UnitKind::Inches)];
        ParamDialog::new("units", "Hairlines", rows).values(Bind::bound(&mut fields)).show(ui);
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    assert!(
        fixture.texts().iter().any(|run| run.text == "in"),
        "the unit symbol is painted: {:?}",
        fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()
    );
}
