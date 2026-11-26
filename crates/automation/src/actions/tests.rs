//! The action list, its file and the player.

use craft_ui::testing::trial;
use serde_json::{Value, json};

use super::*;

fn step(command: &str) -> ActionStep {
    ActionStep::new(command, Value::Null)
}

fn action(name: &str, commands: &[&str]) -> Action {
    Action::new(name, commands.iter().map(|c| step(c)).collect())
}

fn list_of(actions: Vec<Action>) -> ActionList {
    ActionList::from_file(ActionsFile::new(actions, Vec::new()))
}

/// Runs every step by recording its command.
#[derive(Default)]
struct Script {
    ran: Vec<String>,
}

impl ActionRunner for Script {
    fn run_step(&mut self, _: &Action, _: usize, step: &ActionStep, _: bool, _: &mut PlaybackBudget) -> StepOutcome {
        self.ran.push(step.command.clone());
        StepOutcome::Done(Value::Null)
    }
}

// `moved_index` cases (the moved item, items between, items outside); `move_step` within an action
// remaps `disabled` and `dialogs`, and between actions carries the flags along.
#[trial]
fn action_list_moves_follow_the_selection() {
    assert_eq!(moved_index(1, 1, 3), 3);
    assert_eq!(moved_index(2, 1, 3), 1);
    assert_eq!(moved_index(2, 3, 1), 3);
    assert_eq!(moved_index(0, 1, 3), 0);
    assert_eq!(moved_index(4, 1, 3), 4);

    let mut list = list_of(vec![action("A", &["a", "b", "c"]), action("B", &["x"])]);
    list.actions[0].disabled.insert(0);
    list.actions[0].dialogs.insert(2);
    list.move_step(0, 0, 0, 2).unwrap();
    let commands: Vec<_> = list.actions[0].steps.iter().map(|s| s.command.as_str()).collect();
    assert_eq!(commands, ["b", "c", "a"]);
    assert_eq!(list.actions[0].disabled.iter().copied().collect::<Vec<_>>(), [2]);
    assert_eq!(list.actions[0].dialogs.iter().copied().collect::<Vec<_>>(), [1]);

    list.move_step(0, 2, 1, 0).unwrap();
    assert_eq!(list.actions[0].steps.len(), 2);
    assert!(list.actions[0].disabled.is_empty());
    assert_eq!(list.actions[1].steps[0].command, "a");
    assert!(list.actions[1].disabled.contains(&0));
    assert_eq!(list.move_step(0, 0, 1, 5), Err(ActionListError::OutOfRange));

    list.move_action(0, 1).unwrap();
    assert_eq!(list.actions[1].name, "A");
}
// PhotoCraft's `actions.json` shape (steps as `[id, params]`, colour names, no `sets`, the
// `automation` mark only when set) loads and saves unchanged; empty sets survive in `sets`; a file
// of another version is refused.
#[trial]
fn action_list_file_round_trip_keeps_pc_json() {
    let text = r#"{"version":1,"actions":[{"name":"A","steps":[["image.levels",{"gamma":1.2}]],"disabled":[0],"set":"S","color":"red"},{"name":"B","steps":[],"automation":true}]}"#;
    let file = ActionsFile::from_json(text).unwrap();
    assert!(!file.actions[0].automation && file.actions[1].automation);
    assert!(matches!(ActionsFile::from_json(r#"{"version":2,"actions":[]}"#), Err(ActionsFileError::Version(2))));
    assert_eq!(file.actions[0].color, ActionColor::Red);
    assert_eq!(file.actions[0].steps[0], ActionStep::new("image.levels", json!({"gamma": 1.2})));
    let mut list = ActionList::from_file(file);
    let saved: Value = serde_json::from_str(&list.to_file().to_json().unwrap()).unwrap();
    assert_eq!(saved, serde_json::from_str::<Value>(text).unwrap());

    list.new_set("Empty").unwrap();
    assert_eq!(list.sets(), ["S", "Empty"]);
    let back = ActionsFile::from_json(&list.to_file().to_json().unwrap()).unwrap();
    assert_eq!(back.sets, ["Empty"]);
    assert_eq!(ActionList::from_file(back).sets(), ["S", "Empty"]);
}
// An empty set made before filled ones keeps its place through a save; a file of the older layout
// (empty sets in `sets`, no `set_order`) loads with its filled sets first.
#[trial]
fn set_order_survives_the_file() {
    let mut list = ActionList::new();
    list.new_set("A").unwrap();
    list.new_set("B").unwrap();
    list.new_set("C").unwrap();
    list.push(Action { set: Some("B".into()), ..action("X", &[]) }).unwrap();
    list.push(Action { set: Some("C".into()), ..action("Y", &[]) }).unwrap();
    let back = ActionList::from_file(ActionsFile::from_json(&list.to_file().to_json().unwrap()).unwrap());
    assert_eq!(back.sets(), ["A", "B", "C"]);

    let old = r#"{"version":1,"actions":[{"name":"X","steps":[],"set":"F"}],"sets":["E"]}"#;
    assert_eq!(ActionList::from_file(ActionsFile::from_json(old).unwrap()).sets(), ["F", "E"]);
}
// A file that repeats a name keeps a later action's own name: `Foo, Foo, Foo 2` renames the repeat
// (`Foo 3`), never the real `Foo 2`; a merge does the same against the names already in the list.
#[trial]
fn action_names_in_a_file_keep_their_own_names_and_rename_only_repeats() {
    let names = |list: &ActionList| list.actions.iter().map(|action| action.name.clone()).collect::<Vec<_>>();
    let file = || ActionsFile::new(vec![action("Foo", &[]), action("Foo", &[]), action("Foo 2", &[])], Vec::new());
    assert_eq!(names(&ActionList::from_file(file())), ["Foo", "Foo 3", "Foo 2"]);

    let mut list = list_of(vec![action("Foo", &[])]);
    let merged = list.merge_file(file(), false, |_, _| {});
    assert_eq!(names(&list), ["Foo", "Foo 3", "Foo 4", "Foo 2"]);
    assert_eq!(merged.renames, [("Foo".to_owned(), "Foo 3".to_owned()), ("Foo".to_owned(), "Foo 4".to_owned())]);
}

// A PhotoCraft file with folder occurrences: two folders named "Basics" and an empty one become
// sets with unique names, the empty one an empty set; of two actions named "Play", the shortcut
// owner keeps the name; each unique stored id maps to its action's final name, a repeated one to
// none; and the occurrence fields are never written back.
#[trial]
fn action_file_folder_occurrences_become_unique_names() {
    let text = r#"{"version":1,"set_rows":["Basics","Basics","Empty"],"actions":[
        {"name":"Play","steps":[],"set":"Basics","set_slot":0,"action_id":"0123456789abcdef0123456789abcdef"},
        {"name":"Play","steps":[],"set":"Basics","set_slot":1,"legacy_shortcut_owner":true},
        {"name":"Loose","steps":[],"action_id":"ffffffffffffffffffffffffffffffff"},
        {"name":"Twin","steps":[],"action_id":"ffffffffffffffffffffffffffffffff"}]}"#;
    let (list, merged) = ActionList::load(ActionsFile::from_json(text).unwrap());
    let names: Vec<(&str, Option<&str>)> = list.actions().iter().map(|action| (action.name.as_str(), action.set.as_deref())).collect();
    assert_eq!(names, [("Play 2", Some("Basics")), ("Play", Some("Basics 2")), ("Loose", None), ("Twin", None)]);
    assert_eq!(list.sets(), ["Basics", "Basics 2", "Empty"]);
    assert_eq!(merged.ids, [("0123456789abcdef0123456789abcdef".to_owned(), "Play 2".to_owned())]);
    let saved = list.to_file().to_json().unwrap();
    assert!(["set_rows", "set_slot", "action_id", "legacy_shortcut_owner"].iter().all(|field| !saved.contains(field)));
}

// Clear All, Reset (append and replace) and Replace each keep one undo that restores the list as it
// was, actions and empty sets alike; any other edit after them leaves nothing to undo.
#[trial]
fn action_library_commands_keep_one_undo() {
    let mut list = list_of(vec![action("Invert Colors", &["image.invert"])]);
    list.new_set("Empty").unwrap();
    let before = (list.actions().to_vec(), list.sets().into_iter().map(str::to_owned).collect::<Vec<_>>());
    let state = |list: &ActionList| (list.actions().to_vec(), list.sets().into_iter().map(str::to_owned).collect::<Vec<_>>());
    let defaults = || ActionsFile::new(vec![Action { set: Some("Basics".into()), ..action("Invert Colors", &["image.invert"]) }], Vec::new());

    list.clear_all();
    assert!(list.actions().is_empty() && list.sets().is_empty());
    assert_eq!(list.library_undo(), Some(LibraryEdit::ClearAll));
    assert_eq!(list.undo_library(), Ok(LibraryEdit::ClearAll));
    assert_eq!(state(&list), before);

    list.reset(defaults(), true, |_, _| {});
    let names: Vec<&str> = list.actions().iter().map(|action| action.name.as_str()).collect();
    assert_eq!(names, ["Invert Colors", "Invert Colors 2"]);
    assert_eq!(list.undo_library(), Ok(LibraryEdit::Reset));
    assert_eq!(state(&list), before);

    list.reset(defaults(), false, |_, _| {});
    assert_eq!(list.sets(), ["Basics"]);
    list.undo_library().unwrap();
    assert_eq!(state(&list), before);

    list.replace_all(ActionsFile::new(vec![action("Other", &[])], Vec::new()), |_, _| {});
    assert_eq!(list.actions().len(), 1);
    list.set_color(0, ActionColor::Red).unwrap();
    assert_eq!(list.library_undo(), None);
    assert_eq!(list.undo_library(), Err(ActionListError::NothingToUndo));
}

/// A runner whose steps are named by what they do: `pause`, `wait`, `stop`, `deny` (refused by the
/// gate), `noop`; any other is `Done`. Its own `charge` refuses once `charges` are spent.
struct Scripted {
    ran: Vec<String>,
    charges: u64,
}

impl Scripted {
    fn new(charges: u64) -> Self {
        Self { ran: Vec::new(), charges }
    }
}

impl ActionRunner for Scripted {
    fn run_step(&mut self, _: &Action, _: usize, step: &ActionStep, _: bool, _: &mut PlaybackBudget) -> StepOutcome {
        self.ran.push(step.command.clone());
        match step.command.as_str() {
            "pause" => StepOutcome::Paused,
            "wait" => StepOutcome::Wait(Value::Null),
            "stop" => StepOutcome::Stopped,
            _ => StepOutcome::Done(Value::Null),
        }
    }

    fn charge(&mut self, _: &mut PlaybackBudget) -> bool {
        if self.charges == 0 {
            return false;
        }
        self.charges -= 1;
        true
    }

    fn authorize(&mut self, step: &ActionStep) -> Result<(), String> {
        if step.command == "deny" { Err("denied".to_owned()) } else { Ok(()) }
    }

    fn is_noop(&self, step: &ActionStep) -> bool {
        step.command == "noop"
    }
}

// The runner's `charge` ends playback with the step limit; `Paused` is not counted, `Wait` is; a
// resume continues after `paused_at`; `Stopped` is no failure; a denial fails before the step runs;
// a no-op counts without running.
#[trial]
fn paused_waiting_and_stopped_steps_end_playback_as_reported() {
    let list = list_of(vec![
        action("Limit", &["a", "b"]),
        action("Pause", &["a", "pause", "b"]),
        action("Wait", &["a", "wait", "b"]),
        action("Stop", &["a", "stop", "b"]),
        action("Deny", &["a", "deny", "b"]),
        action("Noop", &["noop", "a"]),
    ]);
    let play = |index: usize, from: usize, runner: &mut Scripted| ActionPlayer::play(&list, index, from, runner, &mut PlaybackBudget::new());

    let mut runner = Scripted::new(1);
    let report = play(0, 0, &mut runner);
    assert_eq!((report.ran, report.refused), (1, Some(PlaybackRefusal::StepLimit)));
    assert_eq!(report.failed.map(|failure| failure.step), Some(1));

    let mut runner = Scripted::new(99);
    let report = play(1, 0, &mut runner);
    assert_eq!((report.ran, report.paused_at, report.waiting), (1, Some(1), false));
    let resumed = play(1, 2, &mut runner);
    assert_eq!((resumed.ran, resumed.paused_at), (1, None));
    assert_eq!(runner.ran, ["a", "pause", "b"]);

    let mut runner = Scripted::new(99);
    let report = play(2, 0, &mut runner);
    assert_eq!((report.ran, report.paused_at, report.waiting), (2, Some(1), true));
    assert_eq!(runner.ran, ["a", "wait"]);

    let mut runner = Scripted::new(99);
    let report = play(3, 0, &mut runner);
    assert_eq!((report.ran, report.stopped_at, report.paused_at), (1, Some(1), None));
    assert!(report.failed.is_none());

    let mut runner = Scripted::new(99);
    let report = play(4, 0, &mut runner);
    assert_eq!(report.failed.map(|failure| (failure.step, failure.message)), Some((1, "denied".to_owned())));
    assert_eq!(runner.ran, ["a"], "the denied step never ran");

    let mut runner = Scripted::new(99);
    let report = play(5, 0, &mut runner);
    assert_eq!(report.ran, 2);
    assert_eq!(runner.ran, ["a"], "the no-op counted without running");
}
// One budget is charged across nested plays and stops at the limit; a call back to a running
// action is refused as `Recursive`.
#[trial]
fn action_player_charges_a_shared_budget_and_detects_loops() {
    struct Nested<'a>(&'a ActionList, Vec<PlaybackRefusal>);
    impl ActionRunner for Nested<'_> {
        fn run_step(&mut self, _: &Action, _: usize, step: &ActionStep, _: bool, budget: &mut PlaybackBudget) -> StepOutcome {
            if let Some(callee) = step.command.strip_prefix("play:") {
                let index = self.0.actions().iter().position(|a| a.name == callee).unwrap();
                let report = ActionPlayer::play(self.0, index, 0, self, budget);
                self.1.extend(report.refused);
            }
            StepOutcome::Done(Value::Null)
        }
    }
    let list = list_of(vec![action("A", &["x", "play:B", "y"]), action("B", &["z", "play:A"])]);
    let mut runner = Nested(&list, Vec::new());
    let mut budget = PlaybackBudget::new();
    let report = ActionPlayer::play(&list, 0, 0, &mut runner, &mut budget);
    assert_eq!(report.ran, 3);
    assert_eq!(runner.1, [PlaybackRefusal::Recursive("A".to_owned())]);
    assert_eq!(budget.steps(), 5);

    let mut small = PlaybackBudget { limit: 2, ..PlaybackBudget::new() };
    let report = ActionPlayer::play(&list, 0, 0, &mut Script::default(), &mut small);
    assert_eq!(report.ran, 2);
    assert_eq!(report.refused, Some(PlaybackRefusal::StepLimit));
    assert_eq!(report.failed.map(|f| f.step), Some(2));

    let mut deep = PlaybackBudget::new();
    for n in 0..MAX_PLAYBACK_DEPTH {
        deep.enter(&n.to_string()).unwrap();
    }
    assert_eq!(deep.enter("more"), Err(PlaybackRefusal::TooDeep));
}

// A set keeps its place as it gets its first action and loses its last; a duplicate goes right after its source;
// rename keeps the place and moves the members and refuses empty, taken and unknown names; delete returns the members
// in list order.
#[trial]
fn sets_keep_their_place_as_they_fill_empty_rename_and_go() {
    let mut list = ActionList::new();
    for set in ["A", "B", "C"] {
        list.new_set(set).unwrap();
    }
    list.new_action("x", Some("B".into())).unwrap();
    assert_eq!(list.sets(), ["A", "B", "C"], "B keeps its place when it gets its first action");
    list.delete_action(0).unwrap();
    assert_eq!(list.sets(), ["A", "B", "C"], "B keeps its place when it loses its last");

    assert_eq!(list.duplicate_set("A", |_, _, _| {}).unwrap(), "A copy");
    list.new_action("y", Some("B".into())).unwrap();
    assert_eq!(list.duplicate_set("B", |_, _, _| {}).unwrap(), "B copy");
    list.new_action("z", Some("B".into())).unwrap();
    assert_eq!(list.sets(), ["A", "A copy", "B", "B copy", "C"], "a copy follows its source, with or without actions");

    let revision = list.revision();
    list.rename_set("B", "B").unwrap();
    assert_eq!(list.revision(), revision, "the same name changes nothing");
    assert_eq!(list.rename_set("B", "  "), Err(ActionListError::EmptyName));
    assert_eq!(list.rename_set("B", "C"), Err(ActionListError::NameTaken("C".into())));
    assert_eq!(list.rename_set("Nope", "Z"), Err(ActionListError::NoSuchSet("Nope".into())));
    assert_eq!(list.revision(), revision, "a refused rename changes nothing");
    list.rename_set("B", " Beta ").unwrap();
    assert_eq!(list.sets(), ["A", "A copy", "Beta", "B copy", "C"]);
    let members = list.actions().iter().filter(|action| action.set.as_deref() == Some("Beta")).count();
    assert_eq!(members, 2, "the members follow the rename");

    let gone = list.delete_set("Beta").unwrap();
    assert_eq!(gone.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["y", "z"]);
    assert_eq!(list.sets(), ["A", "A copy", "B copy", "C"]);
    assert_eq!(list.actions().len(), 1);
    assert_eq!(list.delete_set("Beta"), Err(ActionListError::NoSuchSet("Beta".into())));
}

// An option of an action stays with it through a duplicate, a set duplicate, a step move, the file and a merge.
#[trial]
fn app_options_ride_with_an_action_through_edits_and_files() {
    let mut base = action("A", &["a", "b"]);
    base.set = Some("S".into());
    base.options.insert("key".into(), json!(3));
    let mut list = list_of(vec![base]);
    list.duplicate_action(0, |_, _, _| {}).unwrap();
    list.duplicate_set("S", |_, _, _| {}).unwrap();
    list.move_step(0, 0, 0, 1).unwrap();
    assert_eq!(list.actions().len(), 4);
    assert!(list.actions().iter().all(|action| action.options.get("key") == Some(&json!(3))));

    let text = list.to_file().to_json().unwrap();
    assert!(text.contains("\"options\""));
    let loaded = ActionList::from_file(ActionsFile::from_json(&text).unwrap());
    assert!(loaded.actions().iter().all(|action| action.options.get("key") == Some(&json!(3))));
    let mut merged = ActionList::new();
    merged.merge_file(ActionsFile::from_json(&text).unwrap(), false, |_, _| {});
    assert!(merged.actions().iter().all(|action| action.options.get("key") == Some(&json!(3))));
}

// An action's options are set in the list, kept through a duplicate and the file, and the same options or an unknown
// action change nothing.
#[trial]
fn an_actions_options_are_set_and_kept_through_a_duplicate_and_the_file() {
    let mut list = list_of(vec![action("a", &["one"])]);
    let options = json!({"key": "F5", "automaticAlignTo": "artboard"}).as_object().cloned().unwrap();
    list.set_options(0, options.clone()).unwrap();
    assert_eq!(list.action(0).unwrap().options, options);
    let revision = list.revision();
    list.set_options(0, options.clone()).unwrap();
    assert_eq!(list.revision(), revision, "the options it holds change nothing");
    assert_eq!(list.set_options(7, serde_json::Map::new()), Err(ActionListError::NoSuchAction(7)));
    let copy = list.duplicate_action(0, |_, _, _| {}).unwrap();
    assert_eq!(list.action(copy).unwrap().options, options, "a duplicate keeps them");
    assert_eq!(ActionList::from_file(list.to_file()).action(0).unwrap().options, options, "the file keeps them");
}
