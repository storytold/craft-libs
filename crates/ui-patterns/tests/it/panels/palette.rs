//! The palette lists the app's commands as the window under it has them, and runs the one picked: its own modal
//! layer closes before the command runs, so it blocks none of them. A palette without a `recents` binding still
//! remembers what it ran.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CommandSet, Overlay, Role, Show as _, ShowContainer as _, clicked};
use craft_ui::menu::search::{HelpEntry, SearchIndex, SearchWords, Synonyms};
use craft_ui::patterns::panels::{CommandPalette, PaletteFace};
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::{self, CommandRef, CommandState, EditCommands, FireSource};
use craft_ui::runtime::env::{Label, WordTemplate};
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::{Fixture, Registry, trial};
use egui::{Key, Modifiers, Pos2, Ui, Vec2};
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

/// A palette over `entries` and what the trial reads of it.
struct Rig {
    fixture: Fixture,
    open: Rc<Cell<bool>>,
    /// The last command picked.
    picked: Rc<RefCell<Option<CommandRef>>>,
    /// The last disabled command Enter was refused on.
    refused: Rc<RefCell<Option<CommandRef>>>,
    /// The bound `recents`, newest first; empty and unused when the rig keeps them in the palette.
    recents: Rc<RefCell<Vec<CommandRef>>>,
}

/// A palette open over `entries`, inside a modal dialog when `in_dialog`, with `beneath` shown before it in the same
/// window. `recents` binds the palette's recent commands to the rig when given; else the palette keeps them.
fn rig(entries: Vec<HelpEntry>, recents: Option<Vec<CommandRef>>, in_dialog: bool, beneath: impl Fn(&mut Ui) + 'static) -> Rig {
    let open = Rc::new(Cell::new(true));
    let picked = Rc::new(RefCell::new(None::<CommandRef>));
    let refused = Rc::new(RefCell::new(None::<CommandRef>));
    let bound = recents.is_some();
    let held = Rc::new(RefCell::new(recents.unwrap_or_default()));
    let (shown_open, shown_picked, shown_refused, shown_held) = (open.clone(), picked.clone(), refused.clone(), held.clone());
    let fixture = Fixture::new(move |ui| {
        beneath(ui);
        let show = |ui: &mut Ui| {
            let mut is_open = shown_open.get();
            let env = craft_ui::runtime::env::Env::of(ui);
            let view = env.view();
            let index = SearchIndex::new(&view, &entries, &Synonyms::of(&view.words::<SearchWords>()));
            let palette = CommandPalette::new("Commands", &entries, &index, Bind::bound(&mut is_open));
            let output = if bound {
                let mut recents = shown_held.borrow().clone();
                let output = palette.recents(Bind::bound(&mut recents)).show(ui);
                *shown_held.borrow_mut() = recents;
                output
            } else {
                palette.show(ui)
            };
            shown_open.set(is_open);
            if let Some(command) = output.picked {
                *shown_picked.borrow_mut() = Some(command);
            }
            if let Some(command) = output.refused {
                *shown_refused.borrow_mut() = Some(command);
            }
        };
        if in_dialog {
            Popover::new("Levels", Bind::given(true)).overlay(|_| Overlay::modal()).show(ui, |ui, _| show(ui));
        } else {
            show(ui);
        }
    });
    Rig { fixture, open, picked, refused, recents: held }
}

/// A palette open over `entries`, inside a modal dialog when `in_dialog`; `picked` holds the last pick.
fn palette(entries: Vec<HelpEntry>, in_dialog: bool) -> PaletteRig {
    let rig = rig(entries, None, in_dialog, |_| ());
    (rig.fixture, rig.open, rig.picked)
}

/// An entry for a command the trial makes up, found by `label`.
fn entry(id: &str, label: &str) -> HelpEntry {
    HelpEntry::new(CommandRef::new(format!("paletteTrial.{id}")), label.to_owned())
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
        let env = craft_ui::runtime::env::Env::of(ui);
        let view = env.view();
        let index = SearchIndex::new(&view, &entries, &Synonyms::of(&view.words::<SearchWords>()));
        let mut palette = CommandPalette::new("Commands", &entries, &index, Bind::bound(&mut is_open)).face(face).placeholder(Label::from("Search"));
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

/// The names of the listed rows.
fn names(fixture: &Fixture) -> Vec<String> {
    rows(fixture).into_iter().map(|(name, _)| name).collect()
}

/// How many texts equal to `text` the window paints.
fn painted(fixture: &Fixture, text: &str) -> usize {
    fixture.texts().iter().filter(|painted| painted.text == text).count()
}

#[trial]
fn a_result_list_is_capped_at_the_limit_and_keeps_the_active_result_in_the_last_row() {
    let entries: Vec<HelpEntry> = (0..60).map(|n| entry(&format!("cap{n}"), &format!("Alpha {n:02}"))).collect();
    let mut rig = rig(entries.clone(), Some(Vec::new()), false, |_| ());
    // Room for every listed row in the capped box: the list builds only the rows in view.
    rig.fixture.room(Vec2::new(800.0, 4000.0)).settle();
    rig.fixture.type_text("alpha").settle();
    assert_eq!(names(&rig.fixture).len(), 50, "sixty matches list fifty");

    // Up wraps to the last row, which is the active result.
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    let before = names(&rig.fixture);
    let active = before.last().expect("a last row").clone();
    let listed = |entry: &HelpEntry| before.iter().any(|name| name.starts_with(entry.label().as_str()));

    // Recent picks rank ahead of equal matches: eight results that were past the cap now rank above the active one.
    let ahead: Vec<&HelpEntry> = entries.iter().filter(|entry| !listed(entry)).take(8).collect();
    *rig.recents.borrow_mut() = ahead.iter().map(|entry| entry.command().clone()).collect();
    rig.fixture.settle();
    let after = names(&rig.fixture);
    assert_eq!(after.len(), 50, "the list stays at the cap: {after:?}");
    assert_eq!(after.last(), Some(&active), "the active result keeps the last row when better matches push it past the cap");
    assert!(ahead.iter().any(|entry| after.iter().any(|name| name.starts_with(entry.label().as_str()))), "the better matches are listed above it");

    rig.fixture.key(Key::Enter, Modifiers::NONE).settle();
    let command = rig.picked.borrow().clone().expect("Enter picked the active result");
    let picked = entries.iter().find(|entry| *entry.command() == command).expect("a listed entry");
    assert!(active.starts_with(picked.label().as_str()), "the pick is the active result: {active} / {}", picked.label().as_str());
}

#[trial]
fn recents_keep_twenty_and_the_empty_query_lists_the_newest_eight_that_have_an_entry() {
    let trees = ["Alder", "Birch", "Cedar", "Dogwood", "Elm", "Fir", "Ginkgo", "Hazel", "Ironwood", "Juniper", "Larch", "Maple"];
    let entries: Vec<HelpEntry> = trees.iter().map(|tree| entry(tree, tree)).collect();
    let ghost = |n: usize| CommandRef::new(format!("paletteTrial.ghost{n}"));
    // A command with no entry first, every entry, then a dozen more commands with none: 25 in all.
    let mut recents = vec![ghost(0)];
    recents.extend(entries.iter().map(|entry| entry.command().clone()));
    recents.extend((1..=12).map(ghost));
    let mut rig = rig(entries.clone(), Some(recents), false, |_| ());
    rig.fixture.settle();

    rig.fixture.type_text("Fir").settle();
    rig.fixture.key(Key::Enter, Modifiers::NONE).settle();
    let fir = entries[5].command().clone();
    assert_eq!(*rig.picked.borrow(), Some(fir.clone()), "Enter picked Fir");
    let kept = rig.recents.borrow().clone();
    assert_eq!(kept.first(), Some(&fir), "a pick goes first");
    assert_eq!(kept.get(1), Some(&ghost(0)), "a command with no entry keeps its place");
    assert_eq!(kept.len(), 20, "twenty are kept");
    assert_eq!(kept.iter().filter(|command| **command == fir).count(), 1, "a pick is in the list once");

    rig.open.set(true);
    rig.fixture.settle();
    let listed = names(&rig.fixture);
    let expected: Vec<&str> = kept
        .iter()
        .filter_map(|command| entries.iter().find(|entry| entry.command() == command))
        .map(|entry| entry.label().as_str())
        .take(8)
        .collect();
    assert_eq!(listed.len(), 8, "the empty query lists eight: {listed:?}");
    assert!(
        listed.iter().zip(&expected).all(|(name, label)| name.starts_with(label)),
        "the newest recents that have an entry, newest first: {listed:?} against {expected:?}"
    );
}

#[trial]
fn a_press_outside_closes_the_palette() {
    let clicks = Rc::new(Cell::new(0));
    let shown = Rc::new(Cell::new(egui::Rect::NOTHING));
    let (counted, placed) = (clicks.clone(), shown.clone());
    let entries = vec![HelpEntry::new(TrialCommand::Rotate, "Rotate")];
    let mut rig = rig(entries, None, false, move |ui| {
        let response = Button::new("Below").show(ui);
        placed.set(response.rect);
        if clicked(&response) {
            counted.set(counted.get() + 1);
        }
    });
    rig.fixture.settle();
    assert!(rig.open.get(), "the palette starts open");
    // What a modal layer covers is hidden from assistive technology, so the press goes where the button is shown.
    let below = shown.get();
    assert!(below.is_positive(), "the button under the palette");
    rig.fixture.click_at(below.center()).settle();
    assert!(!rig.open.get(), "a press outside closed the palette");
    assert_eq!(clicks.get(), 0, "the press that closes the palette presses nothing under it");
    rig.fixture.click_at(below.center()).settle();
    assert_eq!(clicks.get(), 1, "with the palette closed, the same press reaches the window below");
}

#[trial]
fn the_footer_says_why_the_active_row_is_off_or_how_many_results_there_are() {
    let entries = vec![
        HelpEntry::new(TrialCommand::Rotate, "Rotate"),
        HelpEntry::new(TrialCommand::Zoom, "Rotation"),
        HelpEntry::new(TrialCommand::Flip, "Flip"),
        HelpEntry::new(TrialCommand::Crop, "Crop"),
    ];
    let (mut fixture, ..) = palette(entries, false);
    let why = "Needs a selection";
    fixture.command_state(TrialCommand::Crop, CommandState::new().enabled(false).hint(why));
    fixture.settle();

    fixture.type_text("crop").settle();
    assert!(disabled(&fixture, "Crop"), "the active row is off");
    assert_eq!(painted(&fixture, why), 1, "the footer gives the command's hint while the active row is off");

    // The count follows the rows: digits stand in the footer only, as no label has one.
    fixture.key(Key::Escape, Modifiers::NONE).type_text("rotat").settle();
    let many = names(&fixture).len();
    fixture.key(Key::Escape, Modifiers::NONE).type_text("flip").settle();
    let few = names(&fixture).len();
    assert_ne!(many, few, "the two queries match different numbers of commands");
    assert_eq!(painted(&fixture, why), 0, "the hint goes with the off row");
    let counted = |n: usize| fixture.texts().iter().any(|text| text.text.contains(&n.to_string()));
    assert!(counted(few), "the footer counts {few} results: {:?}", fixture.texts());
    assert!(!counted(many), "the count of the earlier query is gone");

    let before: Vec<String> = fixture.texts().iter().map(|text| text.text.clone()).collect();
    fixture.key(Key::Escape, Modifiers::NONE).type_text("qqzx").settle();
    assert!(names(&fixture).is_empty(), "nothing matches");
    let said = fixture.texts().iter().any(|text| text.text != "qqzx" && !before.contains(&text.text));
    assert!(said, "the footer says that nothing matches: {:?}", fixture.texts());
}

#[trial]
fn the_active_row_marks_its_shown_control_and_a_hidden_home_goes_to_the_footer() {
    let entries = vec![
        HelpEntry::new(TrialCommand::Flip, "Flip"),
        HelpEntry::new(TrialCommand::Crop, "Crop").with_home("Toolbox"),
        HelpEntry::new(TrialCommand::Rotate, "Rotate"),
    ];
    // Flip is held by a rect in the corner, the way a menu title holds its commands; Rotate has a bound button.
    let mut rig = rig(entries, Some(vec![TrialCommand::Flip.into()]), false, |ui| {
        let corner = egui::Rect::from_min_size(ui.max_rect().min, Vec2::new(60.0, 20.0));
        commands::holds(ui.ctx(), ui.layer_id(), corner, &[TrialCommand::Flip.into()], Label::from("Rail slot"));
        ui.add_space(24.0);
        Button::command(TrialCommand::Rotate).show(ui);
    });
    rig.fixture.room(Vec2::new(1000.0, 600.0));
    rig.fixture.settle();
    assert_eq!(painted(&rig.fixture, "Rail slot"), 1, "the box beside the control names it");
    assert_eq!(painted(&rig.fixture, "Toolbox"), 0, "a command in view has no home line");

    // Rotate has a button in view: the box shows its label beside it, over the button's own and the row's.
    *rig.recents.borrow_mut() = vec![TrialCommand::Rotate.into()];
    rig.fixture.settle();
    assert_eq!(painted(&rig.fixture, "Rail slot"), 0, "the box follows the active row");
    assert!(painted(&rig.fixture, "Rotate") >= 3, "the button, the row and the box paint the label");

    // Crop is shown nowhere: its home stands in the row and the footer.
    *rig.recents.borrow_mut() = vec![TrialCommand::Crop.into()];
    rig.fixture.settle();
    assert_eq!(painted(&rig.fixture, "Toolbox"), 2, "the footer names the home of a command shown nowhere");
}

/// Enter on a query whose first match is disabled runs nothing, though an enabled match follows it: the first match is
/// the active row, the one the footer explains, and the output names it as refused so an app can close and say why.
/// Guards against an active row that skipped the disabled first match, so Enter ran the match after it.
#[trial]
fn enter_on_a_disabled_first_match_runs_nothing() {
    let entries = vec![HelpEntry::new(EditCommands::Undo, "Undo").with_path(["Edit"]), HelpEntry::new(TrialCommand::Rotate, "Undo Rotation")];
    let Rig { mut fixture, open, picked, refused, .. } = rig(entries, None, false, |_| ());
    fixture.command_state(EditCommands::Undo, CommandState::new().enabled(false));
    fixture.settle();
    fixture.type_text("undo").settle();
    let listed = rows(&fixture);
    assert!(listed.len() == 2 && listed[0].0.starts_with("Undo") && listed[0].1 && !listed[1].1, "the disabled Undo matches first: {listed:?}");
    let _ = take_output(fixture.ctx());
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*picked.borrow(), None, "Enter on the disabled first match picks nothing");
    assert_eq!(*refused.borrow(), Some(CommandRef::from(EditCommands::Undo)), "the output names the refused command");
    assert!(open.get(), "the palette stays open");
    assert!(take_output(fixture.ctx()).fired.is_empty(), "nothing ran");
}
