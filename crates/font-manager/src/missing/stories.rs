//! Stories of `MissingFontsDialog`: two missing fonts and one installed.

use std::sync::{Arc, Mutex, PoisonError};

use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{DocumentFont, MissingFontsDialog, document_fonts};
use crate::panel::fixtures::story_catalog;

/// The dialog, kept across passes.
#[derive(Clone)]
struct Held(Arc<Mutex<MissingFontsDialog>>);

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Held(..)")
    }
}

fn dialog() -> Held {
    let font = |family: &str, style: &str| DocumentFont { family: family.to_owned(), style: style.to_owned(), postscript_name: None };
    let catalog = story_catalog();
    let rows = document_fonts(&catalog, &[font("Ubuntu", "Light"), font("Gotham", "Bold"), font("Proxima Nova", "Regular")]);
    Held(Arc::new(Mutex::new(MissingFontsDialog::new(catalog, rows))))
}

#[story(MissingFontsDialog, covers = [], drive = Drive::Never)]
fn missing_fonts_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let held = data.value("dialog", dialog()).clone();
    let mut dialog = held.0.lock().unwrap_or_else(PoisonError::into_inner);
    let mut stack = DialogStack::default();
    stack.push(dialog.id(), None);
    let _ = dialog.show(ui, &stack);
    StoryOutput::default()
}
