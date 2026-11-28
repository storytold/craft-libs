//! Stories of `GradientSwatch`.

use super::GradientSwatch;
use crate::ui::gradient::{FocusOutline, GradientStyle, GradientTile};
use craft_color_ui::ui::gradient::{ColorStop, GradientStops, OpacityStop, StopRows};
use craft_theme::Srgba;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

craft_ui::words! {
    /// The status line of the gradient swatch story.
    #[non_exhaustive]
    pub(crate) struct GradientSwatchStoryWords {
        /// How many times a swatch was clicked.
        clicked: template "gradientSwatchStory.clicked" = "Clicked {count} times";
    }
}

fn stop(at: f32, [r, g, b]: [f32; 3]) -> ColorStop {
    ColorStop::new(at, Srgba::new(r, g, b, 1.0))
}

/// Three gradients: fading in, with a skewed midpoint, and with the opacity in the colour. Each shows as a swatch
/// with its name, then filling the width.
#[story(GradientSwatch, covers = [Default], drive = Drive::Never)]
fn gradient_swatch(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<GradientSwatchStoryWords>();
    let mut fading = GradientStops::new(vec![stop(0.0, [0.96, 0.65, 0.14]), stop(1.0, [0.24, 0.39, 0.87])]);
    let mut skewed = fading.clone();
    skewed.color[0].midpoint = 0.15;
    fading.opacity = vec![OpacityStop::new(0.0, 0.0), OpacityStop::new(1.0, 1.0)];
    let combined = GradientStops::new(vec![
        ColorStop::new(0.0, Srgba::new(0.9, 0.28, 0.3, 1.0)),
        ColorStop::new(0.5, Srgba::new(0.9, 0.28, 0.3, 0.0)),
        ColorStop::new(1.0, Srgba::new(0.19, 0.64, 0.42, 1.0)),
    ]);
    let gradients = [
        ("Fading in", &fading, StopRows::Split),
        ("Midpoint at 15 %", &skewed, StopRows::Split),
        ("Opacity in the colour", &combined, StopRows::Combined),
    ];
    let clicks = data.note("clicks", 0_u32);
    let mut clicked = false;
    Stack::new().show(ui, |ui, _| {
        for (name, stops, rows) in gradients {
            Row::new().show(ui, |ui, _| {
                clicked |= GradientSwatch::new(name, stops).rows(rows).has_popup().show(ui).clicked;
                Text::new(name).shade(Shade::Muted).show(ui);
            });
            clicked |= GradientSwatch::new(name, stops).rows(rows).sizing(Sizing::FillWidth).show(ui).clicked;
        }
    });
    if clicked {
        *clicks += 1;
    }
    StoryOutput { clicked, ..StoryOutput::default() }.readout(craft_i18n::fmt(words.clicked.as_str(), &[("count", clicks.to_string().as_str())]))
}

/// One square tile per gradient style at an angle, and a focus tile in each outline, each with its name.
#[story(GradientSwatch, covers = [], drive = Drive::Never, title = "Tiles")]
fn gradient_swatch_tiles(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let stops = GradientStops::new(vec![stop(0.0, [0.96, 0.65, 0.14]), stop(1.0, [0.24, 0.39, 0.87])]);
    let focus = egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.5, 0.5));
    let mut tiles: Vec<(String, GradientTile)> =
        GradientStyle::ALL.into_iter().map(|style| (format!("{style:?}"), GradientTile::Style { style, angle: -45.0 })).collect();
    tiles.push(("Focus, circle".to_owned(), GradientTile::Focus { outline: FocusOutline::Circle, focus }));
    tiles.push(("Focus, rectangle".to_owned(), GradientTile::Focus { outline: FocusOutline::Rect, focus }));
    Stack::new().show(ui, |ui, _| {
        for (name, tile) in &tiles {
            Row::new().show(ui, |ui, _| {
                GradientSwatch::new(name.clone(), &stops).tile(*tile).show(ui);
                Text::new(name.clone()).shade(Shade::Muted).show(ui);
            });
        }
    });
    StoryOutput::default()
}

/// A gradient fading in, enabled and disabled, each with its name.
#[story(GradientSwatch, covers = [], drive = Drive::Never, title = "Disabled")]
fn gradient_swatch_disabled(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut fading = GradientStops::new(vec![stop(0.0, [0.96, 0.65, 0.14]), stop(1.0, [0.24, 0.39, 0.87])]);
    fading.opacity = vec![OpacityStop::new(0.0, 0.0), OpacityStop::new(1.0, 1.0)];
    Stack::new().show(ui, |ui, _| {
        for (name, enabled) in [("Enabled", true), ("Disabled", false)] {
            Row::new().show(ui, |ui, _| {
                GradientSwatch::new(name, &fading).enabled(enabled).show(ui);
                Text::new(name).shade(Shade::Muted).show(ui);
            });
        }
    });
    StoryOutput::default()
}
