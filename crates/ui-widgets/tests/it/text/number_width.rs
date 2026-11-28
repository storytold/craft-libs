//! A number field is sized by the shaped width of the widest value it shows, not by the longest text.

use craft_ui::contract::{Bind, Props as _, Show as _, Sizing};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{NumberField, NumberLook};
use std::cell::Cell;
use std::rc::Rc;

/// The ends of a range whose two texts are the same length and different widths: the inline look sets the number in
/// the proportional face, where "1" is narrower than "0", so "100" is wider than "111".
const LOW: f64 = 100.0;
const HIGH: f64 = 111.0;

/// The widths of three content-sized fields in the inline look: one over the range showing its low end, and one over
/// no range for each end, which is sized by that value alone.
fn widths() -> [f32; 3] {
    let shown = Rc::new(Cell::new([0.0; 3]));
    let seen = shown.clone();
    let mut fixture = Fixture::new(move |ui| {
        let width = |ui: &mut egui::Ui, value: Option<f64>, ranged: bool, salt: &'static str| {
            let mut field = NumberField::new("N", Bind::given(value)).fixed_decimals(0).sizing(Sizing::Content).id_salt(salt);
            if ranged {
                field = field.range(LOW..=HIGH);
            }
            field.show(ui).response.rect.width()
        };
        seen.set([width(ui, Some(LOW), true, "range"), width(ui, Some(LOW), false, "low"), width(ui, Some(HIGH), false, "high")]);
    });
    fixture.root_value(NumberLook::Inline);
    fixture.settle();
    shown.get()
}

/// A field over a range is as wide as the widest value the range can show, measured in the face it paints. Both ends
/// of this range are three characters long, so a field sized by the longest text is sized by only one of them: "100"
/// and "111" set at different widths in the proportional label face, and the field over the range keeps the narrower
/// one's room, so its text is clipped whenever it shows the wider value. The fields over no range are sized by their
/// own value alone, which gives the room each end needs; the field over the range must find at least that much.
///
/// Without the fix `Ruled::sample` keeps whichever candidate `NumberCodec::widest` counts longest, which for two ends
/// of three characters is the last of them, "111": the first assertion passes and the second fails.
#[trial]
fn a_range_is_sized_by_the_shaped_width_of_every_value_it_shows() {
    let [ranged, low, high] = widths();
    assert!(low > high + 1.0, "the label face sets 100 wider than 111: {low} against {high}");
    assert!(ranged + 0.5 >= low, "the range is as wide as its wider end: {ranged} against {low}");
    assert!(ranged + 0.5 >= high, "the range is as wide as its other end: {ranged} against {high}");
}
