//! Where a new window goes: cascaded from the most recent window, kept on the screen.

/// A rectangle in points: top-left corner and size.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

#[cfg(feature = "egui")]
impl From<egui::Rect> for Rect {
    fn from(r: egui::Rect) -> Self {
        Self::new(r.min.x, r.min.y, r.width(), r.height())
    }
}

#[cfg(feature = "egui")]
impl From<Rect> for egui::Rect {
    fn from(r: Rect) -> Self {
        egui::Rect::from_min_size(egui::pos2(r.x, r.y), egui::vec2(r.width, r.height))
    }
}

/// The default cascade step: about one title bar.
pub const CASCADE_STEP: f32 = 28.0;

/// Place a window of `size` (outer, in points) one `step` right of and below `previous`, the most
/// recent window's outer rectangle.
///
/// Each axis that would run past `bounds` (the monitor or its work area) starts over at the
/// bounds' edge, so a long run of new windows walks down the screen and wraps back to the top
/// left. A window larger than `bounds` goes to the bounds' corner; its size is never changed.
pub fn cascade(previous: Rect, size: [f32; 2], bounds: Rect, step: f32) -> Rect {
    let axis = |prev: f32, len: f32, lo: f32, hi: f32| {
        let pos = prev + step;
        if pos < lo || pos + len > hi { lo } else { pos }
    };
    Rect::new(axis(previous.x, size[0], bounds.x, bounds.right()), axis(previous.y, size[1], bounds.y, bounds.bottom()), size[0], size[1])
}

/// [`cascade`] in a right-to-left interface: the x step measures from the bounds' reading-end
/// (right) edge and runs toward reading start, wrapped back inside `bounds` on both sides — the
/// same rule as the floating-panel `float_window_x`. The y step is unchanged.
pub fn cascade_rtl(previous: Rect, size: [f32; 2], bounds: Rect, step: f32) -> Rect {
    // Reflect the x axis about the bounds, cascade left-to-right there, reflect back.
    let flip = |x: f32, w: f32| bounds.x + bounds.right() - x - w;
    let mirrored = Rect::new(flip(previous.x, previous.width), previous.y, previous.width, previous.height);
    let placed = cascade(mirrored, size, bounds, step);
    Rect::new(flip(placed.x, placed.width), placed.y, placed.width, placed.height)
}

/// [`cascade`], or [`cascade_rtl`] when `rtl`: where a new window goes in the interface direction.
pub fn cascade_in(previous: Rect, size: [f32; 2], bounds: Rect, step: f32, rtl: bool) -> Rect {
    if rtl { cascade_rtl(previous, size, bounds, step) } else { cascade(previous, size, bounds, step) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);

    #[test]
    fn steps_down_and_right() {
        let prev = Rect::new(100.0, 50.0, 400.0, 300.0);
        assert_eq!(cascade(prev, [400.0, 300.0], SCREEN, 28.0), Rect::new(128.0, 78.0, 400.0, 300.0));
    }

    #[test]
    fn wraps_each_axis_at_the_bounds() {
        // Bottom edge: y starts over, x keeps stepping.
        let prev = Rect::new(100.0, 480.0, 400.0, 300.0);
        assert_eq!(cascade(prev, [400.0, 300.0], SCREEN, 28.0), Rect::new(128.0, 0.0, 400.0, 300.0));
        // Right edge on a second monitor whose work area starts at (1000, 30).
        let second = Rect::new(1000.0, 30.0, 1200.0, 770.0);
        let prev = Rect::new(1780.0, 100.0, 400.0, 300.0);
        assert_eq!(cascade(prev, [400.0, 300.0], second, 28.0), Rect::new(1000.0, 128.0, 400.0, 300.0));
    }

    #[test]
    fn a_window_off_the_bounds_or_too_big_goes_to_the_corner() {
        let prev = Rect::new(-500.0, -200.0, 400.0, 300.0);
        assert_eq!(cascade(prev, [400.0, 300.0], SCREEN, 28.0), Rect::new(0.0, 0.0, 400.0, 300.0));
        let prev = Rect::new(10.0, 10.0, 400.0, 300.0);
        assert_eq!(cascade(prev, [1200.0, 900.0], SCREEN, 28.0), Rect::new(0.0, 0.0, 1200.0, 900.0));
    }

    #[test]
    fn cascade_rtl_steps_left_from_the_right_edge() {
        // Mirrored x of 500 is 1000 - 500 - 400 = 100; LTR cascade gives 128; back: 1000 - 128 - 400 = 472.
        let prev = Rect::new(500.0, 50.0, 400.0, 300.0);
        assert_eq!(cascade_rtl(prev, [400.0, 300.0], SCREEN, 28.0), Rect::new(472.0, 78.0, 400.0, 300.0));
        assert_eq!(cascade_in(prev, [400.0, 300.0], SCREEN, 28.0, true), Rect::new(472.0, 78.0, 400.0, 300.0));
        assert_eq!(cascade_in(prev, [400.0, 300.0], SCREEN, 28.0, false), cascade(prev, [400.0, 300.0], SCREEN, 28.0));
    }

    #[test]
    fn cascade_rtl_wraps_at_the_reading_start_edge() {
        // A window at the physical left (the RTL reading start) wraps to the right edge.
        let prev = Rect::new(0.0, 480.0, 400.0, 300.0);
        assert_eq!(cascade_rtl(prev, [400.0, 300.0], SCREEN, 28.0), Rect::new(600.0, 0.0, 400.0, 300.0));
        // A window too wide for the bounds goes to the reading-end corner (physical right).
        let prev = Rect::new(10.0, 10.0, 400.0, 300.0);
        assert_eq!(cascade_rtl(prev, [1200.0, 900.0], SCREEN, 28.0), Rect::new(1000.0 - 1200.0, 0.0, 1200.0, 900.0));
    }
}
