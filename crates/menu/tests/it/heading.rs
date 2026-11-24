//! A heading is a caption over the rows after it: structure, never a command.

use craft_host::target::Os;
use craft_menu::{Item, LayoutOptions, Menu, MenuBar, MenuBarUi, Node, Target, tidy};
use craft_ui::contract::Tracked;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

use crate::sample::ids;

/// `tidy` keeps a heading only with an item after it in its group; the help search lists the items and not the
/// heading; the in-window menu shows it as a caption that takes no mnemonic letter, leaves its siblings' letters as
/// they are and runs nothing when clicked.
#[trial]
fn a_heading_is_structure_and_never_a_command() {
    let mut nodes = vec![
        Node::heading("最近"),
        Item::new("file.new", "新建").english("New").into(),
        Item::new("file.open", "打开").english("Open").into(),
        Node::Separator,
        Node::heading("Empty"),
        Node::Separator,
        Item::new("file.close", "关闭").english("Close").into(),
        Node::heading("Trailing"),
    ];
    tidy(&mut nodes);
    assert_eq!(
        ids(&nodes),
        ["#最近", "file.new", "file.open", "---", "file.close"],
        "a heading with no item after it goes, and the separators it leaves collapse"
    );

    let bar = MenuBar::new(vec![Menu::new("文件", nodes).english("File")]);
    let entries = bar.help_entries(Target::Linux, &LayoutOptions::new("Trial"));
    let listed: Vec<String> = entries.iter().map(|entry| entry.command().key()).filter(|key| key.starts_with("file.")).collect();
    assert_eq!(listed, ["file.new", "file.open", "file.close"], "the help search lists the items, never the heading");

    let bar = Tracked::new(bar);
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar).show(ui);
    });
    // A Mac bar has no letters; a Chinese interface shows the English letters after the labels.
    fixture.os(Os::Linux).host(crate::mnemonic::host_in("zh"));
    fixture.settle();
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    let heading = fixture.by_label("最近").expect("the heading shows as written, with no letter");
    for row in ["新建(N)", "打开(O)", "关闭(C)"] {
        assert!(fixture.by_label(row).is_some(), "{row}: the rows after the heading keep their own letters");
    }
    fixture.click(&heading).settle();
    assert!(fixture.by_label("新建(N)").is_some(), "a click on the heading runs nothing, so the menu stays open");
}
