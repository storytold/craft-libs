//! The details column of New Document: the name, the settings type's own rows, and the Save Document Preset pane that
//! stands in for them while a preset is named.

use super::presets::{
    BackgroundContents, ColorMode, DocumentOptions, MAX_NAME, MAX_SAVED, Orientation, PIXEL_ASPECTS, RESOLUTION_UNITS, SIZE_UNITS, SQUARE_PIXELS,
};
use super::settings::{DetailsCx, DocumentSettings, RasterMemory};
use super::{BACKGROUND_COLOR_KEY, NewDocumentWords, Read, Session, background_rgb, depth_label, mode_label};
use crate::buttons::{Button, IconButton};
use crate::choice::{Checkbox, Segmented, Swatch};
use crate::contract::{Bind, Entry, FieldLayout, Gap, MemoProps, Props as _, Show as _, ShowContainer as _, Sizing, Validity, clicked};
use crate::display::{Shade, Text};
use crate::form::{Field, FieldGroup};
use crate::layout_components::{Grid, Row};
use crate::patterns::form_dialogs::LinkedSize;
use crate::patterns::presets::{PresetError, PresetNames};
use crate::pickers::{Select, UnitSelect};
use crate::runtime::env::{Env, Label, Message, UnitKind, Units};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::requests::{ColorRequest, HostAnswer};
use crate::runtime::words::CommonWords;
use crate::style::TypeRole;
use crate::text::{NumberField, TextField};
use craft_behavior::{ColumnWidth, EditEvent, EditEvents, IconId, NumberCodec, PixelSize};
use craft_theme::Srgba;
use egui::Ui;

/// What the details column asks the dialog to do after the pass.
#[derive(Default)]
pub(super) struct Right {
    /// The colours the rows ask the app for.
    pub(super) requests: Vec<ColorRequest>,
    /// The `request_action` ids of the rows.
    pub(super) actions: Vec<String>,
    /// Save the settings as a preset under the typed name.
    pub(super) save: bool,
}

/// The Background Contents list's choices; Custom stands for any colour of the user's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackgroundKind {
    White,
    Black,
    BackgroundColor,
    Transparent,
    Custom,
}

impl BackgroundKind {
    fn of(background: BackgroundContents) -> Self {
        match background {
            BackgroundContents::White => Self::White,
            BackgroundContents::Black => Self::Black,
            BackgroundContents::BackgroundColor => Self::BackgroundColor,
            BackgroundContents::Transparent => Self::Transparent,
            BackgroundContents::Custom(_) => Self::Custom,
        }
    }
}

/// Applies the edits of a width, height or resolution field to the linked fields, in order. A gesture that is cancelled
/// puts back the fields from before it.
fn apply_number(memory: &mut RasterMemory, events: &EditEvents<Option<f64>>, set: impl Fn(&mut LinkedSize, f64)) {
    for event in events {
        match event {
            EditEvent::Begin { .. } => memory.gesture = Some(memory.size.clone()),
            EditEvent::Cancel { .. } => {
                if let Some(before) = memory.gesture.take() {
                    memory.size = before;
                }
            }
            _ => {
                if memory.gesture.is_none() && matches!(event, EditEvent::Update { .. }) {
                    memory.gesture = Some(memory.size.clone());
                }
                if let Some(Some(value)) = event.value() {
                    set(&mut memory.size, *value);
                }
                if matches!(event, EditEvent::Commit { .. } | EditEvent::Set { .. }) {
                    memory.gesture = None;
                }
            }
        }
    }
}

/// The entries of a unit list, named by the units' names.
fn unit_entries(units: &Units, kinds: &[UnitKind]) -> Vec<Entry<UnitKind>> {
    kinds.iter().map(|kind| Entry::new(*kind, units.name(*kind))).collect()
}

/// Shows the column: the name, then the settings type's rows, then the settings made valid.
pub(super) fn show<S: DocumentSettings>(
    ui: &mut Ui,
    session: &mut Session<S>,
    values: &mut S,
    read: &Read<'_, S>,
    answers: &mut Vec<HostAnswer>,
    right: &mut Right,
) {
    if session.naming.is_some() {
        save_pane(ui, session, read, right);
        return;
    }
    let words = read.words;
    let mut cx =
        DetailsCx::new(read.setup, &mut session.memory, read.units, words, &mut session.orientation, &mut session.card, std::mem::take(answers));

    Text::new(words.preset_details.clone()).role(TypeRole::SectionCaption).shade(Shade::Muted).show(ui);
    FieldGroup::new(words.preset_details.clone()).gap(Gap::Tight).show(ui, |ui, _| {
        Field::new(words.name.clone()).layout(FieldLayout::Above).id_salt("name").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let shown = TextField::new(words.name.clone(), Bind::given(values.name().to_owned()))
                    .limit(MAX_NAME)
                    .select_on_focus()
                    .sizing(Sizing::FillWidth)
                    .show(ui);
                if let Some(name) = shown.value().cloned() {
                    values.set_name(name);
                }
                if std::mem::take(&mut session.focus_name) {
                    focus::request(ui.ctx(), FocusTarget::Id(shown.response.id));
                }
                let title = words.save_preset_title.clone();
                let save = IconButton::new(IconId::new("square_plus")).label(title.clone()).tooltip(title).id_salt("save-preset").show(ui);
                if clicked(&save) {
                    session.naming = Some(proposed_name(read));
                    session.name_tried = false;
                    session.focus_preset_name = true;
                }
            });
        });
        S::details(ui, values, &mut cx);
    });

    let fitted = values.clone().fitted();
    if fitted != *values {
        *values = fitted;
    }
    (right.requests, right.actions) = cx.into_requests();
}

/// The raster rows. They hold the session's memory apart from the context, so that the context stays free for the
/// requests the rows make; the memory goes back when they are done.
pub(super) fn raster_details(ui: &mut Ui, values: &mut DocumentOptions, cx: &mut DetailsCx<'_, DocumentOptions>) {
    let mut memory = std::mem::take(cx.memory());
    raster_rows(ui, values, &mut memory, cx);
    *cx.memory() = memory;
}

fn raster_rows(ui: &mut Ui, values: &mut DocumentOptions, memory: &mut RasterMemory, cx: &mut DetailsCx<'_, DocumentOptions>) {
    let (words, setup, units) = (cx.words(), cx.setup(), cx.units());
    for answer in cx.take_answers() {
        if let HostAnswer::Color { key, color } = answer
            && key == BACKGROUND_COLOR_KEY
        {
            values.background = BackgroundContents::Custom(color.to_rgb_array());
        }
    }
    if memory.relink || !values.shown_by(&memory.size) {
        memory.size = values.linked(units.points_per_inch());
        memory.gesture = None;
        memory.relink = false;
    }
    let before = values.clone();
    let mut keep_selection = false;
    let mut ask_color = false;

    let size_units = unit_entries(units, &SIZE_UNITS);
    let resolution_units = unit_entries(units, &RESOLUTION_UNITS);
    let orientations = [
        Entry::new(Orientation::Portrait, words.portrait.clone()).icon(IconId::new("orientation_portrait")),
        Entry::new(Orientation::Landscape, words.landscape.clone()).icon(IconId::new("orientation_landscape")),
    ];
    let modes = ColorMode::ALL.map(|mode| Entry::new(mode, mode_label(words, mode).clone()));
    let depths: Vec<_> = values.color_mode.depths().iter().map(|&depth| Entry::new(depth, depth_label(words, depth).clone())).collect();
    let working = setup.profiles.iter().find(|profiles| profiles.mode == values.color_mode);
    let working_label =
        working.map_or_else(|| words.working.clone(), |profiles| Label::from(words.working_profile.fill(&[("name", profiles.working.as_str())])));
    let others: Vec<&String> = working.map_or_else(Vec::new, |profiles| profiles.others.iter().collect());
    let mut profiles = vec![Entry::new(None, working_label)];
    profiles.extend(others.iter().map(|name| Entry::new(Some((*name).clone()), (*name).clone())));
    let mut backgrounds = vec![
        Entry::new(BackgroundKind::White, words.white.clone()),
        Entry::new(BackgroundKind::Black, words.black.clone()),
        Entry::new(BackgroundKind::BackgroundColor, words.background_color.clone()),
        Entry::new(BackgroundKind::Transparent, words.transparent.clone()),
        Entry::new(BackgroundKind::Custom, words.custom_color.clone()),
    ];
    if !values.color_mode.allows_transparent() {
        backgrounds.retain(|entry| entry.value() != Some(&BackgroundKind::Transparent));
    }
    let mut pixel_aspects: Vec<Entry<String>> =
        PIXEL_ASPECTS.iter().map(|(id, _)| Entry::new((*id).to_owned(), pixel_aspect_label(words, id).clone())).collect();
    pixel_aspects.extend(setup.custom_pixel_aspects.iter().map(|(id, name)| Entry::new(id.clone(), name.clone())));

    let dimension = |ui: &mut Ui, memory: &mut RasterMemory, side: Side| {
        Field::new(side.name(words).clone()).layout(FieldLayout::Above).id_salt(side.key()).show(ui, |ui, _| {
            let (value, range) = match side {
                Side::Width => (memory.size.width(), memory.size.width_range()),
                Side::Height => (memory.size.height(), memory.size.height_range()),
            };
            if matches!(side, Side::Height) {
                // One unit serves both sides, so its select sits on the Width row only and Height leaves that
                // room to Orientation beside it. The field reads and steps in the shared unit.
                let kind = memory.size.unit();
                let units = Env::of(ui).units();
                let shown = NumberField::new(side.name(words).clone(), Bind::given(Some(value)))
                    .range(range)
                    .codec(NumberCodec::new(units.reader(Some(kind)), *kind.decimals().end()))
                    .step(kind.step())
                    .sizing(Sizing::FillWidth)
                    .show(ui);
                apply_number(memory, &shown.edit.events, |size, value| size.set_height(value));
                return;
            }
            let shown = UnitSelect::new(side.name(words).clone(), Bind::given(Some(value)), Bind::given(memory.size.unit()), &size_units)
                .range(range)
                .unit_label(side.unit_name(words).clone())
                .sizing(Sizing::FillWidth)
                .show(ui);
            apply_number(memory, &shown.edit.events, |size, value| size.set_width(value));
            if let Some(kind) = shown.unit.value().copied() {
                memory.size.set_unit(kind);
            }
        });
    };
    dimension(ui, memory, Side::Width);
    // Two shares, as Color Mode and Depth: a content-sized column here was measured empty, which clipped Orientation
    // out of reach.
    const HEIGHT_AND_ORIENTATION: [ColumnWidth; 2] = [ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];
    Grid::new(&HEIGHT_AND_ORIENTATION).fill().show(ui, |ui, _| {
        dimension(ui, memory, Side::Height);
        Field::new(words.orientation.clone()).layout(FieldLayout::Above).id_salt("orientation").show(ui, |ui, _| {
            let current = Orientation::of(memory.size.pixels());
            let shown = Segmented::new(words.orientation.clone(), Bind::given(current), &orientations).show(ui);
            if let Some(turned) = shown.value().cloned() {
                cx.set_orientation(turned);
                if current != turned {
                    values.take_linked(&memory.size);
                    let PixelSize { width, height, .. } = values.size;
                    values.size = PixelSize::new(height, width);
                    memory.size = values.linked(units.points_per_inch());
                }
                keep_selection = true;
            }
        });
    });

    let shown = Checkbox::new(Bind::given(values.artboard)).label(words.artboards.clone()).show(ui);
    if let Some(on) = shown.value().cloned() {
        values.artboard = on;
        memory.preset_artboard = false;
    }

    Field::new(words.resolution.clone()).layout(FieldLayout::Above).id_salt("resolution").show(ui, |ui, _| {
        let shown = UnitSelect::new(
            words.resolution.clone(),
            Bind::given(Some(memory.size.resolution())),
            Bind::given(memory.size.resolution_unit()),
            &resolution_units,
        )
        .range(memory.size.resolution_range())
        .unit_label(words.resolution_unit.clone())
        .sizing(Sizing::FillWidth)
        .show(ui);
        apply_number(memory, &shown.edit.events, LinkedSize::set_resolution);
        if let Some(kind) = shown.unit.value().copied() {
            memory.size.set_resolution_unit(kind);
        }
    });
    values.take_linked(&memory.size);

    const MODE_AND_DEPTH: [ColumnWidth; 2] = [ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];
    Grid::new(&MODE_AND_DEPTH).fill().show(ui, |ui, _| {
        Field::new(words.color_mode.clone()).layout(FieldLayout::Above).id_salt("mode").show(ui, |ui, _| {
            let shown = Select::new(words.color_mode.clone(), Bind::given(values.color_mode), &modes).sizing(Sizing::FillWidth).show(ui);
            if let Some(mode) = shown.value().cloned() {
                values.color_mode = mode;
                // A profile of the old mode's colour space does not suit the new one.
                values.profile = None;
            }
        });
        Field::new(words.depth.clone()).layout(FieldLayout::Above).id_salt("depth").show(ui, |ui, _| {
            let shown = Select::new(words.depth.clone(), Bind::given(values.depth), &depths).sizing(Sizing::FillWidth).show(ui);
            if let Some(depth) = shown.value().cloned() {
                values.depth = depth;
            }
        });
    });
    Field::new(words.background.clone()).layout(FieldLayout::Above).id_salt("background").show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let kind = BackgroundKind::of(values.background);
            let shown = Select::new(words.background.clone(), Bind::given(kind), &backgrounds).sizing(Sizing::FillWidth).show(ui);
            match shown.value().cloned() {
                Some(BackgroundKind::White) => values.background = BackgroundContents::White,
                Some(BackgroundKind::Black) => values.background = BackgroundContents::Black,
                Some(BackgroundKind::BackgroundColor) => values.background = BackgroundContents::BackgroundColor,
                Some(BackgroundKind::Transparent) => values.background = BackgroundContents::Transparent,
                Some(BackgroundKind::Custom) => ask_color = true,
                None => {}
            }
            // A transparent background shows as a clear well over the checkerboard.
            let color = background_rgb(values.background, setup.background).map_or(Srgba::new(0.0, 0.0, 0.0, 0.0), Srgba::from_rgb_array);
            // A click asks for a colour again, also while the choice is Custom.
            if Swatch::new(words.custom_swatch.clone(), color).id_salt("choose").show(ui).clicked {
                ask_color = true;
            }
        });
    });
    Field::new(words.profile.clone()).layout(FieldLayout::Above).id_salt("profile").show(ui, |ui, _| {
        let current = values.profile.clone().filter(|profile| others.contains(&profile));
        let shown = Select::new(words.profile.clone(), Bind::given(current), &profiles).sizing(Sizing::FillWidth).show(ui);
        if let Some(profile) = shown.value().cloned() {
            values.profile = profile;
        }
    });
    Field::new(words.pixel_aspect.clone()).layout(FieldLayout::Above).id_salt("pixel-aspect").show(ui, |ui, _| {
        let listed = pixel_aspects.iter().any(|entry| entry.value() == Some(&values.pixel_aspect));
        let current = if listed { values.pixel_aspect.clone() } else { SQUARE_PIXELS.to_owned() };
        let shown = Select::new(words.pixel_aspect.clone(), Bind::given(current), &pixel_aspects).sizing(Sizing::FillWidth).show(ui);
        if let Some(id) = shown.value().cloned() {
            values.pixel_aspect = id;
        }
    });

    if ask_color {
        let current = background_rgb(values.background, setup.background).unwrap_or([1.0; 3]);
        cx.request_color(ColorRequest::new(BACKGROUND_COLOR_KEY, Srgba::from_rgb_array(current)).title(words.custom_swatch.clone()));
    }

    // An edit of anything but the name and the display units no longer matches the card the settings came from; the
    // orientation control turns the card with it.
    let mut same = values.clone();
    same.name.clone_from(&before.name);
    same.unit = before.unit;
    same.resolution_unit = before.resolution_unit;
    if same != before && !keep_selection {
        cx.drop_card();
    }
}

/// Which side of the page a size row sets.
#[derive(Clone, Copy)]
enum Side {
    Width,
    Height,
}

impl Side {
    fn name(self, words: &NewDocumentWords) -> &Label {
        match self {
            Side::Width => &words.width,
            Side::Height => &words.height,
        }
    }

    fn unit_name(self, words: &NewDocumentWords) -> &Label {
        match self {
            Side::Width => &words.width_unit,
            Side::Height => &words.height_unit,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Side::Width => "width",
            Side::Height => "height",
        }
    }
}

/// Why `name` cannot be saved as a preset.
pub(super) enum Refusal {
    /// The library refuses the name.
    Name(PresetError),
    /// The library holds the most presets a user can save.
    Full,
}

/// Whether `name` can be saved beside the presets named `names`: the library's name check, then its size limit.
pub(super) fn refusal(names: &PresetNames, name: &str) -> Option<Refusal> {
    names.name_error(name).map(Refusal::Name).or_else(|| (names.len() >= MAX_SAVED).then_some(Refusal::Full))
}

/// The texts that say why a preset cannot be saved.
#[derive(MemoProps)]
struct RefusalWords<'w> {
    taken: &'w Label,
    invalid: &'w Label,
    full: &'w Label,
}

impl RefusalWords<'_> {
    fn message(&self, refusal: &Refusal) -> Label {
        match refusal {
            Refusal::Name(PresetError::NameTaken) => self.taken.clone(),
            Refusal::Name(_) => self.invalid.clone(),
            Refusal::Full => self.full.clone(),
        }
    }
}

/// The first `Saved N` the saved presets leave free: the name the naming pane proposes.
fn proposed_name<S: DocumentSettings>(read: &Read<'_, S>) -> String {
    (1..)
        .map(|n: usize| read.words.saved_name.fill(&[("n", n.to_string().as_str())]))
        .find(|name| read.library.name_error(name).is_none())
        .unwrap_or_default()
}

/// The name of a built-in pixel aspect ratio; an unknown id reads as square pixels.
fn pixel_aspect_label<'w>(words: &'w NewDocumentWords, id: &str) -> &'w Label {
    match id {
        "d1DvNtsc" => &words.d1_dv_ntsc,
        "d1DvPal" => &words.d1_dv_pal,
        "d1DvNtscWidescreen" => &words.d1_dv_ntsc_widescreen,
        "hdv1080" => &words.hdv_1080,
        "d1DvPalWidescreen" => &words.d1_dv_pal_widescreen,
        "anamorphic2To1" => &words.anamorphic_2_to_1,
        "dvcproHd1080" => &words.dvcpro_hd_1080,
        _ => &words.square_pixels,
    }
}

/// The Save Document Preset pane in place of the details: its header, the name (the proposed one selected, so typing
/// replaces it) and Save Preset and Cancel. Enter in the name saves the preset and never creates the document; Cancel
/// goes back to the details.
fn save_pane<S: DocumentSettings>(ui: &mut Ui, session: &mut Session<S>, read: &Read<'_, S>, right: &mut Right) {
    let words = read.words;
    let cancel = Env::of(ui).words::<CommonWords>().cancel.clone();
    let Some(name) = session.naming.clone() else { return };
    let tried = session.name_tried;
    let refusals = RefusalWords { taken: &read.library_words.name_taken, invalid: &read.library_words.name_invalid, full: &read.words.too_many };
    let inputs = (read.library.names(), tried, refusals);
    Text::new(words.save_preset_title.clone()).role(TypeRole::Label).shade(Shade::Muted).show(ui);
    Field::new(words.preset_name.clone()).id_salt("preset-name").show(ui, |ui, _| {
        let shown = TextField::new(
            words.preset_name.clone(),
            Bind::given(name.clone()).check_with(&inputs, |(names, tried, refusals), typed| match refusal(names, typed) {
                Some(refused) if *tried => Validity::Error(Message::new(refusals.message(&refused))),
                _ => Validity::Ok,
            }),
        )
        .limit(MAX_NAME)
        .select_on_focus()
        .submits()
        .show(ui);
        if let Some(typed) = shown.value().cloned() {
            session.naming = Some(typed);
            session.name_tried = false;
        }
        if shown.submitted {
            right.save = true;
        }
        if session.focus_preset_name {
            focus::request(ui.ctx(), FocusTarget::Id(shown.response.id));
        }
    });
    session.focus_preset_name = false;
    Row::new().show(ui, |ui, _| {
        if clicked(&Button::new(words.save_preset.clone()).id_salt("save").show(ui)) {
            right.save = true;
        }
        if clicked(&Button::new(cancel.clone()).id_salt("cancel-save").show(ui)) {
            session.naming = None;
            session.name_tried = false;
        }
    });
}
