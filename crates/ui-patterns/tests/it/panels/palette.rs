//! A palette without a `recents` binding still remembers what it ran.

use craft_ui::contract::{Bind, CommandSet, Role};
use craft_ui::menu::search::HelpEntry;
use craft_ui::patterns::panels::CommandPalette;
use craft_ui::runtime::commands::CommandRef;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Commands that may run while the palette's modal layer is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "paletteTrial")]
#[non_exhaustive]
enum TrialCommand {
    #[command(label = "Rotate", modal = "always")]
    Rotate,
    #[command(label = "Flip", modal = "always")]
    Flip,
    #[command(label = "Crop", modal = "always")]
    Crop,
}

#[trial]
fn a_picked_command_comes_first_when_the_palette_opens_again() {
    let entries = vec![
        HelpEntry::new(TrialCommand::Rotate, "Rotate").with_path(["Image"]),
        HelpEntry::new(TrialCommand::Flip, "Flip").with_path(["Image"]),
        HelpEntry::new(TrialCommand::Crop, "Crop").with_path(["Image"]),
    ];
    let open = Rc::new(Cell::new(true));
    let picked = Rc::new(RefCell::new(None::<CommandRef>));
    let (shown_open, shown_picked) = (open.clone(), picked.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut is_open = shown_open.get();
        let output = CommandPalette::new("Commands", &entries, Bind::bound(&mut is_open)).show(ui);
        shown_open.set(is_open);
        if let Some(command) = output.picked {
            *shown_picked.borrow_mut() = Some(command);
        }
    });
    fixture.settle();
    let first_row = |fixture: &Fixture| {
        let rows = fixture.by_role(Role::LIST_BOX_OPTION);
        rows.first().and_then(|row| fixture.tree().name_of(row.node)).map(str::to_owned).unwrap_or_default()
    };
    assert!(first_row(&fixture).starts_with("Rotate"), "with nothing run, the entries come in their order");

    fixture.type_text("Flip").settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*picked.borrow(), Some(CommandRef::from(TrialCommand::Flip)), "Enter picked the match");
    assert!(!open.get(), "picking closed the palette");

    open.set(true);
    fixture.settle();
    assert!(first_row(&fixture).starts_with("Flip"), "the command just run leads the empty query");
}
