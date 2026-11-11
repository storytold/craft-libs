//! `PdfExportDialog`: File > Save as PDF over [`craft_pdf::settings`].

mod sections;
mod summary;
mod words;

#[cfg(feature = "testing")]
mod stories;

use self::sections::{ArtboardChoice, PasswordSwitches, Sections};
pub use self::summary::RangeError;
pub(crate) use self::summary::{check_range, differences};
pub use self::words::PdfExportWords;
use super::form::FormDialog;
use crate::contract::{
    ActivateOn, Bind, CollectionEvent, Entry, NoValue, Outcome, Props as _, Remember as _, Scroll, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, Sizing,
};
use crate::form::{Field, FieldGroup};
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow};
use crate::notices::{Notice, NoticeLayout};
use crate::patterns::presets::{NamedPresetEvent, NamedPresetRow, PickerField, PresetError, PresetPicker, PresetRow, PresetState};
use crate::pickers::Select;
use crate::runtime::env::{Env, Label, UnitKind};
use crate::runtime::overlay::CloseReason;
use craft_behavior::{DisplayOrder, ItemSource, Selection, SelectionChange};
use craft_pdf::settings::{ColorMode, Compatibility, PdfPreset, PdfSettings, ProfileCatalog, Standard, builtin_presets, default_preset_name};
use craft_theme::variant::Tone;
use egui::{Response, Ui};
use std::borrow::Cow;

/// Which artboards to write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PdfArtboards {
    /// Every artboard, one page each.
    #[default]
    All,
    /// A 1-based range such as `"1-3, 5"`, which the app reads.
    Range(String),
}

/// What Save PDF answers with at OK: the settings and the artboards. The app asks for the file.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PdfExportResult {
    /// The settings, with the passwords typed (a preset never stores them).
    pub options: PdfSettings,
    /// Which artboards to include.
    pub artboards: PdfArtboards,
}

/// What the user did to the app's presets in this pass; the app carries it out and passes the presets back.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PdfPresetEvent {
    /// The current settings were saved under a new name; the passwords are cleared.
    Saved(Box<PdfPreset>),
    /// A saved preset was renamed.
    #[non_exhaustive]
    Renamed {
        /// The old name.
        from: String,
        /// The new name.
        to: String,
    },
    /// A saved preset was deleted.
    Deleted(String),
}

/// The sections of the settings page, in the order of the section list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PdfSection {
    /// Options and artboards.
    #[default]
    General,
    /// Image resampling and compression.
    Compression,
    /// Printer's marks and bleeds.
    Marks,
    /// Colour conversion and the output intent.
    Output,
    /// Fonts, overprint and the transparency flattener.
    Advanced,
    /// Passwords and permissions.
    Security,
    /// The standard, what differs from the preset, and the warnings.
    Summary,
}

impl PdfSection {
    /// Every section, in the order of the list.
    pub const ALL: [PdfSection; 7] = [
        PdfSection::General,
        PdfSection::Compression,
        PdfSection::Marks,
        PdfSection::Output,
        PdfSection::Advanced,
        PdfSection::Security,
        PdfSection::Summary,
    ];

    /// The section's name.
    pub fn title(self, words: &PdfExportWords) -> &Label {
        match self {
            PdfSection::General => &words.section_general,
            PdfSection::Compression => &words.section_compression,
            PdfSection::Marks => &words.section_marks,
            PdfSection::Output => &words.section_output,
            PdfSection::Advanced => &words.section_advanced,
            PdfSection::Security => &words.section_security,
            PdfSection::Summary => &words.section_summary,
        }
    }
}

/// A standard sets the version it allows (unless the chosen one is allowed) and turns off what its files can't have:
/// editing data, passwords and, for PDF/X-1a and PDF/X-3, PDF layers.
pub(crate) fn apply_standard(settings: &mut PdfSettings) {
    let standard = settings.standard;
    if !standard.allows(settings.compatibility) {
        settings.compatibility = standard.version();
    }
    if standard != Standard::None {
        settings.preserve_editing = false;
        settings.clear_passwords();
    }
    if !standard.allows_layers() {
        settings.create_layers = false;
    }
}

/// What the dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PdfExportOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer, the pass the dialog closed as OK.
    pub result: Option<PdfExportResult>,
    /// The settings after the pass when they differ from the binding's, for a binding that does not write through.
    pub settings: Option<PdfSettings>,
    /// What the user did to the app's presets, in order.
    pub presets: Vec<PdfPresetEvent>,
}

impl Outcome for PdfExportOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the dialog keeps while it is open besides the settings.
#[derive(Clone, Default)]
struct Live {
    section: PdfSection,
    artboards: ArtboardChoice,
    switches: PasswordSwitches,
    /// The preset the settings started from or were last filled from.
    preset: Option<String>,
    picker: PresetState<String>,
    /// The artboard choice and preset when the dialog opened, which Reset restores; `None` before the first pass.
    start: Option<(ArtboardChoice, Option<String>)>,
}

impl Live {
    /// Puts back what the dialog opened with: the artboard choice, the preset and the password switches.
    fn restore(&mut self) {
        if let Some((artboards, preset)) = self.start.clone() {
            self.artboards = artboards;
            self.preset = preset;
        }
        self.switches = PasswordSwitches::default();
    }
}

/// The section list's rows.
struct Pages<'a>(&'a PdfExportWords);

impl DisplayOrder<PdfSection> for Pages<'_> {
    fn len(&self) -> usize {
        PdfSection::ALL.len()
    }

    fn key(&self, at: usize) -> PdfSection {
        PdfSection::ALL.get(at).copied().unwrap_or_default()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        PdfSection::ALL.get(at).map(|section| Cow::Borrowed(section.title(self.0).as_str()))
    }
}

impl ItemSource<PdfSection> for Pages<'_> {}

/// File > Save as PDF: a preset row, Standard and Compatibility, and a list of sections (General, Compression, Marks
/// and Bleeds, Output, Advanced, Security and Summary) beside the shown section.
///
/// - **Presets.** The row lists the built-in presets and the app's `saved_presets`. Choosing one fills every row; a
///   change to any value marks the preset as modified. New, Rename and Delete come back as [`PdfPresetEvent`]s, which
///   the app stores and hands back with `saved_presets`.
/// - **Standard.** A standard sets the PDF version it allows and turns off what its files can't have (editing data,
///   passwords, PDF layers in PDF/X-1a and PDF/X-3).
/// - **Checks.** The checks of [`craft_pdf::settings`] run every pass; while one fails its message shows under the
///   sections and OK is disabled. A typed artboard range outside `1..=artboard_count` shows its message beside the
///   range field and disables OK.
/// - **Keys.** Tab walks the rows, Enter is OK, Escape cancels, and holding Alt turns Cancel into Reset, which
///   restores the settings from when the dialog opened and keeps it open.
/// - **Result.** OK answers with [`PdfExportResult`]; the app asks for the file to write.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct PdfExportDialog<'a> {
    app: String,
    settings: Bind<'a, PdfSettings>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The preset the settings start from, by name; the app's default preset without it.
    #[prop(into)]
    preset: Option<String>,
    /// The presets the user saved, after the built-in ones in the preset list.
    #[prop]
    saved_presets: &'a [PdfPreset],
    /// The names of the saved flattener presets, after the three built-in ones.
    #[prop]
    flattener_presets: &'a [String],
    /// The colour profiles the app knows: Destination and Output intent profile list them.
    #[prop]
    profiles: &'a [String],
    /// The lookup the checks use for the profiles; without one every profile name is unknown.
    #[prop]
    catalog: Option<&'a dyn ProfileCatalog>,
    /// The colour mode of the document, which a blank destination profile follows.
    #[prop]
    color_mode: ColorMode,
    /// The unit lengths (offset, bleed) are shown in; kinds that are not lengths show points.
    #[prop]
    length_unit: UnitKind,
    /// The document's bleed in points (top, bottom, left, right), which "Use document bleed settings" shows.
    #[prop]
    document_bleed: [f64; 4],
    /// The artboards chosen when the dialog opens.
    #[prop]
    artboards: PdfArtboards,
    /// The number of artboards of the document, which a typed range is checked against.
    #[prop]
    artboard_count: Option<usize>,
}

impl<'a> PdfExportDialog<'a> {
    /// The dialog for the application called `app`, which names the default preset (`"{app} Default"`), editing
    /// `settings`.
    pub fn new(app: impl Into<String>, settings: Bind<'a, PdfSettings>) -> Self {
        PdfExportDialog {
            app: app.into(),
            settings,
            open: None,
            preset: None,
            saved_presets: &[],
            flattener_presets: &[],
            profiles: &[],
            catalog: None,
            color_mode: ColorMode::Rgb,
            length_unit: UnitKind::Points,
            document_bleed: [0.0; 4],
            artboards: PdfArtboards::All,
            artboard_count: None,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> PdfExportOutput {
        let PdfExportDialog {
            app,
            settings: bound,
            open,
            preset,
            saved_presets,
            flattener_presets,
            profiles,
            catalog,
            color_mode,
            length_unit,
            document_bleed,
            artboards,
            artboard_count,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<PdfExportWords>();
        let units = env.units();
        let catalog: &dyn ProfileCatalog = catalog.unwrap_or(&());
        let mut presets = builtin_presets(&app);
        let built_in: Vec<String> = presets.iter().map(|preset| preset.name.clone()).collect();
        presets.extend(saved_presets.iter().cloned());

        let mut form = FormDialog::new(words.title.clone(), bound)
            .ok_label(words.save.clone())
            .collection_body(true)
            .outer_gesture(true)
            .on_reset(|cx| cx.memory::<Live>().restore())
            .shaped(|dialog| dialog.resizable(true));
        if let Some(open) = open {
            form = form.opened(open);
        }

        let mut events: Vec<PdfPresetEvent> = Vec::new();
        let mut chosen = PdfArtboards::All;
        let shown = form.show(ui, |ui, cx, work| {
            events.clear();
            let mut live = cx.memory::<Live>().clone();
            if cx.first() {
                live.start = Some((
                    match &artboards {
                        PdfArtboards::All => ArtboardChoice::default(),
                        PdfArtboards::Range(range) => ArtboardChoice { range_only: true, range: range.clone() },
                    },
                    Some(preset.clone().unwrap_or_else(|| default_preset_name(&app))),
                ));
                live.restore();
            }

            // The preset row.
            let rows: Vec<PresetRow<String>> = presets
                .iter()
                .map(|preset| PresetRow::item(preset.name.clone(), words.preset_name(&preset.name, &app).as_str().to_owned(), 0))
                .collect();
            let selected = live.preset.as_ref().and_then(|name| presets.iter().find(|preset| preset.name == *name));
            let modified = selected.is_some_and(|preset| {
                let mut shown = work.clone();
                shown.clear_passwords();
                let mut base = preset.settings.clone();
                base.clear_passwords();
                shown != base
            });
            let held = selected.filter(|_| !modified).map(|preset| preset.name.clone());
            let base = selected.map(|preset| preset.settings.clone()).unwrap_or_default();
            let preset_name = selected.map_or_else(Label::default, |preset| words.preset_name(&preset.name, &app));
            let mut picker =
                PresetPicker::new(words.preset.clone(), Bind::given(held), &rows, Bind::bound(&mut live.picker)).field(PickerField::Name);
            if modified {
                picker = picker.placeholder(Label::from(words.preset_modified.fill(&[("name", preset_name.as_str())])));
            }
            let named = NamedPresetRow::new(words.preset.clone(), picker)
                .suggest_name(words.new_preset_name.as_str().to_owned())
                .check_name(|typed| presets.iter().any(|preset| preset.name.eq_ignore_ascii_case(typed)).then_some(PresetError::NameTaken))
                .built_in(|key| built_in.contains(key))
                .show(ui);
            for event in named.events {
                match event {
                    NamedPresetEvent::Apply(key) => {
                        if let Some(preset) = presets.iter().find(|preset| preset.name == key) {
                            *work = preset.settings.clone();
                            live.switches = PasswordSwitches::default();
                            live.preset = Some(key);
                        }
                    }
                    NamedPresetEvent::Create { name: created, .. } => {
                        let mut settings = work.clone();
                        settings.clear_passwords();
                        events.push(PdfPresetEvent::Saved(Box::new(PdfPreset { name: created.clone(), description: String::new(), settings })));
                        live.preset = Some(created);
                    }
                    NamedPresetEvent::Rename { key, text, .. } => {
                        if live.preset.as_ref() == Some(&key) {
                            live.preset = Some(text.clone());
                        }
                        events.push(PdfPresetEvent::Renamed { from: key, to: text });
                    }
                    NamedPresetEvent::Delete(key) => {
                        if live.preset.as_ref() == Some(&key) {
                            live.preset = None;
                        }
                        events.push(PdfPresetEvent::Deleted(key));
                    }
                    _ => {}
                }
            }

            // Standard and Compatibility.
            let mut standard_picked = false;
            FieldGroup::new(words.standard.clone()).show(ui, |ui, _| {
                let standards: Vec<Entry<Standard>> = <Standard as craft_pdf::settings::Choice>::ALL
                    .iter()
                    .map(|standard| Entry::new(*standard, words.choice("standard", standard.id(), standard.label())))
                    .collect();
                Field::new(words.standard.clone()).id_salt("standard").show(ui, |ui, _| {
                    let chosen = Select::new(words.standard.clone(), Bind::given(work.standard), &standards).show(ui);
                    if let Some(next) = chosen.value().copied() {
                        work.standard = next;
                        standard_picked = true;
                    }
                });
                let allowed = work.standard;
                let versions: Vec<Entry<Compatibility>> = <Compatibility as craft_pdf::settings::Choice>::ALL
                    .iter()
                    .map(|version| Entry::new(*version, Label::from(version.label())).enabled(allowed.allows(*version)))
                    .collect();
                Field::new(words.compatibility.clone()).id_salt("compatibility").show(ui, |ui, _| {
                    let chosen = Select::new(words.compatibility.clone(), Bind::given(work.compatibility), &versions).show(ui);
                    if let Some(next) = chosen.value().copied() {
                        work.compatibility = next;
                    }
                });
            });
            if standard_picked {
                apply_standard(work);
                if work.standard != Standard::None {
                    live.switches = PasswordSwitches::default();
                }
            }

            // What the sections show of the checks.
            let range_error = |choice: &ArtboardChoice| -> Option<String> {
                let text = choice.range.trim();
                let count = artboard_count?;
                if !choice.range_only || text.is_empty() {
                    return None;
                }
                check_range(text, count).err().map(|error| words.range_error_text(text, count, &error))
            };
            let settings_error =
                |settings: &PdfSettings| settings.check(catalog).and_then(|()| settings.check_destination(catalog, color_mode)).err();
            let warnings: Vec<String> = work.warnings(catalog).iter().map(|warning| words.warning_text(warning)).collect();
            let changed = differences(&words, work, &base);
            let range_text = range_error(&live.artboards);

            // The section list and the shown section.
            let mut choice = live.artboards.clone();
            let mut switches = live.switches;
            let mut section = live.section;
            Row::new().fill().show(ui, |ui, _| {
                let pages = Pages(&words);
                let listed = List::<PdfSection>::new(words.sections.clone())
                    .select(SelectMode::Single)
                    .activate(ActivateOn::Click)
                    .allow_empty(false)
                    .header(false)
                    .sizing(Sizing::Content)
                    .selection(Bind::given(Selection::single(section)))
                    .show_items(ui, &pages, &mut |key, _| ListRow::new(key.title(&words).clone()));
                for event in listed.events {
                    if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                        && let Some(key) = keys.into_iter().last()
                    {
                        section = key;
                    }
                }
                Stack::new().fill().scroll(Scroll::vertical()).label(section.title(&words).clone()).show(ui, |ui, _| {
                    Sections {
                        words: &words,
                        units: &units,
                        length: length_unit,
                        profiles,
                        flatteners: flattener_presets,
                        document_bleed,
                        work: &mut *work,
                        artboards: &mut choice,
                        switches: &mut switches,
                        range_error: range_text.as_deref(),
                        warnings: &warnings,
                        changed: &changed,
                        preset_name: &preset_name,
                    }
                    .show(ui, section);
                });
            });
            live.section = section;
            live.artboards = choice;
            live.switches = switches;

            // The failing check, under the sections.
            let error = settings_error(work);
            if let Some(error) = &error {
                Notice::new(Tone::Bad, Label::from(words.error_text(error))).layout(NoticeLayout::Inline).show(ui);
            }
            let blocked = error.is_some() || range_error(&live.artboards).is_some();
            chosen = {
                let range = live.artboards.range.trim();
                if live.artboards.range_only && !range.is_empty() { PdfArtboards::Range(range.to_owned()) } else { PdfArtboards::All }
            };
            *cx.memory::<Live>() = live;
            !blocked
        });

        let result = shown.accepted.map(|options| PdfExportResult { options, artboards: chosen });
        PdfExportOutput { response: shown.response, closed: shown.closed, result, settings: shown.value, presets: events }
    }
}
