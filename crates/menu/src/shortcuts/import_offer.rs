//! The first-launch offer to bring in another editor's shortcut set (Photoshop's live set on a machine that has
//! Photoshop): asked once, never applied unasked.
//!
//! The app finds the file (it knows where its sibling editor keeps it), reads it with
//! `craft_keymap::import::kys::read`, and hands the set to [`ImportOffer::prepare`] with what its preferences
//! remember. The offer is made only when the user has no shortcut overrides of their own and the set changes at
//! least one shortcut; every outcome is remembered so a later launch never asks again.

use craft_keymap::{ForeignSet, ImportOptions, ImportPlan, Keymap, plan};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, Prompt, StackDialog, dialog_id};
use craft_ui::runtime::env::Env;
use craft_ui::sections::{DialogRole, FooterButton};
use serde::{Deserialize, Serialize};

/// What the app's preferences remember about the offer, under its own key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct OfferRecord {
    /// Where the set was found.
    pub source: String,
    /// Whether the user imported it.
    pub imported: bool,
}

impl OfferRecord {
    /// A record of the set found at `source`.
    pub fn new(source: impl Into<String>, imported: bool) -> Self {
        OfferRecord { source: source.into(), imported }
    }
}

/// Why no offer is made.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NoOffer {
    /// Asked before (the record exists).
    Answered,
    /// The user has overrides of their own; remember the source as not imported.
    OwnShortcuts,
    /// The set changes nothing; remember the source as not imported.
    NothingChanges,
}

impl NoOffer {
    /// What to store so a later launch never asks again; `None` when the record already exists.
    pub fn record(&self, source: &str) -> Option<OfferRecord> {
        match self {
            NoOffer::Answered => None,
            NoOffer::OwnShortcuts | NoOffer::NothingChanges => Some(OfferRecord::new(source, false)),
        }
    }
}

/// A set waiting for the user's answer.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ImportOffer {
    /// Where the set was found (Photoshop's `Keyboard Shortcuts.psp`).
    pub source: String,
    /// The set's name, else its file name.
    pub set_name: String,
    /// The plan the answer Import applies.
    pub plan: ImportPlan,
    /// Commands whose shortcuts end up different from the app's defaults; the offer is made only when this is
    /// above 0.
    pub changes: usize,
    words: Option<ImportOfferWords>,
}

/// The user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OfferAnswer {
    /// Import the set.
    Import,
    /// Leave the keymap alone.
    DontImport,
}

impl ImportOffer {
    /// The offer for `set` found at `source`, or why none is made. `record` is what the preferences hold, `None` on
    /// first launch.
    pub fn prepare(
        source: &str,
        set: &ForeignSet,
        keymap: &Keymap,
        options: &ImportOptions,
        record: Option<&OfferRecord>,
    ) -> Result<ImportOffer, NoOffer> {
        if record.is_some() {
            return Err(NoOffer::Answered);
        }
        if !keymap.overrides().is_empty() {
            return Err(NoOffer::OwnShortcuts);
        }
        let plan = plan(set, keymap, options);
        // Counted against the defaults, not the import report: a command the set leaves on its defaults is not a
        // change, whatever the keymap passed in held before.
        let mut applied = keymap.clone();
        plan.apply(&mut applied);
        let changes = plan.commands.iter().filter(|p| applied.bindings(&p.command) != applied.default_bindings(&p.command)).count();
        if changes == 0 {
            return Err(NoOffer::NothingChanges);
        }
        let set_name = set
            .name
            .clone()
            .or_else(|| std::path::Path::new(source).file_name().map(|f| f.to_string_lossy().into_owned()))
            .unwrap_or_else(|| source.to_owned());
        Ok(ImportOffer { source: source.to_owned(), set_name, plan, changes, words: None })
    }

    /// The record to store for `answer`.
    pub fn record(&self, answer: OfferAnswer) -> OfferRecord {
        OfferRecord::new(self.source.clone(), answer == OfferAnswer::Import)
    }

    /// The offer's texts, instead of the ones of the scope's language.
    #[must_use]
    pub fn words(mut self, words: ImportOfferWords) -> Self {
        self.words = Some(words);
        self
    }
}

/// The prompt on the dialog stack, queued at `StartupStage::Offer`: "Import keyboard shortcuts?", the line naming
/// the set and the count, the line on undoing it (Reset All to Defaults), and Import (default, Enter) and Don't
/// Import (Escape). Escape and the button both answer [`OfferAnswer::DontImport`]; a dialog closed from outside
/// gives [`DialogResult::Cancel`], which the app does not record, so the next launch asks again. On `Ok` the app
/// applies the plan on Import (`ImportPlan::apply`), stores [`ImportOffer::record`] either way and posts a notice
/// that the set is in use.
impl StackDialog for ImportOffer {
    type Result = OfferAnswer;

    fn id(&self) -> DialogId {
        dialog_id(("keymap-import-offer", &self.source))
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<OfferAnswer>> {
        let env = Env::of(ui);
        let words = self.words.clone().unwrap_or_else(|| (*env.words::<ImportOfferWords>()).clone());
        let app = env.view().host().map_or("Craft", |host| host.app().name);
        let changes = env.format().integer(self.changes as u64);
        let found = words.found.fill(&[("app", app), ("set", self.set_name.as_str()), ("n", changes.as_str())]);
        let answers = [FooterButton::new(words.import.clone(), DialogRole::Accept), FooterButton::new(words.dont_import.clone(), DialogRole::Cancel)];
        let shown = Prompt::new(words.title.clone(), format!("{found}\n\n{}", words.undo.as_str())).answers(answers).show(ui);
        let answer = if shown.picked? == 0 { OfferAnswer::Import } else { OfferAnswer::DontImport };
        Some(DialogResult::Ok(answer))
    }
}

craft_ui::words! {
    /// The offer's texts. `{set}` is the set's name, `{n}` the number of shortcuts that change, `{app}` the app's
    /// name, `{error}` why the import failed. The last three rows are the notice after an import and the status
    /// line after a failed one; the app writes them with `fill`.
    pub struct ImportOfferWords {
        title: "importOffer.title" = "Import keyboard shortcuts?";
        found: template "importOffer.found" = "{app} found your keyboard shortcut set {set} from another image editor on this computer: {n} of its shortcuts differ from {app}'s. Import it?";
        undo: "importOffer.undo" = "Edit › Keyboard Shortcuts › Reset All to Defaults undoes an import.";
        import: "importOffer.import" = "Import";
        dont_import: "importOffer.dontImport" = "Don't Import";
        imported_title: "importOffer.importedTitle" = "Keyboard shortcuts imported";
        imported: template "importOffer.imported" = "Your keyboard shortcut set {set} is in use here: {n} shortcuts differ from the defaults. Edit › Keyboard Shortcuts shows them; Reset All to Defaults undoes this.";
        not_imported: template "importOffer.notImported" = "The keyboard shortcut set was not imported: {error}";
    }
}
