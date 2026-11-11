//! Stories of `CommandPalette`.

use super::CommandPalette;
use crate::contract::Bind;
use crate::menu::search::HelpEntry;
use crate::runtime::commands::{CommandRef, EditCommands};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::LazyLock;

static ENTRIES: LazyLock<Vec<HelpEntry>> = LazyLock::new(|| {
    vec![
        HelpEntry::new(EditCommands::Undo, "Undo").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Redo, "Redo").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Cut, "Cut").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Copy, "Copy").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Paste, "Paste").with_path(["Edit"]),
        HelpEntry::new(EditCommands::SelectAll, "Select All").with_path(["Select"]),
    ]
});

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never)]
fn command_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let recents = data.value("recents", vec![CommandRef::from(EditCommands::Paste)]).clone();
    CommandPalette::new("Command palette", ENTRIES.as_slice(), Bind::bound(data.value("open", true))).recents(Bind::given(recents)).show(ui).into()
}
