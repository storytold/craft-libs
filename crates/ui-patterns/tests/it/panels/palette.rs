//! The palette lists the app's commands as the window under it has them, and runs the one picked: its own modal
//! layer closes before the command runs, so it blocks none of them. A palette without a `recents` binding still
//! remembers what it ran.

use craft_ui::contract::{Bind, CommandSet, Overlay, Role, ShowContainer as _};
use craft_ui::menu::search::HelpEntry;
use craft_ui::patterns::panels::CommandPalette;
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::{CommandRef, CommandState, EditCommands, FireSource};
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// App commands with the default modal rule: blocked while a dialog is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "paletteTrial")]
#[non_exhaustive]
enum TrialCommand {
    #[command(label = "Rotate")]
    Rotate,
    #[command(label = "Flip")]
    Flip,
    #[command(label = "Crop")]
    Crop,
    #[command(label = "Zoom", modal = "always")]
    Zoom,
}

type PaletteRig = (Fixture, Rc<Cell<bool>>, Rc<RefCell<Option<CommandRef>>>);

/// A palette open over `entries`, inside a modal dialog when `in_dialog`; `picked` holds the last pick.
fn palette(entries: Vec<HelpEntry>, in_dialog: bool) -> PaletteRig {
    let open = Rc::new(Cell::new(true));
    let picked = Rc::new(RefCell::new(None::<CommandRef>));
    let (shown_open, shown_picked) = (open.clone(), picked.clone());
    let fixture = Fixture::new(move |ui| {
        let show = |ui: &mut Ui| {
            let mut is_open = shown_open.get();
            let output = CommandPalette::new("Commands", &entries, Bind::bound(&mut is_open)).show(ui);
            shown_open.set(is_open);
            if let Some(command) = output.picked {
                *shown_picked.borrow_mut() = Some(command);
            }
        };
        if in_dialog {
            Popover::new("Levels", Bind::given(true)).overlay(|_| Overlay::modal()).show(ui, |ui, _| show(ui));
        } else {
            show(ui);
        }
    });
    (fixture, open, picked)
}

/// The listed rows, by label: whether each is disabled.
fn rows(fixture: &Fixture) -> Vec<(String, bool)> {
    let tree = fixture.tree();
    fixture
        .by_role(Role::LIST_BOX_OPTION)
        .iter()
        .map(|row| (tree.name_of(row.node).unwrap_or_default().to_owned(), tree.is_disabled_key(row.node)))
        .collect()
}

fn disabled(fixture: &Fixture, label: &str) -> bool {
    rows(fixture).into_iter().find(|(name, _)| name.starts_with(label)).unwrap_or_else(|| panic!("no row {label}")).1
}

#[trial]
fn a_picked_command_runs_from_the_palette_and_comes_first_when_it_opens_again() {
    let entries = vec![
        HelpEntry::new(TrialCommand::Rotate, "Rotate").with_path(["Image"]),
        HelpEntry::new(TrialCommand::Flip, "Flip").with_path(["Image"]),
        HelpEntry::new(TrialCommand::Crop, "Crop").with_path(["Image"]),
    ];
    let (mut fixture, open, picked) = palette(entries, false);
    fixture.settle();
    assert!(rows(&fixture).iter().all(|(_, disabled)| !disabled), "the palette's own layer blocks no app command: {:?}", rows(&fixture));
    assert!(rows(&fixture)[0].0.starts_with("Rotate"), "with nothing run, the entries come in their order");
    let _ = take_output(fixture.ctx());

    fixture.type_text("Flip").settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*picked.borrow(), Some(CommandRef::from(TrialCommand::Flip)), "Enter picked the match");
    assert!(!open.get(), "picking closed the palette");
    let fired = take_output(fixture.ctx()).fired;
    assert!(
        fired.iter().any(|fired| fired.command == CommandRef::from(TrialCommand::Flip) && fired.source == FireSource::Palette),
        "the palette ran the pick: {fired:?}"
    );

    open.set(true);
    fixture.settle();
    assert!(rows(&fixture)[0].0.starts_with("Flip"), "the command just run leads the empty query");
}

#[trial]
fn a_command_the_app_disables_lists_disabled_and_does_not_run_even_after_typing_into_the_field() {
    let entries = vec![HelpEntry::new(TrialCommand::Rotate, "Rotate"), HelpEntry::new(EditCommands::Undo, "Undo").with_path(["Edit"])];
    let (mut fixture, open, picked) = palette(entries, false);
    fixture.command_state(EditCommands::Undo, CommandState::new().enabled(false));
    fixture.settle();
    assert!(!disabled(&fixture, "Rotate"), "an enabled app command lists enabled");
    assert!(disabled(&fixture, "Undo"), "the app's disabled Undo lists disabled");

    // The typing gives the search field an undo of its own; the row stays the app's.
    fixture.type_text("undo").settle();
    assert!(disabled(&fixture, "Undo"), "the Undo row follows the app, not the search field");
    let _ = take_output(fixture.ctx());
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*picked.borrow(), None, "Enter on a disabled row picks nothing");
    assert!(open.get(), "the palette stays open");
    assert!(take_output(fixture.ctx()).fired.is_empty(), "nothing ran");
}

#[trial]
fn a_palette_opened_in_a_modal_dialog_lists_what_the_dialog_blocks_as_disabled() {
    let entries = vec![HelpEntry::new(TrialCommand::Rotate, "Rotate"), HelpEntry::new(TrialCommand::Zoom, "Zoom")];
    let (mut fixture, ..) = palette(entries, true);
    fixture.settle();
    assert!(disabled(&fixture, "Rotate"), "the dialog under the palette blocks a default-rule app command");
    assert!(!disabled(&fixture, "Zoom"), "a command that runs whatever is open stays enabled");
}
