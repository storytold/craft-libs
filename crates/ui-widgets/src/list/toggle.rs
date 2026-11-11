//! `ToggleCell`: what a row draws in the cell of a toggle column.
//!
//! The column itself is the collection's (`ColumnKind::Toggle`): it flips a cell on a press, sweeps one value over
//! the cells a drag crosses and reports `CollectionEvent::Toggle`. The row only shows the state, so a cell carries
//! the state the source reports for it (`ItemSource::toggled`) and how to draw it.

use super::words::ListWords;
use crate::runtime::env::Label;
use crate::style::DataColor;
use craft_behavior::{ColumnKey, IconId};

/// The state a toggle cell shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ToggleState {
    /// Off.
    #[default]
    Off,
    /// On.
    On,
    /// Some of what the row stands for is on and some off. A press makes it on.
    Mixed,
}

impl ToggleState {
    /// `On` for `true`, `Off` for `false`.
    pub const fn of(on: bool) -> Self {
        if on { ToggleState::On } else { ToggleState::Off }
    }
}

/// How a toggle cell draws its state.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ToggleFace {
    /// An icon while on; the off icon shows under the pointer, or always with `always_visible`.
    #[non_exhaustive]
    Icons {
        /// Drawn while on (and while mixed).
        on: IconId,
        /// Drawn while off.
        off: IconId,
    },
    /// A check box: the box while off, the check while on, the dash while mixed.
    Check,
}

impl ToggleFace {
    /// The icon pair: `on` while on (and mixed), `off` while off.
    pub const fn icons(on: IconId, off: IconId) -> Self {
        ToggleFace::Icons { on, off }
    }
}

/// Where a cell's accessible name comes from.
#[derive(Clone, Debug)]
enum CellName {
    /// The name the cell's caller gave.
    Given(Label),
    /// The list words' visibility name.
    Visibility,
    /// The list words' lock name.
    Lock,
}

/// One toggle cell of a row, in the column `column`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ToggleCell {
    /// The column the cell belongs to.
    pub column: ColumnKey,
    /// How the cell draws.
    pub face: ToggleFace,
    /// The state it shows.
    pub state: ToggleState,
    name: CellName,
    /// The tooltip, the same in every row.
    pub tooltip: Option<Label>,
    /// A fill under the icon (a layer's label colour).
    pub fill: Option<DataColor>,
    /// The off icon shows in every row, not only under the pointer. No effect on a check face.
    pub always_visible: bool,
    /// A screen reader announces a check box instead of a toggle button: for a setting the row carries rather than
    /// an action on the row.
    pub check_box: bool,
}

impl ToggleCell {
    /// A cell of `column` showing `state` with `face`, named `name`: a whole pattern with `{row}` for the row's label
    /// ("Keep {row}").
    pub fn new(column: ColumnKey, face: ToggleFace, state: ToggleState, name: impl Into<Label>) -> Self {
        ToggleCell { column, face, state, name: CellName::Given(name.into()), tooltip: None, fill: None, always_visible: false, check_box: false }
    }

    /// An eye cell, named "Visibility {row}" unless [`name`](Self::name) says otherwise.
    pub fn eye(column: ColumnKey, on: bool) -> Self {
        let mut cell = Self::new(column, ToggleFace::Icons { on: IconId::new("eye"), off: IconId::new("eye_off") }, ToggleState::of(on), "");
        cell.name = CellName::Visibility;
        cell
    }

    /// A lock cell, named "Lock {row}" unless [`name`](Self::name) says otherwise.
    pub fn lock(column: ColumnKey, on: bool) -> Self {
        let mut cell = Self::new(column, ToggleFace::Icons { on: IconId::new("lock"), off: IconId::new("lock_open") }, ToggleState::of(on), "");
        cell.name = CellName::Lock;
        cell
    }

    /// A check box cell, announced as a check box.
    pub fn check(column: ColumnKey, state: ToggleState, name: impl Into<Label>) -> Self {
        let mut cell = Self::new(column, ToggleFace::Check, state, name);
        cell.check_box = true;
        cell
    }

    /// The cell's name for a screen reader in place of its default: a whole pattern with `{row}` for the row's label.
    #[must_use]
    pub fn name(mut self, name: impl Into<Label>) -> Self {
        self.name = CellName::Given(name.into());
        self
    }

    /// The name pattern the cell announces: the one given, else the list words' for its kind.
    pub(super) fn name_pattern<'a>(&'a self, words: &'a ListWords) -> &'a Label {
        match &self.name {
            CellName::Given(name) => name,
            CellName::Visibility => &words.toggle_visibility,
            CellName::Lock => &words.toggle_lock,
        }
    }

    /// The tooltip of the cell.
    #[must_use]
    pub fn tooltip(mut self, tooltip: impl Into<Label>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Fills the cell under its icon.
    #[must_use]
    pub fn fill(mut self, color: DataColor) -> Self {
        self.fill = Some(color);
        self
    }

    /// The off icon shows in every row.
    #[must_use]
    pub fn always_visible(mut self) -> Self {
        self.always_visible = true;
        self
    }
}
