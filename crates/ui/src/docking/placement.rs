//! Where native floating windows go on the desktop (feature `native-windows`). Rects are
//! `[x, y, width, height]` in desktop logical points.

use egui::{Pos2, Rect, Vec2, pos2};

use super::Monitor;

/// Largest native window, and the farthest it may sit from the desktop origin.
const MAX_SIZE: f32 = 16_384.0;
const MAX_POSITION: f32 = 100_000.0;
const MIN_SIZE: [f32; 2] = [160.0, 100.0];
/// Height of the bar at the top of a native window.
pub(super) const BAR_HEIGHT: f32 = 18.0;
/// How much of a window's bar must lie on one monitor for the window to be reachable: wide
/// enough to grab, and half its height.
const REACHABLE: [f32; 2] = [40.0, BAR_HEIGHT / 2.0];

/// A rect safe to hand to the windowing system: finite, its size and position bounded.
/// Hostile workspace files and bad monitor reports included.
pub(super) fn sanitized(rect: [f32; 4]) -> [f32; 4] {
    let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
    let [x, y, w, h] = rect;
    [
        finite(x, 0.0).clamp(-MAX_POSITION, MAX_POSITION),
        finite(y, 0.0).clamp(-MAX_POSITION, MAX_POSITION),
        finite(w, 420.0).clamp(MIN_SIZE[0], MAX_SIZE),
        finite(h, 320.0).clamp(MIN_SIZE[1], MAX_SIZE),
    ]
}

fn usable(monitor: &Monitor) -> bool {
    let [x, y, w, h] = monitor.rect;
    [x, y, w, h].iter().all(|v| v.is_finite() && v.abs() <= MAX_POSITION) && w > 0.0 && h > 0.0
}

fn to_rect([x, y, w, h]: [f32; 4]) -> Rect {
    Rect::from_min_size(pos2(x, y), Vec2::new(w, h))
}

/// The monitor holding the center of `rect`, else the one nearest to it. None for an empty
/// list.
pub(super) fn monitor_of(monitors: &[Monitor], rect: [f32; 4]) -> Option<&Monitor> {
    let center = to_rect(rect).center();
    let monitors = || monitors.iter().filter(|m| usable(m));
    monitors()
        .find(|m| to_rect(m.rect).contains(center))
        .or_else(|| monitors().min_by(|a, b| to_rect(a.rect).distance_sq_to_pos(center).total_cmp(&to_rect(b.rect).distance_sq_to_pos(center))))
}

/// Whether enough of the bar along the top of `rect` lies on one of `monitors` to grab it.
fn reachable(rect: [f32; 4], monitors: &[Monitor]) -> bool {
    let [x, y, w, _] = rect;
    let bar = to_rect([x, y, w, BAR_HEIGHT]);
    monitors.iter().filter(|m| usable(m)).any(|m| {
        let on = bar.intersect(to_rect(m.rect));
        on.width() >= REACHABLE[0].min(w) && on.height() >= REACHABLE[1]
    })
}

/// `rect` made safe, and moved wholly onto the nearest of `monitors` when too little of its
/// bar lies on any to grab it. Without a monitor list, only made safe.
pub(super) fn kept_reachable(rect: [f32; 4], monitors: &[Monitor]) -> [f32; 4] {
    let rect = sanitized(rect);
    if !monitors.iter().any(usable) || reachable(rect, monitors) {
        return rect;
    }
    monitor_of(monitors, rect).map_or(rect, |m| clamp_onto(rect, m.rect))
}

/// `rect` moved, and shrunk if it must be, to lie wholly on `area`.
fn clamp_onto(rect: [f32; 4], area: [f32; 4]) -> [f32; 4] {
    let [x, y, w, h] = rect;
    let [ax, ay, aw, ah] = area;
    let (w, h) = (w.min(aw), h.min(ah));
    // `ax + aw - w` can round below `ax` when `w == aw`, so no `clamp`, which panics then.
    sanitized([x.min(ax + aw - w).max(ax), y.min(ay + ah - h).max(ay), w, h])
}

/// Where a native window from a saved layout opens.
///
/// - Its saved monitor (`monitor`), found in `monitors` by name or else by rect, is connected:
///   the saved rect, moved in proportion if that monitor's rect changed, and onto it if too
///   little of its bar lies on a monitor to grab it.
/// - It is not: the main window's monitor, at the same offset from its origin.
/// - No monitor list: the main window's monitor as egui reports it, by its size alone. Its
///   origin is taken as the multiple of that size nearest below the main window's center,
///   which holds for a row or grid of monitors of one size.
///
/// A window that moves to another monitor ends up wholly on it.
pub(super) fn restore(saved: [f32; 4], monitor: Option<&Monitor>, monitors: &[Monitor], main: Option<Rect>, monitor_size: Option<Vec2>) -> [f32; 4] {
    let saved = sanitized(saved);
    let main = main.filter(|r| r.is_finite()).map(|r| [r.left(), r.top(), r.width().max(1.0), r.height().max(1.0)]);
    if !monitors.iter().any(usable) {
        let (Some(main), Some(size)) = (main, monitor_size.filter(|s| s.is_finite() && s.x >= 1.0 && s.y >= 1.0)) else { return saved };
        let center = to_rect(main).center();
        let origin = Pos2::new((center.x / size.x).floor() * size.x, (center.y / size.y).floor() * size.y);
        return clamp_onto(saved, [origin.x, origin.y, size.x, size.y]);
    }
    let main_monitor = main.and_then(|main| monitor_of(monitors, main)).or_else(|| monitors.iter().find(|m| usable(m)));
    let connected = monitor.and_then(|old| {
        let by_name = old.name.as_ref().and_then(|name| monitors.iter().filter(|m| usable(m)).find(|m| m.name.as_ref() == Some(name)));
        by_name.or_else(|| monitors.iter().filter(|m| usable(m)).find(|m| m.rect == old.rect)).map(|now| (old, now))
    });
    let [x, y, w, h] = saved;
    match (connected, monitor) {
        (Some((old, now)), _) if old.rect == now.rect => {
            if reachable(saved, monitors) {
                saved
            } else {
                clamp_onto(saved, now.rect)
            }
        }
        (Some((old, now)), _) => {
            let [ox, oy, ow, oh] = old.rect;
            let [nx, ny, nw, nh] = now.rect;
            let (sx, sy) = if usable(old) { (nw / ow, nh / oh) } else { (1.0, 1.0) };
            clamp_onto([nx + (x - ox) * sx, ny + (y - oy) * sy, w, h], now.rect)
        }
        (None, Some(old)) => {
            let Some(target) = main_monitor else { return saved };
            let [ox, oy, ..] = old.rect;
            let [tx, ty, ..] = target.rect;
            let (dx, dy) = if usable(old) { (x - ox, y - oy) } else { (0.0, 0.0) };
            clamp_onto([tx + dx, ty + dy, w, h], target.rect)
        }
        // No saved monitor: the window stays where its bar can be grabbed.
        (None, None) => {
            if reachable(saved, monitors) {
                return saved;
            }
            main_monitor.map_or(saved, |target| clamp_onto(saved, target.rect))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn monitor(name: &str, rect: [f32; 4], scale: f32) -> Monitor {
        Monitor { name: Some(name.into()), rect, scale }
    }

    fn main_at(x: f32, y: f32) -> Option<Rect> {
        Some(Rect::from_min_size(pos2(x, y), vec2(1200.0, 800.0)))
    }

    const LEFT: [f32; 4] = [0.0, 0.0, 1920.0, 1080.0];
    const RIGHT: [f32; 4] = [1920.0, 0.0, 2560.0, 1440.0];

    #[test]
    fn a_connected_monitor_keeps_the_saved_rect() {
        let monitors = [monitor("left", LEFT, 1.0), monitor("right", RIGHT, 2.0)];
        let saved = [2400.0, 300.0, 400.0, 500.0];
        assert_eq!(restore(saved, Some(&monitors[1]), &monitors, main_at(100.0, 100.0), None), saved);
        // Renamed, but the same rect.
        let renamed = Monitor { name: Some("DP-2".into()), ..monitors[1].clone() };
        assert_eq!(restore(saved, Some(&renamed), &monitors, main_at(100.0, 100.0), None), saved);
        // Partly off its monitor, as the user left it.
        let edge = [4300.0, 300.0, 400.0, 500.0];
        assert_eq!(restore(edge, Some(&monitors[1]), &monitors, main_at(100.0, 100.0), None), edge);
        // Wholly off it, or with only its body on it: back onto it.
        let off = [5000.0, 300.0, 400.0, 500.0];
        assert_eq!(restore(off, Some(&monitors[1]), &monitors, main_at(100.0, 100.0), None), [4080.0, 300.0, 400.0, 500.0]);
        let above = [2400.0, -10.0, 400.0, 500.0];
        assert_eq!(restore(above, Some(&monitors[1]), &monitors, main_at(100.0, 100.0), None), [2400.0, 0.0, 400.0, 500.0]);
    }

    #[test]
    fn a_window_is_kept_where_its_bar_can_be_grabbed() {
        let monitors = [monitor("left", LEFT, 1.0), monitor("right", RIGHT, 2.0)];
        // Across the seam, or mostly off screen with enough of its bar on one.
        for rect in [[1800.0, 100.0, 400.0, 300.0], [-360.0, 1071.0, 400.0, 300.0]] {
            assert_eq!(kept_reachable(rect, &monitors), rect);
        }
        assert_eq!(kept_reachable([-370.0, 100.0, 400.0, 300.0], &monitors), [0.0, 100.0, 400.0, 300.0]);
        assert_eq!(kept_reachable([500.0, 1075.0, 400.0, 300.0], &monitors), [500.0, 780.0, 400.0, 300.0]);
        assert_eq!(kept_reachable([9000.0, 100.0, 400.0, 300.0], &monitors), [4080.0, 100.0, 400.0, 300.0]);
        // No monitor list: only made safe.
        assert_eq!(kept_reachable([9000.0, f32::NAN, 400.0, 300.0], &[]), [9000.0, 0.0, 400.0, 300.0]);
    }

    #[test]
    fn a_missing_monitor_moves_the_window_to_the_main_window_monitor() {
        let saved_on = monitor("right", RIGHT, 2.0);
        let monitors = [monitor("left", LEFT, 1.0)];
        // Same offset from the monitor's origin.
        assert_eq!(restore([2020.0, 50.0, 400.0, 500.0], Some(&saved_on), &monitors, main_at(100.0, 100.0), None), [100.0, 50.0, 400.0, 500.0]);
        // Clamped wholly on screen, shrunk if larger than the monitor.
        assert_eq!(restore([4000.0, 1200.0, 400.0, 500.0], Some(&saved_on), &monitors, main_at(100.0, 100.0), None), [1520.0, 580.0, 400.0, 500.0]);
        assert_eq!(restore([1920.0, 0.0, 3000.0, 1300.0], Some(&saved_on), &monitors, main_at(100.0, 100.0), None), [0.0, 0.0, 1920.0, 1080.0]);
    }

    #[test]
    fn a_resized_monitor_moves_the_window_in_proportion() {
        let old = monitor("left", LEFT, 1.0);
        let monitors = [monitor("left", [0.0, 0.0, 960.0, 540.0], 2.0)];
        assert_eq!(restore([960.0, 540.0, 200.0, 200.0], Some(&old), &monitors, main_at(0.0, 0.0), None), [480.0, 270.0, 200.0, 200.0]);
        assert_eq!(restore([1800.0, 1000.0, 200.0, 200.0], Some(&old), &monitors, main_at(0.0, 0.0), None), [760.0, 340.0, 200.0, 200.0]);
    }

    #[test]
    fn negative_coordinates_and_mixed_scales() {
        let monitors = [monitor("left", [-2560.0, -200.0, 2560.0, 1440.0], 2.0), monitor("main", LEFT, 1.0)];
        assert_eq!(monitor_of(&monitors, [-500.0, 0.0, 300.0, 300.0]).and_then(|m| m.name.as_deref()), Some("left"));
        assert_eq!(monitor_of(&monitors, [100.0, 0.0, 300.0, 300.0]).and_then(|m| m.name.as_deref()), Some("main"));
        // Off every monitor: the nearest.
        assert_eq!(monitor_of(&monitors, [-3000.0, 2000.0, 100.0, 100.0]).and_then(|m| m.name.as_deref()), Some("left"));
        let saved = [-1000.0, -100.0, 300.0, 300.0];
        assert_eq!(restore(saved, Some(&monitors[0]), &monitors, main_at(100.0, 100.0), None), saved);
        // Unplugged: the same offset on the main window's monitor, at its own scale.
        let rest = &monitors[1..];
        assert_eq!(restore(saved, Some(&monitors[0]), rest, main_at(100.0, 100.0), None), [1560.0, 100.0, 300.0, 300.0]);
        // A main window on the left monitor takes the window there.
        let main = monitor("main", LEFT, 1.0);
        assert_eq!(restore([100.0, 100.0, 400.0, 300.0], Some(&main), &monitors[..1], main_at(-2000.0, 0.0), None), [-2460.0, -100.0, 400.0, 300.0]);
    }

    #[test]
    fn without_a_monitor_list_the_window_goes_on_the_main_window_monitor_by_size() {
        let size = Some(vec2(1920.0, 1080.0));
        let saved = [2500.0, 300.0, 400.0, 500.0];
        assert_eq!(restore(saved, None, &[], main_at(100.0, 100.0), size), [1520.0, 300.0, 400.0, 500.0]);
        assert_eq!(restore(saved, None, &[], main_at(2000.0, 100.0), size), saved);
        assert_eq!(restore(saved, None, &[], main_at(-1500.0, 100.0), size), [-400.0, 300.0, 400.0, 500.0]);
        // Nothing known: the saved rect, made safe.
        assert_eq!(restore([f32::NAN, 1e9, 10.0, 400.0], None, &[], None, None), [0.0, 100_000.0, 160.0, 400.0]);
    }

    #[test]
    fn clamping_onto_a_fractional_origin_does_not_panic() {
        let area = [910.6667, 0.333_33, 1280.0, 720.0];
        let [x, y, w, h] = clamp_onto([0.0, 2000.0, 1280.0, 900.0], area);
        assert_eq!([w, h], [1280.0, 720.0]);
        assert!((x - area[0]).abs() < 0.01 && (y - area[1]).abs() < 0.01, "{x} {y}");
        let [x, ..] = clamp_onto([5000.0, 0.0, 2000.0, 100.0], area);
        assert!((x - area[0]).abs() < 0.01, "{x}");
    }

    #[test]
    fn a_window_without_a_saved_monitor_stays_on_screen() {
        let monitors = [monitor("left", LEFT, 1.0), monitor("right", RIGHT, 2.0)];
        let on = [2000.0, 100.0, 400.0, 300.0];
        assert_eq!(restore(on, None, &monitors, main_at(100.0, 100.0), None), on);
        assert_eq!(restore([5000.0, 100.0, 400.0, 300.0], None, &monitors, main_at(100.0, 100.0), None), [1520.0, 100.0, 400.0, 300.0]);
    }
}
