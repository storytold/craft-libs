//! The in-window bar is a memoized view: idle frames replay it, a title rename or Alt runs it once, and an open menu
//! replays with it while nothing reaches the menu.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_host::target::Os;
use craft_keymap::Overrides;
use craft_menu::{CommandRef, MenuBar, MenuBarUi, Row, Target, from_items};
use craft_ui::contract::Tracked;
use craft_ui::testing::{Fixture, MemoRun, audit_replays, last_frame_derived_builds, last_frame_memo_runs, last_pass_memo_key_bytes, trial};
use egui::{Event, Key, Modifiers};

fn model(title: &str) -> Tracked<MenuBar> {
    Tracked::new(from_items(
        &[title, "Edit"],
        &[Row::new(&[title], CommandRef::new("file.new"), "New"), Row::new(&["Edit"], CommandRef::new("edit.undo"), "Undo")],
    ))
}

fn fixture(bar: &Rc<RefCell<Tracked<MenuBar>>>) -> Fixture {
    let bar = Rc::clone(bar);
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar.borrow()).target(Target::Linux).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    fixture
}

/// The bar's runs and replays in the last frame.
fn counted(fixture: &Fixture) -> (usize, usize) {
    let runs = last_frame_memo_runs(fixture.ctx());
    let of = |kind: MemoRun| runs.iter().filter(|(name, _, run)| name.contains("BarProps") && *run == kind).count();
    (of(MemoRun::Ran), of(MemoRun::Replayed))
}

#[trial]
fn an_idle_menu_bar_replays_and_a_title_rename_runs_it_once() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    for _ in 0..4 {
        fixture.frame(vec![]);
        assert_eq!(counted(&fixture), (0, 1), "an idle frame replays the bar");
    }
    assert!(fixture.by_label("File").is_some(), "a replayed bar keeps its titles' nodes");

    bar.borrow_mut().edit(|bar| *bar = model("Document").into_inner());
    fixture.frame(vec![]);
    assert_eq!(counted(&fixture).0, 1, "a model change runs the bar once in that frame");
    assert!(fixture.by_label("Document").is_some(), "and it shows the new title");
    fixture.frame(vec![]);
    assert_eq!(counted(&fixture), (0, 1), "then it settles again");
}

/// Holding Alt underlines the titles' mnemonics: the bar runs once when Alt goes down and once when it goes up.
#[trial]
fn alt_runs_the_menu_bar_once_each_way() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    fixture.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
    assert_eq!(counted(&fixture).0, 1, "Alt down runs the bar once");
    fixture.frame(vec![]);
    assert_eq!(counted(&fixture), (0, 1), "holding Alt replays it");
    fixture.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(counted(&fixture).0, 1, "Alt up runs the bar once");
}

/// A menu the bar opens is an overlay of its run: with the focus on the menu's first row, an idle frame replays the bar
/// and the open menu, and a key the menu takes runs the bar in its frame.
#[trial]
fn an_idle_open_menu_replays_and_a_key_in_it_runs_the_bar() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    audit_replays(fixture.ctx(), true);
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "Alt+F opened File");
    for _ in 0..3 {
        fixture.frame(vec![]);
        assert_eq!(counted(&fixture), (0, 1), "an idle frame replays the bar and its open menu");
        assert!(fixture.by_label("New").is_some(), "the replayed menu keeps its rows' nodes");
    }
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    assert!(counted(&fixture).0 > 0, "a key in the open menu runs the bar in its frame");
    fixture.settle();
    fixture.frame(vec![]);
    assert_eq!(counted(&fixture), (0, 1), "then the bar replays again");
    // The first Escape closes the menu; the bar keeps the keyboard until the second one hands it back.
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    fixture.frame(vec![]);
    assert_eq!(counted(&fixture), (0, 1), "after the menu closes and the keyboard leaves, the bar replays");
}

/// The label builds of the last frame.
fn label_builds(fixture: &Fixture) -> u32 {
    last_frame_derived_builds(fixture.ctx()).iter().map(|(_, builds)| builds).sum()
}

#[trial]
fn an_idle_bar_and_an_open_menu_build_no_labels() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    for _ in 0..3 {
        fixture.frame(vec![]);
        assert_eq!(label_builds(&fixture), 0, "an idle bar builds no labels");
    }
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "Alt+F opened File");
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    assert!(counted(&fixture).0 > 0, "a key in the open menu runs the bar");
    assert_eq!(label_builds(&fixture), 0, "and builds no labels");
    for _ in 0..3 {
        fixture.frame(vec![]);
        assert_eq!(label_builds(&fixture), 0, "an idle open menu builds no labels");
    }
}

#[trial]
fn a_model_edit_builds_the_labels_once() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    bar.borrow_mut().edit(|bar| *bar = model("Document").into_inner());
    fixture.frame(vec![]);
    assert_eq!(label_builds(&fixture), 1, "one edit builds the labels once");
    fixture.frame(vec![]);
    assert_eq!(label_builds(&fixture), 0, "and the next frame none");
}

#[trial]
fn a_locale_change_builds_the_labels_once() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    fixture.host(crate::mnemonic::host_in("tr"));
    fixture.frame(vec![]);
    assert_eq!(label_builds(&fixture), 1, "a language change builds the labels once");
    fixture.frame(vec![]);
    assert_eq!(label_builds(&fixture), 0, "and the next frame none");
}

#[trial]
fn a_keymap_change_builds_the_labels_once() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    fixture.overrides(Arc::new(Overrides::default()));
    fixture.frame(vec![]);
    assert_eq!(label_builds(&fixture), 1, "a new keymap builds the labels once");
    fixture.frame(vec![]);
    assert_eq!(label_builds(&fixture), 0, "and the next frame none");
}

/// The key bytes of an idle pass of a bar whose File menu has `rows` rows.
fn idle_key_bytes(rows: usize) -> usize {
    let items: Vec<Row> = (0..rows).map(|row| Row::new(&["File"], CommandRef::new(format!("file.row{row}")), format!("Row {row}"))).collect();
    let bar = Rc::new(RefCell::new(Tracked::new(from_items(&["File"], &items))));
    let mut fixture = fixture(&bar);
    fixture.frame(vec![]);
    assert_eq!(label_builds(&fixture), 0, "an idle bar builds no labels");
    last_pass_memo_key_bytes(fixture.ctx())
}

/// An idle bar feeds the same key bytes whatever its model's size.
#[trial]
fn a_menu_bars_idle_key_bytes_do_not_grow_with_its_rows() {
    assert_eq!(idle_key_bytes(20), idle_key_bytes(2_000), "the key follows the model's revision, not its rows");
}
