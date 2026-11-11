//! [`CropBar`]: the Crop tool's options as children of the app's options [`Bar`](craft_ui::bars::Bar).

use super::bar_words::CropBarWords;
use super::menus::{CropOptionsMenu, CropOverlayMenu};
use super::options::{
    CROP_RATIOS, CROP_SIZES, CropLength, CropPresetChoice, CropRatio, CropSettings, CropSizeFields, CropUserPreset, FIELD_MAX, LENGTH_MIN,
    RATIO_DECIMALS, RATIO_MAX, RATIO_STEP, ResolutionUnit,
};
use super::session::CropDocument;
use craft_behavior::{EditEvent, EditEvents, IconId, NumberCodec};
use craft_ui::buttons::{Button, IconButton, ToggleButton};
use craft_ui::contract::{
    Bind, BodyCx, Entry, LabelKind, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, SideLabel, Sizing, clicked,
};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, LengthUnits, UnitKind, Units};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::text::NumberField;
use egui::{Response, Ui};

#[cfg(feature = "screen-testing")]
mod stories;

/// The ratio, a ratio field, a preset or W × H × Resolution changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CropRatioChange {
    /// True while a ratio field is scrubbed, so the frame refits from the frame the edit began with. A typed edit
    /// reports once, when the typing run ends, so typing never refits live.
    pub editing: bool,
}

/// What the bar reported this pass. Edits of the options go straight into the settings the bar was given
/// ([`settings_changed`](Self::settings_changed) says they changed); these are the rest. ✓ and ⊘ are the app's
/// [`Bar::commit_cancel`](craft_ui::bars::Bar::commit_cancel).
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app refits the frame, ends the session or opens a dialog for each field that is set"]
pub struct CropBarOutput {
    /// The strip's response.
    pub response: Response,
    /// The ratio changed: the app calls `CropTool::refit` with [`CropRatioChange::editing`].
    pub ratio: Option<CropRatioChange>,
    /// The Straighten button was pressed: the app calls `CropTool::toggle_straighten`.
    pub straighten: bool,
    /// The Angle field was edited: the app turns the frame to this angle, in degrees.
    pub angle: Option<f64>,
    /// New Crop Preset…: the app opens its dialog with [`CropSettings::preset_name`].
    pub new_preset: bool,
    /// Delete Crop Preset…: the app opens its dialog.
    pub delete_preset: bool,
    /// The gear menu's Custom… or the shield swatch: the app opens its colour picker with this request and sets
    /// `CropShield::custom_color` from the answer.
    pub pick_color: Option<ColorRequest>,
    /// Any option of the settings changed this pass: the app persists them.
    pub settings_changed: bool,
}

impl Outcome for CropBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Crop tool's options, shown as children of the app's `Bar::new(label, BarKind::Options)`; ✓ and ⊘ are the bar's
/// `commit_cancel`.
///
/// - Order (Photoshop's): the preset list (Ratio, W x H x Resolution, the ratio presets, the size presets, the user's
///   presets, Front Image, a rule, New Crop Preset…, Delete Crop Preset…); the W and H ratio fields (0 to 99,999 with 3
///   decimals) with the swap button between them, or in W × H × Resolution mode the W, H and Resolution fields with
///   the px/in or px/cm select; Clear; the Angle field; the Straighten toggle (pressed while armed); the overlay menu
///   ([`CropOverlayMenu`]); the gear popover ([`CropOptionsMenu`]).
/// - W and H keep their lengths with the unit they were entered in ([`CropLength`]); a cleared field is unset.
/// - A scrub or a step of a ratio field refits live; Escape puts the ratio back.
/// - Delete Crop Preset… is disabled when no user preset is given; New Crop Preset… when [`CropSettings::preset_name`]
///   is `None`; Original Ratio and Front Image without a document.
/// - **Keyboard and AccessKit:** the list is a select named "Crop ratio" with the chosen entry as its value; the
///   fields are named Width, Height, Resolution and Angle; the Straighten toggle has its pressed state; the menus
///   open with Enter or Space.
#[must_use = "show the bar's children with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct CropBar<'a> {
    settings: &'a mut CropSettings,
    document: &'a CropDocument,
    /// The user's saved crop presets, oldest first.
    #[prop]
    user_presets: &'a [CropUserPreset],
    /// The Straighten toggle's pressed state (`CropTool::straighten_armed`).
    #[prop]
    straighten: bool,
    /// The frame's angle in degrees, for the Angle field; without a frame the field is disabled.
    #[prop(quantity = "angle in degrees")]
    angle: Option<f64>,
}

impl<'a> CropBar<'a> {
    /// The bar editing `settings`, over `document` (Original Ratio's numbers and Front Image).
    pub fn new(settings: &'a mut CropSettings, document: &'a CropDocument) -> Self {
        Self { settings, document, user_presets: &[], straighten: false, angle: None }
    }

    /// Shows the bar's children.
    pub fn show(self, ui: &mut Ui) -> CropBarOutput {
        let Self { settings, document, user_presets, straighten, angle } = self;
        let env = Env::of(ui);
        let words = env.words::<CropBarWords>();
        let units = env.units();
        let has_document = document.size.width > 0.0 && document.size.height > 0.0;
        let before = settings.clone();
        let mut report = Report::default();
        let shown = Row::new().show(ui, |ui, cx| {
            preset_list(ui, settings, &words, document, has_document, user_presets, &units, &mut report);
            if settings.ratio == CropRatio::Size {
                size_fields(ui, settings, &words, document, &units, &mut report);
            } else {
                ratio_fields(ui, cx, settings, &words, document, has_document, &units, &mut report);
            }
            Divider::vertical().show(ui);
            if clicked(&Button::new(words.clear.clone()).show(ui)) {
                settings.clear();
                report.ratio = Some(CropRatioChange { editing: false });
            }
            Field::new(words.angle.clone()).id_salt("angle").sizing(Sizing::Content).enabled(angle.is_some()).show(ui, |ui, _| {
                let field = NumberField::new(words.angle.clone(), Bind::given(angle)).unit_kind(UnitKind::Degrees).range(-180.0..=180.0).show(ui);
                if let Some(turned) = field.value().copied().flatten() {
                    report.angle = Some(turned);
                }
            });
            let toggle = ToggleButton::new(words.straighten.clone(), Bind::given(straighten)).icon(IconId::new("ruler")).icon_only().show(ui);
            report.straighten |= toggle.value().is_some();
            CropOverlayMenu::new(settings).show(ui);
            report.pick_color = CropOptionsMenu::new(settings).show(ui).pick_color;
        });
        let changed = *settings != before;
        CropBarOutput {
            response: shown.output,
            ratio: report.ratio,
            straighten: report.straighten,
            angle: report.angle,
            new_preset: report.new_preset,
            delete_preset: report.delete_preset,
            pick_color: report.pick_color,
            settings_changed: changed,
        }
    }
}

/// What the cells reported this pass.
#[derive(Default)]
struct Report {
    ratio: Option<CropRatioChange>,
    straighten: bool,
    angle: Option<f64>,
    new_preset: bool,
    delete_preset: bool,
    pick_color: Option<ColorRequest>,
}

/// The ratio and its fields when a scrub of a ratio field began, for Escape.
#[derive(Clone, Default)]
struct RatioStart(Option<(CropRatio, [f64; 2])>);

/// The name of size preset `index` of [`CROP_SIZES`]: its values as New Crop Preset… would name them.
fn size_name(words: &CropBarWords, index: usize, units: &Units) -> String {
    let Some(preset) = CROP_SIZES.get(index) else { return String::new() };
    let size = CropSizeFields {
        width: Some(preset.width),
        height: Some(preset.height),
        resolution: preset.resolution.to_owned(),
        ..CropSizeFields::default()
    };
    let settings = CropSettings { ratio: CropRatio::Size, size, ..CropSettings::default() };
    settings.preset_name(words, units).unwrap_or_default()
}

/// The preset list: Ratio, W x H x Resolution, the ratio presets, the size presets, the user's presets, Front Image,
/// then New Crop Preset… and Delete Crop Preset….
#[expect(clippy::too_many_arguments, reason = "the bar's inputs")]
fn preset_list(
    ui: &mut Ui,
    settings: &mut CropSettings,
    words: &CropBarWords,
    document: &CropDocument,
    has_document: bool,
    user_presets: &[CropUserPreset],
    units: &Units,
    report: &mut Report,
) {
    let mut entries = vec![Entry::new(CropPresetChoice::Free, words.ratio.clone()), Entry::new(CropPresetChoice::Size, words.size_mode.clone())];
    for (i, ratio) in CROP_RATIOS.iter().enumerate() {
        let entry = Entry::new(CropPresetChoice::Ratio(i), words.ratio_name(i).to_owned());
        entries.push(if *ratio == CropRatio::Original { entry.enabled(has_document) } else { entry });
    }
    for i in 0..CROP_SIZES.len() {
        entries.push(Entry::new(CropPresetChoice::SizePreset(i), size_name(words, i, units)));
    }
    for (i, preset) in user_presets.iter().enumerate() {
        entries.push(Entry::new(CropPresetChoice::User(i), preset.name.clone()));
    }
    entries.push(Entry::new(CropPresetChoice::FrontImage, words.front_image.clone()).enabled(has_document));
    entries.push(Entry::separator());
    entries.push(Entry::new(CropPresetChoice::NewPreset, words.new_preset.clone()).enabled(settings.preset_name(words, units).is_some()));
    entries.push(Entry::new(CropPresetChoice::DeletePreset, words.delete_preset.clone()).enabled(!user_presets.is_empty()));
    let current = settings.choice(user_presets);
    let pick = Select::new(words.presets.clone(), Bind::given(current), &entries).show(ui);
    let Some(choice) = pick.value().copied() else { return };
    match choice {
        CropPresetChoice::NewPreset => report.new_preset = true,
        CropPresetChoice::DeletePreset => report.delete_preset = true,
        choice => {
            if settings.choose(choice, document, user_presets) {
                report.ratio = Some(CropRatioChange { editing: false });
            }
        }
    }
}

/// The two ratio fields with the swap button between them.
#[expect(clippy::too_many_arguments, reason = "the bar's inputs")]
fn ratio_fields(
    ui: &mut Ui,
    cx: &mut BodyCx<'_>,
    settings: &mut CropSettings,
    words: &CropBarWords,
    document: &CropDocument,
    has_document: bool,
    units: &Units,
    report: &mut Report,
) {
    let shown = settings.shown_ratio_fields(document);
    let mut changed = false;
    let mut editing = false;
    ratio_field(ui, cx, 0, &words.width, shown, settings, units, &mut changed, &mut editing);
    let original = settings.ratio == CropRatio::Original;
    let swap = IconButton::new(IconId::new("swap_horizontal")).label(words.swap.clone()).enabled(has_document || !original).show(ui);
    ratio_field(ui, cx, 1, &words.height, shown, settings, units, &mut changed, &mut editing);
    if clicked(&swap) {
        if original {
            settings.swap(document);
        } else {
            settings.swap_fields();
        }
        changed = true;
    }
    if changed {
        report.ratio = Some(CropRatioChange { editing });
    }
}

/// One ratio field: `shown` is both fields' numbers as drawn; an empty field is 0.
#[expect(clippy::too_many_arguments, reason = "the bar's inputs")]
fn ratio_field(
    ui: &mut Ui,
    cx: &mut BodyCx<'_>,
    side: usize,
    caption: &Label,
    shown: [f64; 2],
    settings: &mut CropSettings,
    units: &Units,
    changed: &mut bool,
    editing: &mut bool,
) {
    let current = shown.get(side).copied().unwrap_or(0.0);
    let salt = if side == 0 { "ratio-width" } else { "ratio-height" };
    let mut events = EditEvents::new();
    Field::new(caption.clone()).side(SideLabel::new().kind(LabelKind::Hidden)).id_salt(salt).sizing(Sizing::Content).show(ui, |ui, _| {
        let codec = NumberCodec::new(units.reader(None), RATIO_DECIMALS);
        let field = NumberField::new(caption.clone(), Bind::given((current > 0.0).then_some(current)))
            .codec(codec)
            .range(0.0..=RATIO_MAX)
            .step(RATIO_STEP)
            .automatic(true)
            .show(ui);
        events = field.edit.events.clone();
    });
    // Returns whether the value changed the settings.
    let apply = |settings: &mut CropSettings, value: Option<f64>| {
        let new = value.unwrap_or(0.0);
        if (new - current).abs() < 0.5 * 10f64.powi(-(RATIO_DECIMALS as i32)) {
            return false;
        }
        // The other side keeps what the bar shows, so an Original ratio becomes its sides.
        if let Some(other) = settings.ratio_fields.get_mut(1 - side) {
            *other = shown.get(1 - side).copied().unwrap_or(0.0);
        }
        settings.set_ratio_field(side, new);
        true
    };
    for event in events {
        match event {
            EditEvent::Begin { .. } => {
                cx.memory::<RatioStart>().0 = Some((settings.ratio, settings.ratio_fields));
                *editing = true;
            }
            EditEvent::Update { value, .. } => {
                *editing = true;
                *changed |= apply(settings, value);
            }
            EditEvent::Commit { after: value, .. } | EditEvent::Set { after: value, .. } => {
                cx.memory::<RatioStart>().0 = None;
                *changed |= apply(settings, value);
            }
            EditEvent::Cancel { .. } => {
                if let Some((ratio, fields)) = cx.memory::<RatioStart>().0.take() {
                    settings.ratio = ratio;
                    settings.ratio_fields = fields;
                    *changed = true;
                }
            }
            _ => {}
        }
    }
}

/// W, swap, H, Resolution and its unit, in W × H × Resolution mode.
fn size_fields(ui: &mut Ui, settings: &mut CropSettings, words: &CropBarWords, document: &CropDocument, units: &Units, report: &mut Report) {
    let mut changed = false;
    let pixels_per_inch = settings.size.resolution_ppi().unwrap_or(document.resolution);
    changed |= length_field(ui, "size-width", &words.width, &mut settings.size.width, pixels_per_inch, units.points_per_inch());
    if clicked(&IconButton::new(IconId::new("swap_horizontal")).label(words.swap.clone()).show(ui)) {
        settings.swap_fields();
        changed = true;
    }
    changed |= length_field(ui, "size-height", &words.height, &mut settings.size.height, pixels_per_inch, units.points_per_inch());

    let resolution = settings.size.resolution.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0);
    let mut typed = None;
    Field::new(words.resolution.clone()).id_salt("size-resolution").sizing(Sizing::Content).show(ui, |ui, _| {
        let codec = NumberCodec::new(units.reader(None), 2);
        let field = NumberField::new(words.resolution.clone(), Bind::given(resolution)).codec(codec).range(0.0..=FIELD_MAX).automatic(true).show(ui);
        typed = field.value().copied();
    });
    if let Some(value) = typed {
        let text = match value.filter(|v| *v > 0.0) {
            Some(v) => CropSizeFields::normalize_resolution(&v.to_string(), &settings.size.resolution),
            None => String::new(),
        };
        if text != settings.size.resolution {
            settings.size.resolution = text;
            changed = true;
        }
    }

    let unit_entries = [
        Entry::new(ResolutionUnit::PixelsPerInch, words.pixels_per_inch.clone()),
        Entry::new(ResolutionUnit::PixelsPerCentimetre, words.pixels_per_centimetre.clone()),
    ];
    let pick = Select::new(words.resolution_unit.clone(), Bind::given(settings.size.resolution_unit), &unit_entries).show(ui);
    if let Some(unit) = pick.value().copied()
        && unit != settings.size.resolution_unit
    {
        // The same resolution in the other unit, as in Photoshop.
        if let Some(v) = resolution {
            let converted = if unit == ResolutionUnit::PixelsPerCentimetre { v / 2.54 } else { v * 2.54 };
            settings.size.resolution = CropSizeFields::normalize_resolution(&converted.to_string(), &settings.size.resolution);
        }
        settings.size.resolution_unit = unit;
        changed = true;
    }
    if changed {
        report.ratio = Some(CropRatioChange { editing: false });
    }
}

/// A length field over a length kept in its own unit: a typed number is in that unit (pixels while unset), a typed
/// unit is converted into it, and a cleared field is unset. Returns whether `value` changed.
fn length_field(
    ui: &mut Ui,
    salt: &'static str,
    caption: &Label,
    value: &mut Option<CropLength>,
    pixels_per_inch: f64,
    points_per_inch: f64,
) -> bool {
    let unit = value.map_or(UnitKind::Pixels, |length| length.unit);
    let mut typed = None;
    let mut typed_unit = None;
    Field::new(caption.clone()).id_salt(salt).sizing(Sizing::Content).show(ui, |ui, _| {
        let lengths = LengthUnits::new(unit, unit, pixels_per_inch);
        let field =
            NumberField::length(caption.clone(), Bind::given(value.map(|length| length.value)), lengths).range(LENGTH_MIN..=FIELD_MAX).show(ui);
        typed = field.value().copied();
        typed_unit = field.typed_unit;
    });
    let Some(next) = typed else { return false };
    let next = next.map(|length| match typed_unit {
        // The value is in the stored unit; a typed unit becomes the stored one, as in Photoshop.
        Some(to) if to != unit => {
            let pixels = unit.to_pixels(length, 1.0, pixels_per_inch, points_per_inch);
            let converted = pixels.and_then(|pixels| to.from_pixels(pixels, 1.0, pixels_per_inch, points_per_inch));
            converted.map_or(CropLength::new(length, unit), |value| CropLength::new(value, to))
        }
        _ => CropLength::new(length, unit),
    });
    let changed = next != *value;
    *value = next;
    changed
}
