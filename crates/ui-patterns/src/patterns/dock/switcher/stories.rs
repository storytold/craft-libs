//! Stories of `WorkspaceSwitcher`.

use super::WorkspaceSwitcher;
use crate::docking::WorkspaceCommands;
use crate::pickers::SelectLook;
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

/// A switcher whose list offers the app's own command rows in place of the default ones.
#[story(WorkspaceSwitcher<'static, u32>, covers = [], drive = Drive::Never)]
fn workspace_switcher_commands(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let essentials = NamedWorkspace::new("essentials", "Essentials", Workspace::<u32>::default());
    let set = WorkspaceSet::new(vec![essentials]);
    WorkspaceSwitcher::new(&set).commands(vec![WorkspaceCommands::New.into(), WorkspaceCommands::Reset.into()]).show(ui).into()
}

/// A quiet switcher, as wide as the current workspace's name, beside a longer one: it sits on the panel surface with
/// no edge until it has focus.
#[story(WorkspaceSwitcher<'static, u32>, covers = [], drive = Drive::Never)]
fn workspace_switcher_quiet(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let essentials = NamedWorkspace::new("essentials", "Essentials", Workspace::<u32>::default());
    let long = NamedWorkspace::new("print", "Print production and prepress", Workspace::<u32>::default());
    let set = WorkspaceSet::new(vec![essentials, long]);
    WorkspaceSwitcher::new(&set).look(SelectLook::Quiet).show(ui).into()
}
