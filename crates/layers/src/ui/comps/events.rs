//! What the Layer Comps panel reports and the commands of its row menu.

use craft_behavior::EditEvent;
use craft_ui::contract::{CommandSet, NoValue, Outcome};
use egui::Response;

use super::CompOption;

/// What the user did in the Layer Comps panel, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LayerCompsEvent {
    /// The fixed Last Document State row was activated.
    RestoreLastDocumentState,
    /// A row was selected.
    Select(u64),
    /// A comp was applied (double click or its apply mark).
    Apply(u64),
    /// A rename ended.
    #[non_exhaustive]
    Rename {
        /// The comp.
        comp: u64,
        /// The new name.
        name: String,
    },
    /// A comp was dragged or moved by key to `index`, counted among the comps.
    #[non_exhaustive]
    Reorder {
        /// The comp.
        comp: u64,
        /// Its new place.
        index: usize,
    },
    /// An option toggle of one or more comps, one gesture per sweep (`Update`s, a `Commit` on
    /// release, a `Cancel` carrying the earlier state).
    #[non_exhaustive]
    SetOption {
        /// The comps the gesture touches.
        comps: Vec<u64>,
        /// The option.
        option: CompOption,
        /// The edit.
        edit: EditEvent<bool>,
    },
    /// The warning on this comp was cleared.
    ClearWarning(u64),
    /// Apply the previous comp.
    Previous,
    /// Apply the next comp.
    Next,
    /// Update this comp from the document.
    Update(u64),
    /// Create a comp.
    New,
    /// Delete this comp.
    Delete(u64),
}

/// What [`LayerCompsPanel::show`](super::LayerCompsPanel::show) returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app applies the panel's events"]
pub struct LayerCompsOutput {
    /// The panel.
    pub response: Response,
    /// What the user did this pass, in order.
    pub events: Vec<LayerCompsEvent>,
}

impl Outcome for LayerCompsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The row menu's commands: move the comp. They have no default chords: the list's stacking keys already move rows and
/// arrive as [`LayerCompsEvent::Reorder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "layerCompsPanel")]
#[non_exhaustive]
pub enum LayerCompsRowCommands {
    /// Move the comp up.
    #[command(label = "Move Up")]
    MoveUp,
    /// Move the comp down.
    #[command(label = "Move Down")]
    MoveDown,
}
