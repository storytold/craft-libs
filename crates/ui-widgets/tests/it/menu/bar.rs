//! The menu bar: what the help search finds.

use craft_host::target::Os;
use craft_i18n::Direction;
use craft_ui::contract::Show as _;
use craft_ui::menu::MenuItem;
use craft_ui::menu::bar::{MenuBar, MenuHelp, MenuTitle};
use craft_ui::menu::search::HelpEntry;
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

/// A menu bar with File, Edit and Help.
struct Rig {
    fixture: Fixture,
}

fn entries() -> Vec<HelpEntry> {
    // The help rows show their command's label, so each entry has its own command.
    vec![
        HelpEntry::new(EditCommands::Paste, "Café").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Copy, "Copy").with_english("Duplicate").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Cut, "Cut").with_path(["Edit", "Clipboard"]),
    ]
}

fn rig(direction: Direction) -> Rig {
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![
            MenuTitle::new("&File", |ui| {
                MenuItem::new("&New").show(ui);
                MenuItem::new("&Save").show(ui);
            }),
            MenuTitle::new("&Edit", |ui| {
                MenuItem::new("Cu&t").show(ui);
            }),
            MenuTitle::new("&Help", |ui| {
                MenuItem::new("&About").show(ui);
            }),
        ];
        MenuBar::new("Main menu", titles).help(MenuHelp::new(2, &entries)).show(ui);
    });
    // The in-window menu bar and its Alt, F10 and letter keys are PC behaviour: a Mac app's menu bar is native.
    fixture.os(Os::Linux);
    fixture.direction(direction).settle();
    Rig { fixture }
}

impl Rig {
    fn open(&self, label: &str) -> bool {
        self.fixture.by_label(label).is_some()
    }
}

#[trial]
fn help_search_ignores_accents_and_matches_the_english_label_and_the_path() {
    let mut rig = rig(Direction::Ltr);
    let help = rig.fixture.by_label("Help").expect("the Help title");
    rig.fixture.click(&help);
    rig.fixture.settle();
    let search = rig.fixture.by_label("Search menus").expect("the search field");
    rig.fixture.click(&search);
    for (typed, found, absent) in [("cafe", "Paste", "Copy"), ("duplic", "Copy", "Paste"), ("clipboard", "Cut", "Copy")] {
        rig.fixture.key(Key::A, Modifiers::COMMAND);
        rig.fixture.type_text(typed);
        rig.fixture.settle();
        assert!(rig.open(found), "{typed:?} finds the entry whose command row reads {found:?}");
        assert!(!rig.open(absent), "{typed:?} leaves {absent:?} out");
    }
}

/// A menu a title click opened takes the letter a row marks: the row runs and the run closes the menu.
#[trial]
fn a_letter_runs_the_row_it_marks_in_a_menu_a_title_click_opened() {
    use std::cell::Cell;
    use std::rc::Rc;
    let saved = Rc::new(Cell::new(0));
    let seen = saved.clone();
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&File", |ui| {
            MenuItem::new("&New").show(ui);
            if MenuItem::new("&Save").show(ui).ran {
                seen.set(seen.get() + 1);
            }
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("Save").is_some(), "File is open");
    fixture.type_key(Key::S, "s");
    fixture.settle();
    assert_eq!(saved.get(), 1, "the letter runs the row it marks");
    assert!(fixture.by_label("Save").is_none(), "and the run closes the menu");
}

/// The pass a pointer move over a menu's rows runs paints the row under the pointer as the active row and no other,
/// moving down or up, at 1x and at a fractional scale. The two frames egui runs after any frame with input change
/// nothing, and then the menu asks for none.
#[trial]
fn a_pointer_move_over_menu_rows_lights_the_row_under_it_in_that_frame() {
    use craft_behavior::MotionMode;
    use craft_ui::runtime::env::Scale;
    use craft_ui::testing::pointer;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    const ROWS: usize = 8;
    let names: Vec<String> = (1..=ROWS).map(|at| format!("Item {at:02}")).collect();
    for scale in [1.0, 1.5] {
        let lit: Rc<RefCell<HashMap<String, bool>>> = Rc::default();
        let seen = lit.clone();
        let rows = names.clone();
        let mut fixture = Fixture::new(move |ui| {
            let seen = seen.clone();
            let rows = rows.clone();
            let titles = vec![MenuTitle::new("&File", move |ui| {
                for name in &rows {
                    let active = MenuItem::new(name.clone()).show(ui).active;
                    // The last run of the frame is the one that paints.
                    seen.borrow_mut().insert(name.clone(), active);
                }
            })];
            MenuBar::new("Main menu", titles).show(ui);
        });
        fixture.os(Os::Linux);
        fixture.room(egui::vec2(480.0, 320.0)).scale(Scale::new(scale)).motion(MotionMode::Off).settle();
        let file = fixture.by_label("File").expect("the File title");
        fixture.click(&file);
        fixture.settle();
        let shown: Vec<(String, egui::Pos2)> =
            names.iter().map(|name| (name.clone(), fixture.by_label(name).expect("the File menu shows every row").center())).collect();
        let downward = shown.iter();
        let upward = shown.iter().rev().skip(1);
        for (name, center) in downward.chain(upward) {
            lit.borrow_mut().clear();
            fixture.frame(vec![pointer::moved(*center)]);
            let active: Vec<String> = lit.borrow().iter().filter(|(_, on)| **on).map(|(row, _)| row.clone()).collect();
            assert_eq!(active, vec![name.clone()], "at {scale}x the move frame onto {name} lights it and no other row");
            // egui repaints twice for a frame with input; the menu asks for nothing beyond that.
            for _ in 0..2 {
                lit.borrow_mut().clear();
                fixture.frame(Vec::new());
                assert_eq!(lit.borrow().get(name), Some(&true), "at {scale}x {name} stays lit with the pointer at rest");
            }
            assert!(!fixture.repaint_requested(), "at {scale}x a pointer at rest on {name} asks for no frame");
        }
    }
}

/// A help search over built entries builds them only in a pass that shows the Help menu: a bar whose menus stay closed
/// never runs the builder, an open File menu doesn't either, and the open Help menu searches what it built.
#[trial]
fn built_help_entries_are_collected_only_while_the_help_menu_shows() {
    use std::cell::Cell;
    use std::rc::Rc;
    let builds = Rc::new(Cell::new(0u32));
    let counted = builds.clone();
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![
            MenuTitle::new("&File", |ui| {
                MenuItem::new("&New").show(ui);
            }),
            MenuTitle::new("&Help", |ui| {
                MenuItem::new("&About").show(ui);
            }),
        ];
        let counted = counted.clone();
        let build = move || {
            counted.set(counted.get() + 1);
            entries()
        };
        MenuBar::new("Main menu", titles).help(MenuHelp::new(1, &build)).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    fixture.settle();
    assert_eq!(builds.get(), 0, "closed menus build nothing");

    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "File is open");
    assert_eq!(builds.get(), 0, "another open menu builds nothing");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();

    let help = fixture.by_label("Help").expect("the Help title");
    fixture.click(&help);
    fixture.settle();
    assert!(builds.get() > 0, "the open Help menu builds its entries");
    let search = fixture.by_label("Search menus").expect("the search field");
    fixture.click(&search);
    fixture.type_text("duplic");
    fixture.settle();
    assert!(fixture.by_label("Copy").is_some(), "the search finds a built entry by its English label");
}

/// The level keeps its letters between passes and assigns them again when the rows' labels change: a row that read
/// Save when the menu last opened and reads Open now takes O, so typing O runs it.
#[trial]
fn a_row_whose_label_changed_since_the_menu_last_opened_takes_its_new_letter() {
    use std::cell::Cell;
    use std::rc::Rc;
    let renamed = Rc::new(Cell::new(false));
    let runs = Rc::new(Cell::new(0));
    let (label, ran) = (renamed.clone(), runs.clone());
    let mut fixture = Fixture::new(move |ui| {
        let (label, ran) = (label.clone(), ran.clone());
        let titles = vec![MenuTitle::new("&File", move |ui| {
            MenuItem::new("Alpha").show(ui);
            MenuItem::new("Beta").show(ui);
            if MenuItem::new(if label.get() { "Open" } else { "Save" }).show(ui).ran {
                ran.set(ran.get() + 1);
            }
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("Save").is_some(), "File is open");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Save").is_none(), "File is closed");

    renamed.set(true);
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("Open").is_some(), "File is open again, the row reads Open");
    fixture.type_key(Key::O, "o");
    fixture.settle();
    assert_eq!(runs.get(), 1, "O runs the row that reads Open");
}

/// A menu bar that settles (in a title bar slot with a content version) still takes its keys while it replays its
/// last run: F10 takes the focus to the bar, and Alt with a title's marked letter opens that menu.
#[trial]
fn a_settled_menu_bar_takes_its_mnemonics_and_f10() {
    use craft_ui::contract::Lazy;
    let mut fixture = Fixture::new(|ui| {
        Lazy::new("title", 0.0).settles(true).content(1).show(ui, |ui| {
            let titles = vec![MenuTitle::new("&File", |ui| {
                MenuItem::new("&New").show(ui);
            })];
            MenuBar::new("Main menu", titles).show(ui);
        });
    });
    fixture.os(Os::Linux);
    fixture.settle().idle(2);
    assert!(fixture.focused_stop().is_none(), "nothing holds the focus at the start");
    fixture.key(Key::F10, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.focused_stop().is_some(), "F10 takes the focus to the bar");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle().idle(2);
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "Alt+F opens File");
}
