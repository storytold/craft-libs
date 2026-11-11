//! What a preset browser reports.

use super::footer::PresetFooterEvent;
use crate::contract::{ListDrag, NoValue, OpenChange, Outcome, ReorderMove};
use craft_behavior::EditEvent;
use craft_keymap::Mods;
use egui::{Pos2, Response};

/// What a preset browser reports, in the order it happened. The host performs every command; the browser changes
/// only its `PresetState`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PresetEvent<K> {
    /// A click or Space on a preset. Not an edit: the host sets its selection (and applies the preset); `mods` let
    /// Alt, Ctrl or Cmd choose a variant of the application.
    #[non_exhaustive]
    Select {
        /// The preset.
        key: K,
        /// The modifiers held.
        mods: Mods,
    },
    /// A double click or Enter on a preset.
    #[non_exhaustive]
    Activate {
        /// The preset.
        key: K,
        /// The modifiers held.
        mods: Mods,
    },
    /// A group was opened or closed. The browser has already changed its state; the event lets a host with its own
    /// open state follow.
    Open(OpenChange<K>),
    /// A rename was committed with a changed, non-empty name.
    #[non_exhaustive]
    Rename {
        /// The preset or group.
        key: K,
        /// The new name.
        text: String,
    },
    /// Delete was chosen (context menu, Delete Group) for a preset or group.
    Delete(K),
    /// Move to was chosen: the preset goes to the end of `group`.
    #[non_exhaustive]
    MoveTo {
        /// The preset.
        key: K,
        /// The group (or folder) it goes to.
        group: K,
    },
    /// A drag to reorder, or a Cmd/Ctrl+] or [ move, as one edit gesture.
    Reorder(EditEvent<ReorderMove<K>>),
    /// A preset was dragged from the browser and released inside its drop zone (a canvas, a timeline, a layer list):
    /// the host applies `key` at `at`. Items only: a group dropped on the zone reports nothing.
    #[non_exhaustive]
    DropAt {
        /// The preset.
        key: K,
        /// The pointer at the release, in points.
        at: Pos2,
        /// The modifiers held at the release.
        mods: Mods,
    },
    /// A button of the browser's footer was pressed, in the order it happened among the other events.
    Footer(PresetFooterEvent<K>),
    /// An action row (`PresetBrowser::actions`) was clicked or took Enter: its index among the actions.
    Action(usize),
    /// A row the host added to the context menu was chosen for `target`.
    #[non_exhaustive]
    Menu {
        /// The preset or group the menu was opened on.
        target: K,
        /// The command the host gave the row.
        command: String,
    },
}

/// What `PresetBrowser::show` returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the browser's events"]
pub struct PresetOutput<K> {
    /// The browser: its focus and its rect.
    pub response: Response,
    /// What happened this frame, in order.
    pub events: Vec<PresetEvent<K>>,
    /// While a preset or group is dragged, its payload, so the host can draw its own drop highlight; `None`
    /// otherwise.
    pub drag: Option<ListDrag<K>>,
    /// The preset under the pointer this pass, for a hover preview; `None` off the list, over a header, or while a
    /// drag is under way.
    pub hovered: Option<K>,
}

impl<K> PresetOutput<K> {
    /// The output of a browser pass.
    pub fn new(response: Response, events: Vec<PresetEvent<K>>, drag: Option<ListDrag<K>>, hovered: Option<K>) -> Self {
        PresetOutput { response, events, drag, hovered }
    }
}

impl<K> Outcome for PresetOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}
