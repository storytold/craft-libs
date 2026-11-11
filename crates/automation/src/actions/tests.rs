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
    ActionList::from_file(ActionsFile { version: ActionsFile::VERSION, actions, sets: Vec::new() })
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
// PhotoCraft's `actions.json` shape (steps as `[id, params]`, colour names, no `sets`) loads and
// saves unchanged; empty sets survive in `sets`.
#[trial]
fn action_list_file_round_trip_keeps_pc_json() {
    let text = r#"{"version":1,"actions":[{"name":"A","steps":[["image.levels",{"gamma":1.2}]],"disabled":[0],"set":"S","color":"red"},{"name":"B","steps":[]}]}"#;
    let file = ActionsFile::from_json(text).unwrap();
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
