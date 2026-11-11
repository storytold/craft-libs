//! `DockEvent`, `FloatSpace` and `HidePanels`: what a dock reports to the host and the host tells a dock.

use super::{DockCommands, DockSizes};
use egui::{Rect, Vec2};

/// A request for host state outside the layout, or a report of where something ended up.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockEvent<P> {
    /// Maximize Frame: maximize the frame holding `panel`, or restore it when it is the one the host passed as
    /// maximized.
    #[non_exhaustive]
    ToggleMaximize {
        /// A panel of the frame to maximize or restore.
        panel: P,
    },
    /// Auto-Collapse Iconic Panels was chosen in a column menu. The host flips its preference and passes it back.
    ToggleAutoCollapse,
    /// The area opened, switched or closed its flyout: an icon click, Escape, a press outside, or the focus
    /// leaving it under Auto-Collapse. A host that binds the flyout takes it as the request when its binding is given.
    FlyoutChanged {
        /// The panel now in the flyout; `None` when it closed.
        panel: Option<P>,
    },
    /// A panel command that only the host can carry out reached `panel`'s group: Move Panel To and Move Group To ask
    /// for a destination ([`DockCommands::moves`]), which the host passes to `move_actions`.
    PanelCommand {
        /// The shown panel of the group the command reached.
        panel: P,
        /// The command.
        command: DockCommands,
    },
    /// A floating frame holding `panel` was moved or resized to `rect`.
    FloatPlaced {
        /// A panel of the frame.
        panel: P,
        /// The frame's rect in the area's layer space.
        rect: Rect,
    },
}

/// Which panels the host hides. Protected panels always stay.
#[non_exhaustive]
pub enum HidePanels<'a, P> {
    /// Hides none.
    None,
    /// Hides every panel.
    All,
    /// Every panel except those for which the function returns true.
    Except(&'a dyn Fn(&P) -> bool),
}

impl<P> Clone for HidePanels<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for HidePanels<'_, P> {}

/// How much shorter than its body content must be to count as not filling it.
const FILL_TOLERANCE: f32 = 1.0;

/// Where a docking command floats a panel, group or column when it gives no rect. A dock records one each pass;
/// the host reads it to turn a command into placements.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FloatSpace<P> {
    /// The rect the area drew in. A floated unit goes inside it, `inset` from its top-left corner.
    pub area: Rect,
    /// Room between the area's top-left corner and a unit a command floats.
    pub inset: f32,
    /// The height of a floating window's title row (`DockSizes::bar`), which a drop keeps under the pointer.
    pub title_row: f32,
    /// The size a command floats a panel at when the area never drew it.
    pub default_size: Vec2,
    /// What a drag floats from each drawn panel: the panel alone, its group and its dock column.
    panels: Vec<(P, Vec2)>,
    groups: Vec<(P, Vec2)>,
    columns: Vec<(P, Vec2)>,
    /// The height each panel's content used when it was last drawn, which fits a floated group to its content;
    /// `None` for content that filled its body.
    contents: Vec<(P, Option<f32>)>,
    /// The area draws with `DockConfig::side_edge_zones`: an outer edge drop wraps the part between the docks.
    side_edge_zones: bool,
}

impl<P: PartialEq> FloatSpace<P> {
    /// An area with no recorded sizes: commands float a panel at the default float size of `sizes`, its cascade
    /// inset inside the area.
    pub fn new(area: Rect, sizes: &DockSizes) -> Self {
        FloatSpace {
            area,
            inset: sizes.cascade_inset,
            title_row: sizes.bar,
            default_size: sizes.float_size,
            panels: Vec::new(),
            groups: Vec::new(),
            columns: Vec::new(),
            contents: Vec::new(),
            side_edge_zones: false,
        }
    }

    /// For an area drawn with `DockConfig::side_edge_zones` when the area has not drawn yet (a headless run).
    #[must_use]
    pub fn with_side_edge_zones(mut self, on: bool) -> Self {
        self.side_edge_zones = on;
        self
    }

    /// Whether the area draws with `DockConfig::side_edge_zones`.
    pub fn side_edge_zones(&self) -> bool {
        self.side_edge_zones
    }

    /// Adds the size a command floats `panel` at alone when the area has not drawn it, such as a hidden panel or a
    /// headless run. A size the area recorded for the panel takes precedence.
    #[must_use]
    pub fn with_panel_size(mut self, panel: P, size: Vec2) -> Self {
        self.panels.push((panel, size));
        self
    }

    /// The size a command floats `panel` at alone, when recorded.
    pub fn panel(&self, panel: &P) -> Option<Vec2> {
        Self::find(&self.panels, panel)
    }

    /// The size a command floats the group of `panel` at, when recorded.
    pub fn group(&self, panel: &P) -> Option<Vec2> {
        Self::find(&self.groups, panel)
    }

    /// The size a command floats the dock column of `panel` at, when recorded.
    pub fn column(&self, panel: &P) -> Option<Vec2> {
        Self::find(&self.columns, panel)
    }

    fn find(sizes: &[(P, Vec2)], panel: &P) -> Option<Vec2> {
        sizes.iter().find(|(held, _)| held == panel).map(|(_, size)| *size)
    }

    /// The rect of `size` inside the area, inset from its top-left corner and shrunk to fit.
    pub fn place(&self, size: Vec2) -> Rect {
        let size = size.min(self.area.size());
        let min = (self.area.min + Vec2::splat(self.inset)).min(self.area.max - size).max(self.area.min);
        Rect::from_min_size(min, size)
    }

    /// Records the height `panel`'s content used in a body `available` tall. Content that reaches the body's bottom
    /// filled it and has no height of its own, unless the body was sized to the height recorded before (a fitted
    /// window).
    pub fn record(&mut self, panel: P, used: f32, available: f32) {
        let used = if used.is_finite() { used.max(0.0) } else { 0.0 };
        let at = self.contents.iter().position(|(held, _)| *held == panel);
        let fitted = at.and_then(|at| self.contents[at].1).is_some_and(|height| height >= available - FILL_TOLERANCE);
        let height = (used < available - FILL_TOLERANCE || fitted).then_some(used);
        match at {
            Some(at) => self.contents[at].1 = height,
            None => self.contents.push((panel, height)),
        }
    }
}
