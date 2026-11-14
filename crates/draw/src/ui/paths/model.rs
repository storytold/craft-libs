//! The data of the Paths panel: its rows, events and output.

use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_ui::contract::{NoValue, Outcome};
use egui::Response;

/// What a path row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PathKind {
    /// A saved path.
    Saved,
    /// The work path: italic, one per document.
    Work,
    /// The path of the selected shape layer.
    ShapePath,
    /// The path of the selected layer's vector mask.
    VectorMask,
}

impl PathKind {
    /// Whether a row of this kind is renamed in place.
    pub fn renames(self) -> bool {
        let _ = self;
        todo!("tl.T34")
    }

    /// Whether a row of this kind can be dragged.
    pub fn drags(self) -> bool {
        let _ = self;
        todo!("tl.T34")
    }

    /// Whether a row of this kind is shown in italics: the work path.
    pub fn italic(self) -> bool {
        matches!(self, PathKind::Work)
    }
}

/// One row of the panel.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PathRow<K> {
    /// The row's key.
    pub key: K,
    /// What the row is.
    pub kind: PathKind,
    /// The path's name.
    pub name: String,
    /// The outline the thumbnail draws, in document pixels.
    pub outline: kurbo::BezPath,
    /// The row is the selected path.
    pub selected: bool,
}

impl<K> PathRow<K> {
    /// An unselected row of `kind` named `name` with `outline`.
    pub fn new(key: K, kind: PathKind, name: impl Into<String>, outline: kurbo::BezPath) -> Self {
        PathRow { key, kind, name: name.into(), outline, selected: false }
    }
}

/// What the Paths panel reports, in the order it happened.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PathsEvent<K> {
    /// A row was selected; `None` for a click on the empty area, which deselects.
    // P3(tools-w10): tl.T34 `Select(Option<K>)`
    Select(Option<K>),
    /// A rename committed: never empty after trimming, never unchanged.
    #[non_exhaustive]
    Rename {
        /// The path.
        key: K,
        /// The new name, trimmed.
        text: String,
    },
    /// A Cmd/Ctrl-click on the thumbnail: load the path as a selection with the operation of Shift and Alt.
    #[non_exhaustive]
    LoadSelection {
        /// The path.
        key: K,
        /// New, add, subtract or intersect.
        operation: SelectionOperation,
    },
    /// A command of the panel menu or the row menu, with the row the menu was opened on (`None` for a panel-menu
    /// command with no target row).
    // P3(tools-w10): tl.T34 `Command { command, key }`
    #[non_exhaustive]
    Command {
        /// The command.
        command: super::PathsCommands,
        /// The row it acts on.
        key: Option<K>,
    },
    /// A footer button was clicked, with the selected rows and the click's modifiers.
    // P3(tools-w10): tl.T34 `Footer`
    #[non_exhaustive]
    Footer {
        /// The button.
        button: super::PathsFooterButton,
        /// The selected rows.
        rows: Vec<K>,
        /// The click's modifiers.
        mods: craft_keymap::Mods,
    },
    /// Rows were dropped on a footer button.
    // P3(tools-w10): tl.T34 `FooterDrop`
    #[non_exhaustive]
    FooterDrop {
        /// The button.
        button: super::PathsFooterButton,
        /// The dropped rows.
        rows: Vec<K>,
    },
}

impl<K> PathsEvent<K> {
    /// A committed rename of `key` to `text`.
    // P3(tools-w10): tl.T34 constructors
    pub fn rename(key: K, text: impl Into<String>) -> Self {
        let _ = (key, text);
        todo!("tl.T34")
    }

    /// A load of `key` as a selection with `operation`.
    pub fn load_selection(key: K, operation: SelectionOperation) -> Self {
        let _ = (key, operation);
        todo!("tl.T34")
    }

    /// `command` on the row `key`.
    pub fn command(command: super::PathsCommands, key: Option<K>) -> Self {
        let _ = (command, key);
        todo!("tl.T34")
    }

    /// A click on the footer `button` with `rows` selected and `mods` held.
    pub fn footer(button: super::PathsFooterButton, rows: Vec<K>, mods: craft_keymap::Mods) -> Self {
        let _ = (button, rows, mods);
        todo!("tl.T34")
    }

    /// `rows` dropped on the footer `button`.
    pub fn footer_drop(button: super::PathsFooterButton, rows: Vec<K>) -> Self {
        let _ = (button, rows);
        todo!("tl.T34")
    }
}

/// What [`PathsPanel::show`](super::PathsPanel::show) returns.
#[derive(Debug)]
#[must_use = "the app applies the panel's events"]
#[non_exhaustive]
pub struct PathsOutput<K> {
    /// What the user did this pass, in order.
    pub events: Vec<PathsEvent<K>>,
    /// The panel.
    pub response: Response,
}

impl<K> Outcome for PathsOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}
