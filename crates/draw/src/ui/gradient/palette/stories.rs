//! Stories of the gradient palette.

use craft_theme::Srgba;
use craft_ui::patterns::presets::{PresetState, PresetView};
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::Ui;

use super::{GradientGroup, GradientItem, GradientPalette, GradientRef};
use crate::ui::gradient::{FocusOutline, GradientStyle, GradientTile};
use craft_color_ui::ui::gradient::{ColorStop, GradientStops};

fn pair(a: [f32; 3], b: [f32; 3]) -> GradientStops {
    let color = |c: [f32; 3]| Srgba { r: c[0], g: c[1], b: c[2], a: 1.0 };
    GradientStops::new(vec![ColorStop::new(0.0, color(a)), ColorStop::new(1.0, color(b))])
}

fn groups() -> Vec<GradientGroup> {
    vec![GradientGroup::new(
        "Basics",
        vec![
            GradientItem::new("Black to white", pair([0.0; 3], [1.0; 3])),
            GradientItem::new("Red to blue", pair([0.9, 0.1, 0.1], [0.1, 0.1, 0.9])),
            GradientItem::new("Sunset", pair([0.95, 0.6, 0.1], [0.5, 0.1, 0.5])),
        ],
    )]
}

fn palette(ui: &mut Ui, data: &mut StoryData, view: PresetView) -> StoryOutput {
    let groups = groups();
    let recent = [pair([0.2, 0.8, 0.4], [0.1, 0.2, 0.6])];
    let mut initial = PresetState::new();
    initial.view = view;
    let (state, state_at) = data.value_at("state", initial);
    let shown = GradientPalette::new("Gradients", &groups, state_at.bind(&mut *state))
        .recent(&recent)
        .selected(GradientRef::Item { group: 0, item: 1 })
        .show(ui);
    let last = data.note("last", String::new());
    if let Some(picked) = shown.picked {
        *last = format!("{picked:?}");
    }
    let readout = format!("Last event: {last}");
    StoryOutput::from(shown).readout(readout)
}

#[story(GradientPalette<'static>, covers = [Default])]
fn gradient_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    palette(ui, data, PresetView::Grid)
}

/// Presets drawn as square tiles: a style at an angle, a focus tile, and a plain ramp among them.
#[story(GradientPalette<'static>, covers = [], title = "Tiles")]
fn gradient_palette_tiles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let stops = pair([0.95, 0.6, 0.1], [0.5, 0.1, 0.5]);
    let focus = egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.75, 0.75));
    let mut items: Vec<GradientItem> = GradientStyle::ALL
        .into_iter()
        .map(|style| GradientItem::new(format!("{style:?}"), stops.clone()).tile(GradientTile::Style { style, angle: -45.0 }))
        .collect();
    items.push(GradientItem::new("Focus, circle", stops.clone()).tile(GradientTile::Focus { outline: FocusOutline::Circle, focus }));
    items.push(GradientItem::new("Focus, rectangle", stops.clone()).tile(GradientTile::Focus { outline: FocusOutline::Rect, focus }));
    items.push(GradientItem::new("Ramp", stops));
    let groups = [GradientGroup::new("Tiles", items)];
    let (state, state_at) = data.value_at("state", PresetState::<GradientRef>::new());
    let shown = GradientPalette::new("Gradient tiles", &groups, state_at.bind(&mut *state)).show(ui);
    StoryOutput::from(shown)
}

#[story(GradientPalette<'static>, covers = [Default])]
fn gradient_palette_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    palette(ui, data, PresetView::List)
}
