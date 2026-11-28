//! The control column: sections of sliders over the settings.

use std::ops::RangeInclusive;

use craft_behavior::EditEvents;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, EditResponse, Entry, NoValue, Outcome, Pattern, Props, SaveKey, Show as _, ShowContainer as _, clicked};
use craft_ui::form::SliderField;
use craft_ui::layout_components::Stack;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::Section;
use craft_ui::style::DataColor;
use craft_ui::value_drag::TrackFace;
use egui::{Color32, Response, Ui};

use super::Demosaic;
use super::mixer;
use super::words::RawDevelopWords;
use crate::raw::{RawDevelopSettings, VignetteStyle};
use crate::ui::curve::{Curve, CurveEditor, ToneRamp};

/// What the control column reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct RawDevelopPanelOutput {
    /// The column's response.
    pub response: Response,
    /// The edit of the settings.
    pub edit: EditResponse<RawDevelopSettings>,
    /// The Auto button was pressed.
    pub auto: bool,
    /// The demosaic the user picked this pass, while a raw is being opened.
    pub demosaic: Option<Demosaic>,
}

impl Outcome for RawDevelopPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// One plain slider row: its label word, the settings field it edits, its range, step and optional track colours.
/// One table of these drives every plain row of the column, and the scope's tone zones find their field here by key.
#[derive(Clone, Copy)]
pub(super) struct Slider {
    /// The settings field's name, the key of a tone zone.
    pub(super) key: &'static str,
    word: fn(&RawDevelopWords) -> &Label,
    pub(super) get: fn(&RawDevelopSettings) -> f32,
    pub(super) set: fn(&mut RawDevelopSettings, f32),
    pub(super) min: f64,
    pub(super) max: f64,
    pub(super) step: f64,
    stops: Option<fn() -> Vec<DataColor>>,
}

/// A slider row over the settings field `$field`; its label is the word of the same name, or `$word` when given
/// (`field: word`).
macro_rules! slider {
    ($field:ident, $min:expr, $max:expr) => {
        slider!($field, $min, $max, 1.0, None)
    };
    ($field:ident, $min:expr, $max:expr, $step:expr) => {
        slider!($field, $min, $max, $step, None)
    };
    ($field:ident, $min:expr, $max:expr, $step:expr, $stops:expr) => {
        slider!($field: $field, $min, $max, $step, $stops)
    };
    ($field:ident: $word:ident, $min:expr, $max:expr) => {
        slider!($field: $word, $min, $max, 1.0, None)
    };
    ($field:ident: $word:ident, $min:expr, $max:expr, $step:expr, $stops:expr) => {
        Slider {
            key: stringify!($field),
            word: |w| &w.$word,
            get: |s| s.$field,
            set: |s, v| s.$field = v,
            min: $min,
            max: $max,
            step: $step,
            stops: $stops,
        }
    };
}

/// The Light section's rows below Auto.
pub(super) const LIGHT: &[Slider] = &[
    slider!(exposure, -5.0, 5.0, 0.05),
    slider!(contrast, -100.0, 100.0),
    slider!(highlights, -100.0, 100.0),
    slider!(shadows, -100.0, 100.0),
    slider!(whites, -100.0, 100.0),
    slider!(blacks, -100.0, 100.0),
];

const COLOR: &[Slider] = &[
    slider!(temperature, -100.0, 100.0, 1.0, Some(temperature_stops)),
    slider!(tint, -100.0, 100.0, 1.0, Some(tint_stops)),
    slider!(vibrance, -100.0, 100.0),
    slider!(saturation, -100.0, 100.0),
];

const EFFECTS: &[Slider] = &[slider!(texture, -100.0, 100.0), slider!(clarity, -100.0, 100.0), slider!(dehaze, -100.0, 100.0)];

const VIGNETTE: &[Slider] = &[
    slider!(vignette_amount, -100.0, 100.0),
    slider!(vignette_midpoint, 0.0, 100.0),
    slider!(vignette_roundness, -100.0, 100.0),
    slider!(vignette_feather, 0.0, 100.0),
    slider!(vignette_highlights, 0.0, 100.0),
];

const GRAIN: &[Slider] = &[slider!(grain_amount, 0.0, 100.0), slider!(grain_size, 0.0, 100.0), slider!(grain_roughness, 0.0, 100.0)];

const CURVE_REGIONS: &[Slider] = &[
    slider!(curve_highlights, -100.0, 100.0),
    slider!(curve_lights, -100.0, 100.0),
    slider!(curve_darks, -100.0, 100.0),
    slider!(curve_shadows, -100.0, 100.0),
];

const DETAIL: &[Slider] = &[
    slider!(sharpen_amount, 0.0, 150.0),
    slider!(sharpen_radius, 0.5, 3.0, 0.1),
    slider!(sharpen_detail, 0.0, 100.0),
    slider!(sharpen_masking, 0.0, 100.0),
    slider!(noise_luminance, 0.0, 100.0),
    slider!(noise_luminance_detail, 0.0, 100.0),
    slider!(noise_color, 0.0, 100.0),
    slider!(noise_color_detail, 0.0, 100.0),
];

/// The Color Grading section's rows below the four wheels.
pub(super) const GRADING_TAIL: &[Slider] = &[slider!(grade_blending: blending, 0.0, 100.0), slider!(grade_balance: balance, -100.0, 100.0)];

/// The Calibration section's first row.
const CALIBRATION: &[Slider] = &[slider!(shadow_tint, -100.0, 100.0)];

/// Every table of plain rows.
const TABLES: [&[Slider]; 9] = [LIGHT, COLOR, EFFECTS, VIGNETTE, GRAIN, CURVE_REGIONS, DETAIL, GRADING_TAIL, CALIBRATION];

/// The label word of the one row that edits the settings field `field` (its serde name, as the automation surface
/// writes it); `None` for a field no one row edits.
pub(super) fn field_word<'w>(words: &'w RawDevelopWords, field: &str) -> Option<&'w Label> {
    let row = TABLES.iter().flat_map(|table| table.iter()).find(|row| row.key.replace('_', "").eq_ignore_ascii_case(field))?;
    Some((row.word)(words))
}

/// The track of Temperature: cool to warm.
fn temperature_stops() -> Vec<DataColor> {
    vec![
        DataColor::new(Color32::from_rgb(0x46, 0x82, 0xff)),
        DataColor::new(Color32::from_rgb(0xe8, 0xe8, 0xe8)),
        DataColor::new(Color32::from_rgb(0xf5, 0xc2, 0x30)),
    ]
}

/// The track of Tint: green to magenta.
fn tint_stops() -> Vec<DataColor> {
    vec![
        DataColor::new(Color32::from_rgb(0x4c, 0xaf, 0x50)),
        DataColor::new(Color32::from_rgb(0xe8, 0xe8, 0xe8)),
        DataColor::new(Color32::from_rgb(0xd0, 0x40, 0xb0)),
    ]
}

/// The curve's levels: the settings hold point curves in 0..=255.
const LEVELS: f32 = 255.0;

/// The curve editor's value for the settings' points; an empty list is the linear curve.
fn curve_of(points: &[[f32; 2]]) -> Curve {
    if points.is_empty() { Curve::identity(LEVELS) } else { Curve::smooth(points.to_vec()) }
}

/// The settings' points for the editor's curve: the unchanged curve is the empty list. A drawn curve has no points and
/// reads as linear; the column offers no pencil.
fn points_of(curve: &Curve) -> Vec<[f32; 2]> {
    const IDENTITY: [[f32; 2]; 2] = [[0.0, 0.0], [LEVELS, LEVELS]];
    match curve.points() {
        Some(points) if points != &IDENTITY[..] => points.to_vec(),
        _ => Vec::new(),
    }
}

/// A control's edit of one field as an edit of the settings: the settings at the start of the pass with that field
/// replaced.
pub(super) fn lift<T>(events: EditEvents<T>, base: &RawDevelopSettings, set: &impl Fn(&mut RawDevelopSettings, T)) -> EditEvents<RawDevelopSettings> {
    events.map(|value| {
        let mut settings = base.clone();
        set(&mut settings, value);
        settings
    })
}

/// What the rows of one pass add up to.
#[derive(Default)]
struct Pass {
    edit: EditEvents<RawDevelopSettings>,
    auto: bool,
    demosaic: Option<Demosaic>,
}

/// The rows of one pass: the settings they started from, those Reset restores, and what they add up to.
pub(super) struct Rows<'p> {
    pub(super) base: &'p RawDevelopSettings,
    pub(super) defaults: &'p RawDevelopSettings,
    pass: Pass,
}

impl<'p> Rows<'p> {
    fn new(base: &'p RawDevelopSettings, defaults: &'p RawDevelopSettings) -> Self {
        Rows { base, defaults, pass: Pass::default() }
    }

    /// A slider with a number field over `value`; `set` writes an edit into the settings, and the edit is the pass's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn slider(
        &mut self,
        ui: &mut Ui,
        label: &Label,
        value: f32,
        default: f32,
        range: RangeInclusive<f64>,
        step: f64,
        stops: Option<Vec<DataColor>>,
        set: impl Fn(&mut RawDevelopSettings, f32),
    ) {
        let mut field = SliderField::new(label.clone(), Bind::given(f64::from(value)).reset(f64::from(default)), range).step(step);
        if let Some(stops) = &stops {
            field = field.track_face(TrackFace::new(ToneRamp::new(stops)));
        }
        let events = field.show(ui).edit.events;
        self.pass.edit.extend(lift(events, self.base, &|settings: &mut RawDevelopSettings, v: f64| set(settings, v as f32)));
    }

    /// One slider row for each entry of `table`.
    pub(super) fn table(&mut self, ui: &mut Ui, words: &RawDevelopWords, table: &[Slider]) {
        for row in table {
            let stops = row.stops.map(|stops| stops());
            self.slider(ui, (row.word)(words), (row.get)(self.base), (row.get)(self.defaults), row.min..=row.max, row.step, stops, row.set);
        }
    }

    /// The Curve section's editor over the point curve.
    fn curve(&mut self, ui: &mut Ui, label: &Label) {
        let curve = curve_of(&self.base.point_curve);
        let shown = CurveEditor::new(label.clone(), Bind::given(curve)).levels(LEVELS as u32).show(ui);
        self.pass.edit.extend(lift(shown.edit.events, self.base, &|settings: &mut RawDevelopSettings, curve: Curve| {
            settings.point_curve = points_of(&curve);
        }));
    }

    /// The three region splits under the editor: between shadows and darks, darks and lights, lights and highlights.
    fn splits(&mut self, ui: &mut Ui, words: &RawDevelopWords) {
        let labels = [&words.curve_shadow_split, &words.curve_midtone_split, &words.curve_highlight_split];
        for (i, label) in labels.into_iter().enumerate() {
            let (value, default) = (self.base.curve_splits[i], self.defaults.curve_splits[i]);
            self.slider(ui, label, value, default, 0.0..=100.0, 1.0, None, move |settings, v| settings.curve_splits[i] = v);
        }
    }

    /// The Calibration section's rows: Shadows Tint, then each primary's Hue and Saturation.
    fn calibration(&mut self, ui: &mut Ui, words: &RawDevelopWords) {
        self.table(ui, words, CALIBRATION);
        let primaries = [&words.primary_red, &words.primary_green, &words.primary_blue];
        for (i, primary) in primaries.into_iter().enumerate() {
            let fill = [("primary", primary.as_str())];
            let hue = Label::from(words.calibration_hue.fill(&fill));
            let (value, default) = (self.base.calibration_hue[i], self.defaults.calibration_hue[i]);
            self.slider(ui, &hue, value, default, -100.0..=100.0, 1.0, None, move |settings, v| settings.calibration_hue[i] = v);
            let sat = Label::from(words.calibration_sat.fill(&fill));
            let (value, default) = (self.base.calibration_sat[i], self.defaults.calibration_sat[i]);
            self.slider(ui, &sat, value, default, -100.0..=100.0, 1.0, None, move |settings, v| settings.calibration_sat[i] = v);
        }
    }

    /// The Vignetting style select.
    fn vignette_style(&mut self, ui: &mut Ui, words: &RawDevelopWords) {
        let entries = [
            Entry::new(VignetteStyle::HighlightPriority, words.highlight_priority.clone()),
            Entry::new(VignetteStyle::ColorPriority, words.color_priority.clone()),
            Entry::new(VignetteStyle::PaintOverlay, words.paint_overlay.clone()),
        ];
        let events = Select::new(words.vignette_style.clone(), Bind::given(self.base.vignette_style), &entries).show(ui).events;
        self.pass.edit.extend(lift(events, self.base, &|settings: &mut RawDevelopSettings, style: VignetteStyle| settings.vignette_style = style));
    }

    /// The Demosaic select of the Detail section.
    fn demosaic(&mut self, ui: &mut Ui, words: &RawDevelopWords, current: Demosaic) {
        let entries = [
            Entry::new(Demosaic::Ahd, words.demosaic_ahd.clone()),
            Entry::new(Demosaic::Mhc, words.demosaic_mhc.clone()),
            Entry::new(Demosaic::Bilinear, words.demosaic_bilinear.clone()),
        ];
        let events = Select::new(words.demosaic.clone(), Bind::given(current), &entries).show(ui).events;
        if let Some(picked) = events.value().copied() {
            self.pass.demosaic = Some(picked);
        }
    }
}

/// A collapsible section whose open state is remembered under `craft_imaging.raw_develop.section.<name>`.
pub(super) fn section(label: &Label, name: &str, open: bool) -> Section<'static> {
    Section::new(label.clone()).remember(SaveKey::named(format!("craft_imaging.raw_develop.section.{name}"))).initially_open(open)
}

/// The control column: Light, Color, Effects, Curve, Color Mixer, Color Grading, Detail and Calibration sections. Each
/// row shows `Bind::given(f64::from(value))` and writes its edit back as `f32`.
#[must_use = "show the panel with `show`"]
#[derive(Pattern, Props)]
pub struct RawDevelopPanel<'a> {
    settings: Bind<'a, RawDevelopSettings>,
    /// The demosaic select shows while a raw is being opened.
    #[prop]
    demosaic: Option<Demosaic>,
}

impl<'a> RawDevelopPanel<'a> {
    /// The panel over `settings`.
    pub fn new(settings: Bind<'a, RawDevelopSettings>) -> Self {
        RawDevelopPanel { settings, demosaic: None }
    }

    /// Draws the sections. A body that runs twice in one pass (the measuring run, then the real one) builds its rows
    /// afresh each run; only the real run sees input, so the edit is the real run's.
    pub fn show(self, ui: &mut Ui) -> RawDevelopPanelOutput {
        let Self { mut settings, demosaic } = self;
        let words = Env::of(ui).words::<RawDevelopWords>();
        let defaults = RawDevelopSettings::default();
        let shown = Stack::new().show(ui, |ui, root| {
            let Some(base) = settings.reborrow().read(root).and_then(|held| held.same().cloned()) else { return Pass::default() };
            let mut rows = Rows::new(&base, &defaults);
            section(&words.section_light, "light", true).show(ui, |ui, _| {
                if clicked(&Button::new(words.auto.clone()).id_salt("auto").show(ui)) {
                    rows.pass.auto = true;
                }
                rows.table(ui, &words, LIGHT);
            });
            section(&words.section_color, "color", false).show(ui, |ui, _| {
                rows.table(ui, &words, COLOR);
            });
            section(&words.section_effects, "effects", false).show(ui, |ui, _| {
                rows.table(ui, &words, EFFECTS);
                rows.vignette_style(ui, &words);
                rows.table(ui, &words, VIGNETTE);
                rows.table(ui, &words, GRAIN);
            });
            section(&words.section_curve, "curve", false).show(ui, |ui, _| {
                rows.curve(ui, &words.section_curve);
                rows.table(ui, &words, CURVE_REGIONS);
                rows.splits(ui, &words);
            });
            mixer::color_mixer(ui, &mut rows, &words);
            mixer::color_grading(ui, &mut rows, &words);
            section(&words.section_detail, "detail", false).show(ui, |ui, _| {
                if let Some(current) = demosaic {
                    rows.demosaic(ui, &words, current);
                }
                rows.table(ui, &words, DETAIL);
            });
            section(&words.section_calibration, "calibration", false).show(ui, |ui, _| {
                rows.calibration(ui, &words);
            });
            for event in &rows.pass.edit {
                settings.reborrow().write(root, event);
            }
            rows.pass
        });
        let pass = shown.inner.unwrap_or_default();
        RawDevelopPanelOutput {
            edit: EditResponse::lifted(shown.output.clone(), pass.edit, None),
            response: shown.output,
            auto: pass.auto,
            demosaic: pass.demosaic,
        }
    }
}
