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

#[trial]
fn a_rounded_ramp_keeps_inside_its_corners_and_a_square_one_fills_its_rect() {
    use craft_behavior::Axis;
    use craft_ui::style::DataColor;
    use egui::{Color32, CornerRadius, Rect, Shape};

    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(60.0, 20.0));
    let stops = [(0.0, DataColor::new(Color32::RED)), (0.5, DataColor::new(Color32::GREEN)), (1.0, DataColor::new(Color32::BLUE))];
    let points = |radius: CornerRadius, axis: Axis| -> Vec<egui::Pos2> {
        craft_ui::primitives::ramp(rect, radius, &stops, axis)
            .into_iter()
            .flat_map(|shape| match shape {
                Shape::Mesh(mesh) => mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    };

    let square = points(CornerRadius::ZERO, Axis::X);
    for corner in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
        assert!(square.iter().any(|p| p.distance(corner) < EPS), "a square ramp reaches {corner:?}");
    }

    let r = 6.0;
    for axis in [Axis::X, Axis::Y] {
        let rounded = points(CornerRadius::same(6), axis);
        assert!(rounded.len() > square.len(), "the rounded edge has samples along its corners");
        for p in rounded {
            assert!(rect.expand(EPS).contains(p), "{p:?} is outside the rect");
            // Inside a corner square, a point lies within the arc's circle.
            let cx = p.x.clamp(rect.left() + r, rect.right() - r);
            let cy = p.y.clamp(rect.top() + r, rect.bottom() - r);
            assert!(p.distance(pos2(cx, cy)) <= r + EPS, "{p:?} sticks out past a rounded corner along {axis:?}");
        }
    }
}
