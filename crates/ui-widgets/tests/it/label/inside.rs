//! A glyph label inside its control: every measure of the control sees the label, so the control keeps its
//! measurement from frame to frame.

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, FieldLayout, GlyphLabel, LabelKind, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::layout_components::Stack;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, last_pass_measuring_runs, trial};
use craft_ui::text::{LinkedFields, NumberField};

/// A glyph wider than a number cell's room, so the slot widens the field and a measure without it differs.
const WIDE: &str = "Width scale";

/// Once settled, a frame in which the pointer moves over a field with an inside label runs one pass and measures
/// no body again: through a `Field` (the label step's container path), through `BodyCx::labelled` (its leaf path in
/// a body's flow) and through `LinkedFields` in a row (its leaf path, measured by the composite and shown with
/// `PaintUi::labelled`).
#[trial]
fn a_field_with_an_inside_label_keeps_its_measurement() {
    let mut fixture = Fixture::new(|ui| {
        let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(WIDE))).layout(FieldLayout::Inside).scrub(true);
        Field::new("Angle").side(side).show(ui, |ui, _| {
            NumberField::new("Angle", Bind::given(Some(10.0))).scrub_by_label().show(ui);
        });
        Stack::new().show(ui, |ui, cx| {
            let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(WIDE))).layout(FieldLayout::Inside).scrub(true);
            cx.labelled(ui, "turn", "Turn".into(), side, NumberField::new("Turn", Bind::given(Some(20.0))).scrub_by_label());
        });
        let parts: [Mixed<Option<f64>>; 2] = Mixed::each([Some(500.0), Some(400.0)]);
        LinkedFields::new("Size", Bind::given(parts), Bind::given(true)).captions([WIDE, "Height scale"]).show(ui);
    });
    fixture.settle();
    for name in ["Angle", "Turn", WIDE] {
        let field = fixture.by_label(name).unwrap_or_else(|| panic!("the field {name} is named by its label"));
        fixture.frame(vec![moved(field.center())]);
        assert_eq!(fixture.passes(), 1, "moving over {name} runs one pass");
        let runs = last_pass_measuring_runs(fixture.ctx());
        assert!(runs.is_empty(), "moving over {name} measures no body again: {runs:?}");
    }
}
