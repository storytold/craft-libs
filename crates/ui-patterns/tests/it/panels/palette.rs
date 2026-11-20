//! The palette lists the app's commands as the window under it has them, and runs the one picked: its own modal
//! layer closes before the command runs, so it blocks none of them. A palette without a `recents` binding still
//! remembers what it ran.

use craft_ui::contract::{Bind, CommandSet, Overlay, Role, ShowContainer as _};
use craft_ui::menu::search::HelpEntry;
use craft_ui::patterns::panels::{CommandPalette, PaletteFace};
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::{CommandRef, CommandState, EditCommands, FireSource};
use craft_ui::runtime::env::{Label, WordTemplate};
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::{Fixture, Registry, trial};
use egui::{Key, Modifiers, Pos2, Ui};
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

/// The palette story's `commands` function reaches its first frame: Redo lists disabled, a command it does not name
/// lists enabled.
#[trial]
fn the_palette_story_runs_under_its_command_states() {
    let registry = Registry::of(&[craft_ui::testing::register]);
    let entry = registry.stories().iter().find(|entry| entry.story.name == "command_palette").expect("the palette story");
    let mut fixture = Fixture::story(&entry.story);
    fixture.settle();
    assert!(disabled(&fixture, "Redo"), "the story's Redo lists disabled");
    assert!(!disabled(&fixture, "Search"), "a command the story does not name lists enabled");
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

/// A palette of `face` over `entries`, open from the start or not; `picked` and `handed` hold the last pick and the
/// last query the fallback row handed back.
type FacedRig = (Fixture, Rc<Cell<bool>>, Rc<RefCell<Option<CommandRef>>>, Rc<RefCell<Option<String>>>);

fn faced(
    face: PaletteFace,
    entries: Vec<HelpEntry>,
    suggested: Option<(Label, Vec<CommandRef>)>,
    fallback: Option<WordTemplate>,
    start_open: bool,
) -> FacedRig {
    let open = Rc::new(Cell::new(start_open));
    let picked = Rc::new(RefCell::new(None::<CommandRef>));
    let handed = Rc::new(RefCell::new(None::<String>));
    let (shown_open, shown_picked, shown_handed) = (open.clone(), picked.clone(), handed.clone());
    let fixture = Fixture::new(move |ui| {
        let mut is_open = shown_open.get();
        let mut palette = CommandPalette::new("Commands", &entries, Bind::bound(&mut is_open)).face(face).placeholder(Label::from("Search"));
        if let Some(suggested) = suggested.clone() {
            palette = palette.suggested(suggested);
        }
        if let Some(fallback) = fallback.clone() {
            palette = palette.fallback(fallback);
        }
        let output = palette.show(ui);
        shown_open.set(is_open);
        if let Some(command) = output.picked {
            *shown_picked.borrow_mut() = Some(command);
        }
        if let Some(query) = output.fallback {
            *shown_handed.borrow_mut() = Some(query);
        }
    });
    (fixture, open, picked, handed)
}

fn trial_entries() -> Vec<HelpEntry> {
    vec![
        HelpEntry::new(TrialCommand::Rotate, "Rotate").with_path(["Image"]),
        HelpEntry::new(TrialCommand::Flip, "Flip").with_path(["Image"]),
        HelpEntry::new(TrialCommand::Crop, "Crop").with_path(["Image"]),
    ]
}

fn find_template() -> WordTemplate {
    WordTemplate::new("Find “{text}” in the document")
}

#[trial]
fn keys_of_the_opening_pass_go_into_the_field_and_enter_is_used_up() {
    let (mut fixture, open, picked, handed) = faced(PaletteFace::InPlace, trial_entries(), None, None, false);
    fixture.settle();
    assert!(rows(&fixture).is_empty(), "the closed face lists nothing");

    // The first typed letter arrives in the pass that opens the face: a letter lost would leave every row listed.
    open.set(true);
    fixture.type_text("f").settle();
    let listed = rows(&fixture);
    assert_eq!(listed.len(), 1, "the letter typed in the opening pass reached the field: {listed:?}");
    assert!(listed[0].0.starts_with("Flip"), "{listed:?}");

    let _ = take_output(fixture.ctx());
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*picked.borrow(), Some(CommandRef::from(TrialCommand::Flip)), "Enter picked the first row");
    assert_eq!(*handed.borrow(), None, "no fallback was configured");
    assert!(!open.get(), "picking closed the face");
    let fired = take_output(fixture.ctx()).fired;
    assert_eq!(fired.len(), 1, "Enter was used up by the list and ran nothing else: {fired:?}");
    assert_eq!(fired[0].command, CommandRef::from(TrialCommand::Flip));
}

#[trial]
fn escape_and_a_press_outside_close_the_in_place_face_and_the_fallback_row_carries_the_query() {
    let (mut fixture, open, picked, handed) = faced(PaletteFace::InPlace, trial_entries(), None, Some(find_template()), false);
    fixture.settle();

    open.set(true);
    fixture.settle();
    assert!(open.get(), "the face is open");
    // Escape clears a typed query first and closes only on an empty field.
    fixture.type_text("ro").settle();
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(open.get(), "Escape on a typed query only clears it");
    assert_eq!(rows(&fixture).len(), 3, "the cleared field lists every entry again: {:?}", rows(&fixture));
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(!open.get(), "Escape on an empty field closes it");

    open.set(true);
    fixture.settle();
    let below = fixture.by_role(Role::LIST_BOX_OPTION).pop().expect("a listed row").rect;
    fixture.click_at(Pos2::new(below.center().x, below.bottom() + 40.0)).settle();
    assert!(!open.get(), "a press outside closes it");

    // With nothing matching, the fallback row stands alone and Enter takes it.
    open.set(true);
    fixture.settle();
    fixture.type_text("zzz").settle();
    let listed = rows(&fixture);
    assert_eq!(listed.len(), 1, "only the fallback row: {listed:?}");
    assert!(listed[0].0.contains("zzz"), "the fallback row carries the query: {listed:?}");
    let _ = take_output(fixture.ctx());
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*handed.borrow(), Some("zzz".to_owned()), "Enter handed the query back");
    assert_eq!(*picked.borrow(), None, "the fallback picks no command");
    assert!(!open.get(), "the fallback closed the face");
    assert!(take_output(fixture.ctx()).fired.is_empty(), "no command ran");

    // With a match, the fallback row is the last one and a click on it hands the query back.
    *handed.borrow_mut() = None;
    open.set(true);
    fixture.settle();
    fixture.type_text("flip").settle();
    let listed = rows(&fixture);
    assert_eq!(listed.len(), 2, "the match and the fallback row: {listed:?}");
    assert!(listed[0].0.starts_with("Flip"), "{listed:?}");
    assert!(listed[1].0.contains("Find “flip” in the document"), "{listed:?}");
    let last = fixture.by_role(Role::LIST_BOX_OPTION).pop().expect("the fallback row");
    fixture.click(&last).settle();
    assert_eq!(*handed.borrow(), Some("flip".to_owned()), "the click handed the query back");
    assert_eq!(*picked.borrow(), None, "the fallback picks no command");
    assert!(!open.get());
}

#[trial]
fn an_empty_query_lists_the_suggested_commands_and_a_query_lists_matches() {
    for face in [PaletteFace::Dialog, PaletteFace::InPlace] {
        let suggested = (Label::from("Suggested"), vec![CommandRef::from(TrialCommand::Crop), CommandRef::from(TrialCommand::Flip)]);
        let (mut fixture, open, ..) = faced(face, trial_entries(), Some(suggested), None, face == PaletteFace::Dialog);
        fixture.settle();
        open.set(true);
        fixture.settle();
        let names: Vec<String> = rows(&fixture).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names.len(), 2, "{face:?}: only the suggested commands list: {names:?}");
        assert!(names[0].starts_with("Crop") && names[1].starts_with("Flip"), "{face:?}: in the suggested order: {names:?}");

        fixture.type_text("rot").settle();
        let names: Vec<String> = rows(&fixture).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names.len(), 1, "{face:?}: a query lists its matches only: {names:?}");
        assert!(names[0].starts_with("Rotate"), "{face:?}: {names:?}");
    }
}

#[trial]
fn a_palette_opened_in_a_modal_dialog_lists_what_the_dialog_blocks_as_disabled() {
    let entries = vec![HelpEntry::new(TrialCommand::Rotate, "Rotate"), HelpEntry::new(TrialCommand::Zoom, "Zoom")];
    let (mut fixture, ..) = palette(entries, true);
    fixture.settle();
    assert!(disabled(&fixture, "Rotate"), "the dialog under the palette blocks a default-rule app command");
    assert!(!disabled(&fixture, "Zoom"), "a command that runs whatever is open stays enabled");
}

/// The in-place face's field is the palette's face in the window (a title-bar search box), so a screen reader names it
/// by the palette's label, closed and open; the generic "Search commands" would drop the app's own name for it.
#[trial]
fn the_in_place_field_is_named_by_the_palettes_label_closed_and_open() {
    let (mut fixture, open, ..) = faced(PaletteFace::InPlace, trial_entries(), None, None, false);
    fixture.settle();
    for state in ["closed", "open"] {
        let tree = fixture.tree();
        let names: Vec<String> =
            fixture.by_role(Role::TEXT_INPUT).iter().map(|field| tree.name_of(field.node).unwrap_or_default().to_owned()).collect();
        assert_eq!(names, vec!["Commands".to_owned()], "{state}: one search field, named by the palette's label");
        open.set(true);
        fixture.settle();
    }
}
