//! The font discard hook.

use craft_ui::contract::Show as _;
use craft_ui::display::Text;
use craft_ui::runtime::env::Scale;
use craft_ui::testing::{Fixture, trial};

/// 6.3: a text's width in points does not depend on the interface scale: glyph advances are measured unscaled, so hinting
/// the outlines does not change them.
#[trial]
fn a_text_measures_the_same_width_at_every_scale() {
    let width_at = |scale: f32| {
        let mut fixture = Fixture::new(|ui| {
            Text::new("Layers").show(ui);
        });
        fixture.scale(Scale::new(scale));
        fixture.settle();
        fixture.shown().width()
    };
    let (one, two) = (width_at(1.0), width_at(2.0));
    const SAME: f32 = 1.0;
    assert!((one - two).abs() < SAME, "\"Layers\" is {one} wide at scale 1 and {two} at scale 2");
}
