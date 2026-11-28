//! The app's command states (`CommandStates`): lookups by command, with and without parameters.

use craft_ui::runtime::commands::{CommandRef, CommandState, CommandStates, command_state_comparisons};
use craft_ui::testing::trial;
use serde_json::json;

/// The held commands `n` lookups compare, one for each of `n` commands set before.
fn comparisons_for(n: usize) -> u64 {
    let mut states = CommandStates::new();
    let commands: Vec<CommandRef> = (0..n).map(|i| CommandRef::new(format!("test.command{i}"))).collect();
    for command in &commands {
        states.set(command.clone(), CommandState::new().enabled(false));
    }
    let before = command_state_comparisons();
    for command in &commands {
        assert!(states.get(command).is_some(), "{command} has the state it was given");
    }
    command_state_comparisons() - before
}

// A menu bar of a few thousand commands looks each one up while it builds its states: the comparisons grow with the
// commands, not with their square.
#[trial]
fn looking_commands_up_compares_in_proportion_to_the_commands() {
    let (small, large) = (comparisons_for(500), comparisons_for(1_000));
    assert!(small > 0, "a lookup compares the command with what is held");
    assert!(large <= 2 * small, "twice the commands compare at most twice as often: {small} for 500, {large} for 1,000");
}

#[trial]
fn commands_with_one_id_keep_their_own_states_through_a_removal() {
    let plain = CommandRef::new("test.zoom");
    let in_ = CommandRef::with_params("test.zoom", json!({ "by": 2 }));
    let out = CommandRef::with_params("test.zoom", json!({ "by": 0.5 }));
    let other = CommandRef::new("test.other");
    let (off, on, unchecked) = (CommandState::new().enabled(false), CommandState::new().checked(true), CommandState::new().checked(false));
    let mut states = CommandStates::new();
    states.set(plain.clone(), off.clone());
    states.set(other.clone(), off.clone());
    states.set(in_.clone(), on.clone());
    states.set(out.clone(), unchecked.clone());

    assert_eq!(states.get(&CommandRef::with_params("test.zoom", json!({}))), Some(&off), "empty parameters are the command without parameters");
    assert_eq!(states.remove(&plain), Some(off.clone()), "the plain command is held");
    assert_eq!(states.get(&plain), None, "a removed command has no state");
    assert_eq!(states.get(&other), Some(&off), "the command after it keeps its state");
    assert_eq!(states.get(&in_), Some(&on), "each set of parameters keeps its own state");
    assert_eq!(states.get(&out), Some(&unchecked), "each set of parameters keeps its own state");

    states.set(out.clone(), on.clone());
    assert_eq!(states.get(&out), Some(&on), "setting a held command replaces its state");
    assert_eq!(states.remove(&plain), None, "a removed command is removed once");
}
