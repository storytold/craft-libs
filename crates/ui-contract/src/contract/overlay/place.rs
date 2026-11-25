//! Where a layer sits and how it sizes: anchors, places, viewport shares, fit, resize and size limits.

use craft_behavior::overlay::{Align, Side};
use egui::{Align2, Pos2, Rect, Vec2};

/// Where an overlay hangs from.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Anchor {
    /// A rect in the opener's layer space.
    Rect(Rect),
    /// The opener's own rect.
    Response,
    /// A point in the opener's layer space.
    Point(Pos2),
    /// Where the pointer was when the overlay opened.
    Pointer,
    /// A corner or edge of a rect in the opener's layer space, inset by the panel padding and kept inside the safe
    /// rect: the body a toast stack sits in.
    Corner(Rect, Align2),
    /// The placed panel rect of the parent layer the overlay opened from.
    Parent,
}

/// Where a floating layer sits.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FloatPlace {
    /// Where the layer's `corner` sits, in the layer's parent space; `None` centres it in the viewport.
    pub pos: Option<Pos2>,
    /// The corner of the layer that sits at `pos`.
    pub corner: Align2,
}

impl Default for FloatPlace {
    fn default() -> Self {
        FloatPlace::centred()
    }
}

impl FloatPlace {
    /// Centred in the viewport.
    pub const fn centred() -> Self {
        FloatPlace { pos: None, corner: Align2::LEFT_TOP }
    }

    /// With its left-top at `pos`.
    pub const fn at(pos: Pos2) -> Self {
        FloatPlace { pos: Some(pos), corner: Align2::LEFT_TOP }
    }

    /// With its `corner` at `pos`.
    pub const fn corner_at(pos: Pos2, corner: Align2) -> Self {
        FloatPlace { pos: Some(pos), corner }
    }

    /// The top left of a layer `size` large whose `corner` sits at `pos`; `None` for a centred layer.
    pub(crate) fn top_left(&self, size: Vec2) -> Option<Pos2> {
        self.pos.map(|pos| pos - self.corner.pos_in_rect(&Rect::from_min_size(Pos2::ZERO, size)).to_vec2())
    }
}

/// Where an egui layer sits.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Place {
    /// Hung from an anchor.
    #[non_exhaustive]
    Anchored {
        /// What it hangs from.
        anchor: Anchor,
        /// The preferred side.
        side: Side,
        /// The alignment along the anchor's edge.
        align: Align,
        /// Beside the anchor, it lines up with the anchor's other end when that fits and the aligned end does not,
        /// instead of sliding inside the bounds: a submenu of a low row opens upward.
        flips_across: bool,
        /// Where the layer sits from the place it would take: shifts it by this much.
        offset: egui::Vec2,
        /// A rect the layer keeps clear of, in the layer's own space.
        clear: Option<egui::Rect>,
        /// Below or above the anchor, a layer that fits on neither side keeps its size and slides over the anchor,
        /// inside its bounds, instead of being cut to the room left beside it: a task bar under a crop frame that
        /// fills the canvas.
        covers: bool,
    },
    /// Free-standing.
    Float(FloatPlace),
}

impl Place {
    /// An anchored place.
    pub const fn anchored(anchor: Anchor, side: Side, align: Align) -> Self {
        Place::Anchored { anchor, side, align, flips_across: false, offset: egui::Vec2::ZERO, clear: None, covers: false }
    }

    /// The place, beside its anchor, lining up with the anchor's other end when that fits and the aligned end does not.
    pub const fn flipping_across(self) -> Self {
        match self {
            Place::Anchored { anchor, side, align, offset, clear, covers, .. } => {
                Place::Anchored { anchor, side, align, flips_across: true, offset, clear, covers }
            }
            float => float,
        }
    }

    /// The place shifted by `by` from where it would sit; no change off `Anchored`.
    pub fn offset(mut self, by: egui::Vec2) -> Self {
        if let Place::Anchored { offset, .. } = &mut self {
            *offset = by;
        }
        self
    }

    /// The place, below or above its anchor, sliding over the anchor when it fits on neither side; no change off
    /// `Anchored`.
    pub fn covering_anchor(mut self) -> Self {
        if let Place::Anchored { covers, .. } = &mut self {
            *covers = true;
        }
        self
    }

    /// The place keeping clear of `rect`; no change off `Anchored`.
    pub fn clear_of(mut self, rect: egui::Rect) -> Self {
        if let Place::Anchored { clear, .. } = &mut self {
            *clear = Some(rect);
        }
        self
    }
}

/// A fraction of the viewport per axis.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ViewportShare {
    /// The share of the width.
    pub x: f32,
    /// The share of the height.
    pub y: f32,
}

impl ViewportShare {
    /// A share of `x` of the width and `y` of the height.
    pub const fn new(x: f32, y: f32) -> Self {
        ViewportShare { x, y }
    }
}

/// How an egui layer sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum OverlayFit {
    /// From its content.
    Content,
    /// From its content at first; the user's size is saved.
    Resizable,
    /// From its content up to a share of the viewport; it scrolls beyond.
    ContentCapped(ViewportShare),
}

/// How the user may resize a layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Resize {
    /// The edges whose grips resize the layer; a corner grip needs both its edges. A placement that holds an edge
    /// still keeps its grip away.
    pub edges: ResizeEdges,
    /// The caller turns each resize into a size of its own (a window as large as its content in whole steps). In an
    /// OS window the app runs the grips' drag, reports each step, and the OS offers no resize of its own; the window
    /// takes the size the caller places. A layer resizes as any other and reports the same steps.
    pub by_caller: bool,
}

impl Resize {
    /// The layer keeps its size.
    pub const NONE: Resize = Resize { edges: ResizeEdges::NONE, by_caller: false };
    /// Grips on every free edge and corner.
    pub const GRIPS: Resize = Resize { edges: ResizeEdges::ALL, by_caller: false };

    /// Grips on the given edges only.
    pub fn edges(edges: ResizeEdges) -> Resize {
        Resize { edges, by_caller: false }
    }
}

/// A set of layer edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResizeEdges(u8);

impl ResizeEdges {
    /// The left edge.
    pub const LEFT: ResizeEdges = ResizeEdges(1);
    /// The right edge.
    pub const RIGHT: ResizeEdges = ResizeEdges(2);
    /// The top edge.
    pub const TOP: ResizeEdges = ResizeEdges(4);
    /// The bottom edge.
    pub const BOTTOM: ResizeEdges = ResizeEdges(8);
    /// Every edge.
    pub const ALL: ResizeEdges = ResizeEdges(15);
    /// No edge.
    pub const NONE: ResizeEdges = ResizeEdges(0);

    /// Whether every edge of `other` is in this set.
    pub const fn contains(self, other: ResizeEdges) -> bool {
        self.0 & other.0 == other.0
    }

    /// The edges of both sets.
    pub const fn union(self, other: ResizeEdges) -> ResizeEdges {
        ResizeEdges(self.0 | other.0)
    }
}

/// Size limits, in points: the smallest size and, where there is one, the largest. A layer's (`Overlay::size_limits`)
/// and a dock panel's body (`PanelSpec::limits`) take them. Tiny viewports relax minima proportionally rather than
/// producing inverted rects. A maximum constrains a pane when its sibling can take the remainder. A tab group takes
/// the largest minimum and the largest maximum of its panels, so a panel without a maximum lifts the maximum of the
/// panels it joins.
#[derive(Clone, Copy, Debug, Default, PartialEq, crate::contract::MemoProps)]
pub struct SizeLimits {
    /// Smallest size.
    pub min: Vec2,
    /// Largest size; `None` has no maximum.
    pub max: Option<Vec2>,
}

impl SizeLimits {
    /// The largest value a limit or a height may have: a value above it is invalid.
    pub const BOUND: f32 = 1_000_000.0;

    /// Limits between `min` and `max`.
    pub const fn new(min: Vec2, max: Vec2) -> Self {
        SizeLimits { min, max: Some(max) }
    }

    /// No smaller than `min`, with no maximum.
    pub const fn at_least(min: Vec2) -> Self {
        SizeLimits { min, max: None }
    }

    /// No larger than `max`, with no minimum.
    pub const fn at_most(max: Vec2) -> Self {
        SizeLimits { min: Vec2::ZERO, max: Some(max) }
    }

    /// The limits of a frame that holds a size with these limits and `extra` around it: both ends grow by `extra`.
    pub fn around(self, extra: Vec2) -> Self {
        SizeLimits { min: self.min + extra, max: self.max.map(|max| max + extra) }
    }

    /// `size` within these limits; the minimum wins over a smaller maximum.
    pub fn clamp(self, size: Vec2) -> Vec2 {
        size.min(self.max.unwrap_or(Vec2::INFINITY)).max(self.min)
    }

    /// Whether every value is finite and non-negative, the maximum is no less than the minimum and no value is
    /// above [`Self::BOUND`].
    pub fn valid(self) -> bool {
        let within = |size: Vec2| size.is_finite() && size.x >= 0.0 && size.y >= 0.0 && size.x <= Self::BOUND && size.y <= Self::BOUND;
        within(self.min) && self.max.is_none_or(|max| within(max) && max.x >= self.min.x && max.y >= self.min.y)
    }
}
