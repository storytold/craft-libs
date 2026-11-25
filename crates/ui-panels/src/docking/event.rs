//! `DockEvent`, `FloatSpace` and `HidePanels`: what a dock reports to the host and the host tells a dock.

use super::{DockCommands, DockSizes, DragUnit, PanelKey};
use crate::contract::{MemoProps, Tracked};
use egui::{Rect, Vec2};
use std::collections::BTreeSet;

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
    /// The area opened, switched or closed its flyout: an icon click, Escape, or a press outside or the focus leaving
    /// it under Auto-Collapse. A host that binds the flyout takes it as the request when its binding is given.
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
    /// The tab of a panel's document was picked ([`PanelDocument`](super::PanelDocument)): the host shows the document
    /// in the panel. The area also activates the panel, as a pick of its own tab does.
    DocumentPicked {
        /// The panel whose document it is.
        panel: P,
        /// The document's id.
        document: u64,
    },
    /// The close button or a middle click on the tab of a panel's document asked to close it. The host closes it by
    /// removing it from the panel's spec; the layout does not change.
    DocumentClosed {
        /// The panel whose document it is.
        panel: P,
        /// The document's id.
        document: u64,
    },
    /// A floating frame holding `panel` was moved or resized to `rect`.
    FloatPlaced {
        /// A panel of the frame.
        panel: P,
        /// The frame's rect in the area's layer space.
        rect: Rect,
    },
}

/// Which panels the host hides. Protected panels always stay. While the shell hides the panels and keeps the tools
/// (Shift+Tab), the panels `Except` names stay and every other panel hides; while it hides the dock (Tab), every panel
/// hides.
///
/// The set is a `Tracked` value, so a dock keys what it shows by the set's revision and keeps the pruned layout until
/// the set, the workspace or the shell's visibility changes.
#[derive(Clone, Copy, MemoProps)]
#[non_exhaustive]
pub enum HidePanels<'a> {
    /// Hides none.
    None,
    /// Hides every panel.
    All,
    /// Every panel except those whose [`PanelKey`] the set holds.
    Except(&'a Tracked<BTreeSet<PanelKey>>),
}

/// Where a docking command floats a panel, group or column when it gives no rect. A dock records one each pass;
/// the host reads it to turn a command into placements.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FloatSpace<P> {
    /// The rect the area drew in. A floated unit goes inside it, `inset` from its top-left corner.
    pub area: Rect,
    /// Room between the area's top-left corner and a unit a command floats.
    pub inset: f32,
    /// The height above a floated window's body (`DockSizes::window_chrome`): its title row, which a drop keeps under
    /// the pointer, and the band over it where floats show in the app window.
    pub chrome: f32,
    /// The size a unit floats at when neither its panel nor where it came from gives one. Its height is also the
    /// most a panel, group or column floats at.
    pub default_size: Vec2,
    /// The float size of each panel that has one of its own (`PanelSpec::float_size`).
    panels: Vec<(P, Vec2)>,
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
            chrome: sizes.window_chrome(true),
            default_size: sizes.float_size,
            panels: Vec::new(),
            side_edge_zones: false,
        }
    }

    /// For an area whose floats open in OS windows (`on`), whose chrome is their title row alone.
    #[must_use]
    pub fn in_os_windows(mut self, sizes: &DockSizes, on: bool) -> Self {
        self.chrome = sizes.window_chrome(!on);
        self
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

    /// Adds the size `panel` floats at alone, whatever size it had where it came from.
    #[must_use]
    pub fn with_panel_size(mut self, panel: P, size: Vec2) -> Self {
        self.panels.push((panel, size));
        self
    }

    /// The float size of `panel` of its own, when it has one.
    pub fn panel(&self, panel: &P) -> Option<Vec2> {
        Self::find(&self.panels, panel)
    }

    /// The size `unit` floats at: a panel's own float size, else the size it had where it came from (`source`), else
    /// the default. A window keeps its own size. A panel, group or column takes the source's width and its height
    /// with the window's chrome above it, at most the default height: the dock passes the height its content
    /// measures at in that room, so content that fills floats at the default height.
    pub fn size_of(&self, unit: &DragUnit<P>, source: Option<Vec2>) -> Vec2 {
        let own = match unit {
            DragUnit::Panel(panel) => self.panel(panel),
            _ => None,
        };
        let framed = |size: Vec2| match unit {
            DragUnit::Window(_) => size,
            _ => Vec2::new(size.x, (size.y + self.chrome).min(self.default_size.y)),
        };
        own.or(source.map(framed)).unwrap_or(self.default_size)
    }

    /// The height a panel, group or column's content is measured in for a float of it: the default height less the
    /// window's chrome.
    pub fn content_room(&self) -> f32 {
        (self.default_size.y - self.chrome).max(0.0)
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
}
