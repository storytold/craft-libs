//! [`Action`], [`ActionStep`], [`ActionColor`] and [`ActionInclude`].

use std::borrow::Cow;
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

/// An action's Button Mode colour, serialised with PhotoCraft's legacy string names.
/// The twelve known colours retain their measured ATN indices; other indices use
/// `@atn-color:N`. Unrecognised names still read as `None`, as in older files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ActionColor {
    /// No colour.
    #[default]
    None,
    /// Red (ATN index 1).
    Red,
    /// Orange (ATN index 2).
    Orange,
    /// Yellow (ATN index 3).
    Yellow,
    /// Green (ATN index 4).
    Green,
    /// Classic blue, shown as light blue in the modern picker (ATN index 5).
    Blue,
    /// Classic violet, shown as lavender in the modern picker (ATN index 6).
    Violet,
    /// Gray (ATN index 7).
    Gray,
    /// Teal (ATN index 8).
    Teal,
    /// Modern blue, distinct from classic light blue (ATN index 9).
    Blue2,
    /// Magenta (ATN index 10).
    Magenta,
    /// Purple (ATN index 11).
    Purple,
    /// An unrecognised ATN index, retained without inventing a palette colour.
    /// Use [`Self::from_atn_index`] to normalise known indices to their named variants.
    Unknown(u16),
}

impl ActionColor {
    /// The classic Color submenu's eight choices, in ATN order.
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

    /// Every known colour in ATN index order, distinct from the modern picker's display order.
    pub const KNOWN: [ActionColor; 12] = [
        ActionColor::None,
        ActionColor::Red,
        ActionColor::Orange,
        ActionColor::Yellow,
        ActionColor::Green,
        ActionColor::Blue,
        ActionColor::Violet,
        ActionColor::Gray,
        ActionColor::Teal,
        ActionColor::Blue2,
        ActionColor::Magenta,
        ActionColor::Purple,
    ];

    /// The legacy name of a known colour; empty for no colour or an unknown index.
    /// Use [`Self::wire_name`] to retain an unknown index when writing a file or editing metadata.
    pub const fn name(self) -> &'static str {
        match self {
            ActionColor::None | ActionColor::Unknown(_) => "",
            ActionColor::Red => "red",
            ActionColor::Orange => "orange",
            ActionColor::Yellow => "yellow",
            ActionColor::Green => "green",
            ActionColor::Blue => "blue",
            ActionColor::Violet => "violet",
            ActionColor::Gray => "gray",
            ActionColor::Teal => "teal",
            ActionColor::Blue2 => "blue2",
            ActionColor::Magenta => "magenta",
            ActionColor::Purple => "purple",
        }
    }

    /// The lossless file name, including canonical `@atn-color:N` for an unknown index.
    pub fn wire_name(self) -> Cow<'static, str> {
        match Self::from_atn_index(self.atn_index()) {
            ActionColor::Unknown(index) => Cow::Owned(format!("@atn-color:{index}")),
            known => Cow::Borrowed(known.name()),
        }
    }

    /// The colour stored at an ATN header index; indices outside the known palette stay opaque.
    pub const fn from_atn_index(index: u16) -> Self {
        if (index as usize) < Self::KNOWN.len() { Self::KNOWN[index as usize] } else { Self::Unknown(index) }
    }

    /// The original ATN header index, including an opaque unknown value.
    pub const fn atn_index(self) -> u16 {
        match self {
            ActionColor::None => 0,
            ActionColor::Red => 1,
            ActionColor::Orange => 2,
            ActionColor::Yellow => 3,
            ActionColor::Green => 4,
            ActionColor::Blue => 5,
            ActionColor::Violet => 6,
            ActionColor::Gray => 7,
            ActionColor::Teal => 8,
            ActionColor::Blue2 => 9,
            ActionColor::Magenta => 10,
            ActionColor::Purple => 11,
            ActionColor::Unknown(index) => index,
        }
    }

    /// The shared theme colour for words and tints; opaque indices have no guessed tint.
    pub const fn label_color(self) -> Option<LabelColor> {
        match self {
            ActionColor::None | ActionColor::Unknown(_) => None,
            ActionColor::Red => Some(LabelColor::Red),
            ActionColor::Orange => Some(LabelColor::Orange),
            ActionColor::Yellow => Some(LabelColor::Yellow),
            ActionColor::Green => Some(LabelColor::Green),
            ActionColor::Blue => Some(LabelColor::Blue),
            ActionColor::Violet => Some(LabelColor::Violet),
            ActionColor::Gray => Some(LabelColor::Gray),
            ActionColor::Teal => Some(LabelColor::Seafoam),
            ActionColor::Blue2 => Some(LabelColor::Indigo),
            ActionColor::Magenta => Some(LabelColor::Magenta),
            ActionColor::Purple => Some(LabelColor::Fuchsia),
        }
    }

    /// Read a known name or canonical opaque index. Other names retain the legacy `None` fallback.
    pub fn from_name(name: &str) -> ActionColor {
        if let Some(color) = Self::KNOWN.into_iter().find(|color| color.name() == name) {
            return color;
        }
        if let Some(number) = name.strip_prefix("@atn-color:")
            && number.len() <= 5
            && let Ok(index) = number.parse::<u16>()
            && usize::from(index) >= Self::KNOWN.len()
            && number == index.to_string()
        {
            return Self::Unknown(index);
        }
        Self::None
    }
}

impl From<String> for ActionColor {
    fn from(name: String) -> Self {
        ActionColor::from_name(&name)
    }
}

impl From<ActionColor> for String {
    fn from(color: ActionColor) -> Self {
        color.wire_name().into_owned()
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
/// list, `disabled` and `dialogs` hold step indices and are left out of the file when empty, `set`,
/// `color` and `automation` likewise.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Action {
    /// The name, unique in its [`ActionList`](super::ActionList).
    pub name: String,
    /// The action's description; multiline text is kept verbatim.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The action's icon identifier. Unknown identifiers are retained for the app's fallback.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
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
    /// Written by an automation client: playback keeps that client's gate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automation: bool,
    /// The app's own options for the action. The framework never reads them; every edit, duplicate, move, merge and
    /// the file keep them.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub options: serde_json::Map<String, serde_json::Value>,
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
