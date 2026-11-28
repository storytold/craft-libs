//! Stories of New Workspace and Delete Workspace.

use super::{DeleteWorkspaceDialog, NewWorkspaceDialog};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::dock::{NamedWorkspace, Workspace, WorkspaceChrome, WorkspaceKeeps, WorkspaceSet};
use egui::Ui;

/// Essentials and Painting built in, Mine and Review saved, Review current.
fn workspaces() -> WorkspaceSet<u32> {
    let mut set = WorkspaceSet::new(vec![
        NamedWorkspace::new("essentials", "Essentials", Workspace::default()),
        NamedWorkspace::new("painting", "Painting", Workspace::default()),
    ]);
    for name in ["Mine", "Review"] {
        let _ = set.save(name, Workspace::default(), WorkspaceChrome::default(), WorkspaceKeeps::NONE);
    }
    set
}

#[story(NewWorkspaceDialog<'static, u32>, covers = [], drive = Drive::Never)]
fn new_workspace_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let set = data.value("set", workspaces());
    NewWorkspaceDialog::new(set).opened(Bind::given(true)).show(ui).into()
}

#[story(DeleteWorkspaceDialog<'static, u32>, covers = [], drive = Drive::Never)]
fn delete_workspace_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let set = data.value("set", workspaces());
    DeleteWorkspaceDialog::new(set).opened(Bind::given(true)).show(ui).into()
}
