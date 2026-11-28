//! `DockConfig`: how a dock behaves, apart from how it looks (the dock's `ComponentStyle`) and what it says
//! (`DockWords`).

use super::targets::ZoneRule;
use super::{ColumnCards, IconicMode};
use crate::tabs::PanelTabLook;
use craft_behavior::{DockSide, IconId};

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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    /// only moves the window; tabs and drag bars dock, and so does a window that took over a panel's drag
    /// (`DockArea::continue_move`).
    pub title_bar_docks: bool,
    /// Shows the built-in Close items in tab and group menus for closable panels. A host that draws its own Close
    /// items turns this off.
    pub close_menu_items: bool,
    /// Shows a close action on the title bar of a floating window whose panels are all closable.
    pub close_button: bool,
    /// A floating window's title row is a bare grip (no menu, float or close action, no column arrow): a quick-action
    /// strip's look.
    pub floating_grip: bool,
    /// The surfaces inside a floating window keep to its rounded corners, and its edge shows over them along its
    /// sides ([`FloatingGroup::rounded_body`](super::FloatingGroup)): a floating tool rail reads as one rounded piece.
    /// Off: the panels and rails inside paint their own square surfaces to the window's edge.
    pub floating_rounded_body: bool,
    /// Which docked columns show as cards on the dock's `surface.background` ([`ColumnCards`]), each
    /// painted `surface.panel` with `shape.radius.surface` corners and no frame, inset by `space.gap.stack` on its top
    /// and bottom and, for the column at the window edge, on its outer side; each further column stands
    /// `space.gap.tight` from its outer neighbour's card. A column is wider by its inset, so it keeps its content's
    /// width (a tool rail its steps); its dividers, drop bands and edge handle stay on its outer rect. Off: a column
    /// fills its cell.
    pub column_cards: ColumnCards,
    /// Docks stand flush against the node beside them, with no gap, and their columns collapsed to icons show the
    /// `line.border` frame expanded columns show, so one rule parts a dock from the canvas; the flyouts of those columns
    /// stand square on the panel surface with that frame and no shadow ([`Flyout::docked`](super::Flyout::docked)):
    /// VectorCraft 0.24's dock.
    /// Off: a `space.gap.tight` gap of the window's background parts them.
    pub flush_docks: bool,
    /// Shows a close button on the tabs of closable panels in docked and floating groups, where a middle click also
    /// closes them, for tabs that act as documents. Off: panels close from their menu. Accordion headers and iconic
    /// columns never show it.
    pub tab_close_buttons: bool,
    /// A group's header shows the shown panel's title, its count and a close button in place of a tab strip, as a
    /// single titled panel does; the panels are shown from a rail or a menu instead. Closing follows
    /// `tab_close_buttons`.
    pub titled_headers: bool,
    /// The wheel over a tab strip shows the next or previous panel, one per wheel notch.
    pub wheel_selects_tabs: bool,
    /// The look of the groups' panel tabs: the shown tab on the panel surface ([`PanelTabLook::Plain`]), on a
    /// filled rounded pill ([`PanelTabLook::Pill`]), or a square filled block on a canvas strip
    /// ([`PanelTabLook::Block`]).
    pub tab_look: PanelTabLook,
    /// A floating window opens in an OS window of its own where the window host opens OS windows and reports where
    /// they are on the desktop (not on Wayland) and its panels allow it (`PanelSpec::allow_native`). Elsewhere, and
    /// with this off, it shows in the app window. A window that takes over a held press (`DockArea::continue_move`)
    /// opens its OS window once that press ends.
    pub native_windows: bool,
    /// Docked groups' menus get Minimize and Move Group Up and Down.
    pub group_order_items: bool,
    /// Every group shows its panel menu button, whose menu holds the dock's own rows when the shown panel has none of
    /// its own (`DockPanels::panel_rows`). Off: only a group whose shown panel has rows of its own shows the button.
    pub panel_menu_always: bool,
    /// The panel menu button's icon: `more` (the default), or `menu` for the three-line mark.
    pub panel_menu_icon: IconId,
    /// What the icons of a docked column collapsed to icons open: flyouts, or with [`IconicMode::Rail`] the column
    /// itself, whose icons then stay beside it while it is expanded. Floating windows' icons open flyouts.
    pub iconic_mode: IconicMode,
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
            floating_grip: false,
            floating_rounded_body: false,
            column_cards: ColumnCards::None,
            flush_docks: false,
            tab_close_buttons: false,
            titled_headers: false,
            wheel_selects_tabs: false,
            tab_look: PanelTabLook::Plain,
            native_windows: true,
            group_order_items: false,
            panel_menu_always: false,
            panel_menu_icon: IconId::new("more"),
            iconic_mode: IconicMode::Flyouts,
        }
    }
}

impl DockConfig {
    /// What the icons of docked iconic columns open.
    #[must_use]
    pub fn iconic_mode(mut self, mode: IconicMode) -> Self {
        self.iconic_mode = mode;
        self
    }

    /// The Minimize and Move Group Up and Down items of docked groups' menus on or off.
    #[must_use]
    pub fn group_order_items(mut self, on: bool) -> Self {
        self.group_order_items = on;
        self
    }

    /// The panel menu button on every group on or off ([`panel_menu_always`](Self::panel_menu_always)).
    #[must_use]
    pub fn panel_menu_always(mut self, on: bool) -> Self {
        self.panel_menu_always = on;
        self
    }

    /// The panel menu button's icon ([`panel_menu_icon`](Self::panel_menu_icon)).
    #[must_use]
    pub fn panel_menu_icon(mut self, icon: IconId) -> Self {
        self.panel_menu_icon = icon;
        self
    }

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

    /// A floating window's title row as a bare grip on or off.
    #[must_use]
    pub fn floating_grip(mut self, on: bool) -> Self {
        self.floating_grip = on;
        self
    }

    /// A floating window's body keeping to its rounded corners on or off ([`floating_rounded_body`](Self::floating_rounded_body)).
    #[must_use]
    pub fn floating_rounded_body(mut self, on: bool) -> Self {
        self.floating_rounded_body = on;
        self
    }

    /// Which docked columns show as cards ([`column_cards`](Self::column_cards)).
    #[must_use]
    pub fn column_cards(mut self, cards: ColumnCards) -> Self {
        self.column_cards = cards;
        self
    }

    /// The look of the groups' panel tabs ([`tab_look`](Self::tab_look)).
    #[must_use]
    pub fn tab_look(mut self, look: PanelTabLook) -> Self {
        self.tab_look = look;
        self
    }

    /// Flush docks on or off ([`flush_docks`](Self::flush_docks)).
    #[must_use]
    pub fn flush_docks(mut self, on: bool) -> Self {
        self.flush_docks = on;
        self
    }

    /// Close buttons on tabs on or off.
    #[must_use]
    pub fn tab_close_buttons(mut self, on: bool) -> Self {
        self.tab_close_buttons = on;
        self
    }

    /// Titled group headers on or off.
    #[must_use]
    pub fn titled_headers(mut self, on: bool) -> Self {
        self.titled_headers = on;
        self
    }

    /// See [`DockConfig::wheel_selects_tabs`].
    #[must_use]
    pub fn wheel_selects_tabs(mut self, on: bool) -> Self {
        self.wheel_selects_tabs = on;
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
