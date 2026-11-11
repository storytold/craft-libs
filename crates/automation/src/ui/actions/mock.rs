//! `FakeActionsHost` and sample lists for stories and tests.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{ActionsHost, ShortcutChoice};
use crate::actions::{Action, ActionList, ActionStep, ActionsFile};

/// The app's side for stories and tests: step labels by command id (a command without one shows as
/// `command_prefix` and the id), the recording and its pending steps, the playing flag and
/// shortcuts by action name.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeActionsHost {
    /// Step labels by command id.
    pub step_labels: BTreeMap<String, String>,
    /// Put before the id of a command without a label.
    pub command_prefix: String,
    /// The action being recorded into.
    pub recording: Option<usize>,
    /// The steps recorded since the recording started.
    pub pending: Vec<ActionStep>,
    /// An action is playing.
    pub playing: bool,
    /// Shortcuts by action name.
    pub shortcuts: BTreeMap<String, ShortcutChoice>,
}

impl FakeActionsHost {
    /// A host whose labels for commands without one start with `prefix`.
    pub fn prefixed(prefix: impl Into<String>) -> Self {
        Self { command_prefix: prefix.into(), ..Self::default() }
    }
}

impl ActionsHost for FakeActionsHost {
    fn step_label(&self, step: &ActionStep) -> String {
        self.step_labels.get(&step.command).cloned().unwrap_or_else(|| format!("{}{}", self.command_prefix, step.command))
    }

    fn shortcut(&self, action: &Action) -> Option<ShortcutChoice> {
        self.shortcuts.get(&action.name).copied()
    }

    fn recording(&self) -> Option<usize> {
        self.recording
    }

    fn pending_steps(&self, action: usize) -> &[ActionStep] {
        if self.recording == Some(action) { &self.pending } else { &[] }
    }

    fn playing(&self) -> bool {
        self.playing
    }
}

/// A step of the command `command` with no parameters.
pub fn step(command: &str) -> ActionStep {
    ActionStep::new(command, Value::Null)
}

/// The action `name` in `set` (`None`: loose) with a step for each of `commands`.
pub fn action(name: &str, set: Option<&str>, commands: &[&str]) -> Action {
    let mut action = Action::new(name, commands.iter().map(|command| step(command)).collect());
    action.set = set.map(str::to_owned);
    action
}

/// The list of `actions`.
pub fn list_of(actions: Vec<Action>) -> ActionList {
    ActionList::from_file(ActionsFile { version: ActionsFile::VERSION, actions, sets: Vec::new() })
}

/// Sharpen (levels, curves, blur) and Vignette (mask, blur) loose; Glow (glow) in the set Effects.
pub fn sample_list() -> ActionList {
    list_of(vec![
        action("Sharpen", None, &["levels", "curves", "blur"]),
        action("Vignette", None, &["mask", "blur"]),
        action("Glow", Some("Effects"), &["glow"]),
    ])
}
