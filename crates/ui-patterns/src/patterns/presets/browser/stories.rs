//! Stories of `PresetBrowser`.

use super::{PresetBrowser, PresetReorder};
use crate::choice::SwatchFace;
use crate::contract::Bind;
use crate::list::ThumbAspect;
use crate::patterns::presets::{PresetEvent, PresetFooterOptions, PresetRow, PresetState};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::Srgba;
use egui::Ui;
use egui::ecolor::{Color32, Hsva};

/// The deepest level a group sits at: the Standard folders.
const NESTING: usize = 2;

/// The swatch colour of preset `n`: hues spread round the wheel, three saturations.
fn swatch(n: u32) -> Srgba {
    let saturation = 0.55 + 0.3 * ((n % 3) as f32 / 2.0);
    Srgba::from(Color32::from(Hsva::new((n as f32 * 0.137).fract(), saturation, 0.85, 1.0)))
}

/// A library of 28 swatches: two loose ones, then Warm and Cool, each with three swatches and the folders Pastel and
/// Deep, which hold three swatches and a Standard folder of two. Presets are keyed 1 to 28, groups from 100 on. No
/// preset has a caption, so the grid shows bare tiles.
fn library_rows() -> Vec<PresetRow<u32>> {
    let mut rows = Vec::new();
    let mut n = 0;
    let mut group = 100;
    let mut fill = |rows: &mut Vec<PresetRow<u32>>, depth: usize, count: u32| {
        for _ in 0..count {
            n += 1;
            let color = swatch(n);
            rows.push(
                PresetRow::item(n, format!("Preset {n}"), depth).preview(ThumbAspect::SQUARE, move |cx, rect| SwatchFace::new(color).paint(cx, rect)),
            );
        }
    };
    let mut open = |rows: &mut Vec<PresetRow<u32>>, name: &str, depth: usize, count: usize| {
        group += 1;
        rows.push(PresetRow::group(group, name, depth).count(count));
    };
    fill(&mut rows, 0, 2);
    for top in ["Warm", "Cool"] {
        open(&mut rows, top, 0, 13);
        fill(&mut rows, 1, 3);
        for inner in ["Pastel", "Deep"] {
            open(&mut rows, inner, 1, 5);
            fill(&mut rows, 2, 3);
            open(&mut rows, "Standard", 2, 2);
            fill(&mut rows, 3, 2);
        }
    }
    rows
}

#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = library_rows();
    let slider = *data.value("slider", true);
    let new_group = *data.value("new_group", true);
    let selected = *data.value("selected", None::<u32>);
    let mut browser = PresetBrowser::new("Swatches", Bind::bound(data.value("state", PresetState::<u32>::new())), &rows)
        .toolbar(true)
        .reorder(PresetReorder::items_and_groups().nest(NESTING))
        .footer(PresetFooterOptions::new().slider(slider).new_group(new_group).can_delete(selected.is_some()));
    if let Some(key) = selected {
        browser = browser.selected(key);
    }
    let out = browser.show(ui);
    let selected = data.value("selected", None::<u32>);
    for event in &out.events {
        if let PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } = event {
            *selected = Some(*key);
        }
    }
    let readout = match *selected {
        Some(n) => format!("Selected: Preset {n}"),
        None => String::from("Nothing selected."),
    };
    StoryOutput::from(out).readout(readout)
}
