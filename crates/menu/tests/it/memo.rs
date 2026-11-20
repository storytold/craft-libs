//! The in-window bar is a memoized view: idle frames replay it, a title rename or Alt runs it once, and an open menu
//! runs it while it is open.

use std::cell::RefCell;
use std::rc::Rc;

use craft_host::target::Os;
use craft_menu::{CommandRef, MenuBar, MenuBarUi, Row, Target, from_items};
use craft_ui::testing::{Fixture, MemoRun, audit_replays, last_frame_memo_runs, trial};
use egui::{Event, Key, Modifiers};

fn model(title: &str) -> MenuBar {
    from_items(&[title, "Edit"], &[Row::new(&[title], CommandRef::new("file.new"), "New"), Row::new(&["Edit"], CommandRef::new("edit.undo"), "Undo")])
}

fn fixture(bar: &Rc<RefCell<MenuBar>>) -> Fixture {
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

    *bar.borrow_mut() = model("Document");
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

/// A menu the bar opens is an overlay inside its run, which keeps the run from being replayed while it is open. The
/// run in which the menu's fade ends is the last that shows it, so the replays after it match a real run (the audit).
#[trial]
fn an_open_menu_runs_the_bar_and_closing_it_replays_it() {
    let bar = Rc::new(RefCell::new(model("File")));
    let mut fixture = fixture(&bar);
    audit_replays(fixture.ctx(), true);
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "Alt+F opened File");
    fixture.frame(vec![]);
    assert!(counted(&fixture).0 > 0, "the bar runs while a menu is open");
    // The first Escape closes the menu; the bar keeps the keyboard until the second one hands it back.
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    fixture.frame(vec![]);
    assert_eq!(counted(&fixture), (0, 1), "after the menu closes and the keyboard leaves, the bar replays");
}
