//! `Insets`, `Flow` and `Axis`: start and end geometry resolved once for the layout direction.

use emath::{Align, Pos2, Rect, Vec2};

/// Distances from the four edges of a rect; `start` and `end` follow the reading direction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Insets {
    /// From the start edge.
    pub start: f32,
    /// From the end edge.
    pub end: f32,
    /// From the top edge.
    pub top: f32,
    /// From the bottom edge.
    pub bottom: f32,
}

impl Insets {
    /// Insets of `start`, `end`, `top` and `bottom`.
    pub const fn new(start: f32, end: f32, top: f32, bottom: f32) -> Self {
        Insets { start, end, top, bottom }
    }

    /// `h` at the start and the end.
    pub const fn horizontal(h: f32) -> Self {
        Insets::new(h, h, 0.0, 0.0)
    }

    /// `v` at the top and the bottom.
    pub const fn vertical(v: f32) -> Self {
        Insets::new(0.0, 0.0, v, v)
    }

    /// `all` on every side.
    pub const fn uniform(all: f32) -> Self {
        Insets::new(all, all, all, all)
    }
}

/// An axis of the screen or of a document. Saved layouts written with the former split axis names load through
/// the aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Axis {
    /// Left to right.
    #[serde(alias = "Horizontal", alias = "horizontal")]
    X,
    /// Top to bottom.
    #[serde(alias = "Vertical", alias = "vertical")]
    Y,
}

/// The geometry of one rect in one direction. Components place by start and end and never test direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flow {
    rect: Rect,
    rtl: bool,
}

impl Flow {
    /// The flow of `rect`, right to left when `rtl`.
    pub const fn new(rect: Rect, rtl: bool) -> Self {
        Flow { rect, rtl }
    }

    /// The rect the flow places in.
    pub const fn rect(&self) -> Rect {
        self.rect
    }

    /// Whether start is the right edge.
    pub const fn is_rtl(&self) -> bool {
        self.rtl
    }

    /// The x of the start edge: the left edge, or the right edge under RTL.
    pub fn start_x(&self) -> f32 {
        if self.rtl { self.rect.right() } else { self.rect.left() }
    }

    /// The x of the end edge.
    pub fn end_x(&self) -> f32 {
        if self.rtl { self.rect.left() } else { self.rect.right() }
    }

    /// The full-height rect `width` wide that begins `start` from the start edge.
    pub fn span(&self, start: f32, width: f32) -> Rect {
        let (min, max) = if self.rtl {
            (self.rect.right() - start - width, self.rect.right() - start)
        } else {
            (self.rect.left() + start, self.rect.left() + start + width)
        };
        Rect::from_min_max(Pos2::new(min, self.rect.top()), Pos2::new(max, self.rect.bottom()))
    }

    /// Takes `width` from the start edge: the taken rect, and the flow of the rest.
    pub fn take_start(&self, width: f32) -> (Rect, Flow) {
        let (taken, rest) = self.split_start(self.rect, width);
        (taken, Flow { rect: rest, rtl: self.rtl })
    }

    /// Takes `width` from the end edge: the taken rect, and the flow of the rest.
    pub fn take_end(&self, width: f32) -> (Rect, Flow) {
        let (rest, taken) = self.split_start(self.rect, (self.rect.width() - width).max(0.0));
        (taken, Flow { rect: rest, rtl: self.rtl })
    }

    /// Splits `width` from the start edge of `rect`: `(the start part, the rest)`.
    fn split_start(&self, rect: Rect, width: f32) -> (Rect, Rect) {
        let width = width.clamp(0.0, rect.width().max(0.0));
        if self.rtl {
            let cut = rect.right() - width;
            (Rect::from_min_max(Pos2::new(cut, rect.top()), rect.right_bottom()), Rect::from_min_max(rect.min, Pos2::new(cut, rect.bottom())))
        } else {
            let cut = rect.left() + width;
            (Rect::from_min_max(rect.min, Pos2::new(cut, rect.bottom())), Rect::from_min_max(Pos2::new(cut, rect.top()), rect.max))
        }
    }

    /// The rect shrunk by `insets`.
    pub fn inset(&self, insets: Insets) -> Rect {
        let (left, right) = if self.rtl { (insets.end, insets.start) } else { (insets.start, insets.end) };
        Rect::from_min_max(
            Pos2::new(self.rect.left() + left, self.rect.top() + insets.top),
            Pos2::new(self.rect.right() - right, self.rect.bottom() - insets.bottom),
        )
    }

    /// A rect of `size` in the flow's rect: `h` along the reading direction (`Min` is the start edge), `v`
    /// from the top.
    pub fn align(&self, size: Vec2, h: Align, v: Align) -> Rect {
        let free = self.rect.width() - size.x;
        let along = match h {
            Align::Min => 0.0,
            Align::Center => free / 2.0,
            Align::Max => free,
        };
        let top = match v {
            Align::Min => self.rect.top(),
            Align::Center => self.rect.top() + (self.rect.height() - size.y) / 2.0,
            Align::Max => self.rect.bottom() - size.y,
        };
        let left = if self.rtl { self.rect.right() - along - size.x } else { self.rect.left() + along };
        Rect::from_min_size(Pos2::new(left, top), size)
    }

    /// A rect given in logical x (from the start edge of this flow's rect) as a physical rect.
    pub fn to_physical(&self, logical: Rect) -> Rect {
        if !self.rtl {
            return logical;
        }
        let mirrored = |x: f32| self.rect.right() - (x - self.rect.left());
        Rect::from_min_max(Pos2::new(mirrored(logical.max.x), logical.min.y), Pos2::new(mirrored(logical.min.x), logical.max.y))
    }

    /// The same rect placed left to right, for a site that places by explicit rects and must not mirror (a
    /// free-position board, a waveform). `reason` names why; every call site is a counted exception.
    pub const fn unmirrored(rect: Rect, reason: &'static str) -> Flow {
        let _ = reason;
        Flow { rect, rtl: false }
    }
}
