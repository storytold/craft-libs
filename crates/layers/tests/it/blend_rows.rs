//! `BlendRows`: a percent field keeps its side label while the field's tooltip shows.

use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::rows::BlendRows;
use craft_layers::ui::tree::{LayerProperties, LayerTree as _};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use egui::Rect;

/// The rect of the run whose text starts with `start`, if the last frame painted one.
fn run(fixture: &Fixture, start: &str) -> Option<Rect> {
    fixture.texts().iter().find(|run| run.text.starts_with(start)).map(|run| run.rect)
}

#[trial]
fn a_field_keeps_its_side_label_while_its_tooltip_shows() {
    let modes = FakeLayers::sample().blend_modes(false, "normal");
    let mut properties = LayerProperties::new("normal");
    properties.opacity = 80.0;
    properties.editable = true;
    let mut fixture = Fixture::new(move |ui| {
        ui.set_width(320.0);
        let _ = BlendRows::new(Some(&properties), &modes).show(ui);
    });
    fixture.settle();
    let label = run(&fixture, "Opacity").expect("the Opacity label shows");
    let value = run(&fixture, "80").expect("the Opacity value shows");
    fixture.frame(vec![moved(value.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    let texts: Vec<_> = fixture.texts().iter().map(|run| (run.text.clone(), run.rect)).collect();
    let shown: Vec<Rect> = fixture.texts().iter().filter(|run| run.text.starts_with("Opacity")).map(|run| run.rect).collect();
    assert!(shown.len() >= 2, "the tooltip shows beside the label: {texts:?}");
    assert!(shown.contains(&label), "the label stays where it was while the tooltip shows: {label:?} in {texts:?}");
}
