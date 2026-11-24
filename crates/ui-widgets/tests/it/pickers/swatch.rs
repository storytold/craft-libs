//! `Swatch`: the focus ring of a shaped well follows its outline.

use craft_theme::Srgba;
use craft_ui::choice::{RadioGroup, Swatch, SwatchShape};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Shape, vec2};

/// Every shape of the frame, groups opened.
fn flat(shapes: Vec<Shape>) -> Vec<Shape> {
    shapes.into_iter().flat_map(|shape| if let Shape::Vec(inner) = shape { flat(inner) } else { vec![shape] }).collect()
}

/// A focused round member swatch draws its ring as a circle round its well, a square one round its rect: the component's
/// declared focus shape (`Cx::focus_shape`) reaches the ring.
#[trial]
fn a_focused_round_swatch_has_a_round_ring() {
    for (shape, round) in [(SwatchShape::Round, true), (SwatchShape::Square, false)] {
        let mut fixture = Fixture::new(move |ui| {
            let _ = RadioGroup::new("Ranges", Bind::given(0u8)).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    for n in 0..3u8 {
                        Swatch::new(format!("Range {n}"), Srgba::rgb(0.2, 0.4, 0.8)).shape(shape).member(n).show(ui);
                    }
                });
            });
        });
        fixture.room(vec2(300.0, 100.0));
        fixture.settle();
        fixture.key(Key::Tab, Modifiers::NONE).settle();
        let well = fixture.by_label("Range 0").expect("the first swatch").rect;
        let rings = flat(fixture.frame_shapes())
            .into_iter()
            .filter(|shape| {
                matches!(shape, Shape::Circle(circle) if circle.stroke.width > 0.0 && circle.center.distance(well.center()) < 0.5 && circle.radius >= well.width() / 2.0)
            })
            .count();
        assert_eq!(rings > 0, round, "{shape:?}: a circle ring round the focused well");
    }
}
