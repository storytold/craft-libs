//! [`Action`], [`ActionStep`], [`ActionColor`] and [`ActionInclude`].

use std::collections::BTreeSet;

use craft_theme::LabelColor;

use serde_json::Value;

/// One recorded step: a command id and its parameters, the shape batch and droplets accept.
/// Serialised as the pair `[id, params]` (PhotoCraft's `(String, Value)`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "(String, Value)", into = "(String, Value)")]
pub struct ActionStep {
    /// The app's command id ("image.adjustments.levels", "actions.play").
    pub command: String,
    /// The command's parameters.
    pub params: Value,
}

impl ActionStep {
    /// A step running `command` with `params`.
    pub fn new(command: impl Into<String>, params: Value) -> Self {
        Self { command: command.into(), params }
    }
}

impl From<(String, Value)> for ActionStep {
    fn from((command, params): (String, Value)) -> Self {
        Self { command, params }
    }
}

impl From<ActionStep> for (String, Value) {
    fn from(step: ActionStep) -> Self {
        (step.command, step.params)
    }
}

/// An action's Button Mode colour: none and Photoshop's seven, serialised as
/// PhotoCraft's `BUTTON_COLORS` names (`""` for none, `"red"` … `"gray"`). An unknown name reads as
/// `None`. The file keeps this type (lead call 40) because [`LabelColor`] has eleven colours and
/// no "none"; [`label_color`](Self::label_color) maps it for words and tints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ActionColor {
    /// No colour.
    #[default]
    None,
    #[allow(missing_docs)]
    Red,
    #[allow(missing_docs)]
    Orange,
    #[allow(missing_docs)]
    Yellow,
    #[allow(missing_docs)]
    Green,
    #[allow(missing_docs)]
    Blue,
    #[allow(missing_docs)]
    Violet,
    #[allow(missing_docs)]
    Gray,
}

impl ActionColor {
    /// Every colour in Photoshop's order, `None` first (the Color submenu's order).
    pub const ALL: [ActionColor; 8] = [
        ActionColor::None,
        ActionColor::Red,
        ActionColor::Orange,
        ActionColor::Yellow,
        ActionColor::Green,
        ActionColor::Blue,
        ActionColor::Violet,
        ActionColor::Gray,
    ];

    /// The file name: `""`, `"red"`, `"orange"`, `"yellow"`, `"green"`, `"blue"`, `"violet"`,
    /// `"gray"`.
    pub const fn name(self) -> &'static str {
        match self {
            ActionColor::None => "",
            ActionColor::Red => "red",
            ActionColor::Orange => "orange",
            ActionColor::Yellow => "yellow",
            ActionColor::Green => "green",
            ActionColor::Blue => "blue",
            ActionColor::Violet => "violet",
            ActionColor::Gray => "gray",
        }
    }

    /// The [`LabelColor`] this colour is, for the shared colour words
    /// ([`LabelColorLabels::name`](craft_ui::localized::LabelColorLabels::name)); `None` for no
    /// colour.
    pub const fn label_color(self) -> Option<LabelColor> {
        match self {
            ActionColor::None => None,
            ActionColor::Red => Some(LabelColor::Red),
            ActionColor::Orange => Some(LabelColor::Orange),
            ActionColor::Yellow => Some(LabelColor::Yellow),
            ActionColor::Green => Some(LabelColor::Green),
            ActionColor::Blue => Some(LabelColor::Blue),
            ActionColor::Violet => Some(LabelColor::Violet),
            ActionColor::Gray => Some(LabelColor::Gray),
        }
    }

    /// The colour a file name stands for; `None` for `""` and any unknown name.
    pub fn from_name(name: &str) -> ActionColor {
        ActionColor::ALL.into_iter().find(|color| color.name() == name).unwrap_or(ActionColor::None)
    }
}

impl From<String> for ActionColor {
    fn from(name: String) -> Self {
        ActionColor::from_name(&name)
    }
}

impl From<ActionColor> for String {
    fn from(color: ActionColor) -> Self {
        color.name().to_owned()
    }
}

/// How many of an action's steps playback runs: the include column's mark on an action row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionInclude {
    /// Every step (a check).
    All,
    /// Some steps are switched off (Photoshop's partial mark, "2/3 steps").
    Partial,
    /// No step, or the action has none.
    None,
}

/// One recorded action, in PhotoCraft's `actions.json` shape: `steps` is the `[[id, params], …]`
/// list, `disabled` and `dialogs` hold step indices and are left out of the file when empty, `set`
/// and `color` likewise.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Action {
    /// The name, unique in its [`ActionList`](super::ActionList).
    pub name: String,
    /// The steps in order.
    #[serde(default)]
    pub steps: Vec<ActionStep>,
    /// Steps playback skips (unchecked in the include column).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled: BTreeSet<usize>,
    /// Steps whose dialog shows during playback (the dialog column); what that means is the
    /// app's ([`ActionRunner::run_step`](super::ActionRunner::run_step) gets the flag).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub dialogs: BTreeSet<usize>,
    /// The set the action belongs to (Photoshop's set folder); `None` for a loose action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// The Button Mode colour.
    #[serde(default, skip_serializing_if = "is_no_color")]
    pub color: ActionColor,
}

fn is_no_color(color: &ActionColor) -> bool {
    *color == ActionColor::None
}

impl Action {
    /// An action named `name` with `steps`, all included, no dialogs, no set, no colour.
    pub fn new(name: impl Into<String>, steps: Vec<ActionStep>) -> Self {
        Self { name: name.into(), steps, ..Self::default() }
    }

    /// Whether playback runs step `step`.
    pub fn step_enabled(&self, step: usize) -> bool {
        !self.disabled.contains(&step)
    }

    /// The steps playback runs, with their indices.
    pub fn enabled_steps(&self) -> impl Iterator<Item = (usize, &ActionStep)> {
        self.steps.iter().enumerate().filter(|(i, _)| self.step_enabled(*i))
    }

    /// The include column's mark.
    pub fn include(&self) -> ActionInclude {
        let enabled = self.enabled_steps().count();
        if enabled == 0 {
            ActionInclude::None
        } else if enabled == self.steps.len() {
            ActionInclude::All
        } else {
            ActionInclude::Partial
        }
    }

    /// Applies `map` to every index in `disabled` and `dialogs` after steps moved; `None` drops the
    /// index.
    pub fn remap_steps(&mut self, map: impl Fn(usize) -> Option<usize>) {
        self.disabled = self.disabled.iter().filter_map(|&i| map(i)).collect();
        self.dialogs = self.dialogs.iter().filter_map(|&i| map(i)).collect();
    }
}
