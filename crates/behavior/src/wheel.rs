//! Wheel gesture values, as craft-behavior's copy of egui's types so the wheel split is egui-free.

/// How far one wheel event reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WheelUnit {
    /// Points.
    #[default]
    Point,
    /// Lines of text.
    Line,
    /// Pages.
    Page,
}

/// A wheel gesture's phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WheelPhase {
    /// The first event of a gesture.
    Start,
    /// A later event.
    #[default]
    Move,
    /// The fingers lifted.
    End,
    /// The system cancelled the gesture.
    Cancel,
}

use crate::Sense;
use craft_keymap::Mods;
use emath::{Pos2, Rect, Vec2};

/// One wheel event, or the sum of one pass's, as a taker reads it with `take_wheel`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Wheel {
    /// Scroll in points, wheel notches and lines already converted by the service.
    pub scroll: Vec2,
    /// Zoom factor; 1 for none.
    pub zoom: f32,
    /// The modifiers held.
    pub mods: Mods,
    /// The unit the device reported.
    pub unit: WheelUnit,
    /// The gesture phase.
    pub phase: WheelPhase,
    /// Where the pointer was, in screen points; a zoom is about this point.
    pub at: Option<Pos2>,
}

impl Wheel {
    /// A scroll of `scroll` points with no zoom.
    pub const fn new(scroll: Vec2, mods: Mods, unit: WheelUnit, phase: WheelPhase) -> Self {
        Wheel { scroll, zoom: 1.0, mods, unit, phase, at: None }
    }

    /// The same gesture with zoom factor `zoom`.
    pub const fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    /// The same gesture with the pointer at `at`.
    pub const fn at(mut self, at: Pos2) -> Self {
        self.at = Some(at);
        self
    }
}

/// A component that takes wheel input, as registered last pass.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WheelTaker<O> {
    /// The component.
    pub owner: O,
    /// Its view rect.
    pub rect: Rect,
    /// It takes horizontal scroll.
    pub horizontal: bool,
    /// It takes vertical scroll.
    pub vertical: bool,
    /// It takes zoom.
    pub zoom: bool,
    /// It takes any axis (scroll or zoom) only while focused or armed: a value control without wheel bits, or any
    /// taker inside a wheel-gated scope.
    pub gated: bool,
    /// A scroll region (a scrolling body or collection): Shift turns the vertical wheel sideways over it.
    pub scrolls: bool,
    /// The distance of one wheel line.
    pub notch: WheelNotch,
    /// How far a scroll region can still move each way; `None` for a taker without an end (a value control, a canvas).
    pub room: Option<WheelRoom>,
}

/// The scroll left in a region, in points of wheel scroll: `positive` is what a wheel with positive `scroll` can
/// still move, `negative` the same for negative `scroll`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[non_exhaustive]
pub struct WheelRoom {
    /// Room for a positive scroll on each axis.
    pub positive: Vec2,
    /// Room for a negative scroll on each axis.
    pub negative: Vec2,
}

impl WheelRoom {
    /// A room of `positive` and `negative` points.
    pub const fn new(positive: Vec2, negative: Vec2) -> Self {
        WheelRoom { positive, negative }
    }
}

/// The distance one wheel line scrolls a taker.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WheelNotch {
    /// This many points.
    Points(f32),
    /// This share of the taker's view along the axis.
    ViewShare(f32),
}

impl WheelNotch {
    /// The distance of a taker that states none.
    pub const DEFAULT: WheelNotch = WheelNotch::Points(40.0);
}

impl<O> WheelTaker<O> {
    /// A taker over `rect` that takes no axis yet.
    pub const fn new(owner: O, rect: Rect) -> Self {
        WheelTaker {
            owner,
            rect,
            horizontal: false,
            vertical: false,
            zoom: false,
            gated: false,
            scrolls: false,
            notch: WheelNotch::DEFAULT,
            room: None,
        }
    }

    /// The taker a shape with `sense` registers, `None` when `sense` has no wheel bit: `WHEEL` takes both scroll
    /// axes and zoom; `WHEEL_X`, `WHEEL_Y` and `WHEEL_ZOOM` take one share each and combine.
    pub fn for_sense(owner: O, rect: Rect, sense: Sense) -> Option<Self> {
        let all = sense.contains(Sense::WHEEL);
        let taker = WheelTaker::new(owner, rect)
            .horizontal(all || sense.contains(Sense::WHEEL_X))
            .vertical(all || sense.contains(Sense::WHEEL_Y))
            .zoom(all || sense.contains(Sense::WHEEL_ZOOM));
        (taker.horizontal || taker.vertical || taker.zoom).then_some(taker)
    }

    /// Whether it takes horizontal scroll.
    pub const fn horizontal(mut self, on: bool) -> Self {
        self.horizontal = on;
        self
    }

    /// Whether it takes vertical scroll.
    pub const fn vertical(mut self, on: bool) -> Self {
        self.vertical = on;
        self
    }

    /// Whether it takes zoom.
    pub const fn zoom(mut self, on: bool) -> Self {
        self.zoom = on;
        self
    }

    /// Whether every axis needs focus or arming.
    pub const fn gated(mut self, on: bool) -> Self {
        self.gated = on;
        self
    }

    /// The distance of one wheel line.
    pub const fn notch(mut self, notch: WheelNotch) -> Self {
        self.notch = notch;
        self
    }

    /// The taker is a scroll region (`WheelTaker::scrolls`).
    #[must_use]
    pub const fn scrolls(mut self, on: bool) -> Self {
        self.scrolls = on;
        self
    }

    /// The taker's scroll room: it passes a wheel on an axis it has no room left for to the next enclosing taker.
    #[must_use]
    pub const fn room(mut self, room: WheelRoom) -> Self {
        self.room = Some(room);
        self
    }

    /// Whether the taker can move by `delta` on the axis (`vertical` or horizontal); a taker without a room always can.
    fn has_room(&self, vertical: bool, delta: f32) -> bool {
        let Some(room) = self.room else { return true };
        let side = if delta > 0.0 { room.positive } else { room.negative };
        (if vertical { side.y } else { side.x }) > 0.0
    }

    /// `scroll`, given in `unit`s of the device, as points of this taker's view: a line is one notch, a page is
    /// the view.
    pub fn points(&self, scroll: Vec2, unit: WheelUnit) -> Vec2 {
        let size = self.rect.size();
        match unit {
            WheelUnit::Point => scroll,
            WheelUnit::Line => match self.notch {
                WheelNotch::Points(points) => scroll * points,
                WheelNotch::ViewShare(share) => scroll * size * share,
            },
            WheelUnit::Page => scroll * size,
        }
    }
}

/// The split rule: each axis of `wheel` goes to the innermost taker under `at` that takes it; a share no taker
/// takes, or that a scroll region at its end in that direction cannot move, passes to the next enclosing one. A gated taker takes no axis unless it is `focused` or
/// `armed`. Shift turns a vertical wheel sideways when its vertical owner is a scroll region. Shares of one owner merge.
pub fn split<O: Copy + PartialEq>(takers: &[WheelTaker<O>], at: Pos2, wheel: Wheel, focused: Option<O>, armed: Option<O>) -> Vec<(O, Wheel)> {
    // Takers register outer first, so among equal areas the later one is the deeper (or the upper) and goes first.
    let mut under: Vec<&WheelTaker<O>> = takers.iter().rev().filter(|taker| taker.rect.contains(at)).collect();
    under.sort_by(|a, b| a.rect.area().total_cmp(&b.rect.area()));
    let open = |taker: &&WheelTaker<O>| !taker.gated || armed == Some(taker.owner) || focused == Some(taker.owner);
    let vertical_owner = (wheel.scroll.y != 0.0).then(|| under.iter().find(|t| t.vertical && open(t))).flatten();
    // Shift turns the vertical wheel sideways over a scroll region (Photoshop); a value control that takes the
    // vertical wheel keeps Shift as its modifier, and a device that already sends a sideways axis keeps it.
    let wheel = if wheel.mods.shift && wheel.scroll.x == 0.0 && vertical_owner.is_some_and(|t| t.scrolls) {
        Wheel { scroll: Vec2::new(wheel.scroll.y, 0.0), ..wheel }
    } else {
        wheel
    };
    let vertical =
        (wheel.scroll.y != 0.0).then(|| under.iter().find(|t| t.vertical && open(t) && t.has_room(true, wheel.scroll.y))).flatten().map(|t| t.owner);
    let horizontal = (wheel.scroll.x != 0.0)
        .then(|| under.iter().find(|t| t.horizontal && open(t) && t.has_room(false, wheel.scroll.x)))
        .flatten()
        .map(|t| t.owner);
    let zoom = (wheel.zoom != 1.0).then(|| under.iter().find(|t| t.zoom && open(t))).flatten().map(|t| t.owner);
    let mut shares: Vec<(O, Wheel)> = Vec::new();
    let parts = [(vertical, Vec2::new(0.0, wheel.scroll.y), 1.0), (horizontal, Vec2::new(wheel.scroll.x, 0.0), 1.0), (zoom, Vec2::ZERO, wheel.zoom)];
    for (owner, scroll, zoom) in parts {
        let Some(owner) = owner else { continue };
        match shares.iter_mut().find(|(o, _)| *o == owner) {
            Some((_, share)) => {
                share.scroll += scroll;
                share.zoom *= zoom;
            }
            None => shares.push((owner, Wheel { scroll, zoom, ..wheel })),
        }
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::*;
    use emath::pos2;

    fn wheel(x: f32, y: f32) -> Wheel {
        Wheel::new(Vec2::new(x, y), Mods::NONE, WheelUnit::Point, WheelPhase::Move)
    }

    fn rect(size: f32) -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(size))
    }

    #[test]
    fn a_scroll_region_at_its_end_hands_the_wheel_to_the_enclosing_one() {
        let outer = WheelTaker::new(1u8, rect(200.0)).vertical(true).scrolls(true).room(WheelRoom::new(Vec2::splat(50.0), Vec2::splat(50.0)));
        let inner = WheelTaker::new(2u8, rect(100.0)).vertical(true).scrolls(true).room(WheelRoom::new(Vec2::ZERO, Vec2::splat(30.0)));
        let at = pos2(10.0, 10.0);
        let owner = |y| split(&[outer, inner], at, wheel(0.0, y), None, None).first().map(|(o, _)| *o);
        assert_eq!(owner(7.0), Some(1), "no room toward positive: the outer takes it");
        assert_eq!(owner(-7.0), Some(2), "room toward negative: the inner keeps it");
    }
}
