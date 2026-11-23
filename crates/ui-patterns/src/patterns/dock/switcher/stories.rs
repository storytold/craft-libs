//! Stories of `WorkspaceSwitcher`.

use super::WorkspaceSwitcher;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::dock::{NamedWorkspace, Workspace, WorkspaceSet};
use egui::Ui;

#[story(WorkspaceSwitcher<'static, u32>, covers = [], drive = Drive::Never)]
fn workspace_switcher(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let essentials = NamedWorkspace::new("essentials", "Essentials", Workspace::<u32>::default());
    let painting = NamedWorkspace::new("painting", "Painting", Workspace::<u32>::default());
    let set = WorkspaceSet::new(vec![essentials, painting]);
    WorkspaceSwitcher::new(&set).show(ui).into()
}
