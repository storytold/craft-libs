//! `HostedPdfExport`: the Save PDF dialog owned by a `DialogHost`.

use super::sections::enabled;
use super::{
    ArtboardChoice, PasswordSwitches, PdfArtboards, PdfExportDialog, PdfExportMode, PdfExportResult, PdfExportWords, PdfPresetEvent, PdfSection,
    PresetDraft, apply_standard, check_range, summary_text, warning_lines,
};
use crate::contract::Bind;
use crate::dialog::{
    DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id, serde_fields, set_serde_field,
};
use crate::patterns::presets::PresetError;
use crate::runtime::env::{Label, UnitKind};
use crate::runtime::overlay::CloseReason;
use craft_pdf::settings::{ColorMode, PdfPreset, PdfSettings, ProfileCatalog, Standard, builtin_presets, default_preset_name};
use egui::{Context, Ui};
use serde_json::Value;
use std::sync::Arc;

/// What a hosted Save PDF answers with at OK: the dialog's result and, in the preset editor, the name and description.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PdfExportAnswer {
    /// The settings and the artboards.
    pub result: PdfExportResult,
    /// The preset editor's name and description; `None` in the save mode.
    pub draft: Option<PresetDraft>,
}

/// The ids automation names the sections by, in the order of [`PdfSection::ALL`].
const SECTION_IDS: [&str; PdfSection::ALL.len()] = ["general", "compression", "marks", "output", "advanced", "security", "summary"];

/// A check a typed preset name goes through.
type NameChecker = Box<dyn Fn(&str) -> Option<PresetError> + Send>;

/// A [`PdfExportDialog`](super::PdfExportDialog) that the `DialogHost` owns, under the key `"pdf-export"`. OK is held
/// until the app has written the file ([`answer_ok`](HostedPdfExport::answer_ok)).
pub struct HostedPdfExport {
    id: DialogId,
    app: String,
    settings: PdfSettings,
    /// The preset the dialog opens on; the app's default preset without one.
    opening_preset: Option<String>,
    /// The preset the settings started from or were last filled from.
    chosen: Option<String>,
    saved_presets: Vec<PdfPreset>,
    flattener_presets: Vec<String>,
    profiles: Vec<String>,
    catalog: Option<Arc<dyn ProfileCatalog + Send + Sync>>,
    color_mode: ColorMode,
    length_unit: UnitKind,
    document_bleed: [f64; 4],
    /// The artboards chosen when the dialog opens.
    opening: PdfArtboards,
    choice: ArtboardChoice,
    artboard_count: Option<usize>,
    mode: PdfExportMode,
    draft: PresetDraft,
    check_name: Option<NameChecker>,
    section: PdfSection,
    switches: PasswordSwitches,
    custom_flattener: bool,
    host_warnings: Vec<String>,
    /// What the dialog's passes reported and the app has not taken yet.
    events: Vec<PdfPresetEvent>,
    custom_pressed: bool,
    save_summary: Option<String>,
    held: HeldAnswer<PdfExportAnswer>,
}

/// The artboard choice a dialog that opens with `artboards` starts from.
fn choice_of(artboards: &PdfArtboards) -> ArtboardChoice {
    match artboards {
        PdfArtboards::Range(range) => ArtboardChoice { range_only: true, range: range.clone() },
        _ => ArtboardChoice::default(),
    }
}

impl HostedPdfExport {
    /// The dialog for the application called `app`, editing `settings`.
    pub fn new(app: impl Into<String>, settings: PdfSettings) -> Self {
        let app = app.into();
        HostedPdfExport {
            id: dialog_id(KEY),
            chosen: Some(default_preset_name(&app)),
            app,
            settings,
            opening_preset: None,
            saved_presets: Vec::new(),
            flattener_presets: Vec::new(),
            profiles: Vec::new(),
            catalog: None,
            color_mode: ColorMode::Rgb,
            length_unit: UnitKind::Points,
            document_bleed: [0.0; 4],
            opening: PdfArtboards::All,
            choice: ArtboardChoice::default(),
            artboard_count: None,
            mode: PdfExportMode::default(),
            draft: PresetDraft::default(),
            check_name: None,
            section: PdfSection::default(),
            switches: PasswordSwitches::default(),
            custom_flattener: false,
            host_warnings: Vec::new(),
            events: Vec::new(),
            custom_pressed: false,
            save_summary: None,
            held: HeldAnswer::new(),
        }
    }

    /// The preset the settings start from, by name.
    #[must_use]
    pub fn preset(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.chosen = Some(name.clone());
        self.opening_preset = Some(name);
        self
    }

    /// The presets the user saved, after the built-in ones in the preset list.
    #[must_use]
    pub fn saved_presets(mut self, presets: Vec<PdfPreset>) -> Self {
        self.saved_presets = presets;
        self
    }

    /// The names of the saved flattener presets.
    #[must_use]
    pub fn flattener_presets(mut self, names: Vec<String>) -> Self {
        self.flattener_presets = names;
        self
    }

    /// The colour profiles the app knows.
    #[must_use]
    pub fn profiles(mut self, names: Vec<String>) -> Self {
        self.profiles = names;
        self
    }

    /// The lookup the checks use for the profiles; it stands for the dialog's `Option<&dyn ProfileCatalog>`.
    #[must_use]
    pub fn catalog(mut self, catalog: Arc<dyn ProfileCatalog + Send + Sync>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// The colour mode of the document.
    #[must_use]
    pub fn color_mode(mut self, mode: ColorMode) -> Self {
        self.color_mode = mode;
        self
    }

    /// The unit lengths are shown in.
    #[must_use]
    pub fn length_unit(mut self, unit: UnitKind) -> Self {
        self.length_unit = unit;
        self
    }

    /// The document's bleed in points (top, bottom, left, right).
    #[must_use]
    pub fn document_bleed(mut self, bleed: [f64; 4]) -> Self {
        self.document_bleed = bleed;
        self
    }

    /// The artboards chosen when the dialog opens.
    #[must_use]
    pub fn artboards(mut self, artboards: PdfArtboards) -> Self {
        self.choice = choice_of(&artboards);
        self.opening = artboards;
        self
    }

    /// The number of artboards of the document.
    #[must_use]
    pub fn artboard_count(mut self, count: usize) -> Self {
        self.artboard_count = Some(count);
        self
    }

    /// What the dialog is for.
    #[must_use]
    pub fn mode(mut self, mode: PdfExportMode) -> Self {
        self.mode = mode;
        self
    }

    /// The preset editor's name and description it opens with.
    #[must_use]
    pub fn draft(mut self, draft: PresetDraft) -> Self {
        self.draft = draft;
        self
    }

    /// Whether a typed preset name may be used, in the preset editor.
    #[must_use]
    pub fn check_name(mut self, check: impl Fn(&str) -> Option<PresetError> + Send + 'static) -> Self {
        self.check_name = Some(Box::new(check));
        self
    }

    /// The settings the dialog shows now.
    pub fn settings(&self) -> &PdfSettings {
        &self.settings
    }

    /// Replaces the saved presets of the preset list.
    pub fn set_saved_presets(&mut self, presets: Vec<PdfPreset>) {
        self.saved_presets = presets;
    }

    /// Replaces the names of the saved flattener presets.
    pub fn set_flattener_presets(&mut self, names: Vec<String>) {
        self.flattener_presets = names;
    }

    /// Whether the flattener select shows `[Custom]`, which also marks the chosen preset as modified.
    pub fn set_custom_flattener(&mut self, on: bool) {
        self.custom_flattener = on;
    }

    /// The warnings of the host, shown after the settings' own.
    pub fn set_host_warnings(&mut self, warnings: Vec<String>) {
        self.host_warnings = warnings;
    }

    /// What the user did to the app's presets in the last pass, once.
    pub fn take_preset_events(&mut self) -> Vec<PdfPresetEvent> {
        std::mem::take(&mut self.events)
    }

    /// Whether Custom… was pressed, once.
    pub fn take_custom_flattener(&mut self) -> bool {
        std::mem::take(&mut self.custom_pressed)
    }

    /// The summary text Save Summary… asked to save, once.
    pub fn take_save_summary(&mut self) -> Option<String> {
        self.save_summary.take()
    }

    /// The Summary's text for the dialog's settings, as Save Summary… saves it, worded by `words`.
    pub fn summary_text(&self, words: &PdfExportWords) -> String {
        let warnings = warning_lines(words, &self.settings, Self::catalog_of(&self.catalog), &self.host_warnings);
        summary_text(words, &self.settings, &self.base(), &warnings)
    }

    /// The answer OK filed for the app to write, once; `None` when none is filed or it was taken already.
    pub fn take_ok(&mut self) -> Option<PdfExportAnswer> {
        self.held.take()
    }

    /// The app's answer to the held OK: `Ok` closes the dialog with the held answer; `Err` shows its label and keeps
    /// the dialog open.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// The profile lookup of the checks; every profile name is unknown without one, as in the dialog.
    fn catalog_of(catalog: &Option<Arc<dyn ProfileCatalog + Send + Sync>>) -> &dyn ProfileCatalog {
        match catalog {
            Some(catalog) => catalog.as_ref(),
            None => &(),
        }
    }

    /// The built-in presets, then the saved ones.
    fn presets(&self) -> Vec<PdfPreset> {
        let mut presets = builtin_presets(&self.app);
        presets.extend(self.saved_presets.iter().cloned());
        presets
    }

    /// The settings the Summary compares with: the chosen preset's (the default preset's in the editor).
    fn base(&self) -> PdfSettings {
        let name = match self.mode {
            PdfExportMode::PresetEditor => Some(default_preset_name(&self.app)),
            _ => self.chosen.clone(),
        };
        self.presets().into_iter().find(|preset| Some(&preset.name) == name.as_ref()).map(|preset| preset.settings).unwrap_or_default()
    }

    /// The artboards the answer names.
    fn chosen_artboards(&self) -> PdfArtboards {
        let range = self.choice.range.trim();
        match &self.opening {
            PdfArtboards::ArtBounds => PdfArtboards::ArtBounds,
            _ if self.choice.range_only && !range.is_empty() => PdfArtboards::Range(range.to_owned()),
            _ => PdfArtboards::All,
        }
    }

    /// The answer OK files.
    fn answer(&self, options: PdfSettings, artboards: PdfArtboards) -> PdfExportAnswer {
        PdfExportAnswer {
            result: PdfExportResult { options, artboards },
            draft: (self.mode == PdfExportMode::PresetEditor).then(|| self.draft.clone()),
        }
    }

    /// Whether a failing check, a bad artboard range or, in the preset editor, an empty or refused name keeps OK off,
    /// as the dialog's own footer has it.
    fn blocked(&self) -> bool {
        let catalog = Self::catalog_of(&self.catalog);
        let range = self.choice.range.trim();
        let range_bad = self.choice.range_only && !range.is_empty() && self.artboard_count.is_some_and(|count| check_range(range, count).is_err());
        let name = self.draft.name.trim();
        let name_refused =
            self.mode == PdfExportMode::PresetEditor && (name.is_empty() || self.check_name.as_ref().is_some_and(|check| check(name).is_some()));
        range_bad || name_refused || self.settings.check(catalog).and_then(|()| self.settings.check_destination(catalog, self.color_mode)).is_err()
    }

    /// Whether the four bleed sides are equal, which is what the link between them means.
    fn bleed_linked(&self) -> bool {
        let [top, rest @ ..] = self.settings.bleed.values();
        rest.iter().all(|side| *side == top)
    }
}

/// The section `id` names.
fn section_of(id: &str) -> Option<PdfSection> {
    PdfSection::ALL.iter().zip(SECTION_IDS).find(|(_, name)| *name == id).map(|(section, _)| *section)
}

impl HoldsOk for HostedPdfExport {
    type Answer = PdfExportAnswer;

    /// OK is always held: the dialog as it is.
    fn held_ok(self) -> Self {
        self
    }

    fn take_ok(&mut self) -> Option<PdfExportAnswer> {
        HostedPdfExport::take_ok(self)
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        HostedPdfExport::answer_ok(self, answer);
    }
}

impl StackDialog for HostedPdfExport {
    type Result = PdfExportAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PdfExportAnswer>> {
        let finish = self.held.is_accepted();
        let refusal = self.held.refusal().map(|(reason, refused)| (reason.clone(), refused.result.options.clone()));
        let catalog = Self::catalog_of(&self.catalog);
        let mut dialog = PdfExportDialog::new(self.app.clone(), Bind::bound(&mut self.settings))
            .saved_presets(&self.saved_presets)
            .flattener_presets(&self.flattener_presets)
            .profiles(&self.profiles)
            .catalog(catalog)
            .color_mode(self.color_mode)
            .length_unit(self.length_unit)
            .document_bleed(self.document_bleed)
            .artboards(self.opening.clone())
            .mode(self.mode)
            .preset_draft(Bind::bound(&mut self.draft))
            .section(Bind::bound(&mut self.section))
            .chosen_preset(Bind::bound(&mut self.chosen))
            .artboard_choice(Bind::bound(&mut self.choice))
            .password_switches(Bind::bound(&mut self.switches))
            .custom_flattener(self.custom_flattener)
            .host_warnings(&self.host_warnings)
            .hold_ok(true)
            .finish_held(finish)
            // OK waits for the app's answer: a second OK cannot file over the held one.
            .ok_enabled(!self.held.is_waiting());
        if let Some(name) = &self.opening_preset {
            dialog = dialog.preset(name.clone());
        }
        if let Some(count) = self.artboard_count {
            dialog = dialog.artboard_count(count);
        }
        if let Some(check) = &self.check_name {
            dialog = dialog.check_name(|typed: &str| check(typed));
        }
        if let Some(refusal) = refusal {
            dialog = dialog.refusal(refusal);
        }
        let shown = dialog.show(ui);
        // A pass reports a press only in the pass that took the input, so a second pass of the frame adds none.
        self.events.extend(shown.presets);
        self.custom_pressed |= shown.custom_flattener;
        if shown.save_summary.is_some() {
            self.save_summary = shown.save_summary;
        }
        if self.held.refusal().is_some_and(|(_, refused)| refused.result.options != self.settings) {
            self.held.clear_refusal();
        }
        if let Some(result) = shown.held {
            let answer = self.answer(result.options, result.artboards);
            self.held.file(answer);
        }
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(self.answer(result.options, result.artboards))),
            (None, Some(CloseReason::Activated)) if finish => Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn confirm(&mut self, _: &Context) -> Option<PdfExportAnswer> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() || self.blocked() {
            return None;
        }
        let answer = self.answer(self.settings.clone(), self.chosen_artboards());
        self.held.file(answer);
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// The settings, `section`, `preset`, `artboards`, `range`, `bleedLinked`, and `name` and `description` in the
    /// editor.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = serde_fields(&self.settings)?;
        let map = fields.as_object_mut()?;
        let artboards = match (&self.opening, self.choice.range_only) {
            (PdfArtboards::ArtBounds, _) => "artBounds",
            (_, true) => "range",
            _ => "all",
        };
        let section = PdfSection::ALL.iter().zip(SECTION_IDS).find(|(section, _)| **section == self.section).map(|(_, id)| id);
        map.insert("section".to_owned(), section.into());
        map.insert("preset".to_owned(), self.chosen.clone().into());
        map.insert("artboards".to_owned(), artboards.into());
        map.insert("range".to_owned(), self.choice.range.clone().into());
        map.insert("bleedLinked".to_owned(), self.bleed_linked().into());
        if self.mode == PdfExportMode::PresetEditor {
            map.insert("name".to_owned(), self.draft.name.clone().into());
            map.insert("description".to_owned(), self.draft.description.clone().into());
        }
        Some(fields)
    }

    /// Sets one of the fields `json_fields` lists, else a settings field, or one of the two passwords, which the settings
    /// never serialise and `json_fields` never lists; a write to a disabled setting is refused.
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        match field {
            "section" => self.section = value.as_str().and_then(section_of).ok_or_else(rejects)?,
            "preset" => {
                let name = value.as_str().ok_or_else(rejects)?;
                let preset = self.presets().into_iter().find(|preset| preset.name == name).ok_or_else(rejects)?;
                self.settings = preset.settings;
                self.switches = PasswordSwitches::default();
                self.chosen = Some(preset.name);
            }
            "artboards" => match value.as_str() {
                Some("all") => self.choice.range_only = false,
                Some("range") => self.choice.range_only = true,
                Some("artBounds") if self.opening == PdfArtboards::ArtBounds => {}
                _ => return Err(rejects()),
            },
            "range" => self.choice.range = value.as_str().ok_or_else(rejects)?.to_owned(),
            "bleedLinked" => {
                let linked = value.as_bool().ok_or_else(rejects)?;
                if !enabled(&self.settings, "bleed.top") {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                if linked {
                    let bleed = &mut self.settings.bleed;
                    (bleed.bottom, bleed.left, bleed.right) = (bleed.top, bleed.top, bleed.top);
                }
            }
            "security.openPassword" | "security.permissionsPassword" => {
                if !enabled(&self.settings, field) {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                let password = value.as_str().ok_or_else(rejects)?.to_owned();
                let security = &mut self.settings.security;
                *(if field == "security.openPassword" { &mut security.open_password } else { &mut security.permissions_password }) = password;
            }
            "name" | "description" if self.mode == PdfExportMode::PresetEditor => {
                let text = value.as_str().ok_or_else(rejects)?.to_owned();
                if field == "name" {
                    self.draft.name = text;
                } else {
                    self.draft.description = text;
                }
            }
            _ => {
                if !enabled(&self.settings, field) {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                let mut next = self.settings.clone();
                set_serde_field(&mut next, field, value.clone())?;
                // The passwords never go through JSON, so the settings read back keep the dialog's.
                let security = &self.settings.security;
                next.security.open_password.clone_from(&security.open_password);
                next.security.permissions_password.clone_from(&security.permissions_password);
                if field == "standard" {
                    apply_standard(&mut next);
                    if next.standard != Standard::None {
                        self.switches = PasswordSwitches::default();
                    }
                }
                self.settings = next;
            }
        }
        Ok(())
    }
}

/// The dialog's key.
const KEY: &str = "pdf-export";
