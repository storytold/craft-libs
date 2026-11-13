//! The brush cursor: the tip's outline as the user's cursor preferences ask for it, and the aliased tip's pixel cell.

use super::{BrushTip, BrushTipOutline, TipPose};
use crate::canvas::ToolContext;
use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, PaintUi, Role};
use crate::primitives::{PaintShape, ellipse};
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{CanvasView, Cursor, CursorBadge, CursorIcon, IconId, Input, Metrics, PaintedCursor};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

/// The side of the box a tool icon is drawn on; an icon's hotspot is in these units.
const ICON_BOX: f32 = 24.0;
/// Two mapped tip axes are equal and square within this much of their squared length.
const ISOTROPY_TOLERANCE: f32 = 0.001;
/// No length.
const NO_LENGTH: f32 = 0.0;

/// What the Standard painting cursor shows: the Brush tool icon.
const BRUSH_ICON: IconId = IconId::new("brush");

/// Preferences › Cursors › Painting Cursors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PaintingCursor {
    /// The tool's icon.
    Standard,
    /// The crosshair.
    Precise,
    /// The outline where a dab reaches 50 %.
    #[default]
    NormalBrushTip,
    /// The outline where a dab reaches 0 %.
    FullSizeBrushTip,
}

/// The user's cursor preferences a brush cursor reads, with the reach of the crosshair that replaces an outline too
/// small to read.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BrushCursorOptions {
    /// Which cursor painting tools show.
    pub painting: PaintingCursor,
    /// Show Only Crosshair While Painting.
    pub show_only_crosshair_while_painting: bool,
    /// Caps Lock switches to the precise crosshair.
    pub caps_lock_precise: bool,
}

impl BrushCursorOptions {
    /// Painting cursors `painting`, Caps Lock switching to the crosshair.
    pub fn new(painting: PaintingCursor) -> Self {
        BrushCursorOptions { painting, show_only_crosshair_while_painting: false, caps_lock_precise: true }
    }

    /// Whether a stroke in progress shows only the crosshair.
    #[must_use]
    pub fn show_only_crosshair_while_painting(mut self, on: bool) -> Self {
        self.show_only_crosshair_while_painting = on;
        self
    }

    /// Whether Caps Lock switches to the crosshair.
    #[must_use]
    pub fn caps_lock_precise(mut self, on: bool) -> Self {
        self.caps_lock_precise = on;
        self
    }
}

/// The pointer of a brush tool: built each pass from the app's [`BrushTip`] and turned into the [`Cursor`] the tool
/// returns from `CanvasTool::cursor` at `CursorPriority::Tool`.
///
/// The rules, in order (Photoshop's Preferences › Cursors):
/// 1. Caps Lock with [`BrushCursorOptions::caps_lock_precise`]: the crosshair, whatever else is set.
/// 2. Standard: the tool's [`icon`](Self::icon), hotspot at its centre. Precise: the crosshair.
/// 3. While [`painting`](Self::painting) with [`BrushCursorOptions::show_only_crosshair_while_painting`]: the
///    crosshair.
/// 4. A [`Pixels`](BrushTipOutline::Pixels) tip: the hidden pointer (the tool shows [`tip_cell`](Self::tip_cell)), or
///    the crosshair when [`centre`](Self::centre) is on or the cell is narrower than the crosshair on screen.
/// 5. Otherwise Normal Brush Tip draws the 50 % outline (`normal_radius`, the `normal` rings), Full Size Brush Tip
///    the full reach (`full_radius`, the `full` rings), mapped through the [`pose`](Self::pose) and then the view
///    (zoom, rotation, flip, pixel aspect). A round tip under an upright pose and an isotropic view is a circle;
///    under a skewed pose or a pixel aspect it is an outline ellipse with the mapped axes' lengths; traced rings are
///    an outline. With [`inner_ring`](Self::inner_ring) a second ring is added.
/// 6. An outline whose farthest point is nearer than the hit target (`size.hit.target`, the arm of the crosshair the
///    host paints) to the hotspot: the crosshair.
///
/// A [`badge`](Self::badge) rides on whichever pointer rules 1 to 6 give: in the Standard icon's own slot, beside the
/// crosshair, centred on the hotspot inside a circle or an outline; the hidden pointer of a pixel tip has none. A
/// circle or an outline draws its centre mark while [`centre`](Self::centre) is on. NaN, infinite, negative and zero
/// sizes never panic and never give a non-finite ring (they fall to rule 6).
#[derive(Clone, Debug)]
pub struct BrushCursor<'a> {
    tip: &'a BrushTip,
    pose: TipPose,
    centre: bool,
    painting: bool,
    icon: IconId,
    inner_ring: Option<f64>,
    badge: Option<CursorBadge>,
}

impl<'a> BrushCursor<'a> {
    /// The cursor of `tip`, upright, no centre mark, not painting, the Brush icon for Standard.
    pub fn new(tip: &'a BrushTip) -> Self {
        BrushCursor { tip, pose: TipPose::upright(), centre: false, painting: false, icon: BRUSH_ICON, inner_ring: None, badge: None }
    }

    /// The pen's pose this pass ([`TipPose::pen`]); default upright.
    #[must_use]
    pub fn pose(mut self, pose: TipPose) -> Self {
        self.pose = pose;
        self
    }

    /// Whether the circle or outline draws its centre mark and an aliased tip shows the crosshair rather than hiding
    /// the pointer: the app's rule (Show Crosshair in Brush Tip; Clone Stamp only while Alt is held; the Background
    /// Eraser always). Default off.
    #[must_use]
    pub fn centre(mut self, on: bool) -> Self {
        self.centre = on;
        self
    }

    /// Whether a stroke is being painted (for Show Only Crosshair While Painting). Default off.
    #[must_use]
    pub fn painting(mut self, on: bool) -> Self {
        self.painting = on;
        self
    }

    /// The tool's icon for the Standard painting cursor. Default the Brush icon.
    #[must_use]
    pub fn icon(mut self, icon: IconId) -> Self {
        self.icon = icon;
        self
    }

    /// A second ring at `radius` document units inside the outline, mapped like it (LightCraft's feather core).
    #[must_use]
    pub fn inner_ring(mut self, radius: f64) -> Self {
        self.inner_ring = Some(radius);
        self
    }

    /// A mark on the pointer: the + or - of the selection mode. Default none.
    #[must_use]
    pub fn badge(mut self, badge: Option<CursorBadge>) -> Self {
        self.badge = badge;
        self
    }

    /// The pointer for this pass, by the rules above. The reach is [`ToolContext::reach`] and Caps Lock is the pass's toggle state.
    pub fn cursor(&self, cx: &ToolContext<'_>, options: &BrushCursorOptions, view: &CanvasView) -> Cursor {
        Cursor::Painted(self.painted(cx.reach(), options, view, cx.caps_lock))
    }

    fn crosshair(&self) -> PaintedCursor {
        match self.badge {
            Some(badge) => PaintedCursor::badged(CursorIcon::Crosshair, badge),
            None => PaintedCursor::Crosshair,
        }
    }

    fn painted(&self, reach: f32, options: &BrushCursorOptions, view: &CanvasView, caps_lock: bool) -> PaintedCursor {
        // Rules 1 and 2.
        if caps_lock && options.caps_lock_precise {
            return self.crosshair();
        }
        match options.painting {
            PaintingCursor::Precise => return self.crosshair(),
            PaintingCursor::Standard => return PaintedCursor::tool(self.icon.clone(), (Vec2::X + Vec2::Y) * (ICON_BOX / 2.0), self.badge),
            PaintingCursor::NormalBrushTip | PaintingCursor::FullSizeBrushTip => {}
        }
        // Rule 3.
        if self.painting && options.show_only_crosshair_while_painting {
            return self.crosshair();
        }
        let full = options.painting == PaintingCursor::FullSizeBrushTip;
        // Rule 5.
        let unit = sanitize(self.tip.diameter) / 2.0;
        let axes_for = |radius: f64| [self.pose.x_axis, self.pose.y_axis].map(|axis| view.map_vec(axis * radius));
        let unit_axes = axes_for(unit);
        let (rings, round) = match &self.tip.outline {
            BrushTipOutline::Round { normal_radius, full_radius } => {
                let axes = axes_for(sanitize(if full { *full_radius } else { *normal_radius }));
                (vec![ellipse(Pos2::ZERO, axes)], Some(axes))
            }
            BrushTipOutline::Traced { normal, full: widest } => {
                let rings = if full { widest } else { normal };
                let mapped: Vec<Vec<Pos2>> =
                    rings.rings().iter().map(|ring| ring.iter().map(|p| (unit_axes[0] * p[0] + unit_axes[1] * p[1]).to_pos2()).collect()).collect();
                (mapped, None)
            }
            // Rule 4. A cell narrower than the crosshair, or not finite, shows the crosshair.
            BrushTipOutline::Pixels { cell, .. } => {
                let corners = view.quad(*cell);
                let (min, max) = corners.iter().fold((corners[0], corners[0]), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
                let side = (max.x - min.x).min(max.y - min.y);
                let narrow = matches!(side.partial_cmp(&reach), None | Some(std::cmp::Ordering::Less));
                return if self.centre || narrow { self.crosshair() } else { PaintedCursor::Hidden };
            }
        };
        // Rule 6, also for a non-finite outline.
        let extent = match round {
            Some(axes) => semi_major(axes),
            None => rings.iter().flatten().fold(NO_LENGTH, |farthest, p| farthest.max(p.to_vec2().length())),
        };
        let finite = rings.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite());
        if !finite || !extent.is_finite() || extent <= NO_LENGTH || extent < reach {
            return self.crosshair();
        }
        let mut rings = rings;
        if let Some(radius) = self.inner_ring {
            let inner = ellipse(Pos2::ZERO, axes_for(sanitize(radius)));
            if inner.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
                if let Some(axes) = round {
                    rings = vec![ellipse(Pos2::ZERO, axes), inner];
                } else {
                    rings.push(inner);
                }
            }
        } else if let Some([x, y]) = round {
            // A round tip stays a plain circle while the mapped axes are equal and square.
            let anisotropic = (x.length_sq() - y.length_sq()).abs() > ISOTROPY_TOLERANCE || x.dot(y).abs() > ISOTROPY_TOLERANCE;
            if !anisotropic {
                return PaintedCursor::circle(x.length()).marked(self.centre, self.badge);
            }
        }
        PaintedCursor::outline(rings).marked(self.centre, self.badge)
    }

    /// The aliased tip's cell as a canvas control, for the tool to show beside the hidden pointer; `None` for another
    /// outline.
    pub fn tip_cell(&self) -> Option<TipCell> {
        match &self.tip.outline {
            BrushTipOutline::Pixels { cell, round } => Some(TipCell::new(*cell, *round)),
            _ => None,
        }
    }
}

/// `v` if it is a finite size, else 0; negative sizes count as 0.
fn sanitize(v: f64) -> f64 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// The farthest distance from the centre of the ellipse with semi-axis vectors `axes`.
fn semi_major([x, y]: [Vec2; 2]) -> f32 {
    let (a, b, d) = (x.length_sq(), y.length_sq(), x.dot(y));
    ((a + b) / 2.0 + (((a - b) / 2.0).powi(2) + d * d).sqrt()).sqrt()
}

/// The screen rect of `cell` through `view` on the pixel grid of `pixels_per_point`; `None` for a cell that is not
/// finite. A `pixels_per_point` that is not finite and positive counts as 1.
fn snapped_cell(view: &CanvasView, cell: DocRect, pixels_per_point: f32) -> Option<Rect> {
    if ![cell.x0, cell.y0, cell.x1, cell.y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let snap = |v: f32| (v * ppp).round() / ppp;
    let corners = view.quad(cell);
    let bounds = corners.iter().skip(1).fold(Rect::from_min_max(corners[0], corners[0]), |r, p| r.union(Rect::from_min_max(*p, *p)));
    Some(Rect::from_min_max(Pos2::new(snap(bounds.min.x), snap(bounds.min.y)), Pos2::new(snap(bounds.max.x), snap(bounds.max.y))))
}

/// The style of the pixel cell.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TipCellStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// An aliased tip's whole-pixel cell on the pixel grid, outlined as a square or a disc: what a pencil shows under the
/// hidden pointer. Shown at the canvas rect, in a canvas given a view.
#[derive(crate::contract::Component)]
#[component(canvas_control, content_painter)]
pub struct TipCell {
    #[common]
    common: Common,
    cell: DocRect,
    round: bool,
}

impl TipCell {
    /// The cell `cell` in document units, a disc when `round`.
    pub fn new(cell: DocRect, round: bool) -> Self {
        TipCell { common: Common::new(), cell, round }
    }
}

impl Component for TipCell {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = TipCellStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TipCellStyleKey) -> TipCellStyle {
        TipCellStyle::default()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let Some(rect) = snapped_cell(&space.view, self.cell, space.pixels_per_point) else { return };
        let painter = ui.content_painter(cx);
        let shape = if self.round { PaintShape::Circle(rect.center(), rect.width() / 2.0) } else { PaintShape::Rect(rect, CornerRadius::ZERO) };
        painter.outline(&shape);
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
