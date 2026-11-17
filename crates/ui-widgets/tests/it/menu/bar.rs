//! The menu bar: what the help search finds.

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
        let help = entries();
        MenuBar::new("Main menu", titles).help(MenuHelp::new(2, &help)).show(ui);
    });
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
