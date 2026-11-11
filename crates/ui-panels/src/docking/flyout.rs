//! `Flyout`: the panel an icon of an iconic column opens beside the column.

use crate::contract::overlay::{Anchor, Dismiss, Place};
use crate::contract::{BodyCx, Overlay, OverlayUi, PopupHandle};
use craft_behavior::dock::DockEdge;
use craft_behavior::overlay::{Align, Side};
use egui::{Rect, Ui, Vec2};

#[cfg(feature = "testing")]
mod stories;

/// Where a group view puts the group it is given: measured at a width, or shown in a rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroupAt {
    /// Measure the group at this width; the view returns the height it takes.
    Measure(f32),
    /// Show the group in this rect.
    Show(Rect),
}

/// A popup beside an iconic column, on the side of the canvas, level with the icon that opened it. Escape and a
/// press outside close it; with Auto-Collapse Iconic Panels the focus leaving it closes it too. Its body is
/// measured at the flyout's width first: the flyout is as tall as the body measures within the room, and at least
/// its `size`'s height, and the body then shows in the rect that takes.
#[derive(crate::contract::Pattern)]
pub struct Flyout {
    edge: DockEdge,
    size: Vec2,
    auto_collapse: bool,
}

impl Flyout {
    /// A flyout at least `size` for the column on the window edge `edge`, closing when the focus leaves it if
    /// `auto_collapse`.
    pub fn new(edge: DockEdge, size: Vec2, auto_collapse: bool) -> Self {
        Flyout { edge, size, auto_collapse }
    }

    /// The layer the flyout shows in: hung from `anchor` (the icon's part) on the side away from the window edge.
    /// The anchored placement mirrors the side in a right-to-left layout.
    pub fn overlay(&self, anchor: Anchor) -> Overlay {
        let side = match self.edge {
            DockEdge::End => Side::Start,
            _ => Side::End,
        };
        Overlay::popover()
            .place(Place::anchored(anchor, side, Align::Start))
            .dismiss(Dismiss::new().focus_left(self.auto_collapse))
            .body_scroll(false)
    }

    /// Shows the flyout of `popup` while it is open, hung from `anchor`, and returns the rect its body showed in;
    /// `None` while it is closed. `body` returns the height it measures for `GroupAt::Measure`, or `None` to take
    /// the flyout's own height.
    pub fn show(
        self,
        ui: &mut OverlayUi<'_>,
        popup: PopupHandle,
        anchor: Anchor,
        mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>, GroupAt) -> Option<f32>,
    ) -> Option<Rect> {
        let size = self.size;
        ui.show(popup, self.overlay(anchor), |ui, cx| {
            let measured = body(ui, cx, GroupAt::Measure(size.x)).map_or(size.y, |height| height.min(ui.available_height()));
            let rect = crate::layout::place_in_flow(ui, Vec2::new(size.x, size.y.max(measured)));
            body(ui, cx, GroupAt::Show(rect));
            rect
        })
    }
}

/// Whether the layer of `popup` was dismissed in this pass of `ui` (Escape, a press outside, the focus leaving it).
pub(super) fn dismissed(ui: &OverlayUi<'_>, popup: PopupHandle) -> bool {
    ui.closed.iter().any(|(id, _)| *id == popup.id())
}
