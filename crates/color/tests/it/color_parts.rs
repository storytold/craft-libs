//! The colour area, strip, ramp, disc and wheel: the colour maths and the edits they report.

use craft_color::AreaMode;
use craft_color::ui::{ColorArea, WheelGeometry, hue_at, triangle_corners, triangle_point, triangle_sv};
use craft_theme::{ColorModel, HsbModel, ModelValues, Srgba};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui, pos2, vec2};

fn within(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// The wheel's pure geometry: a triangle point reads back as its saturation and brightness, and the ring hue is whole
/// degrees counter-clockwise from the right.
#[trial]
fn the_wheel_geometry_round_trips() {
    let center = pos2(100.0, 100.0);
    let corners = triangle_corners(center, 60.0, 210.0);
    for (s, b) in [(0.0, 1.0), (1.0, 1.0), (0.4, 0.7), (0.9, 0.2)] {
        let (read_s, read_b) = triangle_sv(corners, triangle_point(corners, s, b)).expect("a triangle point reads back");
        assert!(within(read_s, s) && within(read_b, b), "({s}, {b}) read as ({read_s}, {read_b})");
    }
    assert_eq!(hue_at(center, center + vec2(10.0, 0.0)), Some(0.0));
    assert_eq!(hue_at(center, center + vec2(0.0, -10.0)), Some(90.0), "up is 90 degrees");
    assert_eq!(hue_at(center, Pos2::new(f32::NAN, 0.0)), None);
    let geometry = WheelGeometry::new(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), 4.0, (8.0, 24.0), 0.09, 1.0);
    assert!(geometry.inner < geometry.outer && geometry.triangle < geometry.inner);
}

fn web_safe_area(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.4, 0.6, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, Bind::bound(values)).quantize(6).show(ui).into()
}

/// A key on a quantising area steps by the channel's step and goes on until the colour is a different one, so each
/// press reports a new web-safe colour.
#[trial]
fn each_key_on_a_web_safe_area_reaches_another_colour() {
    let mut fixture = Fixture::story(&Story::new("web safe", web_safe_area));
    fixture.settle();
    let color = |fixture: &Fixture| HsbModel.to_srgb(&fixture.data().value("values", ModelValues::new("hsb", &[])).values);
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(2);
    for key in [Key::ArrowRight, Key::ArrowRight, Key::ArrowDown, Key::ArrowDown] {
        let before = color(&fixture);
        fixture.key(key, Modifiers::NONE);
        fixture.idle(2);
        assert_ne!(color(&fixture), before, "{key:?} reaches another colour");
    }
}
