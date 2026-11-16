//! The details column of New Document: name, size, orientation, resolution, colour settings, and the Save Document Preset
//! pane that stands in for them while a preset is named.

use super::presets::{
    BackgroundContents, ColorMode, DocumentOptions, MAX_NAME, MAX_SAVED, Orientation, PIXEL_ASPECTS, RESOLUTION_UNITS, SIZE_UNITS, SQUARE_PIXELS,
};
use super::{NewDocumentWords, Read, Session, background_rgb, depth_label, mode_label};
use crate::buttons::{Button, IconButton};
use crate::choice::{Checkbox, Segmented, Swatch};
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Validity, clicked};
use crate::display::{Shade, Text};
use crate::form::{Field, FieldGroup};
use crate::layout_components::Row;
use crate::patterns::form_dialogs::LinkedSize;
use crate::patterns::presets::{PresetError, PresetLibrary};
use crate::pickers::{Select, UnitSelect};
use crate::runtime::env::{Env, Label, Message, UnitKind};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::words::CommonWords;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::{EditEvent, EditEvents, IconId, PixelSize};
use craft_theme::Srgba;
use egui::Ui;

/// What the details column asks the dialog to do after the pass.
#[derive(Default)]
pub(super) struct Right {
    /// Custom was chosen or the colour button pressed: ask the app for a colour.
    pub(super) request_color: bool,
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
fn apply_number(session: &mut Session, events: &EditEvents<Option<f64>>, set: impl Fn(&mut LinkedSize, f64)) {
    for event in events {
        match event {
            EditEvent::Begin { .. } => session.gesture = Some(session.size.clone()),
            EditEvent::Cancel { .. } => {
                if let Some(before) = session.gesture.take() {
                    session.size = before;
                }
            }
            _ => {
                if session.gesture.is_none() && matches!(event, EditEvent::Update { .. }) {
                    session.gesture = Some(session.size.clone());
                }
                if let Some(Some(value)) = event.value() {
                    set(&mut session.size, *value);
                }
                if matches!(event, EditEvent::Commit { .. } | EditEvent::Set { .. }) {
                    session.gesture = None;
                }
            }
        }
    }
}

/// The entries of a unit list, named by the units' symbols.
fn unit_entries(read: &Read<'_>, kinds: &[UnitKind]) -> Vec<Entry<UnitKind>> {
    kinds.iter().map(|kind| Entry::new(*kind, read.units.symbol(*kind))).collect()
}

/// Shows the column.
pub(super) fn show(ui: &mut Ui, session: &mut Session, values: &mut DocumentOptions, read: &Read<'_>, right: &mut Right) {
    if session.naming.is_some() {
        save_pane(ui, session, read, right);
        return;
    }
    let (words, setup) = (read.words, read.setup);
    let before = values.clone();
    let mut keep_selection = false;

    let size_units = unit_entries(read, &SIZE_UNITS);
    let resolution_units = unit_entries(read, &RESOLUTION_UNITS);
    let orientations = [Entry::new(Orientation::Portrait, words.portrait.clone()), Entry::new(Orientation::Landscape, words.landscape.clone())];
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

    Text::new(words.preset_details.clone()).role(TypeRole::Label).shade(Shade::Muted).show(ui);
    FieldGroup::new(words.preset_details.clone()).show(ui, |ui, _| {
        Field::new(words.name.clone()).id_salt("name").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let shown = TextField::new(words.name.clone(), Bind::given(values.name.clone())).limit(MAX_NAME).select_on_focus().show(ui);
                if let Some(name) = shown.value().cloned() {
                    values.name = name;
                }
                if std::mem::take(&mut session.focus_name) {
                    focus::request(ui.ctx(), FocusTarget::Id(shown.response.id));
                }
                let save = IconButton::new(IconId::new("save"))
                    .label(words.save_preset_title.clone())
                    .tooltip(words.save_preset_title.clone())
                    .id_salt("save-preset")
                    .show(ui);
                if clicked(&save) {
                    session.naming = Some(proposed_name(read));
                    session.name_tried = false;
                    session.focus_preset_name = true;
                }
            });
        });

        for side in [Side::Width, Side::Height] {
            Field::new(side.name(words).clone()).id_salt(side.key()).show(ui, |ui, _| {
                let (value, range) = match side {
                    Side::Width => (session.size.width(), session.size.width_range()),
                    Side::Height => (session.size.height(), session.size.height_range()),
                };
                let shown = UnitSelect::new(side.name(words).clone(), Bind::given(Some(value)), Bind::given(session.size.unit()), &size_units)
                    .range(range)
                    .unit_label(side.unit_name(words).clone())
                    .show(ui);
                apply_number(session, &shown.edit.events, |size, value| match side {
                    Side::Width => size.set_width(value),
                    Side::Height => size.set_height(value),
                });
                if let Some(kind) = shown.unit.value().copied() {
                    session.size.set_unit(kind);
                }
            });
        }

        Field::new(words.orientation.clone()).id_salt("orientation").show(ui, |ui, _| {
            let current = Orientation::of(session.size.pixels());
            let shown = Segmented::new(words.orientation.clone(), Bind::given(current), &orientations).show(ui);
            if let Some(turned) = shown.value().cloned() {
                session.orientation = Some(turned);
                if current != turned {
                    values.take_linked(&session.size);
                    let PixelSize { width, height, .. } = values.size;
                    values.size = PixelSize::new(height, width);
                    session.size = values.linked(read.units.points_per_inch());
                }
                keep_selection = true;
            }
        });

        let shown = Checkbox::new(Bind::given(values.artboard)).label(words.artboards.clone()).show(ui);
        if let Some(on) = shown.value().cloned() {
            values.artboard = on;
            session.preset_artboard = false;
        }

        Field::new(words.resolution.clone()).id_salt("resolution").show(ui, |ui, _| {
            let shown = UnitSelect::new(
                words.resolution.clone(),
                Bind::given(Some(session.size.resolution())),
                Bind::given(session.size.resolution_unit()),
                &resolution_units,
            )
            .range(session.size.resolution_range())
            .unit_label(words.resolution_unit.clone())
            .show(ui);
            apply_number(session, &shown.edit.events, LinkedSize::set_resolution);
            if let Some(kind) = shown.unit.value().copied() {
                session.size.set_resolution_unit(kind);
            }
        });
        values.take_linked(&session.size);

        Field::new(words.color_mode.clone()).id_salt("mode").show(ui, |ui, _| {
            let shown = Select::new(words.color_mode.clone(), Bind::given(values.color_mode), &modes).show(ui);
            if let Some(mode) = shown.value().cloned() {
                values.color_mode = mode;
                // A profile of the old mode's colour space does not suit the new one.
                values.profile = None;
            }
        });
        Field::new(words.depth.clone()).id_salt("depth").show(ui, |ui, _| {
            let shown = Select::new(words.depth.clone(), Bind::given(values.depth), &depths).show(ui);
            if let Some(depth) = shown.value().cloned() {
                values.depth = depth;
            }
        });
        Field::new(words.profile.clone()).id_salt("profile").show(ui, |ui, _| {
            let current = values.profile.clone().filter(|profile| others.contains(&profile));
            let shown = Select::new(words.profile.clone(), Bind::given(current), &profiles).show(ui);
            if let Some(profile) = shown.value().cloned() {
                values.profile = profile;
            }
        });
        Field::new(words.pixel_aspect.clone()).id_salt("pixel-aspect").show(ui, |ui, _| {
            let listed = pixel_aspects.iter().any(|entry| entry.value() == Some(&values.pixel_aspect));
            let current = if listed { values.pixel_aspect.clone() } else { SQUARE_PIXELS.to_owned() };
            let shown = Select::new(words.pixel_aspect.clone(), Bind::given(current), &pixel_aspects).show(ui);
            if let Some(id) = shown.value().cloned() {
                values.pixel_aspect = id;
            }
        });

        Field::new(words.background.clone()).id_salt("background").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let kind = BackgroundKind::of(values.background);
                let shown = Select::new(words.background.clone(), Bind::given(kind), &backgrounds).show(ui);
                match shown.value().cloned() {
                    Some(BackgroundKind::White) => values.background = BackgroundContents::White,
                    Some(BackgroundKind::Black) => values.background = BackgroundContents::Black,
                    Some(BackgroundKind::BackgroundColor) => values.background = BackgroundContents::BackgroundColor,
                    Some(BackgroundKind::Transparent) => values.background = BackgroundContents::Transparent,
                    Some(BackgroundKind::Custom) => right.request_color = true,
                    None => {}
                }
                // A transparent background shows as a clear well over the checkerboard.
                let color = background_rgb(values.background, setup.background).map_or(Srgba::new(0.0, 0.0, 0.0, 0.0), Srgba::from_rgb_array);
                // A click asks for a colour again, also while the choice is Custom.
                if Swatch::new(words.custom_swatch.clone(), color).id_salt("choose").show(ui).clicked {
                    right.request_color = true;
                }
            });
        });
    });

    let fitted = values.clone().fitted();
    if fitted != *values {
        *values = fitted;
    }

    let mut same = values.clone();
    same.name.clone_from(&before.name);
    same.unit = before.unit;
    same.resolution_unit = before.resolution_unit;
    if same != before && !keep_selection {
        session.card = None;
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

/// Whether `name` can be saved beside the presets of `library`: the library's name check, then its size limit.
pub(super) fn refusal(library: &PresetLibrary<DocumentOptions>, name: &str) -> Option<Refusal> {
    library.name_error(name).map(Refusal::Name).or_else(|| (library.total() >= MAX_SAVED).then_some(Refusal::Full))
}

/// The text that says why a preset cannot be saved.
fn refusal_message(read: &Read<'_>, refusal: &Refusal) -> Label {
    match refusal {
        Refusal::Name(PresetError::NameTaken) => read.library_words.name_taken.clone(),
        Refusal::Name(_) => read.library_words.name_invalid.clone(),
        Refusal::Full => read.words.too_many.clone(),
    }
}

/// The first `Saved N` the saved presets leave free: the name the naming pane proposes.
fn proposed_name(read: &Read<'_>) -> String {
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
fn save_pane(ui: &mut Ui, session: &mut Session, read: &Read<'_>, right: &mut Right) {
    let words = read.words;
    let cancel = Env::of(ui).words::<CommonWords>().cancel.clone();
    let Some(name) = session.naming.clone() else { return };
    let tried = session.name_tried;
    let library = read.library;
    let check = |typed: &String| match refusal(library, typed) {
        Some(refused) if tried => Validity::Error(Message::new(refusal_message(read, &refused))),
        _ => Validity::Ok,
    };
    Text::new(words.save_preset_title.clone()).role(TypeRole::Label).shade(Shade::Muted).show(ui);
    Field::new(words.preset_name.clone()).id_salt("preset-name").show(ui, |ui, _| {
        let shown =
            TextField::new(words.preset_name.clone(), Bind::given(name.clone()).check(check)).limit(MAX_NAME).select_on_focus().submits().show(ui);
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
