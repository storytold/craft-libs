//! `HostedPdfExport`: the Save PDF dialog owned by a `DialogHost`.

use super::{PdfArtboards, PdfExportMode, PdfExportResult, PdfPresetEvent, PresetDraft};
use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog};
use crate::patterns::presets::PresetError;
use crate::runtime::env::{Label, UnitKind};
use craft_pdf::settings::{ColorMode, PdfPreset, PdfSettings, ProfileCatalog};
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

/// A [`PdfExportDialog`](super::PdfExportDialog) that the `DialogHost` owns, under the key `"pdf-export"`. OK is held
/// until the app has written the file ([`answer_ok`](HostedPdfExport::answer_ok)).
pub struct HostedPdfExport {
    id: DialogId,
}

impl HostedPdfExport {
    /// The dialog for the application called `app`, editing `settings`.
    pub fn new(app: impl Into<String>, settings: PdfSettings) -> Self {
        let _ = (app, settings);
        todo!("ex.P17")
    }

    /// The preset the settings start from, by name.
    #[must_use]
    pub fn preset(self, name: impl Into<String>) -> Self {
        let _ = (self, name);
        todo!("ex.P17")
    }

    /// The presets the user saved, after the built-in ones in the preset list.
    #[must_use]
    pub fn saved_presets(self, presets: Vec<PdfPreset>) -> Self {
        let _ = (self, presets);
        todo!("ex.P17")
    }

    /// The names of the saved flattener presets.
    #[must_use]
    pub fn flattener_presets(self, names: Vec<String>) -> Self {
        let _ = (self, names);
        todo!("ex.P17")
    }

    /// The colour profiles the app knows.
    #[must_use]
    pub fn profiles(self, names: Vec<String>) -> Self {
        let _ = (self, names);
        todo!("ex.P17")
    }

    /// The lookup the checks use for the profiles; it stands for the dialog's `Option<&dyn ProfileCatalog>`.
    #[must_use]
    pub fn catalog(self, catalog: Arc<dyn ProfileCatalog + Send + Sync>) -> Self {
        let _ = (self, catalog);
        todo!("ex.P17")
    }

    /// The colour mode of the document.
    #[must_use]
    pub fn color_mode(self, mode: ColorMode) -> Self {
        let _ = (self, mode);
        todo!("ex.P17")
    }

    /// The unit lengths are shown in.
    #[must_use]
    pub fn length_unit(self, unit: UnitKind) -> Self {
        let _ = (self, unit);
        todo!("ex.P17")
    }

    /// The document's bleed in points (top, bottom, left, right).
    #[must_use]
    pub fn document_bleed(self, bleed: [f64; 4]) -> Self {
        let _ = (self, bleed);
        todo!("ex.P17")
    }

    /// The artboards chosen when the dialog opens.
    #[must_use]
    pub fn artboards(self, artboards: PdfArtboards) -> Self {
        let _ = (self, artboards);
        todo!("ex.P17")
    }

    /// The number of artboards of the document.
    #[must_use]
    pub fn artboard_count(self, count: usize) -> Self {
        let _ = (self, count);
        todo!("ex.P17")
    }

    /// What the dialog is for.
    #[must_use]
    pub fn mode(self, mode: PdfExportMode) -> Self {
        let _ = (self, mode);
        todo!("ex.P17")
    }

    /// The preset editor's name and description it opens with.
    #[must_use]
    pub fn draft(self, draft: PresetDraft) -> Self {
        let _ = (self, draft);
        todo!("ex.P17")
    }

    /// Whether a typed preset name may be used, in the preset editor.
    #[must_use]
    pub fn check_name(self, check: impl Fn(&str) -> Option<PresetError> + Send + 'static) -> Self {
        let _ = (self, check);
        todo!("ex.P17")
    }

    /// The settings the dialog shows now.
    pub fn settings(&self) -> &PdfSettings {
        let _ = self;
        todo!("ex.P18")
    }

    /// Replaces the saved presets of the preset list.
    pub fn set_saved_presets(&mut self, presets: Vec<PdfPreset>) {
        let _ = (self, presets);
        todo!("ex.P18")
    }

    /// Replaces the names of the saved flattener presets.
    pub fn set_flattener_presets(&mut self, names: Vec<String>) {
        let _ = (self, names);
        todo!("ex.P18")
    }

    /// Whether the flattener select shows `[Custom]`.
    pub fn set_custom_flattener(&mut self, on: bool) {
        let _ = (self, on);
        todo!("ex.P18")
    }

    /// The warnings of the host, shown after the settings' own.
    pub fn set_host_warnings(&mut self, warnings: Vec<String>) {
        let _ = (self, warnings);
        todo!("ex.P18")
    }

    /// What the user did to the app's presets in the last pass, once.
    pub fn take_preset_events(&mut self) -> Vec<PdfPresetEvent> {
        let _ = self;
        todo!("ex.P18")
    }

    /// Whether Custom… was pressed, once.
    pub fn take_custom_flattener(&mut self) -> bool {
        let _ = self;
        todo!("ex.P18")
    }

    /// The summary text Save Summary… asked to save, once.
    pub fn take_save_summary(&mut self) -> Option<String> {
        let _ = self;
        todo!("ex.P18")
    }

    /// The Summary's text for the dialog's settings, as Save Summary… saves it.
    pub fn summary_text(&self, ctx: &egui::Context) -> String {
        let _ = (self, ctx);
        todo!("ex.P18")
    }

    /// The app's answer to the held OK: `Ok` closes the dialog with the held answer; `Err` shows its label and keeps
    /// the dialog open.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        let _ = (self, answer);
        todo!("ex.P18")
    }
}

impl StackDialog for HostedPdfExport {
    type Result = PdfExportAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PdfExportAnswer>> {
        let _ = (self.id, ui);
        todo!("ex.P19")
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    /// The settings, `section`, `preset`, `artboards`, `range`, `bleedLinked`, and `name` and `description` in the
    /// editor.
    fn json_fields(&self) -> Option<Value> {
        todo!("ex.P19")
    }

    /// Sets one of the fields `json_fields` lists, else a settings field; a write to a disabled setting is refused.
    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let _ = (ctx, field, value);
        todo!("ex.P19")
    }
}

/// The dialog's key.
const KEY: &str = "pdf-export";
