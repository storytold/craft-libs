//! A label with no typable letter takes its English label's letter as its mnemonic, shown after it: "文件(F)"; in
//! Turkish the dotless ı takes the I key and the dotted i none.

use std::sync::Arc;

use craft_host::target::Os;
use craft_host::{AppInfo, Host};
use craft_menu::{CommandRef, Labeled, MenuBarUi, Row, from_items};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

#[trial]
fn a_cjk_title_shows_its_english_letter_and_alt_with_the_letter_opens_it() {
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
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("新建(N)").is_some(), "Alt+F opens the title's menu, whose row shows its own English letter");
}

/// A host whose interface language is Turkish, with an empty catalog.
fn turkish() -> Arc<Host> {
    let (i18n, _) = craft_i18n::I18n::from_owned([("tr".to_owned(), String::new(), String::new())], &["tr"]);
    let tr = i18n.from_code("tr").unwrap_or_else(|| panic!("tr is offered"));
    Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).lang(tr).build())
}

/// The rows shown after Alt with `key` on a Linux bar titled `ix` and `ı` under a Turkish host: `One` is `ix`'s, `Two`
/// is `ı`'s.
fn opened_by(key: Key) -> (bool, bool) {
    let bar =
        from_items(&["ix", "ı"], &[Row::new(&["ix"], CommandRef::new("menu.one"), "One"), Row::new(&["ı"], CommandRef::new("menu.two"), "Two")]);
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar).show(ui);
    });
    // A Mac bar has no letters; the host gives the Turkish folder.
    fixture.os(Os::Linux).host(turkish());
    fixture.settle();
    assert!(fixture.by_label("ix").is_some() && fixture.by_label("ı").is_some(), "the titles show as written");
    fixture.key(key, Modifiers::ALT);
    fixture.settle();
    (fixture.by_label("One").is_some(), fixture.by_label("Two").is_some())
}

#[trial]
fn turkic_labels_keep_dotted_and_dotless_i_apart() {
    // ı's capital is I, so ı holds the I key; the dotted i has no key of its own, so ix takes X.
    assert_eq!(opened_by(Key::I), (false, true), "Alt+I opens ı");
    assert_eq!(opened_by(Key::X), (true, false), "Alt+X opens ix");
}
