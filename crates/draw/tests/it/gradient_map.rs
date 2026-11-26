//! Gradient Map readout and stop editing.

use std::cell::RefCell;
use std::rc::Rc;

use craft_color_ui::ui::gradient::{ColorStop, GradientStops};
use craft_draw::ui::gradient::{GradientGroup, GradientItem, GradientMap, GradientMapEditor};
use craft_theme::Srgba;
use craft_ui::testing::{Fixture, trial};

#[trial]
fn gradient_map_names_the_current_preset_and_shows_how_to_add_a_stop() {
    let black = Srgba::rgb(0.0, 0.0, 0.0);
    let white = Srgba::rgb(1.0, 1.0, 1.0);
    let blue = Srgba::rgb(0.0, 0.0, 1.0);
    let stops = |first| GradientStops::new(vec![ColorStop::new(0.0, first), ColorStop::new(1.0, white)]);
    let presets =
        [GradientGroup::new("Basics", vec![GradientItem::new("Blue to White Alias", stops(blue)), GradientItem::new("Blue to White", stops(blue))])];
    let value = Rc::new(RefCell::new(GradientMap::new(stops(black), false, true)));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let out = GradientMapEditor::new("Gradient Map", &held.borrow()).presets(&presets).show(ui);
        if let Some(event) = out.event {
            event.apply(&mut held.borrow_mut());
        }
    });
    fixture.room(egui::vec2(380.0, 330.0)).settle();

    let visible = |fixture: &Fixture, text: &str| {
        assert!(
            fixture.texts().iter().any(|run| run.text == text && !run.elided && run.clip.contains_rect(run.rect)),
            "{text:?} is fully visible; painted {:?}",
            fixture.texts().iter().map(|run| run.text.as_str()).collect::<Vec<_>>()
        );
    };
    visible(&fixture, "Preset");
    visible(&fixture, "Custom");

    let picker = fixture.by_label("Preset").expect("a named preset picker");
    fixture.click(&picker).settle();
    let blue_to_white = fixture.by_label("Blue to White").expect("the library preset");
    fixture.click(&blue_to_white).settle();
    visible(&fixture, "Blue to White");
    assert_eq!(value.borrow().stops, stops(blue));

    visible(&fixture, "Click below the ramp to add a stop.");
    let first = fixture.by_label("Color stop 1 of 2").expect("first stop");
    let last = fixture.by_label("Color stop 2 of 2").expect("last stop");
    fixture.click_at(first.center().lerp(last.center(), 0.5)).settle();
    let active = value.borrow();
    assert_eq!(active.stops.color.len(), 3);
    assert!((active.stops.color[1].location - 0.5).abs() < 0.01);
    assert!(active.dither);
    visible(&fixture, "Custom");
}
