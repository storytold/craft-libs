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
