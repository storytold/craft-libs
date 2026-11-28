//! Stories of `ColorPicker` and `ColorPickerDialog`: Compact, Compact with opacity, Compact with Only Web Colors, Full
//! with Photoshop's models and both warnings, Full with an InDesign-like model set, and the dialog behind its button.

use craft_behavior::Mixed;
use craft_theme::{Channel, ColorModel, DefaultOn, HsbModel, LabModel, MAX_CHANNELS, ModelValues, OklchModel, RgbModel, Srgba};
use craft_ui::contract::{Bind, EditResponse, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::stories::hosted;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::channel::{ChannelRow, ChannelRule, ChannelUnit, TrackRamp};
use super::screen_pick::ScreenPickButton;
use super::{ColorPicker, ColorPickerLayout, ColorPickerOutput, PickedColor};
use crate::ui::{AreaMode, ColorPickerDialog, ColorPickerDialogLayout, ColorPickerTab, ColorWarning, PickerLibrary, PickerSwatches};

type StandardColors = (Vec<(Label, Srgba)>, Vec<(Label, Srgba)>);

/// A colour of full brightness and saturation at `hue` (0 to 1).
fn hue_color(hue: f32) -> Srgba {
    let channel = |n: f32| (((hue * 6.0 + n) % 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
    Srgba::rgb(channel(5.0), channel(3.0), channel(1.0))
}

/// The Standard tab's honeycomb of 19 hues and its row of greys.
fn standard_colors() -> StandardColors {
    let cells = (0..19).map(|i| (Label::from(format!("Color {}", i + 1)), hue_color(i as f32 / 19.0))).collect();
    let greys = (0..8)
        .map(|i| {
            let level = i as f32 / 7.0;
            (Label::from(format!("Gray {}", i + 1)), Srgba::rgb(level, level, level))
        })
        .collect();
    (cells, greys)
}

/// The tabbed dialog on the story's dialog host, opened from a button, on `tab`.
fn tabbed(ui: &mut Ui, data: &mut StoryData, tab: ColorPickerTab) -> StoryOutput {
    hosted(ui, data, (), || {
        let (cells, greys) = standard_colors();
        ColorPickerDialog::new("story", "Colors", Srgba::rgb(0.2, 0.5, 0.8))
            .layout(ColorPickerDialogLayout::Tabbed)
            .standard(cells, greys)
            .tab(tab)
            .web_colors_remembered(false)
    })
}

#[story(ColorPickerDialog, covers = [], drive = Drive::Never)]
fn color_picker_dialog_tabbed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    tabbed(ui, data, ColorPickerTab::Standard)
}

#[story(ColorPickerDialog, covers = [], drive = Drive::Never)]
fn color_picker_dialog_tabbed_custom(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    tabbed(ui, data, ColorPickerTab::Custom)
}

/// A naive CMYK in percent, without a working space: a host's model for the stories. A real host converts through its
/// CMYK profile.
struct NaiveCmyk;

const CMYK: [Channel; 4] = [
    Channel { name: "Cyan", qualified: "CMYK cyan", label: "C", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
    Channel { name: "Magenta", qualified: "CMYK magenta", label: "M", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
    Channel { name: "Yellow", qualified: "CMYK yellow", label: "Y", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
    Channel { name: "Black", qualified: "CMYK black", label: "K", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
];

impl ColorModel for NaiveCmyk {
    fn id(&self) -> &'static str {
        "story-cmyk"
    }

    fn name(&self) -> &'static str {
        "CMYK"
    }

    fn channels(&self) -> &[Channel] {
        &CMYK
    }

    fn from_srgb(&self, c: Srgba) -> ModelValues {
        let k = 1.0 - c.r.max(c.g).max(c.b);
        let ink = |v: f32| if k >= 1.0 { 0.0 } else { 100.0 * (1.0 - v - k) / (1.0 - k) };
        ModelValues::new("story-cmyk", &[ink(c.r), ink(c.g), ink(c.b), 100.0 * k])
    }

    fn to_srgb(&self, v: &[f32; MAX_CHANNELS]) -> Srgba {
        let k = 1.0 - v[3] / 100.0;
        Srgba::rgb((1.0 - v[0] / 100.0) * k, (1.0 - v[1] / 100.0) * k, (1.0 - v[2] / 100.0) * k)
    }

    fn default_on(&self) -> DefaultOn {
        DefaultOn { full: true, compact: false }
    }
}

const CMYK_MODEL: NaiveCmyk = NaiveCmyk;

/// The picker's output with the readout of its latest event, kept in the story's data across passes.
fn with_readout(data: &mut StoryData, shown: ColorPickerOutput) -> StoryOutput {
    let last = data.note("last", String::new());
    if let Some(event) = shown.events.last() {
        *last = format!("{event:?}");
    }
    let readout = format!("Last event: {last}");
    StoryOutput::from(shown).readout(readout)
}

fn start() -> PickedColor {
    PickedColor::rgb(Srgba::rgb(0.2, 0.5, 0.8))
}

/// The compact popup's picker: the default models and the screen-pick button.
#[story(ColorPicker<'static>, covers = [Default], drive = Drive::Never)]
fn color_picker_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (color, color_at) = data.value_at("color", start());
    let shown = ColorPicker::new("Foreground color", color_at.bind(color)).eyedropper(true).show(ui);
    with_readout(data, shown)
}

/// The compact picker with the opacity row.
#[story(ColorPicker<'static>, covers = [], drive = Drive::Never)]
fn color_picker_compact_alpha(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (color, color_at) = data.value_at("color", start());
    let shown = ColorPicker::new("Foreground color", color_at.bind(color)).alpha(true).eyedropper(true).show(ui);
    with_readout(data, shown)
}

/// The compact picker with Only Web Colors on: the field and strip show flat cells of the web colours.
#[story(ColorPicker<'static>, covers = [], drive = Drive::Never)]
fn color_picker_web_colors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let web = *data.value("web", true);
    let mut web = web;
    let (color, color_at) = data.value_at("color", start());
    let shown = ColorPicker::new("Foreground color", color_at.bind(color)).web_colors(Bind::bound(&mut web)).show(ui);
    with_readout(data, shown)
}

/// Photoshop's Color Picker body: HSB, RGB, Lab and the host's CMYK, the compare block with both warnings, Only Web
/// Colors and the screen-pick button; outside a dialog the actions cell is empty.
#[story(ColorPicker<'static>, covers = [], drive = Drive::Never)]
fn color_picker_full(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut mode = *data.value("mode", AreaMode::default());
    let current = Srgba::rgb(0.9, 0.2, 0.1);
    let (color, color_at) = data.value_at("color", PickedColor::rgb(Srgba::rgb(1.0, 0.0, 1.0)));
    let models: [&'static dyn ColorModel; 4] = [&HsbModel, &RgbModel, &LabModel, &CMYK_MODEL];
    let shown = ColorPicker::new("Foreground color", color_at.bind(color))
        .layout(ColorPickerLayout::Full)
        .models(models)
        .mode(Bind::bound(&mut mode))
        .compare(PickedColor::rgb(current))
        .warnings([
            ColorWarning::new("Out of gamut for printing", Srgba::rgb(0.9, 0.1, 0.9)),
            ColorWarning::web_colors("Not a web color", Srgba::rgb(1.0, 0.0, 0.8)),
        ])
        .eyedropper(true)
        .show(ui);
    with_readout(data, shown)
}

/// An InDesign-like model set: RGB in the first group; Lab, OKLCH (with its radios) and the host's CMYK in the second.
#[story(ColorPicker<'static>, covers = [Many], drive = Drive::Never)]
fn color_picker_full_models(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (color, color_at) = data.value_at("color", start());
    let models: [&'static dyn ColorModel; 4] = [&RgbModel, &LabModel, &OklchModel, &CMYK_MODEL];
    let shown = ColorPicker::new("Foreground color", color_at.bind(color)).layout(ColorPickerLayout::Full).models(models).show(ui);
    with_readout(data, shown)
}

/// The Color Picker dialog (`ColorPickerDialog`) behind its button: OK, Cancel and Add to Swatches stand in the
/// picker's actions cell.
#[story(ColorPicker<'static>, covers = [], drive = Drive::Never)]
fn color_picker_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || ColorPickerDialog::foreground(Srgba::rgb(0.2, 0.5, 0.8)))
}

/// The dialog with the Color Swatches button after Cancel: two libraries, the first of eight colours.
#[story(ColorPickerDialog, covers = [], drive = Drive::Never)]
fn color_picker_dialog_with_swatches(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || {
        let eight = [
            ("Red", Srgba::rgb(0.9, 0.1, 0.1)),
            ("Orange", Srgba::rgb(0.95, 0.5, 0.1)),
            ("Yellow", Srgba::rgb(0.95, 0.85, 0.1)),
            ("Green", Srgba::rgb(0.1, 0.65, 0.2)),
            ("Cyan", Srgba::rgb(0.1, 0.75, 0.85)),
            ("Blue", Srgba::rgb(0.1, 0.3, 0.9)),
            ("Violet", Srgba::rgb(0.5, 0.2, 0.8)),
            ("Magenta", Srgba::rgb(0.9, 0.1, 0.6)),
        ];
        let basic = PickerLibrary::new("Basic", eight.into_iter().map(|(name, color)| (Label::from(name), color)).collect());
        let grays = PickerLibrary::new("Grays", vec![(Label::from("Black"), Srgba::BLACK), (Label::from("White"), Srgba::rgb(1.0, 1.0, 1.0))]);
        ColorPickerDialog::foreground(Srgba::rgb(0.1, 0.3, 0.9)).swatch_view(PickerSwatches::new(vec![basic, grays], Label::from("No swatches")))
    })
}

/// The dialog whose only library is empty: the Color Swatches view shows the notice.
#[story(ColorPickerDialog, covers = [], drive = Drive::Never)]
fn color_picker_dialog_with_an_empty_library(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || {
        let libraries = vec![PickerLibrary::new("Empty", Vec::new())];
        ColorPickerDialog::foreground(Srgba::rgb(0.2, 0.5, 0.8)).swatch_view(PickerSwatches::new(libraries, Label::from("No swatches")))
    })
}

#[story(ScreenPickButton, covers = [Default], drive = Drive::Never)]
fn screen_pick_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ScreenPickButton::new().label("Sample color from the screen").show(ui).into()
}

#[story(TrackRamp, covers = [Default], drive = Drive::Never)]
fn track_ramp(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TrackRamp::new(vec![Srgba::rgb(0.0, 0.0, 0.0), Srgba::rgb(1.0, 1.0, 1.0)], false).show(ui).into()
}

/// The saturation row the channel-row stories share: HSB's rule over a white-to-red track.
fn saturation_row(value: Bind<'_, f64>) -> ChannelRow<'_> {
    let channel = HsbModel.channels()[1];
    ChannelRow::new("Saturation", value, ChannelRule::of(&channel)).track(vec![Srgba::rgb(1.0, 1.0, 1.0), Srgba::rgb(1.0, 0.0, 0.0)])
}

fn row_output(edit: Option<EditResponse<f64>>) -> StoryOutput {
    edit.map_or_else(StoryOutput::default, StoryOutput::from)
}

#[story(ChannelRow<'static>, covers = [], drive = Drive::Never)]
fn channel_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("value", 40.0_f64);
    row_output(saturation_row(value_at.bind(value)).show(ui))
}

/// Rows whose number counts percent, degrees and a model's own unit text.
#[story(ChannelRow<'static>, covers = [], drive = Drive::Never)]
fn channel_row_units(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("value", 40.0_f64);
    let mut edit = None;
    for unit in [ChannelUnit::Percent, ChannelUnit::Degrees, ChannelUnit::Text("pt")] {
        edit = edit.or(saturation_row(value_at.bind(&mut *value)).unit(unit).show(ui));
    }
    row_output(edit)
}

/// A row over a mixed value: the field shows the mixed readout until an edit makes it concrete.
#[story(ChannelRow<'static>, covers = [], drive = Drive::Never)]
fn channel_row_mixed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    row_output(saturation_row(Bind::given(Mixed::<f64>::Mixed)).show(ui))
}

/// A row in a disabled container.
#[story(ChannelRow<'static>, covers = [], drive = Drive::Never)]
fn channel_row_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("value", 40.0_f64);
    let shown = Stack::new().enabled(false).show(ui, |ui, _| saturation_row(value_at.bind(value)).show(ui));
    row_output(shown.inner.flatten())
}

/// A row with ticks at 0, 50 and 100 under the track.
#[story(ChannelRow<'static>, covers = [], drive = Drive::Never)]
fn channel_row_marks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let marks = [0.0, 50.0, 100.0];
    row_output(saturation_row(data.bind("value", 40.0_f64)).marks(&marks).show(ui))
}
