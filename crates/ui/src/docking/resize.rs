use egui::{Pos2, Rect, Vec2};

use super::PanelLimits;

/// A pointer-anchored resize. The opposite edges and handle grab offset stay fixed
/// through boundary overshoot; viewport constraints never translate the window.
#[derive(Clone, Copy, Debug)]
pub struct AnchoredResize {
    start: Rect,
    offset: Vec2,
    direction: Vec2,
}

impl AnchoredResize {
    pub fn new(start: Rect, pointer: Pos2, direction: Vec2) -> Option<Self> {
        if !start.is_finite()
            || !start.is_positive()
            || !pointer.is_finite()
            || ![-1.0, 0.0, 1.0].contains(&direction.x)
            || ![-1.0, 0.0, 1.0].contains(&direction.y)
            || direction == Vec2::ZERO
        {
            return None;
        }
        let edge =
            egui::pos2(if direction.x < 0.0 { start.left() } else { start.right() }, if direction.y < 0.0 { start.top() } else { start.bottom() });
        Some(Self { start, offset: pointer - edge, direction })
    }

    /// None suspends resizing when the fixed anchor cannot accommodate the minimum.
    pub fn rect(self, pointer: Pos2, viewport: Rect, limits: PanelLimits) -> Option<Rect> {
        if !pointer.is_finite()
            || !viewport.is_finite()
            || !viewport.is_positive()
            || !limits.min.is_finite()
            || !limits.max.is_finite()
            || limits.min.x <= 0.0
            || limits.min.y <= 0.0
            || limits.max.x < limits.min.x
            || limits.max.y < limits.min.y
            || !viewport.contains_rect(self.start)
            || self.start.width() < limits.min.x
            || self.start.height() < limits.min.y
        {
            return None;
        }
        let desired = pointer - self.offset;
        if !desired.is_finite() {
            return None;
        }
        let mut rect = self.start;
        for (axis, direction) in [(0, self.direction.x), (1, self.direction.y)] {
            if direction == 0.0 {
                continue;
            }
            let (lo, hi) = if direction < 0.0 {
                ((self.start.max[axis] - limits.max[axis]).max(viewport.min[axis]), self.start.max[axis] - limits.min[axis])
            } else {
                (self.start.min[axis] + limits.min[axis], (self.start.min[axis] + limits.max[axis]).min(viewport.max[axis]))
            };
            if !lo.is_finite() || !hi.is_finite() || lo > hi {
                return None;
            }
            if direction < 0.0 {
                rect.min[axis] = desired[axis].clamp(lo, hi);
            } else {
                rect.max[axis] = desired[axis].clamp(lo, hi);
            }
        }
        Some(rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn every_handle_tracks_absolute_pointer_and_keeps_opposite_edges() {
        let start = Rect::from_min_max(pos2(100.0, 80.0), pos2(400.0, 300.0));
        let viewport = Rect::from_min_max(Pos2::ZERO, pos2(800.0, 600.0));
        let limits = PanelLimits { min: vec2(160.0, 120.0), max: vec2(500.0, 400.0) };
        for direction in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, 1.0)] {
            let edge = pos2(if direction.x < 0.0 { start.left() } else { start.right() }, start.bottom());
            let pointer = edge + vec2(2.0, -3.0);
            let resize = AnchoredResize::new(start, pointer, direction).unwrap();
            let rect = resize.rect(pointer + vec2(20.0, 25.0), viewport, limits).unwrap();
            assert_eq!(rect.top(), start.top());
            if direction.x < 0.0 {
                assert_eq!(rect.right(), start.right());
                assert_eq!(rect.left(), start.left() + 20.0);
            } else {
                assert_eq!(rect.left(), start.left());
            }
            if direction.x > 0.0 {
                assert_eq!(rect.right(), start.right() + 20.0);
            }
            if direction.y > 0.0 {
                assert_eq!(rect.bottom(), start.bottom() + 25.0);
            } else {
                assert_eq!(rect.bottom(), start.bottom());
            }
        }
    }

    #[test]
    fn overshoot_requires_pointer_to_return_to_the_valid_range() {
        let start = Rect::from_min_max(pos2(100.0, 80.0), pos2(400.0, 300.0));
        let viewport = Rect::from_min_max(Pos2::ZERO, pos2(600.0, 500.0));
        let limits = PanelLimits { min: vec2(160.0, 120.0), max: vec2(1000.0, 1000.0) };
        let resize = AnchoredResize::new(start, pos2(398.0, 200.0), vec2(1.0, 0.0)).unwrap();
        for x in [800.0, 750.0, 598.0] {
            let rect = resize.rect(pos2(x, 200.0), viewport, limits).unwrap();
            assert_eq!((rect.left(), rect.right()), (100.0, 600.0));
        }
        for x in [120.0, 200.0, 258.0] {
            assert_eq!(resize.rect(pos2(x, 200.0), viewport, limits).unwrap().right(), 260.0);
        }
        assert_eq!(resize.rect(pos2(278.0, 200.0), viewport, limits).unwrap().right(), 280.0);
        let left = AnchoredResize::new(start, pos2(102.0, 200.0), vec2(-1.0, 0.0)).unwrap();
        assert_eq!(left.rect(pos2(-300.0, 200.0), viewport, limits).unwrap().min.x, 0.0);
        assert_eq!(left.rect(pos2(350.0, 200.0), viewport, limits).unwrap().min.x, 240.0);
        assert_eq!(left.rect(pos2(300.0, 200.0), viewport, limits).unwrap().min.x, 240.0);
        assert_eq!(left.rect(pos2(222.0, 200.0), viewport, limits).unwrap().min.x, 220.0);
    }

    #[test]
    fn panel_maximum_caps_edges_without_losing_the_absolute_anchor() {
        let start = Rect::from_min_max(pos2(100.0, 80.0), pos2(400.0, 300.0));
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let limits = PanelLimits { min: vec2(160.0, 120.0), max: vec2(320.0, 240.0) };
        let resize = AnchoredResize::new(start, start.max, Vec2::splat(1.0)).unwrap();
        for pointer in [pos2(700.0, 550.0), pos2(600.0, 450.0)] {
            assert_eq!(resize.rect(pointer, viewport, limits).unwrap(), Rect::from_min_max(start.min, pos2(420.0, 320.0)));
        }
        assert_eq!(resize.rect(pos2(410.0, 310.0), viewport, limits).unwrap(), Rect::from_min_max(start.min, pos2(410.0, 310.0)));
    }

    #[test]
    fn constrained_viewports_and_invalid_geometry_suspend_resize() {
        let start = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 90.0));
        let resize = AnchoredResize::new(start, start.max, Vec2::splat(1.0)).unwrap();
        assert!(resize.rect(start.max, start, PanelLimits { min: vec2(160.0, 120.0), ..Default::default() }).is_none());
        assert!(AnchoredResize::new(start, pos2(f32::NAN, 0.0), Vec2::splat(1.0)).is_none());
    }
}
