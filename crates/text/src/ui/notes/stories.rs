//! Stories of the notes editor.

use craft_ui::contract::{Props as _, Show as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{NotesEditor, NotesParagraph, NotesRun, NotesText};
use crate::lists::ListMarker;

fn bullets() -> NotesText {
    let run = |text: &str, bold: bool| NotesRun::new(text).bold(bold);
    let bullet = || Some(ListMarker::Bullet("•".into()));
    NotesText::new(vec![
        NotesParagraph::new(vec![run("Open with ", false), run("the demo", true)]).list(0, bullet()),
        NotesParagraph::new(vec![run("Then the numbers", false)]).list(1, bullet()),
    ])
}

#[story(NotesEditor<'static>, covers = [Default], drive = Drive::Never)]
fn editing_bullets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let notes = data.value("notes", bullets());
    NotesEditor::new("Notes", notes).show(ui).response.into()
}

#[story(NotesEditor<'static>, covers = [LongText], drive = Drive::Never)]
fn long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let notes = data.value("notes", NotesText::from_plain(&"Speaker notes that wrap across the pane. ".repeat(12)));
    NotesEditor::new("Notes", notes).show(ui).response.into()
}

#[story(NotesEditor<'static>, covers = [], drive = Drive::Never)]
fn read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let notes = data.value("notes", bullets());
    NotesEditor::new("Notes", notes).read_only(true).show(ui).response.into()
}
