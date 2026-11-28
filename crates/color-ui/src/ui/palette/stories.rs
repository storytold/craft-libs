//! Stories of `SwatchPalette`.

use super::SwatchPalette;
use craft_behavior::Mixed;
use craft_theme::Srgba;
use craft_ui::choice::SwatchSpacing;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The twelve colours of the palette story.
const PALETTE: [(&str, Srgba); 12] = [
    ("Red", Srgba::rgb(0.90, 0.28, 0.30)),
    ("Orange", Srgba::rgb(0.97, 0.42, 0.08)),
    ("Amber", Srgba::rgb(0.96, 0.65, 0.14)),
    ("Yellow", Srgba::rgb(0.96, 0.85, 0.04)),
    ("Lime", Srgba::rgb(0.48, 0.77, 0.36)),
    ("Green", Srgba::rgb(0.19, 0.64, 0.42)),
    ("Teal", Srgba::rgb(0.07, 0.65, 0.58)),
    ("Cyan", Srgba::rgb(0.00, 0.62, 0.88)),
    ("Blue", Srgba::rgb(0.24, 0.39, 0.87)),
    ("Violet", Srgba::rgb(0.56, 0.31, 0.78)),
    ("Magenta", Srgba::rgb(0.84, 0.25, 0.62)),
    ("Grey", Srgba::rgb(0.53, 0.55, 0.57)),
];

/// The recent colours of the Other… story.
const RECENT: [Srgba; 3] = [Srgba::rgb(0.10, 0.20, 0.30), Srgba::rgb(0.98, 0.80, 0.70), Srgba::rgb(0.40, 0.10, 0.50)];

fn named(entries: impl IntoIterator<Item = (String, Srgba)>) -> Vec<(Label, Srgba)> {
    entries.into_iter().map(|(name, color)| (Label::from(name), color)).collect()
}

/// The twelve colours, the Other… cell after them and a row of recent colours; the readout shows the last request
/// for the host's picker.
#[story(SwatchPalette<'static>, covers = [], drive = Drive::Never)]
fn swatch_palette_other_and_recent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let swatches = named(PALETTE.iter().map(|(name, color)| ((*name).to_owned(), *color)));
    let shown = SwatchPalette::new("Swatches", data.bind("color", PALETTE[8].1), &swatches)
        .other("custom", "Other colour…")
        .recent("Recent colours", &RECENT)
        .show(ui);
    let last = data.note("request", String::new());
    if let Some(request) = &shown.pick_color {
        *last = format!("{} at {}", request.key, request.color.map_or_else(|| String::from("none"), Srgba::to_hex));
    }
    let readout = format!("Last request: {last}");
    StoryOutput::from(shown).readout(readout)
}

/// A palette over no colour: no swatch is selected until a pick sets one; the readout shows the bound value.
#[story(SwatchPalette<'static>, covers = [], drive = Drive::Never)]
fn no_colour(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let swatches = named(PALETTE.iter().map(|(name, color)| ((*name).to_owned(), *color)));
    let shown = SwatchPalette::optional("Swatches", data.bind_mixed("color", Mixed::<Option<Srgba>>::Same(None)), &swatches).show(ui);
    let bound = *data.value("color", Mixed::<Option<Srgba>>::Same(None));
    let readout = match bound {
        Mixed::Same(Some(color)) => format!("Colour: {}", color.to_hex()),
        Mixed::Same(None) => String::from("Colour: none"),
        Mixed::Mixed => String::from("Colour: mixed"),
    };
    StoryOutput::from(shown).readout(readout)
}

/// The palette of the twelve colours, set up by `options`, with the readout of its latest event.
fn shown_palette(ui: &mut Ui, data: &mut StoryData, options: impl for<'a> FnOnce(SwatchPalette<'a>) -> SwatchPalette<'a>) -> StoryOutput {
    let swatches = named(PALETTE.iter().map(|(name, color)| ((*name).to_owned(), *color)));
    let shown = options(SwatchPalette::new("Swatches", data.bind("color", PALETTE[8].1), &swatches)).show(ui);
    let last = data.note("last", String::new());
    if let Some(event) = &shown.edit {
        *last = format!("{event:?}");
    }
    let readout = format!("Last event: {last}");
    StoryOutput::from(shown).readout(readout)
}

#[story(SwatchPalette<'static>, covers = [], drive = Drive::Never)]
fn swatch_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_palette(ui, data, |palette| palette)
}

#[story(SwatchPalette<'static>, covers = [], drive = Drive::Never)]
fn swatch_palette_spaced(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_palette(ui, data, |palette| palette.spacing(SwatchSpacing::Spaced))
}

#[story(SwatchPalette<'static>, covers = [], drive = Drive::Never)]
fn swatch_palette_spaced_draggable(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_palette(ui, data, |palette| palette.spacing(SwatchSpacing::Spaced).drag_payload(|_| Some(String::from("swatch"))))
}
