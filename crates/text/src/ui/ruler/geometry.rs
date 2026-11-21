//! Where the column and the markers sit on the strip.

use super::GapEdge;
use super::RulerValues;
use super::TabAlign;
use super::items::RulerItem;
use egui::{Rect, pos2, vec2};

/// Where the column sits on the strip: positions are points from the column's start edge, which
/// is the strip's right end in a right-to-left paragraph.
#[derive(Clone, Copy, Debug)]
pub(super) struct Geometry {
    pub strip: Rect,
    /// Screen points per point.
    pub scale: f32,
    pub right_to_left: bool,
    pub width: f32,
    /// Where zero sits, points from the strip's start edge (its end edge in a right-to-left paragraph).
    pub origin: f32,
    /// The points from the strip's start edge the ruler covers; the whole strip when unset.
    pub extent: Option<egui::Rangef>,
}

impl Geometry {
    /// The screen x of the distance `d` from the start edge.
    pub(super) fn x(&self, d: f32) -> f32 {
        let from_edge = (self.origin + d) * self.scale;
        if self.right_to_left { self.strip.max.x - from_edge } else { self.strip.min.x + from_edge }
    }

    /// The distance from the start edge at the screen x.
    pub(super) fn at(&self, x: f32) -> f32 {
        let from_edge = if self.right_to_left { (self.strip.max.x - x) / self.scale } else { (x - self.strip.min.x) / self.scale };
        from_edge - self.origin
    }

    /// The part of the strip the ruler covers: the strip cut to [`extent`](Self::extent).
    pub(super) fn visible(&self) -> Rect {
        let Some(extent) = self.extent else { return self.strip };
        let (a, b) = if self.right_to_left {
            (self.strip.max.x - extent.max * self.scale, self.strip.max.x - extent.min * self.scale)
        } else {
            (self.strip.min.x + extent.min * self.scale, self.strip.min.x + extent.max * self.scale)
        };
        Rect::from_x_y_ranges(a.min(b)..=a.max(b), self.strip.y_range()).intersect(self.strip)
    }

    /// The distances from the start edge over the covered part of the strip, lowest first.
    pub(super) fn shown_span(&self) -> (f32, f32) {
        let visible = self.visible();
        let (a, b) = (self.at(visible.min.x), self.at(visible.max.x));
        (a.min(b), a.max(b))
    }

    /// The part of the strip over the column.
    pub(super) fn column(&self) -> Rect {
        let (a, b) = (self.x(0.0), self.x(self.width));
        Rect::from_x_y_ranges(a.min(b)..=a.max(b), self.strip.y_range()).intersect(self.strip)
    }

    /// The direction of increasing distance on screen: 1 or -1.
    pub(super) fn sign(&self) -> f32 {
        if self.right_to_left { -1.0 } else { 1.0 }
    }

    /// The distances from the start edge that the strip shows, lowest first.
    pub(super) fn span(&self) -> (f32, f32) {
        let (a, b) = (self.at(self.strip.min.x), self.at(self.strip.max.x));
        (a.min(b), a.max(b))
    }
}

/// The marker sizes, from the style's named sizes.
#[derive(Clone, Copy, Debug)]
pub(super) struct Shapes {
    /// The width of a marker.
    pub width: f32,
    /// The width of a table column marker.
    pub column: f32,
    /// The height of a triangle and of the left indent's box.
    pub tip: f32,
    /// The height of a stop's stem.
    pub stem: f32,
    /// The length of a stop's foot.
    pub foot: f32,
}

impl Shapes {
    fn half(&self) -> f32 {
        self.width / 2.0
    }

    /// The first-line marker's box: a triangle pointing down from the strip's top.
    pub(super) fn first_line(&self, strip: Rect, x: f32) -> Rect {
        Rect::from_min_size(pos2(x - self.half(), strip.min.y), vec2(self.width, self.tip))
    }

    /// The hanging marker's box: a triangle pointing up from the left box.
    pub(super) fn hanging(&self, strip: Rect, x: f32) -> Rect {
        let base = strip.max.y - self.tip;
        Rect::from_min_max(pos2(x - self.half(), base - self.tip), pos2(x + self.half(), base))
    }

    pub(super) fn left_box(&self, strip: Rect, x: f32) -> Rect {
        Rect::from_min_max(pos2(x - self.half(), strip.max.y - self.tip), pos2(x + self.half(), strip.max.y))
    }

    /// The right marker's box: a triangle pointing up from the strip's bottom.
    pub(super) fn right(&self, strip: Rect, x: f32) -> Rect {
        Rect::from_min_max(pos2(x - self.half(), strip.max.y - self.tip), pos2(x + self.half(), strip.max.y))
    }

    /// The box around a stop's glyph, standing on the strip's bottom.
    pub(super) fn stop(&self, strip: Rect, x: f32) -> Rect {
        let reach = self.foot.max(self.half());
        Rect::from_min_max(pos2(x - reach, strip.max.y - self.stem), pos2(x + reach, strip.max.y))
    }

    /// The box of a table column marker at screen x `x`: the strip's depth tall.
    pub(super) fn column_marker(&self, strip: Rect, x: f32) -> Rect {
        Rect::from_min_max(pos2(x - self.column / 2.0, strip.min.y), pos2(x + self.column / 2.0, strip.max.y))
    }

    /// The grab box of one edge of the gap from `start` to `end` (points): a handle wide, centred on the edge, or, when
    /// the gap is narrower than two handles, the half of the gap that edge is nearer, split at its middle.
    pub(super) fn gap_edge(&self, g: &Geometry, gap: (f32, f32), edge: GapEdge) -> Rect {
        let (a, b) = (g.x(gap.0), g.x(gap.1));
        let (lo, hi) = (a.min(b), a.max(b));
        let at = g.x(if edge == GapEdge::End { gap.0 } else { gap.1 });
        let (from, to) = if hi - lo < 2.0 * self.column {
            let middle = (lo + hi) / 2.0;
            if at <= middle { (lo, middle) } else { (middle, hi) }
        } else {
            (at - self.column / 2.0, at + self.column / 2.0)
        };
        Rect::from_x_y_ranges(from..=to, g.strip.y_range())
    }

    /// The box the item occupies on the strip.
    pub(super) fn rect(&self, g: &Geometry, item: RulerItem, values: &RulerValues) -> Rect {
        let x = g.x(item.position(values, g.width));
        match item {
            RulerItem::FirstLine => self.first_line(g.strip, x),
            RulerItem::Hanging => self.hanging(g.strip, x),
            RulerItem::LeftBox => self.left_box(g.strip, x),
            RulerItem::Right => self.right(g.strip, x),
            RulerItem::Stop(_) => self.stop(g.strip, x),
        }
    }
}

/// The glyph of a stop of `kind` as lines standing on `base` at `x` with a dot: segments, and the decimal
/// dot's centre. A right-to-left paragraph mirrors the foot.
pub(super) fn glyph(shapes: &Shapes, x: f32, base: f32, kind: TabAlign, right_to_left: bool) -> (Vec<[egui::Pos2; 2]>, Option<egui::Pos2>) {
    /// A decimal stop's dot sits this share of the foot along, and this share of the stem up.
    const DOT_ALONG: f32 = 0.7;
    const DOT_UP: f32 = 0.7;
    /// A bar stop's stem, as a share of a stop's.
    const BAR_STEM: f32 = 1.4;
    let foot = if right_to_left { -shapes.foot } else { shapes.foot };
    let stem = |height: f32| [pos2(x, base - height), pos2(x, base)];
    let along = |a: f32, b: f32| [pos2(x + a, base), pos2(x + b, base)];
    match kind {
        TabAlign::Left => (vec![stem(shapes.stem), along(0.0, foot)], None),
        TabAlign::Right => (vec![stem(shapes.stem), along(-foot, 0.0)], None),
        TabAlign::Center => (vec![stem(shapes.stem), along(-shapes.foot, shapes.foot)], None),
        TabAlign::Decimal => {
            (vec![stem(shapes.stem), along(-shapes.foot, shapes.foot)], Some(pos2(x + foot * DOT_ALONG, base - shapes.stem * DOT_UP)))
        }
        TabAlign::Bar => (vec![stem(shapes.stem * BAR_STEM)], None),
    }
}
