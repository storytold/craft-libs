//! What may happen to a panel, and why an edit was refused.

use std::fmt;

/// What may happen to a panel. `Layout::apply_with_permissions` enforces these for every
/// action, whoever issues it. They also decide which gestures and menu items the renderer
/// offers; a renderer hides the built-in close controls of a panel that stays closable by command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::PlainData)]
#[non_exhaustive]
pub struct Permissions {
    /// The panel may be moved.
    pub movable: bool,
    /// The panel may be floated.
    pub floatable: bool,
    /// The panel may be closed.
    pub closable: bool,
    /// Other panels may join this panel's group as tabs.
    pub accepts_tabs: bool,
    /// Other panels may split this panel's frame (`DockPlacement::Split` with a side zone).
    /// Outer-edge frames (`DockPlacement::Edge`) are not splits of the anchor.
    pub accepts_splits: bool,
    /// The panel's column may collapse to icons: refused are `SetIconic` to icons on a column holding it, and a
    /// move of it into a collapsed column.
    pub collapses: bool,
    /// Other groups may share the panel's column, and the panel may join a column or group that holds other
    /// panels: refused are tab, center, group and seam placements across that boundary in either direction, while a
    /// new column, a new dock and a float stay allowed. A toolbar takes its own column.
    pub shares_column: bool,
    /// The panel may dock outside a dock column: in a frame of the docked tree, as a split beside another frame or at
    /// an edge of the docked area, where it can lie across the window. Without it the panel docks only in dock columns
    /// or floats: refused is any action that leaves it in a frame, so a toolbar is always a vertical strip.
    pub docks_outside_columns: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self::ALL
    }
}

impl Permissions {
    /// Every right: the panel moves, floats, closes, takes tabs and splits, collapses, shares its column and docks
    /// outside dock columns.
    pub const ALL: Self = Self {
        movable: true,
        floatable: true,
        closable: true,
        accepts_tabs: true,
        accepts_splits: true,
        collapses: true,
        shares_column: true,
        docks_outside_columns: true,
    };

    /// A document canvas: it stays docked and open, and takes neither tabs nor side splits.
    /// A group dropped on it floats there. An application that opens panels beside its canvas
    /// uses `Permissions::PROTECTED.accepts_splits(true)`.
    pub const PROTECTED: Self = Self { movable: false, floatable: false, closable: false, accepts_tabs: false, accepts_splits: false, ..Self::ALL };

    /// These permissions with [`Self::movable`] set to `movable`.
    pub const fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    /// These permissions with [`Self::floatable`] set to `floatable`.
    pub const fn floatable(mut self, floatable: bool) -> Self {
        self.floatable = floatable;
        self
    }

    /// These permissions with [`Self::closable`] set to `closable`.
    pub const fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// These permissions with [`Self::accepts_tabs`] set to `accepts_tabs`.
    pub const fn accepts_tabs(mut self, accepts_tabs: bool) -> Self {
        self.accepts_tabs = accepts_tabs;
        self
    }

    /// These permissions with [`Self::accepts_splits`] set to `accepts_splits`.
    pub const fn accepts_splits(mut self, accepts_splits: bool) -> Self {
        self.accepts_splits = accepts_splits;
        self
    }

    /// These permissions with [`Self::collapses`] set to `collapses`.
    pub const fn collapses(mut self, collapses: bool) -> Self {
        self.collapses = collapses;
        self
    }

    /// These permissions with [`Self::shares_column`] set to `shares_column`.
    pub const fn shares_column(mut self, shares_column: bool) -> Self {
        self.shares_column = shares_column;
        self
    }

    /// These permissions with [`Self::docks_outside_columns`] set to `docks_outside_columns`.
    pub const fn docks_outside_columns(mut self, docks_outside_columns: bool) -> Self {
        self.docks_outside_columns = docks_outside_columns;
        self
    }

    /// A canvas: it can neither move, close nor take tabs. `accepts_splits` and `floatable`
    /// do not matter, so a canvas that opens panels beside it still counts.
    pub fn is_protected(self) -> bool {
        !self.movable && !self.closable && !self.accepts_tabs
    }

    /// Lock Workspace: nothing moves, floats or closes; tab and split drops and the column rights keep their policy.
    /// A workspace applies this when it is locked; an application with a global lock preference
    /// applies it in its own permissions callback.
    pub fn locked(self) -> Self {
        Self { movable: false, floatable: false, closable: false, ..self }
    }
}

/// Why a layout edit or load was refused. The layout is unchanged when an edit fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockError {
    /// The layout is deeper or larger than `MAX_DEPTH` or `MAX_PANELS` allow.
    LimitExceeded,
    /// A panel occurs in more than one place.
    DuplicatePanel,
    /// The panel to act on is not in the layout.
    MissingPanel,
    /// The anchor or destination is not in the layout.
    MissingTarget,
    /// An active tab index is out of range.
    InvalidSelection,
    /// A size or rect is not finite or is out of bounds.
    InvalidGeometry,
    /// The destination does not support the placement.
    InvalidPlacement,
    /// The panel's [`Permissions`] forbid the action.
    NotAllowed,
    /// Saved by a newer version of the docking format.
    UnsupportedVersion,
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LimitExceeded => "docking layout exceeds its size or depth limit",
            Self::DuplicatePanel => "panel occurs in more than one docking location",
            Self::MissingPanel => "source panel is not present in the docking layout",
            Self::MissingTarget => "docking destination is no longer present",
            Self::InvalidSelection => "active docking tab is out of bounds",
            Self::InvalidGeometry => "docking geometry is not finite and bounded",
            Self::InvalidPlacement => "docking destination does not support this placement",
            Self::NotAllowed => "this panel does not allow the requested docking operation",
            Self::UnsupportedVersion => "docking data was saved by a newer version",
        })
    }
}

impl std::error::Error for DockError {}
