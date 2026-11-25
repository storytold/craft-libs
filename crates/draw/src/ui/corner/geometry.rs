//! The corner widgets' geometry over [`fillet`](crate::geometry::fillet::fillet):
//! placement, the hide rules, the hit test and the drag maths. Document units unless a name says
//! screen.

use egui::Pos2;
use kurbo::{Point, Vec2};

use super::{CornerEdit, CornerPoint, CornerRadii, CornerRef, CornerTarget};
use crate::geometry::fillet::fillet;
use craft_behavior::CanvasView;

/// A widget the pass shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Widget {
    pub corner: CornerRef,
    /// Its centre on the screen.
    pub at: Pos2,
    /// How far the arc's centre moves per unit of radius ([`Fillet::step`](crate::geometry::fillet::Fillet::step)).
    pub step: Vec2,
    /// The corner's radius now, capped at [`max_radius`].
    pub radius: f64,
}

/// The largest radius `corner` takes: `fillet(prev, vertex, next, 1e12)`'s radius, 0 for a
/// degenerate corner. For a rectangle's corner, `min(w, h) / 2` (lead call 23).
pub(super) fn max_radius(corner: &CornerPoint) -> f64 {
    // pc: crates/ui-egui/src/tools/corner_radius.rs:91-97 `max_radius`, :113 (`fillet(.., 1e12)`).
    fillet(corner.prev, corner.vertex, corner.next, 1e12).map_or(0.0, |f| f.radius)
}

/// Where `corner`'s widget sits for `radius`: at the arc's centre (`vertex + step * r`), with
/// `r = radius.max(inset).min(max_radius)` where the inset is `inset_screen / zoom * √2 / |step|`
/// (measured like a rectangle's: `inset_screen` points along each axis of a right angle), so a
/// sharp or slightly rounded corner's widget sits the inset points inside. `None` for a
/// degenerate corner.
pub(super) fn widget_at(corner: &CornerPoint, radius: f64, inset_screen: f32, zoom: f64) -> Option<Point> {
    // pc: crates/ui-egui/src/tools/corner_radius.rs:107-120 `widget`, :22 `MIN_INSET`.
    let f = fillet(corner.prev, corner.vertex, corner.next, 1.0)?;
    let step = f.step();
    let most = max_radius(corner);
    let inset = f64::from(inset_screen) / zoom * std::f64::consts::SQRT_2 / step.hypot().max(1e-9);
    let r = radius.max(inset).min(most);
    Some(corner.vertex + step * r)
}

/// Whether `corner`'s widget shows: both its sides at least `min_side_screen` points long on the
/// screen (the min side), and its inner angle (`2 * half_angle`, degrees) not wider than
/// `hide_above` degrees (180 by default; VectorCraft's preference passes 177).
pub(super) fn shown(corner: &CornerPoint, view: &CanvasView, min_side_screen: f32, hide_above: f64) -> bool {
    // vectorcraft: crates/tools/src/corners.rs:23-25 (`MIN_INSET_PX` 10, `MIN_SIDE_PX` 30);
    // crates/engine/src/prefs.rs:60, :355 (`hide_corner_widget_above`, 177).
    let Some(f) = fillet(corner.prev, corner.vertex, corner.next, 1.0) else { return false };
    let vertex = view.to_screen(corner.vertex);
    let long = |to: Point| vertex.distance(view.to_screen(to)) >= min_side_screen;
    long(corner.prev) && long(corner.next) && (2.0 * f.half_angle).to_degrees() <= hide_above
}

/// The widgets of `targets` the pass shows, in target order and corner order (the host lists the
/// topmost target first).
pub(super) fn widgets(targets: &[CornerTarget], view: &CanvasView, inset_screen: f32, min_side_screen: f32, hide_above: f64) -> Vec<Widget> {
    // pc: crates/ui-egui/src/tools/corner_radius.rs:322-327 (every corner of every target).
    let mut out = Vec::new();
    for target in targets {
        for (i, corner) in target.corners.iter().enumerate() {
            if !shown(corner, view, min_side_screen, hide_above) {
                continue;
            }
            let radius = corner.radius.clamp(0.0, max_radius(corner));
            let Some(centre) = widget_at(corner, radius, inset_screen, view.zoom) else { continue };
            let Some(f) = fillet(corner.prev, corner.vertex, corner.next, 1.0) else { continue };
            out.push(Widget { corner: CornerRef { target: target.key, corner: i }, at: view.to_screen(centre), step: f.step(), radius });
        }
    }
    out
}

/// The dragged radius: the radius at the press plus the pointer's move from `press` to `pointer`
/// projected on `step` (`dot(move, step) / |step|²`), not below 0; `from` when not finite. A
/// click without movement gives `from`, so it changes nothing.
pub(super) fn dragged_radius(from: f64, step: Vec2, press: Point, pointer: Point) -> f64 {
    // pc: crates/ui-egui/src/tools/corner_radius.rs:250-256.
    let len2 = step.hypot2().max(1e-12);
    let r = from + (pointer - press).dot(step) / len2;
    if r.is_finite() { r.max(0.0) } else { from }
}

/// `targets` with the radii of `edit` applied.
pub(super) fn apply(targets: &[CornerTarget], edit: &CornerEdit) -> Vec<CornerTarget> {
    let mut out = targets.to_vec();
    for radii in &edit.targets {
        if let Some(target) = out.iter_mut().find(|t| t.key == radii.key) {
            for (corner, radius) in target.corners.iter_mut().zip(&radii.radii) {
                corner.radius = *radius;
            }
        }
    }
    out
}

/// The edit a drag of `grabbed` to `radius` makes: every target takes `radius`, capped at each
/// corner's [`max_radius`]; with `one_corner` (⌥) an `individual` target changes only the grabbed
/// corner (the grabbed target's; other individual targets the corner with the same index), and a
/// target without `individual` changes all its corners anyway. Targets whose radii stay the same
/// are left out.
pub(super) fn drag_edit(targets: &[CornerTarget], grabbed: CornerRef, radius: f64, one_corner: bool) -> CornerEdit {
    // pc: crates/ui-egui/src/tools/corner_radius.rs:257-268 (same radius within each shape's
    // maximum; ⌥ for rectangles only), :279-282 (unchanged layers skipped).
    let mut edit = CornerEdit::default();
    for target in targets {
        let n = target.corners.len();
        // A target without `individual` shares one radius, so its cap is its tightest corner's.
        let shared = target.corners.iter().map(max_radius).filter(|m| *m > 0.0).fold(f64::INFINITY, f64::min);
        let radii: Vec<f64> = target
            .corners
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if !target.individual {
                    radius.min(shared)
                } else if !one_corner || i == grabbed.corner {
                    radius.min(max_radius(c))
                } else {
                    c.radius
                }
            })
            .collect();
        let now: Vec<f64> = target.corners.iter().map(|c| c.radius).collect();
        if n > 0 && radii != now {
            edit.targets.push(CornerRadii { key: target.key, radii });
        }
    }
    edit
}
