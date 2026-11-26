//! A label with no typable letter takes its English label's letter as its mnemonic: shown after it in a CJK language,
//! "文件(F)", and hidden in any other script, so "Файл" stays as written and Alt+F still opens it. In Turkish the
//! dotless ı takes the I key and the dotted i none.

use std::sync::Arc;

use craft_host::target::Os;
use craft_host::{AppInfo, Host};
use craft_menu::{CommandRef, Item, LayoutOptions, Menu, MenuBar, MenuBarUi, Node, Row, Target, from_items};
use craft_ui::contract::Tracked;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

#[trial]
fn a_cjk_title_shows_its_english_letter_and_alt_with_the_letter_opens_it() {
    let mut bar = from_items(&["文件"], &[Row::new(&["文件"], CommandRef::new("file.new"), "新建").english("New")]);
    bar.menus[0].english = Some("File".to_owned());
    let bar = Tracked::new(bar);
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar).show(ui);
    });
    // A Mac bar has no letters.
    fixture.os(Os::Linux).host(host_in("zh"));
    fixture.settle();
    assert!(fixture.by_label("文件(F)").is_some(), "the title shows the English letter after it");
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("新建(N)").is_some(), "Alt+F opens the title's menu, whose row shows its own English letter");
}

#[trial]
fn a_cyrillic_label_hides_its_english_letter_and_the_letter_still_opens_it() {
    let recent = Node::submenu("Последние", vec![Item::new("file.open", "Открыть").english("Open").into()]).english("Open Recent");
    let bar = Tracked::new(MenuBar::new(vec![Menu::new("Файл", vec![recent]).english("File")]));
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar).show(ui);
    });
    fixture.os(Os::Linux).host(host_in("ru"));
    fixture.settle();
    assert!(fixture.by_label("Файл").is_some() && fixture.by_label("Файл(F)").is_none(), "the title shows as written");
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("Последние").is_some(), "Alt+F opens the title by its hidden English letter");
    assert!(fixture.by_label("Последние(O)").is_none(), "the row shows as written");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    let title = fixture.by_label("Файл").expect("the title");
    fixture.click(&title).settle();
    // The O key on a Russian layout types щ.
    fixture.type_key(Key::O, "щ");
    fixture.settle();
    assert!(fixture.by_label("Открыть").is_some(), "O opens the submenu by its hidden English letter");
}

#[trial]
fn help_search_lists_an_item_by_its_english_name_in_a_translated_model() {
    let bar = from_items(&["文件"], &[Row::new(&["文件"], CommandRef::new("file.new"), "新建").english("New")]);
    let entries = bar.help_entries(Target::Linux, &LayoutOptions::new("Sample"));
    let entry = entries.iter().find(|entry| *entry.command() == CommandRef::new("file.new")).expect("the command is listed");
    assert_eq!(entry.english().map(|label| label.as_str()), Some("New"));
}

/// A host whose interface language is `code`, with an empty catalog.
pub(crate) fn host_in(code: &str) -> Arc<Host> {
    let (i18n, _) = craft_i18n::I18n::from_owned([(code.to_owned(), String::new(), String::new())], &[code]);
    let lang = i18n.from_code(code).unwrap_or_else(|| panic!("{code} is offered"));
    Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).lang(lang).build())
}

/// The rows shown after Alt with `key` on a Linux bar titled `ix` and `ı` under a Turkish host: `One` is `ix`'s, `Two`
/// is `ı`'s.
fn opened_by(key: Key) -> (bool, bool) {
    let bar = Tracked::new(from_items(
        &["ix", "ı"],
        &[Row::new(&["ix"], CommandRef::new("menu.one"), "One"), Row::new(&["ı"], CommandRef::new("menu.two"), "Two")],
    ));
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar).show(ui);
    });
    // A Mac bar has no letters; the host gives the Turkish folder.
    fixture.os(Os::Linux).host(host_in("tr"));
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
