//! Stories of `TreePicker`.

use super::{TreeEntry, TreePicker};
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::{Key, Modifiers, Ui};

const ARCHIVE: u32 = 1;
const SUMMER: u32 = 2;
const WINTER: u32 = 3;
const INBOX: u32 = 4;
const DRAFTS: u32 = 5;

fn albums() -> Vec<TreeEntry<u32>> {
    vec![
        TreeEntry::folder(ARCHIVE, "Archive"),
        TreeEntry::new(SUMMER, "Summer").within(ARCHIVE),
        TreeEntry::new(WINTER, "Winter").within(ARCHIVE),
        TreeEntry::new(INBOX, "Inbox"),
        TreeEntry::new(DRAFTS, "Drafts").blocked_by("Drafts are not shared"),
    ]
}

#[story(TreePicker<'static, u32>, covers = [Default], drive = Drive::Pick { row: 1 })]
fn tree_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = albums();
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    TreePicker::new("Album", Bind::bound(data.value("album", None::<u32>)), &entries).size(size).enabled(!disabled).show(ui).into()
}

#[story(TreePicker<'static, u32>, covers = [Mixed], drive = Drive::Never)]
fn tree_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = albums();
    TreePicker::new("Album", Bind::bound_mixed(data.value("mixed", Mixed::<Option<u32>>::Mixed)), &entries).show(ui).into()
}

#[story(TreePicker<'static, u32>, covers = [Invalid], drive = Drive::Never)]
fn tree_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = albums();
    let refuse_winter = |album: &Option<u32>| match album {
        Some(WINTER) => Validity::Refuse(Message::new("Winter is full")),
        _ => Validity::Ok,
    };
    TreePicker::new("Album", Bind::bound(data.value("invalid", Some(WINTER))).check(refuse_winter), &entries).show(ui).into()
}

/// Focuses the field and opens the tree with Alt+Down.
fn open_tree(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.idle(4);
}

#[story(TreePicker<'static, u32>, covers = [Open], drive = Drive::Custom(open_tree))]
fn tree_picker_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = albums();
    TreePicker::new("Album", Bind::bound(data.value("open", Some(SUMMER))), &entries).show(ui).into()
}
