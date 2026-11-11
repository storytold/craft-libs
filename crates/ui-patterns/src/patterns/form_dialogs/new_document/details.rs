//! The details column of New Document: name, size, orientation, resolution, colour settings and the Save Preset row.

use super::presets::{BackgroundContents, BitDepth, ColorMode, DocumentOptions, MAX_NAME, MAX_SAVED, Orientation, RESOLUTION_UNITS, SIZE_UNITS};
use super::{NewDocumentWords, Read, Session, background_rgb, depth_label, mode_label};
use crate::buttons::Button;
use crate::choice::{Checkbox, Segmented};
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
use craft_behavior::{EditEvent, EditEvents, PixelSize};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
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
    let (words, setup) = (read.words, read.setup);
    let before = values.clone();
    let mut keep_selection = false;

    let size_units = unit_entries(read, &SIZE_UNITS);
    let resolution_units = unit_entries(read, &RESOLUTION_UNITS);
    let orientations = [Entry::new(Orientation::Portrait, words.portrait.clone()), Entry::new(Orientation::Landscape, words.landscape.clone())];
    let modes = [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab].map(|mode| Entry::new(mode, mode_label(words, mode).clone()));
    let depths = [BitDepth::Eight, BitDepth::Sixteen, BitDepth::ThirtyTwo].map(|depth| Entry::new(depth, depth_label(words, depth).clone()));
    let working = setup.profiles.iter().find(|profiles| profiles.mode == values.color_mode);
    let working_label =
        working.map_or_else(|| words.working.clone(), |profiles| Label::from(words.working_profile.fill(&[("name", profiles.working.as_str())])));
    let others: Vec<&String> = working.map_or_else(Vec::new, |profiles| profiles.others.iter().collect());
    let mut profiles = vec![Entry::new(None, working_label)];
    profiles.extend(others.iter().map(|name| Entry::new(Some((*name).clone()), (*name).clone())));
    let backgrounds = [
        Entry::new(BackgroundKind::White, words.white.clone()),
        Entry::new(BackgroundKind::Black, words.black.clone()),
        Entry::new(BackgroundKind::BackgroundColor, words.background_color.clone()),
        Entry::new(BackgroundKind::Transparent, words.transparent.clone()),
        Entry::new(BackgroundKind::Custom, words.custom_color.clone()),
    ];

    Text::new(words.preset_details.clone()).role(TypeRole::Label).shade(Shade::Muted).show(ui);
    FieldGroup::new(words.preset_details.clone()).show(ui, |ui, _| {
        Field::new(words.name.clone()).id_salt("name").show(ui, |ui, _| {
            let shown = TextField::new(words.name.clone(), Bind::given(values.name.clone())).limit(MAX_NAME).select_on_focus().show(ui);
            if let Some(name) = shown.value().cloned() {
                values.name = name;
            }
            if std::mem::take(&mut session.focus_name) {
                focus::request(ui.ctx(), FocusTarget::Id(shown.response.id));
            }
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
                let swatch = match background_rgb(values.background, setup.background) {
                    Some(rgb) => Label::from(Srgba::from_rgb_array(rgb).to_hex()),
                    None => words.transparent.clone(),
                };
                let button = Button::new(swatch).id_salt("choose").tooltip(words.custom_swatch.clone()).show(ui);
                if clicked(&button) {
                    right.request_color = true;
                }
            });
        });
    });

    save_row(ui, session, read, right);

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

/// The Save Preset button, or the name row it opens.
fn save_row(ui: &mut Ui, session: &mut Session, read: &Read<'_>, right: &mut Right) {
    let words = read.words;
    let cancel = Env::of(ui).words::<CommonWords>().cancel.clone();
    if session.naming.is_none() && clicked(&Button::new(words.save_preset.clone()).id_salt("save-preset").show(ui)) {
        session.naming = Some(String::new());
        session.name_tried = false;
        session.focus_preset_name = true;
    }
    let Some(name) = session.naming.clone() else { return };
    let tried = session.name_tried;
    let library = read.library;
    let check = |typed: &String| match refusal(library, typed) {
        Some(refused) if tried => Validity::Warn(Message::new(refusal_message(read, &refused)).tone(Tone::Bad)),
        _ => Validity::Ok,
    };
    Field::new(words.preset_name.clone()).id_salt("preset-name").show(ui, |ui, _| {
        let shown = TextField::new(words.preset_name.clone(), Bind::given(name.clone()).check(check)).limit(MAX_NAME).submits().show(ui);
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
        if clicked(&Button::new(words.save.clone()).id_salt("save").show(ui)) {
            right.save = true;
        }
        if clicked(&Button::new(cancel.clone()).id_salt("cancel-save").show(ui)) {
            session.naming = None;
            session.name_tried = false;
        }
    });
}
