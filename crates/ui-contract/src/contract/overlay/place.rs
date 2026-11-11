//! Where a layer sits and how it sizes: anchors, places, viewport shares, fit, resize and size limits.

use craft_behavior::overlay::{Align, Side};
use egui::{Align2, Pos2, Rect, Vec2};

/// Where an overlay hangs from.
#[derive(Clone, Debug)]
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
    /// A corner or edge of the viewport's safe rect, inset by the panel padding.
    Viewport(Align2),
}

/// Where a floating layer sits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FloatPlace {
    /// A screen position for the layer's left-top; `None` centres it in the viewport.
    pub pos: Option<Pos2>,
}

impl FloatPlace {
    /// Centred in the viewport.
    pub const fn centred() -> Self {
        FloatPlace { pos: None }
    }

    /// At `pos`.
    pub const fn at(pos: Pos2) -> Self {
        FloatPlace { pos: Some(pos) }
    }
}

/// Where an egui layer sits.
#[derive(Clone, Debug)]
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
    },
    /// Free-standing.
    Float(FloatPlace),
}

impl Place {
    /// An anchored place.
    pub const fn anchored(anchor: Anchor, side: Side, align: Align) -> Self {
        Place::Anchored { anchor, side, align }
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
    /// Edge and corner grips resize the layer.
    pub grips: bool,
}

impl Resize {
    /// The layer keeps its size.
    pub const NONE: Resize = Resize { grips: false };
    /// Grips on every edge and corner.
    pub const GRIPS: Resize = Resize { grips: true };
}

/// Size limits, in points: the smallest size and, where there is one, the largest. A layer's (`Overlay::size_limits`)
/// and a dock panel's body (`PanelSpec::limits`) take them. Tiny viewports relax minima proportionally rather than
/// producing inverted rects. A maximum constrains a pane when its sibling can take the remainder. A tab group takes
/// the largest minimum and the largest maximum of its panels, so a panel without a maximum lifts the maximum of the
/// panels it joins.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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

    /// Whether every value is finite and non-negative, the maximum is no less than the minimum and no value is
    /// above [`Self::BOUND`].
    pub fn valid(self) -> bool {
        let within = |size: Vec2| size.is_finite() && size.x >= 0.0 && size.y >= 0.0 && size.x <= Self::BOUND && size.y <= Self::BOUND;
        within(self.min) && self.max.is_none_or(|max| within(max) && max.x >= self.min.x && max.y >= self.min.y)
    }
}
