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

    /// The rectangle with its position and size times `factor`: from points to physical pixels
    /// and back.
    pub fn scaled(&self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor, self.width * factor, self.height * factor)
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

/// The monitor `window` is on: the one showing most of it or, when none shows it, the nearest.
/// By area, not by a corner: on Windows a maximized window's corner is outside its monitor. `None`
/// only when there are no monitors.
pub fn monitor_of(window: Rect, monitors: &[Rect]) -> Option<Rect> {
    let shown = |m: &Rect| {
        let width = window.right().min(m.right()) - window.x.max(m.x);
        let height = window.bottom().min(m.bottom()) - window.y.max(m.y);
        width.max(0.0) * height.max(0.0)
    };
    let gap = |m: &Rect| {
        let across = (m.x - window.right()).max(window.x - m.right()).max(0.0);
        let down = (m.y - window.bottom()).max(window.y - m.bottom()).max(0.0);
        across * across + down * down
    };
    monitors.iter().copied().max_by(|a, b| shown(a).total_cmp(&shown(b)).then_with(|| gap(b).total_cmp(&gap(a))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);

    #[test]
    fn a_window_is_on_the_monitor_showing_most_of_it() {
        let left = Rect::new(-1920.0, 200.0, 1920.0, 1080.0);
        let main = Rect::new(0.0, 0.0, 3840.0, 1600.0);
        let below = Rect::new(718.0, 1600.0, 1280.0, 720.0);
        let desktop = [left, main, below];
        assert_eq!(monitor_of(Rect::new(-8.0, -8.0, 3856.0, 1576.0), &desktop), Some(main), "maximized, its corner outside");
        assert_eq!(monitor_of(Rect::new(-1928.0, 192.0, 1936.0, 1056.0), &desktop), Some(left), "maximized, on the left");
        assert_eq!(monitor_of(Rect::new(800.0, 1700.0, 400.0, 300.0), &desktop), Some(below), "offset and smaller");
        assert_eq!(monitor_of(Rect::new(-100.0, 300.0, 1000.0, 500.0), &desktop), Some(main), "across two, mostly on one");
        assert_eq!(monitor_of(Rect::new(3400.0, 100.0, 800.0, 600.0), &[main]), Some(main), "past the right edge");
    }

    #[test]
    fn a_window_no_monitor_shows_goes_to_the_nearest() {
        let main = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let right = Rect::new(1920.0, 0.0, 1920.0, 1080.0);
        assert_eq!(monitor_of(Rect::new(-3840.0, -1600.0, 1456.0, 959.0), &[right, main]), Some(main));
        assert_eq!(monitor_of(Rect::new(5000.0, 100.0, 800.0, 600.0), &[main, right]), Some(right));
        assert_eq!(monitor_of(Rect::new(0.0, 0.0, 800.0, 600.0), &[]), None);
    }

    #[test]
    fn nonsense_numbers_still_give_a_monitor() {
        let main = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(monitor_of(Rect::new(f32::NAN, f32::INFINITY, f32::NAN, 1.0), &[main]), Some(main));
    }

    #[test]
    fn scaling_changes_position_and_size() {
        assert_eq!(Rect::new(10.0, 20.0, 30.0, 40.0).scaled(2.0), Rect::new(20.0, 40.0, 60.0, 80.0));
    }

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
}
