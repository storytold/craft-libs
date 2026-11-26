//! `PdfExportDialog`: File > Save as PDF over [`craft_pdf::settings`].

mod hosted;
mod sections;
mod summary;
mod words;

#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use self::hosted::{HostedPdfExport, PdfExportAnswer};
pub use self::sections::{ArtboardChoice, PasswordSwitches};
use self::sections::{SectionPresses, Sections};
pub(crate) use self::summary::check_range;
pub use self::summary::differences;
use self::summary::warning_lines;
pub use self::summary::{RangeError, summary_text};
pub use self::words::PdfExportWords;
use super::SectionList;
use super::dialog::{ResetFooter, ResetPick};
use super::form::{FormDialog, read_bind, write_bind};
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, Validity};
use crate::display::Text;
use crate::form::{Field, FieldGroup};
use crate::list::ListRow;
use crate::notices::{Notice, NoticeLayout};
use crate::patterns::presets::{
    NameCheck, NameRefuses, NamedPresetEvent, NamedPresetRow, Naming, PickerField, PresetError, PresetFns, PresetPicker, PresetRow, PresetSource,
    PresetState, PresetWords,
};
use crate::pickers::Select;
use crate::runtime::env::{Env, Label, Message, UnitKind};
use crate::runtime::overlay::CloseReason;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::{DisplayOrder, ItemSource, Value};
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
    /// One page of the art's bounds. General has no Artboards group.
    ArtBounds,
}

/// What the dialog is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PdfExportMode {
    /// Save a PDF: OK answers with the settings and the artboards.
    #[default]
    Save,
    /// Edit a preset: a name field with its refusal, a Description box in place of Artboards, no Custom…, the OK
    /// label "Save Preset", and a Summary that compares with the default preset.
    PresetEditor,
}

/// The name and description a preset editor edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PresetDraft {
    /// The preset's name.
    pub name: String,
    /// The preset's description.
    pub description: String,
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
    // PDF/X-3 and PDF/X-4 write the profiles their intent needs; the choice goes back to the default.
    if standard.is_pdfx() && !standard.cmyk_only() {
        settings.output.profiles = None;
    }
}

/// The value `bind` holds, when it is bound and holds one.
fn bound_value<T: Value>(bind: &mut Option<Bind<'_, T>>) -> Option<T> {
    bind.as_mut().and_then(read_bind)
}

/// Writes `value` through `bind` when it holds another.
fn keep_bound<T: Value>(bind: &mut Option<Bind<'_, T>>, value: T) {
    if let Some(bind) = bind.as_mut()
        && read_bind(bind).as_ref() != Some(&value)
    {
        write_bind(bind, value);
    }
}

/// The artboard choice a dialog opens with.
fn opening_choice(artboards: &PdfArtboards) -> ArtboardChoice {
    match artboards {
        PdfArtboards::Range(range) => ArtboardChoice { range_only: true, range: range.clone() },
        PdfArtboards::All | PdfArtboards::ArtBounds => ArtboardChoice::default(),
    }
}

/// Whether the four bleed sides are equal, as the link between them opens.
fn sides_equal(sides: [f64; 4]) -> bool {
    let [top, rest @ ..] = sides;
    rest.iter().all(|side| *side == top)
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
    /// Custom… beside the flattener select was pressed this pass.
    pub custom_flattener: bool,
    /// Save Summary… under the Summary groups was pressed this pass, with the summary text.
    pub save_summary: Option<String>,
    /// The preset editor's name and description, when they changed this pass.
    pub draft: Option<PresetDraft>,
    /// The answer OK filed this pass while `hold_ok` is on; the host accepts or refuses it.
    pub held: Option<PdfExportResult>,
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
    /// Whether the four bleed sides move together; `None` until the sides are read (when the dialog opens, a preset
    /// applies or Reset puts the settings back), which opens it on when they are equal.
    bleed_linked: Option<bool>,
}

impl Live {
    /// What Reset clears beside the settings: the password switches, which no setting holds, and the bleed link.
    fn reset(&mut self) {
        self.switches = PasswordSwitches::default();
        self.bleed_linked = None;
    }
}

/// What a pass of the body reports to the bound values: the shown section, the chosen preset, the artboard choice and
/// the password switches.
struct Kept {
    section: PdfSection,
    preset: Option<String>,
    artboards: ArtboardChoice,
    switches: PasswordSwitches,
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
///   puts back the chosen preset's settings (with no passwords) and keeps it open, on the same section and artboards.
/// - **Preset editor.** With [`PdfExportMode::PresetEditor`] a name field (checked by `check_name`) stands in place of
///   the preset row, Description in place of Artboards, and the Summary compares with the default preset; the name and
///   description go through `preset_draft`.
/// - **Held OK.** With `hold_ok`, OK files its answer in the output's `held` and the dialog stays open until the host
///   sets `finish_held` or a `refusal`.
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
    /// What the dialog is for.
    #[prop]
    mode: PdfExportMode,
    /// Binds the preset editor's name and description.
    #[prop]
    preset_draft: Option<Bind<'a, PresetDraft>>,
    /// Whether a typed preset name may be used, in the preset editor.
    #[prop(boxed = "Fn(&str) -> Option<PresetError> + 'a")]
    check_name: Option<Box<NameRefuses<'a>>>,
    /// Binds the shown section.
    #[prop]
    section: Option<Bind<'a, PdfSection>>,
    /// While on, the flattener select shows `[Custom]`.
    #[prop]
    custom_flattener: bool,
    /// Warnings of the host, shown after the settings' own.
    #[prop]
    host_warnings: &'a [String],
    /// Binds the preset the settings started from or were last filled from.
    #[prop]
    chosen_preset: Option<Bind<'a, Option<String>>>,
    /// Binds the artboard choice.
    #[prop]
    artboard_choice: Option<Bind<'a, ArtboardChoice>>,
    /// Binds the two check boxes that turn a password on.
    #[prop]
    password_switches: Option<Bind<'a, PasswordSwitches>>,
    /// OK files its answer in the output's `held` instead of closing the dialog; the host answers it.
    #[prop]
    hold_ok: bool,
    /// The host accepted the held answer: the dialog closes with it.
    #[prop]
    finish_held: bool,
    /// The host refused the held answer for the reason, which shows under the rows until the settings change.
    #[prop]
    refusal: Option<(Label, PdfSettings)>,
    /// Whether OK is enabled.
    #[prop]
    ok_enabled: bool,
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
            mode: PdfExportMode::default(),
            preset_draft: None,
            check_name: None,
            section: None,
            custom_flattener: false,
            host_warnings: &[],
            chosen_preset: None,
            artboard_choice: None,
            password_switches: None,
            hold_ok: false,
            finish_held: false,
            refusal: None,
            ok_enabled: true,
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
            mode,
            preset_draft: mut draft_bind,
            check_name,
            section: mut section_bind,
            custom_flattener,
            host_warnings,
            chosen_preset: mut preset_bind,
            artboard_choice: mut artboards_bind,
            password_switches: mut switches_bind,
            hold_ok,
            finish_held,
            refusal,
            ok_enabled,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<PdfExportWords>();
        let preset_words = env.words::<PresetWords>();
        let units = env.units();
        let catalog: &dyn ProfileCatalog = catalog.unwrap_or(&());
        let editor = mode == PdfExportMode::PresetEditor;
        let art_bounds = artboards == PdfArtboards::ArtBounds;
        let mut presets = builtin_presets(&app);
        let built_in: Vec<String> = presets.iter().map(|preset| preset.name.clone()).collect();
        presets.extend(saved_presets.iter().cloned());
        let default_name = default_preset_name(&app);

        // The bound values are read and written outside the body, which runs again for a measuring pass.
        let bound_section = bound_value(&mut section_bind);
        let bound_preset = bound_value(&mut preset_bind);
        let bound_artboards = bound_value(&mut artboards_bind);
        let bound_switches = bound_value(&mut switches_bind);
        let mut draft = bound_value(&mut draft_bind).unwrap_or_default();
        let drafted = draft.clone();

        let footer = ResetFooter::new(ui, None, true);
        let reset_app = app.clone();
        let mut form = FormDialog::new(words.title.clone(), bound)
            .ok_label(if editor { words.save_preset.clone() } else { words.save.clone() })
            .ok_enabled(ok_enabled)
            .accept_stays_open(hold_ok)
            .collection_body(true)
            .outer_gesture(true)
            // The chosen preset lives in the body's memory, which only the body's context reaches; a preset no longer
            // listed leaves the settings as they are.
            .reset_with(move |current, _, cx| {
                let Some(name) = cx.memory::<Live>().preset.clone() else { return current.clone() };
                let mut listed = builtin_presets(&reset_app).into_iter().chain(saved_presets.iter().cloned());
                match listed.find(|preset| preset.name == name) {
                    Some(mut found) => {
                        found.settings.clear_passwords();
                        found.settings
                    }
                    None => current.clone(),
                }
            })
            .on_reset(|cx| cx.memory::<Live>().reset())
            .shaped(|dialog| dialog.resizable(true));
        if let Some(open) = open {
            form = form.opened(open);
        }

        let mut events: Vec<PdfPresetEvent> = Vec::new();
        let mut chosen = PdfArtboards::All;
        let mut presses = SectionPresses { custom: false, save_summary: false };
        let mut summary: Option<String> = None;
        let mut kept: Option<Kept> = None;
        let mut latest: Option<PdfSettings> = None;
        let shown = form.show(ui, |ui, cx, work| {
            events.clear();
            let mut live = cx.memory::<Live>().clone();
            if cx.first() {
                live.artboards = opening_choice(&artboards);
                live.preset = Some(preset.clone().unwrap_or_else(|| default_name.clone()));
                live.reset();
            } else {
                if let Some(chosen) = &bound_preset {
                    live.preset.clone_from(chosen);
                }
                if let Some(choice) = &bound_artboards {
                    live.artboards.clone_from(choice);
                }
                if let Some(switches) = bound_switches {
                    live.switches = switches;
                }
            }
            if let Some(section) = bound_section {
                live.section = section;
            }
            // A measuring run closes nothing: the real run of the same pass makes the close.
            if finish_held && !crate::runtime::measure::active(ui.ctx()) {
                cx.request_close_as(CloseReason::Activated);
            }

            // The preset row; the editor names the one preset it edits instead.
            let names: Vec<(String, String)> =
                presets.iter().map(|preset| (preset.name.clone(), words.preset_name(&preset.name, &app).as_str().to_owned())).collect();
            let rows = PresetSource::new(names.as_slice(), PresetFns::new(preset_rows));
            let selected = live.preset.as_ref().and_then(|name| presets.iter().find(|preset| preset.name == *name));
            let reference = if editor { presets.iter().find(|preset| preset.name == default_name) } else { selected };
            let modified = selected.is_some_and(|preset| {
                let mut shown = work.clone();
                shown.clear_passwords();
                let mut base = preset.settings.clone();
                base.clear_passwords();
                shown != base
            });
            let held = selected.filter(|_| !modified).map(|preset| preset.name.clone());
            let base = reference.map(|preset| preset.settings.clone()).unwrap_or_default();
            let preset_name = reference.map_or_else(Label::default, |preset| words.preset_name(&preset.name, &app));
            let mut name_refused = false;
            let named_events = if editor {
                let check = check_name.as_deref().map(NameCheck::of_host);
                let naming = Naming::new(check.as_ref(), &preset_words.name_taken, &preset_words.name_invalid);
                Field::new(words.preset_name.clone()).id_salt("preset-name").show(ui, |ui, _| {
                    let name = Bind::given(draft.name.clone()).check_with(&naming, |naming, text| match naming.problem(text.trim()) {
                        Some(message) => Validity::Refuse(Message::new(message)),
                        None => Validity::Ok,
                    });
                    let typed = TextField::new(words.preset_name.clone(), name).id_salt("name").show(ui);
                    if let Some(text) = typed.value().cloned() {
                        draft.name = text;
                    }
                });
                name_refused = draft.name.trim().is_empty() || naming.problem(draft.name.trim()).is_some();
                Vec::new()
            } else {
                let mut picker =
                    PresetPicker::new(words.preset.clone(), Bind::given(held), &rows, Bind::bound(&mut live.picker)).field(PickerField::Name);
                if modified {
                    picker = picker.placeholder(Label::from(words.preset_modified.fill(&[("name", preset_name.as_str())])));
                }
                NamedPresetRow::new(words.preset.clone(), picker)
                    .suggest_name(words.new_preset_name.as_str().to_owned())
                    .check_name(NameCheck::new(presets.iter().map(|preset| preset.name.as_str()).collect::<Vec<_>>(), |names, typed| {
                        names.iter().any(|name| name.eq_ignore_ascii_case(typed)).then_some(PresetError::NameTaken)
                    }))
                    .built_in(|key| built_in.contains(key))
                    .show(ui)
                    .events
            };
            for event in named_events {
                match event {
                    NamedPresetEvent::Apply(key) => {
                        if let Some(preset) = presets.iter().find(|preset| preset.name == key) {
                            *work = preset.settings.clone();
                            live.reset();
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
                if editor || art_bounds || !choice.range_only || text.is_empty() {
                    return None;
                }
                check_range(text, count).err().map(|error| words.range_error_text(text, count, &error))
            };
            let settings_error =
                |settings: &PdfSettings| settings.check(catalog).and_then(|()| settings.check_destination(catalog, color_mode)).err();
            let warnings = warning_lines(&words, work, catalog, host_warnings);
            let changed = differences(&words, work, &base);
            let range_text = range_error(&live.artboards);

            // The section list and the shown section.
            let mut choice = live.artboards.clone();
            let mut switches = live.switches;
            let sides = if work.bleed.use_document { document_bleed } else { work.bleed.values() };
            let mut linked = live.bleed_linked.unwrap_or_else(|| sides_equal(sides));
            let section = live.section;
            let pages = Pages(&words);
            let picked = SectionList::new(words.sections.clone(), &pages, section).body_label(section.title(&words).clone()).show(
                ui,
                |key| ListRow::new(key.title(&words).clone()),
                |ui| {
                    Text::new(section.title(&words).clone()).role(TypeRole::Title).show(ui);
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
                        bleed_linked: &mut linked,
                        range_error: range_text.as_deref(),
                        warnings: &warnings,
                        changed: &changed,
                        preset_name: &preset_name,
                        mode,
                        custom_flattener,
                        pressed: &mut presses,
                        draft: if editor { Some(&mut draft) } else { None },
                        art_bounds,
                    }
                    .show(ui, section);
                },
            );
            live.section = picked.unwrap_or(section);
            live.artboards = choice;
            live.switches = switches;
            live.bleed_linked = Some(linked);
            if presses.save_summary {
                summary = Some(summary_text(&words, work, &base, &warnings));
            }

            // The failing check, under the sections, and the host's refusal of the held answer while the settings are
            // the ones it refused.
            let error = settings_error(work);
            if let Some(error) = &error {
                Notice::new(Tone::Bad, Label::from(words.error_text(error))).layout(NoticeLayout::Inline).show(ui);
            }
            if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| *refused == *work) {
                Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            let blocked = error.is_some() || name_refused || range_error(&live.artboards).is_some();
            chosen = {
                let range = live.artboards.range.trim();
                if live.artboards.range_only && !range.is_empty() { PdfArtboards::Range(range.to_owned()) } else { PdfArtboards::All }
            };
            if hold_ok {
                latest = Some(work.clone());
            }
            kept = Some(Kept { section: live.section, preset: live.preset.clone(), artboards: live.artboards.clone(), switches: live.switches });
            *cx.memory::<Live>() = live;
            !blocked
        });

        let pick = footer.pick(shown.picked);
        if let Some(kept) = kept {
            keep_bound(&mut section_bind, kept.section);
            keep_bound(&mut preset_bind, kept.preset);
            keep_bound(&mut artboards_bind, kept.artboards);
            // Reset clears the switches in the body's memory after this pass's body ran.
            keep_bound(&mut switches_bind, if pick == Some(ResetPick::Reset) { PasswordSwitches::default() } else { kept.switches });
        }
        keep_bound(&mut draft_bind, draft.clone());
        // The art's bounds come from the host: the section list never offers them.
        let answered = if art_bounds { PdfArtboards::ArtBounds } else { chosen };
        let result = shown.accepted.map(|options| PdfExportResult { options, artboards: answered.clone() });
        let held = match (pick, latest) {
            (Some(ResetPick::Accept), Some(options)) if hold_ok && result.is_none() => Some(PdfExportResult { options, artboards: answered }),
            _ => None,
        };
        PdfExportOutput {
            response: shown.response,
            closed: shown.closed,
            result,
            settings: shown.value,
            presets: events,
            custom_flattener: presses.custom,
            save_summary: summary,
            draft: (draft != drafted).then_some(draft),
            held,
        }
    }
}

/// One preset row per (name, shown name), keyed by the name.
fn preset_rows(names: &&[(String, String)], _: &Env) -> Vec<PresetRow<String>> {
    names.iter().map(|(name, shown)| PresetRow::item(name.clone(), shown.clone(), 0)).collect()
}
