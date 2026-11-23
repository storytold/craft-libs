//! Stories of `ActionsPanel`: a tree with a set open and one action selected; a recording row;
//! Button Mode; the empty list.

use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::mock::{FakeActionsHost, action, list_of, sample_list, step};
use super::{ActionSelection, ActionsPanel, ActionsPanelState, ShortcutChoice};
use crate::actions::{ActionColor, ActionList};

/// The sample list with Sharpen open and selected, one step off, and a shortcut on it.
#[story(ActionsPanel<'static>, covers = [Default], drive = Drive::Never)]
fn actions_panel_tree(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let mut sharpen = action("Sharpen", None, &["levels", "curves", "blur"]);
    sharpen.disabled.insert(1);
    let list = list_of(vec![sharpen, action("Vignette", Some("Effects"), &["mask", "blur"]), action("Glow", Some("Effects"), &["glow"])]);
    let mut state = ActionsPanelState::default();
    state.expanded.insert("Sharpen".to_owned());
    state.selected = Some(ActionSelection::Action(0));
    let mut host = FakeActionsHost::default();
    host.shortcuts.insert("Sharpen".to_owned(), ShortcutChoice { key: 2, shift: false, command: false });
    ActionsPanel::new(&mut state, &list).host(&host).show(ui).into()
}

#[story(ActionsPanel<'static>, covers = [], drive = Drive::Never)]
fn actions_panel_recording(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let list = sample_list();
    let mut state = ActionsPanelState { selected: Some(ActionSelection::Action(0)), ..ActionsPanelState::default() };
    let host = FakeActionsHost { recording: Some(0), pending: vec![step("unsharp"), step("hue")], ..FakeActionsHost::default() };
    ActionsPanel::new(&mut state, &list).host(&host).show(ui).into()
}

#[story(ActionsPanel<'static>, covers = [], drive = Drive::Never)]
fn actions_panel_button_mode(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let mut list = ActionList::new();
    for (name, color) in ["None", "Red", "Orange", "Yellow", "Green", "Blue", "Violet", "Gray"].into_iter().zip(ActionColor::ALL) {
        if let Ok(index) = list.new_action(name, None) {
            let _ = list.set_color(index, color);
        }
    }
    let mut state = ActionsPanelState::default();
    state.settings.button_mode = true;
    ActionsPanel::new(&mut state, &list).host(&FakeActionsHost::default()).show(ui).into()
}

#[story(ActionsPanel<'static>, covers = [], drive = Drive::Never)]
fn actions_panel_empty(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let mut state = ActionsPanelState::default();
    ActionsPanel::new(&mut state, &ActionList::new()).host(&FakeActionsHost::default()).show(ui).into()
}
