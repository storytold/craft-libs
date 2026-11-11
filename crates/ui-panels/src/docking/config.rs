//! `DockConfig`: how a dock behaves, apart from how it looks (the dock's `ComponentStyle`) and what it says
//! (`DockWords`).

use super::targets::ZoneRule;
use craft_behavior::DockSide;

/// A set of the sides of the layout's outer edge or of a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct DockSides {
    /// Before the layout or group along the reading direction.
    pub start: bool,
    /// After it along the reading direction.
    pub end: bool,
    /// Above it.
    pub top: bool,
    /// Below it.
    pub bottom: bool,
}

impl DockSides {
    /// All four sides.
    pub const ALL: DockSides = DockSides { start: true, end: true, top: true, bottom: true };
    /// No side.
    pub const NONE: DockSides = DockSides { start: false, end: false, top: false, bottom: false };

    /// Whether `side` is in the set; never `DockSide::Center`.
    pub const fn contains(self, side: DockSide) -> bool {
        match side {
            DockSide::Start => self.start,
            DockSide::End => self.end,
            DockSide::Top => self.top,
            DockSide::Bottom => self.bottom,
            _ => false,
        }
    }
}

/// How a dock behaves. The sizes of its drop bands and zones are `DockSizes`; the
/// look is the dock's style; the words are `DockWords`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockConfig {
    /// The picture-frame profile: a group's side zones are `DockSizes::zone_depth` deep and the outer edge band
    /// also runs along the start and end edges, where a drop adds a full-height column. Off: only the top and
    /// bottom edges add a full-width row, a group's zones are `SIDE_ZONE_FRACTION` of its body and every preview
    /// is a line or a tab outline.
    pub side_edge_zones: bool,
    /// Which sides of the layout's outer edge, and of a group that takes no tabs (a canvas) without
    /// `new_dock_zones`, take a drop. With `new_dock_zones` a canvas body takes no drop. Without
    /// `side_edge_zones` the outer start and end edges are new docks, so only top and bottom apply there. A
    /// dock-column app whose palettes dock only at the start and end sets `DockSides::NONE` with `new_dock_zones`
    /// on. Not used with `side_edge_zones`. Groups that take tabs keep all four sides.
    pub drop_edges: DockSides,
    /// The docking profile: the docked area's start and end edges, where no dock is yet, take a drop as a new dock.
    /// Without `side_edge_zones`, the body of a frame or floating group that takes no tabs takes no drop: a
    /// release there floats as over no target.
    pub new_dock_zones: bool,
    /// Group menu item Close Other Panels in Group.
    pub close_others_item: bool,
    /// Group menu item Maximize Frame / Restore Frame Size, for groups of the split tree.
    pub maximize_item: bool,
    /// A drag released over no drop target inside the window floats what it carries, such as a panel dropped on a
    /// protected canvas; a release over the dragged unit's own group leaves it where it is. Off: that release cancels
    /// the drag; only Ctrl/Cmd held at the release or a release outside the window floats.
    pub float_on_empty_drop: bool,
    /// Dragging a floating window's title bar onto a drop target docks the whole window there. Off: the title bar
    /// only moves the window; tabs and drag bars dock.
    pub title_bar_docks: bool,
    /// Shows the built-in Close items in tab and group menus for closable panels. A host that draws its own Close
    /// items turns this off.
    pub close_menu_items: bool,
    /// Shows a close action on the title bar of a floating window whose panels are all closable.
    pub close_button: bool,
    /// Shows a close button on the tabs of closable panels in docked and floating groups, where a middle click also
    /// closes them, for tabs that act as documents. Off: panels close from their menu. Accordion headers and iconic
    /// columns never show it.
    pub tab_close_buttons: bool,
    /// A floating window the dock model keeps native opens in an OS window of its own where the platform opens
    /// windows and its panels allow it (`PanelSpec::allow_native`). Off: every floating window shows in the app
    /// window.
    pub native_windows: bool,
}

impl Default for DockConfig {
    fn default() -> Self {
        DockConfig {
            side_edge_zones: false,
            drop_edges: DockSides::ALL,
            new_dock_zones: true,
            close_others_item: false,
            maximize_item: false,
            float_on_empty_drop: true,
            title_bar_docks: true,
            close_menu_items: true,
            close_button: true,
            tab_close_buttons: false,
            native_windows: false,
        }
    }
}

impl DockConfig {
    /// The picture-frame profile on or off.
    #[must_use]
    pub fn side_edge_zones(mut self, on: bool) -> Self {
        self.side_edge_zones = on;
        self
    }

    /// The sides of the outer edge that take a drop.
    #[must_use]
    pub fn drop_edges(mut self, sides: DockSides) -> Self {
        self.drop_edges = sides;
        self
    }

    /// New docks at the start and end edges on or off.
    #[must_use]
    pub fn new_dock_zones(mut self, on: bool) -> Self {
        self.new_dock_zones = on;
        self
    }

    /// The Close Other Panels in Group item on or off.
    #[must_use]
    pub fn close_others_item(mut self, on: bool) -> Self {
        self.close_others_item = on;
        self
    }

    /// The Maximize Frame item on or off.
    #[must_use]
    pub fn maximize_item(mut self, on: bool) -> Self {
        self.maximize_item = on;
        self
    }

    /// Whether a release over no target floats.
    #[must_use]
    pub fn float_on_empty_drop(mut self, on: bool) -> Self {
        self.float_on_empty_drop = on;
        self
    }

    /// Whether a floating window's title bar docks it.
    #[must_use]
    pub fn title_bar_docks(mut self, on: bool) -> Self {
        self.title_bar_docks = on;
        self
    }

    /// The built-in Close items on or off.
    #[must_use]
    pub fn close_menu_items(mut self, on: bool) -> Self {
        self.close_menu_items = on;
        self
    }

    /// The close action on floating windows on or off.
    #[must_use]
    pub fn close_button(mut self, on: bool) -> Self {
        self.close_button = on;
        self
    }

    /// Close buttons on tabs on or off.
    #[must_use]
    pub fn tab_close_buttons(mut self, on: bool) -> Self {
        self.tab_close_buttons = on;
        self
    }

    /// Native floating windows on or off.
    #[must_use]
    pub fn native_windows(mut self, on: bool) -> Self {
        self.native_windows = on;
        self
    }

    /// The sides of a group that takes no tabs that take a drop beside it: all four with `side_edge_zones`, else
    /// `drop_edges`.
    pub fn drop_sides(&self) -> DockSides {
        if self.side_edge_zones { DockSides::ALL } else { self.drop_edges }
    }

    /// How a group's body divides into side zones in this profile.
    pub fn zone_rule(&self) -> ZoneRule {
        if self.side_edge_zones { ZoneRule::Depth } else { ZoneRule::Fraction }
    }
}
