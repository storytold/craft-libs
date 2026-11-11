//! Photo Filter: its presets, value and editor.

use craft_theme::Srgba;
use craft_ui::choice::{Checkbox, Radio, RadioGroup, Swatch};
use craft_ui::contract::{
    Bind, EditResponse, Entry, LabelKind, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, SideLabel,
};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
use egui::Ui;

use crate::ui::adjust::sliders::{Run, lifted, read, words};
use crate::ui::adjust::{AdjustContext, AdjustField, AdjustWords};

/// An opaque colour from 8-bit sRGB channels.
fn srgb8(r: u8, g: u8, b: u8) -> Srgba {
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// The colour request's key.
const COLOR_KEY: &str = "photoFilter.color";

/// Density's default.
const DENSITY_DEFAULT: f64 = 25.0;

/// Photo Filter's presets, in Photoshop's menu order, with their colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PhotoFilterPreset {
    /// Warming Filter (85): (236, 138, 0), the default.
    Warming85,
    /// Warming Filter (LBA): (250, 150, 0).
    WarmingLba,
    /// Warming Filter (81): (235, 177, 19).
    Warming81,
    /// Cooling Filter (80): (0, 109, 255).
    Cooling80,
    /// Cooling Filter (LBB): (0, 93, 255).
    CoolingLbb,
    /// Cooling Filter (82): (0, 181, 255).
    Cooling82,
    /// Red: (234, 26, 26).
    Red,
    /// Orange: (243, 132, 23).
    Orange,
    /// Yellow: (249, 227, 28).
    Yellow,
    /// Green: (25, 201, 25).
    Green,
    /// Cyan: (29, 203, 234).
    Cyan,
    /// Blue: (29, 53, 234).
    Blue,
    /// Violet: (155, 29, 234).
    Violet,
    /// Magenta: (227, 24, 227).
    Magenta,
    /// Sepia: (172, 122, 51).
    Sepia,
    /// Deep Red: (255, 0, 0).
    DeepRed,
    /// Deep Blue: (0, 34, 205).
    DeepBlue,
    /// Deep Emerald: (0, 140, 0).
    DeepEmerald,
    /// Deep Yellow: (255, 213, 0).
    DeepYellow,
    /// Underwater: (0, 194, 177).
    Underwater,
}

impl PhotoFilterPreset {
    /// Every preset, in menu order.
    pub const ALL: [PhotoFilterPreset; 20] = [
        PhotoFilterPreset::Warming85,
        PhotoFilterPreset::WarmingLba,
        PhotoFilterPreset::Warming81,
        PhotoFilterPreset::Cooling80,
        PhotoFilterPreset::CoolingLbb,
        PhotoFilterPreset::Cooling82,
        PhotoFilterPreset::Red,
        PhotoFilterPreset::Orange,
        PhotoFilterPreset::Yellow,
        PhotoFilterPreset::Green,
        PhotoFilterPreset::Cyan,
        PhotoFilterPreset::Blue,
        PhotoFilterPreset::Violet,
        PhotoFilterPreset::Magenta,
        PhotoFilterPreset::Sepia,
        PhotoFilterPreset::DeepRed,
        PhotoFilterPreset::DeepBlue,
        PhotoFilterPreset::DeepEmerald,
        PhotoFilterPreset::DeepYellow,
        PhotoFilterPreset::Underwater,
    ];

    /// The preset's colour, as in each variant's doc (8-bit sRGB).
    pub fn color(self) -> Srgba {
        let (r, g, b) = match self {
            PhotoFilterPreset::Warming85 => (236, 138, 0),
            PhotoFilterPreset::WarmingLba => (250, 150, 0),
            PhotoFilterPreset::Warming81 => (235, 177, 19),
            PhotoFilterPreset::Cooling80 => (0, 109, 255),
            PhotoFilterPreset::CoolingLbb => (0, 93, 255),
            PhotoFilterPreset::Cooling82 => (0, 181, 255),
            PhotoFilterPreset::Red => (234, 26, 26),
            PhotoFilterPreset::Orange => (243, 132, 23),
            PhotoFilterPreset::Yellow => (249, 227, 28),
            PhotoFilterPreset::Green => (25, 201, 25),
            PhotoFilterPreset::Cyan => (29, 203, 234),
            PhotoFilterPreset::Blue => (29, 53, 234),
            PhotoFilterPreset::Violet => (155, 29, 234),
            PhotoFilterPreset::Magenta => (227, 24, 227),
            PhotoFilterPreset::Sepia => (172, 122, 51),
            PhotoFilterPreset::DeepRed => (255, 0, 0),
            PhotoFilterPreset::DeepBlue => (0, 34, 205),
            PhotoFilterPreset::DeepEmerald => (0, 140, 0),
            PhotoFilterPreset::DeepYellow => (255, 213, 0),
            PhotoFilterPreset::Underwater => (0, 194, 177),
        };
        srgb8(r, g, b)
    }

    /// The preset's menu name from `words` (the channel words for Red, Yellow, Green, Cyan, Blue and Magenta).
    pub fn label(self, words: &AdjustWords) -> Label {
        match self {
            PhotoFilterPreset::Warming85 => &words.warming_85,
            PhotoFilterPreset::WarmingLba => &words.warming_lba,
            PhotoFilterPreset::Warming81 => &words.warming_81,
            PhotoFilterPreset::Cooling80 => &words.cooling_80,
            PhotoFilterPreset::CoolingLbb => &words.cooling_lbb,
            PhotoFilterPreset::Cooling82 => &words.cooling_82,
            PhotoFilterPreset::Red => &words.red,
            PhotoFilterPreset::Orange => &words.orange,
            PhotoFilterPreset::Yellow => &words.yellow,
            PhotoFilterPreset::Green => &words.green,
            PhotoFilterPreset::Cyan => &words.cyan,
            PhotoFilterPreset::Blue => &words.blue,
            PhotoFilterPreset::Violet => &words.violet,
            PhotoFilterPreset::Magenta => &words.magenta,
            PhotoFilterPreset::Sepia => &words.sepia,
            PhotoFilterPreset::DeepRed => &words.deep_red,
            PhotoFilterPreset::DeepBlue => &words.deep_blue,
            PhotoFilterPreset::DeepEmerald => &words.deep_emerald,
            PhotoFilterPreset::DeepYellow => &words.deep_yellow,
            PhotoFilterPreset::Underwater => &words.underwater,
        }
        .clone()
    }

    /// The preset whose colour is `color` in 8-bit sRGB, if any (how the editor tells a preset from a custom colour).
    pub fn of(color: Srgba) -> Option<PhotoFilterPreset> {
        PhotoFilterPreset::ALL.into_iter().find(|preset| preset.color().same_rgb8(color))
    }
}

/// A Photo Filter value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PhotoFilter {
    /// The filter colour (a preset's or the user's).
    pub color: Srgba,
    /// Density 0..=100 %, default 25.
    pub density: f64,
    /// Preserve Luminosity; on by default.
    pub preserve_luminosity: bool,
}

impl PhotoFilter {
    /// A value of the filter `color`, `density` and Preserve Luminosity.
    pub fn new(color: Srgba, density: f64, preserve_luminosity: bool) -> Self {
        Self { color, density, preserve_luminosity }
    }
}

impl Default for PhotoFilter {
    fn default() -> Self {
        Self { color: PhotoFilterPreset::Warming85.color(), density: DENSITY_DEFAULT, preserve_luminosity: true }
    }
}

/// What a Photo Filter editor keeps between passes: Filter or Color, and the value its running gesture began with.
#[derive(Clone, Default)]
struct FilterMemory {
    use_color: Option<bool>,
    began: Option<PhotoFilter>,
}

/// What [`PhotoFilterEditor::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PhotoFilterOutput {
    /// The edit of the value, as every editor reports it.
    pub edit: EditResponse<PhotoFilter>,
    /// The user clicked the Color swatch: open the colour picker with this request (key `"photoFilter.color"`, title
    /// "Filter Color") and write the answer into the value's `color`.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for PhotoFilterOutput {
    type Value = PhotoFilter;

    fn response(&self) -> &egui::Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl std::ops::Deref for PhotoFilterOutput {
    type Target = EditResponse<PhotoFilter>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// Photo Filter: the Filter radio with the preset select, the Color radio with a swatch, Density and Preserve
/// Luminosity.
///
/// - Filter or Color is view state of the editor, Color at first when the value's colour is no preset
///   ([`PhotoFilterPreset::of`]). The other row's control is disabled.
/// - The preset select lists [`PhotoFilterPreset::ALL`], with "Custom" first while the colour is no preset; picking
///   one is a `Set` of its colour.
/// - The swatch asks for a colour through [`PhotoFilterOutput::pick_color`].
/// - Density 0..100 %, default 25 on a reset; Preserve Luminosity a `Set`.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct PhotoFilterEditor<'a> {
    label: Label,
    value: Bind<'a, PhotoFilter>,
    context: AdjustContext,
}

impl<'a> PhotoFilterEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, PhotoFilter>) -> Self {
        Self { label: label.into(), value, context: AdjustContext::dialog(true) }
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit and any colour-picker request.
    pub fn show(self, ui: &mut Ui) -> PhotoFilterOutput {
        let PhotoFilterEditor { label, mut value, context } = self;
        let words = words(ui);
        let mut out = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let preset = PhotoFilterPreset::of(now.color);
            let use_color = cx.memory::<FilterMemory>().use_color.unwrap_or(preset.is_none());
            let mut preset_edit = None;
            let mut pick_color = None;
            let group = RadioGroup::new(words.filter.clone(), Bind::given(use_color)).show(ui, |ui, _| {
                let mut entries: Vec<Entry<Option<PhotoFilterPreset>>> = Vec::new();
                if preset.is_none() {
                    entries.push(Entry::new(None, words.custom.clone()));
                }
                entries.extend(PhotoFilterPreset::ALL.into_iter().map(|each| Entry::new(Some(each), each.label(&words))));
                Field::new(words.filter.clone()).side(SideLabel::new().kind(LabelKind::Hidden)).id_salt("filter").show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        Radio::new(false).label(words.filter.clone()).show(ui);
                        preset_edit =
                            Some(Select::new(words.filter.clone(), Bind::given(preset), &entries).enabled(!use_color).id_salt("preset").show(ui));
                    });
                });
                Field::new(words.color.clone()).side(SideLabel::new().kind(LabelKind::Hidden)).id_salt("color").show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        Radio::new(true).label(words.color.clone()).show(ui);
                        let swatch =
                            Swatch::new(words.filter_color.clone(), now.color).picker(COLOR_KEY).enabled(use_color).id_salt("swatch").show(ui);
                        pick_color = swatch.pick_color;
                    });
                });
            });
            let mode = group.output.value().copied();
            let mut run = Run::start(&mut value, context, now, cx.memory::<FilterMemory>().began);
            if let Some(&entry) = preset_edit.as_ref().and_then(|picked| picked.value()).filter(|&&entry| entry != preset) {
                run.set(cx, &words.filter, PhotoFilter { color: entry.map_or(now.color, PhotoFilterPreset::color), ..now });
            }
            let density = AdjustField::new("density", words.density.clone(), 0.0..=100.0, DENSITY_DEFAULT).unit_kind(UnitKind::Percent);
            run.slider_row(ui, cx, &density, now.density, |now, density| PhotoFilter { density, ..*now });
            let preserve = Checkbox::new(Bind::given(now.preserve_luminosity)).label(words.preserve_luminosity.clone()).show(ui);
            run.push(cx, preserve, |now, on| PhotoFilter { preserve_luminosity: *on, ..*now });
            let memory = cx.memory::<FilterMemory>();
            memory.began = run.began();
            memory.use_color = Some(mode.unwrap_or(use_color));
            out = Some((run.finish(), pick_color));
        });
        let (edit, pick_color) = out.map_or((None, None), |(edit, pick_color)| (Some(edit), pick_color));
        PhotoFilterOutput { edit: lifted(edit, shown.output), pick_color }
    }
}
