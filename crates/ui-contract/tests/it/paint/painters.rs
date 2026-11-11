//! The pure painters: focus rings, scroll bars, handles and the face id.

use craft_ui::testing::trial;
use egui::{pos2, vec2};

const EPS: f32 = 1e-3;

#[trial]
fn ellipse_points_lie_on_the_ellipse_whatever_the_conjugate_axes() {
    let center = pos2(50.0, 40.0);
    // A circle of radius 30 turned by 30 degrees and squeezed to a skew: the axes are conjugate, not orthogonal.
    let axes = [vec2(30.0, 0.0), vec2(15.0, 20.0)];
    let points = craft_ui::primitives::ellipse(center, axes);
    assert!(points.len() >= 16);
    // Solve p - center = a * axes[0] + b * axes[1]; the points are where a^2 + b^2 = 1.
    let det = axes[0].x * axes[1].y - axes[0].y * axes[1].x;
    for p in points {
        let d = p - center;
        let a = (d.x * axes[1].y - d.y * axes[1].x) / det;
        let b = (axes[0].x * d.y - axes[0].y * d.x) / det;
        assert!((a * a + b * b - 1.0).abs() < EPS, "{p:?} is off the ellipse");
    }
}
