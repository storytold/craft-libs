//! `Flyout`: the panel an icon of an iconic column opens beside the column.

use crate::contract::overlay::{Anchor, Dismiss, FloatPlace, Outside, Place, PlacementChange, ResizeEdges};
use crate::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, Mount, NoValue, Outcome, Overlay, OverlayUi, Pad, PaintUi, PopupHandle, Role,
    SizeLimits, Sizing,
};
use crate::style::{ComponentStyle, Corners, StyleCx, SurfaceKind};
use craft_behavior::overlay::Side;
use craft_behavior::{Metrics, Sense};
use egui::{Pos2, Rect, Response, Ui, Vec2};

/// Where a group view puts the group it is given: measured at a width, or shown in a rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroupAt {
    /// Measure the group at this width; the view returns the height it takes.
    Measure(f32),
    /// Show the group in this rect.
    Show(Rect),
}

/// The style of a flyout: its surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FlyoutStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a flyout reports in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct FlyoutOutput {
    /// The flyout's response.
    pub response: Response,
    /// The last step of a resize the user made this pass, with the rect the caller keeps.
    pub placed: Option<PlacementChange>,
}

impl Outcome for FlyoutOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    fn unshown(response: Response) -> Option<Self> {
        Some(FlyoutOutput { response, placed: None })
    }
}

/// A popup beside an iconic column, on the side of the canvas, whose top left the caller places. Escape closes it; with
/// Auto-Collapse Iconic Panels a press outside it and the focus leaving it close it too. Its body is measured at the
/// flyout's width first: the flyout is as tall as the body measures, at most the height of `within`, and at least its
/// `size`'s height, and the body then shows in the rect that takes. A flyout that fits below its top stays there; one
/// taller than the room below its top moves up by what it overflows and no further, its bottom on `within`'s (the
/// layer's keep rule moves it the least distance into its bounds), and only one taller than `within` starts at
/// `within`'s top and scrolls.
///
/// The user resizes it by its top and bottom edges, the edge away from the column and the two corners there; the
/// column side stays. The flyout keeps no rect of its own: each step comes back in [`FlyoutShown::placed`] and the
/// caller places the next pass from it.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, unregistered)]
pub struct Flyout {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    opens: Side,
    size: Vec2,
    auto_collapse: bool,
    /// The rect the flyout and every resize of it stay inside.
    #[prop]
    within: Rect,
    /// The least size a resize leaves the flyout.
    #[prop(quantity = "least room a resize leaves, from the dock's measured sizes, in points")]
    least: Vec2,
    /// The flyout stands as a docked column does: [`docked`](Flyout::docked).
    docked: bool,
    placed: Option<PlacementChange>,
}

impl Flyout {
    /// A flyout at least `size` that opens on the side `opens` of its column, in reading order (`Side::Start` or
    /// `Side::End`), closing on a press outside it and when the focus leaves it if `auto_collapse`.
    pub fn new(opens: Side, size: Vec2, auto_collapse: bool) -> Self {
        let props = ContainerProps::new().surface(SurfaceKind::Popup).pad(Pad::None).sizing(Sizing::Content);
        Flyout { common: Common::new(), props, opens, size, auto_collapse, within: Rect::EVERYTHING, least: Vec2::ZERO, docked: false, placed: None }
    }

    /// The flyout stands as a docked column does, in place of a popup's raised surface: square on `surface.panel`, with
    /// the column's `line.border` frame and no shadow (the flyouts of a flush dock's columns,
    /// [`DockConfig::flush_docks`](super::DockConfig::flush_docks)).
    #[must_use]
    pub fn docked(mut self, on: bool) -> Self {
        if on {
            self.props = self.props.surface(SurfaceKind::Panel);
        }
        self.docked = on;
        self
    }

    /// The layer the flyout shows in, its top left at `top_left`: placed by the caller, resizable on every edge but
    /// the one facing the column. The side away from the column mirrors in a right-to-left layout (`rtl`).
    fn overlay(&self, top_left: Pos2, rtl: bool) -> Overlay {
        let free = if (self.opens == Side::End) != rtl { ResizeEdges::RIGHT } else { ResizeEdges::LEFT };
        let bounded = self.within.is_finite();
        let least = if bounded { self.least.min(self.within.size().max(Vec2::ZERO)) } else { self.least };
        // A float place as well as the caller's position: the placement pins no edge, so the edge set alone decides
        // the grips. The host's binding opens and closes the flyout, so a command leaves it open.
        let overlay = Overlay::popover()
            .place(Place::Float(FloatPlace::at(top_left)))
            .placed_at(top_left)
            .size_limits(SizeLimits::at_least(least))
            .resize_edges(free.union(ResizeEdges::TOP).union(ResizeEdges::BOTTOM))
            .dismiss(
                Dismiss::new()
                    .outside(if self.auto_collapse { Outside::Close } else { Outside::Keep })
                    .focus_left(self.auto_collapse)
                    .commands(false),
            )
            .body_scroll(false);
        if bounded { overlay.bounds(self.within) } else { overlay }
    }

    /// Shows the flyout as the layer of `popup` while it is open, its top left at `anchor` (a point, or a rect's top
    /// left), and returns what it showed; `None` while it is closed. `body` returns the height it measures for
    /// `GroupAt::Measure`, or `None` to take the flyout's own height.
    pub fn show_popup(
        mut self,
        ui: &mut OverlayUi<'_>,
        popup: PopupHandle,
        anchor: Anchor,
        mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>, GroupAt) -> Option<f32>,
    ) -> Option<FlyoutShown> {
        let top_left = match anchor {
            Anchor::Point(at) => at,
            Anchor::Rect(rect) => rect.min,
            _ => {
                debug_assert!(false, "a flyout is placed at a point or a rect's top left");
                return None;
            }
        };
        let size = self.size;
        // A body measures no taller than the bounds: a flyout that does not fit below its top moves up by its overflow
        // alone, so only a body taller than the bounds is limited to them.
        let room = self.within.height().max(size.y);
        let overlay = self.overlay(top_left, ui.is_rtl());
        self.props = self.props.mount(Mount::Layer(overlay));
        let shown = ui.show_container(popup, self, |ui, cx| {
            let measured = body(ui, cx, GroupAt::Measure(size.x)).map_or(size.y, |height| height.min(room));
            let rect = crate::layout::place_in_flow(ui, Vec2::new(size.x, size.y.max(measured)));
            body(ui, cx, GroupAt::Show(rect));
            rect
        })?;
        let rect = shown.inner?;
        Some(FlyoutShown { rect, placed: shown.output.placed, dismissed: ui.closing(popup).is_some() })
    }
}

impl Component for Flyout {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FlyoutStyle;
    type Output = FlyoutOutput;

    fn style(_: &StyleCx<'_>, _: FlyoutStyleKey) -> FlyoutStyle {
        FlyoutStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.docked {
            ui.draw().surface(cx.rect, SurfaceKind::Frame, Corners::None);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> FlyoutOutput {
        FlyoutOutput { response: cx.response.clone(), placed: self.placed }
    }
}

impl Container for Flyout {
    fn placement_changed(&mut self, change: &PlacementChange) {
        self.placed = Some(*change);
    }
}

/// What a shown flyout reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FlyoutShown {
    /// The rect the flyout's body showed in.
    pub rect: Rect,
    /// The last step of the resize the user made this pass: the flyout's size or top edge.
    pub placed: Option<PlacementChange>,
    /// The layer closed in this pass: by Escape, a press outside, the focus leaving it or a close its body asked for.
    pub dismissed: bool,
}
