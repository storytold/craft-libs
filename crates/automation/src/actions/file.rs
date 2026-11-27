//! [`ActionsFile`]: the `actions.json` form, compatible with PhotoCraft's.

use super::model::Action;

/// The actions file (PhotoCraft's `actions.json` in its preset store): `{"version": 1,
/// "actions": [...]}`, plus `sets` with the names of empty sets, left out when there are none
/// and optional on load, so PhotoCraft's files load and save unchanged. Unknown fields are
/// ignored.
///
/// A file PhotoCraft wrote with its folder occurrences (`set_rows`, and per action `set_slot`,
/// `action_id`, `legacy_shortcut_owner`) loads too: those fields are read into
/// `FileOccurrences` and [`ActionList::merge_file`](super::ActionList::merge_file) turns them
/// into unique set and action names. Names are the list's only identity, so they are never
/// written back.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "RawFile")]
pub struct ActionsFile {
    /// The file version, [`ActionsFile::VERSION`].
    pub version: u32,
    /// The actions in list order.
    pub actions: Vec<Action>,
    /// Sets that hold no action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<String>,
    /// Every set name in display order, empty sets included. Written only when loading `actions`
    /// and `sets` would not give this order (the actions' sets in order of first appearance, then
    /// the empty sets), so files in that natural order, PhotoCraft's included, save unchanged.
    /// Files without it load in the natural order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set_order: Vec<String>,
    /// The folder occurrences a PhotoCraft file carried; empty for every other file.
    #[serde(skip)]
    pub(super) occurrences: FileOccurrences,
}

/// What a PhotoCraft file says beside the names: its ordered folder occurrences (same-name and
/// empty ones included) and, per action at the same index, its occurrence, stored id and whether
/// it owns the old name-keyed shortcut among actions of its name.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct FileOccurrences {
    pub(super) set_rows: Vec<String>,
    pub(super) actions: Vec<ActionOccurrence>,
}

/// One action's occurrence fields in a PhotoCraft file.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
pub(super) struct ActionOccurrence {
    #[serde(default)]
    pub(super) set_slot: Option<usize>,
    #[serde(default)]
    pub(super) action_id: Option<String>,
    #[serde(default)]
    pub(super) legacy_shortcut_owner: bool,
}

/// The file as it is read: each action with its occurrence fields beside it.
#[derive(serde::Deserialize)]
struct RawFile {
    version: u32,
    actions: Vec<RawAction>,
    #[serde(default)]
    sets: Vec<String>,
    #[serde(default)]
    set_order: Vec<String>,
    #[serde(default)]
    set_rows: Vec<String>,
}

#[derive(serde::Deserialize)]
struct RawAction {
    #[serde(flatten)]
    action: Action,
    #[serde(flatten)]
    occurrence: ActionOccurrence,
}

impl From<RawFile> for ActionsFile {
    fn from(raw: RawFile) -> Self {
        let (actions, occurrences): (Vec<Action>, Vec<ActionOccurrence>) = raw.actions.into_iter().map(|raw| (raw.action, raw.occurrence)).unzip();
        let empty = raw.set_rows.is_empty() && occurrences.iter().all(|occurrence| *occurrence == ActionOccurrence::default());
        let occurrences = if empty { FileOccurrences::default() } else { FileOccurrences { set_rows: raw.set_rows, actions: occurrences } };
        ActionsFile { version: raw.version, actions, sets: raw.sets, set_order: raw.set_order, occurrences }
    }
}

/// Why an actions file did not load.
#[derive(Debug)]
#[non_exhaustive]
pub enum ActionsFileError {
    /// The file's version is not [`ActionsFile::VERSION`]; a newer file is refused so it is never
    /// loaded and then overwritten.
    Version(u32),
    /// The text is not a valid actions file.
    Json(serde_json::Error),
}

impl std::fmt::Display for ActionsFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionsFileError::Version(version) => write!(f, "unsupported actions file version {version}"),
            ActionsFileError::Json(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ActionsFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ActionsFileError::Version(_) => None,
            ActionsFileError::Json(error) => Some(error),
        }
    }
}

impl From<serde_json::Error> for ActionsFileError {
    fn from(error: serde_json::Error) -> Self {
        ActionsFileError::Json(error)
    }
}

impl ActionsFile {
    /// The version this crate writes (PhotoCraft's).
    // pc: crates/engine/src/preset_store.rs:1112.
    pub const VERSION: u32 = 1;

    /// A file of version [`VERSION`](Self::VERSION) with `actions` and the empty `sets`.
    pub fn new(actions: Vec<Action>, sets: Vec<String>) -> Self {
        ActionsFile { version: Self::VERSION, actions, sets, set_order: Vec::new(), occurrences: FileOccurrences::default() }
    }

    /// Reads a file's JSON text.
    pub fn from_json(text: &str) -> Result<ActionsFile, ActionsFileError> {
        let file: ActionsFile = serde_json::from_str(text)?;
        if file.version != Self::VERSION {
            return Err(ActionsFileError::Version(file.version));
        }
        Ok(file)
    }

    /// The file as pretty JSON text.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
