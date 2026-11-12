//! The search of a TreePicker's popup lists the items whose path holds every typed word, across closed folders.

use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::pickers::{TreeEntry, TreePicker};
use craft_ui::testing::access::name;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Ui};

/// A picker over two folders in Archive, with nothing picked.
fn picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [
        TreeEntry::folder(1u32, "Archive"),
        TreeEntry::new(2u32, "Summer beach").within(1),
        TreeEntry::folder(3u32, "Beaches").within(1),
        TreeEntry::new(4u32, "Rocks").within(3),
        TreeEntry::new(5u32, "Beach day").within(1),
        TreeEntry::new(6u32, "Inbox"),
    ];
    TreePicker::new("Album", Bind::bound(data.value("album", None::<u32>)), &entries).show(ui).into()
}

/// The names of the listed items; a row reads as its name, then its detail line.
fn options(fixture: &Fixture) -> Vec<String> {
    let tree = fixture.tree();
    let rows = fixture.by_role(Role::LIST_BOX_OPTION);
    rows.iter().filter_map(|row| name(tree, row.node)).map(|read| read.split(", ").next().unwrap_or(read).to_owned()).collect()
}

#[trial]
fn typing_in_the_popup_lists_the_items_whose_path_holds_every_word_and_enter_picks_the_first() {
    let mut fixture = Fixture::story(&Story::new("tree_list", picker));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    fixture.type_text("beach");
    fixture.settle();
    assert_eq!(
        options(&fixture),
        ["Beach day", "Summer beach", "Rocks"],
        "items in closed folders are found by their path, the name that starts with the query first; no folder is listed"
    );
    fixture.type_text(" rock");
    fixture.settle();
    assert_eq!(options(&fixture), ["Rocks"], "every typed word must be in the path");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert_eq!(*fixture.data().value("album", None::<u32>), Some(4), "Enter picks the first item found");
    assert!(fixture.by_role(Role::LIST_BOX).is_empty(), "and the pick closes the popup");
}
