//! A label with no typable letter takes its English label's letter as its mnemonic, shown after it: "文件(F)".

use craft_menu::{CommandRef, Labeled, MenuBarUi, Row, from_items};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

#[trial]
fn a_cjk_title_shows_its_english_letter_and_the_letter_opens_it() {
    let bar = from_items(&["文件"], &[Row::new(&["文件"], CommandRef::new("file.new"), "新建")]);
    let english = |labeled: Labeled<'_>| match labeled {
        Labeled::Title("文件") => Some("File".to_owned()),
        Labeled::Command(command) if *command == CommandRef::new("file.new") => Some("New".to_owned()),
        _ => None,
    };
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar).english(&english).show(ui);
    });
    fixture.settle();
    assert!(fixture.by_label("文件(F)").is_some(), "the title shows the English letter after it");
    fixture.key(Key::F10, Modifiers::NONE);
    fixture.settle();
    fixture.key(Key::F, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("新建(N)").is_some(), "F opens the title's menu, whose row shows its own English letter");
}
