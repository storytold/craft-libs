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
fn a_concave_polygon_fills_its_area_once_in_either_winding() {
    use egui::{Color32, Pos2};

    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let area = |ring: &[Pos2]| (0..ring.len()).map(|i| cross(Pos2::ZERO, ring[i], ring[(i + 1) % ring.len()])).sum::<f32>().abs() / 2.0;
    // Even-odd ray cast to the right.
    let inside = |ring: &[Pos2], p: Pos2| {
        (0..ring.len()).fold(false, |odd, i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) { !odd } else { odd }
        })
    };
    let ell = vec![pos2(0.0, 0.0), pos2(40.0, 0.0), pos2(40.0, 10.0), pos2(10.0, 10.0), pos2(10.0, 30.0), pos2(0.0, 30.0)];
    // A doughnut sector over 2 rad: the outer arc out, the inner arc back.
    let on = |radius: f32, i: usize| {
        let angle = 2.0 * i as f32 / 40.0;
        pos2(60.0 + radius * angle.sin(), 60.0 - radius * angle.cos())
    };
    let sector: Vec<Pos2> = (0..=40).map(|i| on(50.0, i)).chain((0..=40).rev().map(|i| on(25.0, i))).collect();

    for shape in [ell, sector] {
        for ring in [shape.clone(), shape.iter().rev().copied().collect()] {
            let mesh = craft_ui::primitives::fill_polygon(&ring, Color32::WHITE, 0.0);
            let triangles: Vec<[Pos2; 3]> = mesh.indices.chunks(3).map(|t| [0, 1, 2].map(|k| mesh.vertices[t[k] as usize].pos)).collect();
            let turns: Vec<f32> = triangles.iter().map(|&[a, b, c]| cross(a, b, c)).collect();
            assert!(turns.iter().all(|t| *t > 0.0) || turns.iter().all(|t| *t < 0.0), "every triangle turns the same way");
            let covered: f32 = turns.iter().map(|t| t.abs() / 2.0).sum();
            let whole = area(&ring);
            assert!((covered - whole).abs() < whole * 1e-3, "the triangles cover {covered}, the polygon {whole}");
            for [a, b, c] in triangles {
                let centroid = pos2((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
                assert!(inside(&ring, centroid), "a triangle around {centroid:?} lies outside the polygon");
            }
        }
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

#[trial]
fn an_edge_rule_lies_on_the_pixels_just_inside_its_side_at_any_scale() {
    use craft_ui::primitives::{RectSide, Rule};
    use craft_ui::style::{RuleLook, RuleWidth};
    use egui::{Color32, Rect, Shape};

    for ppp in [1.0, 1.5, 2.0] {
        // Edges on physical pixel boundaries, as laid-out rows and cells have them.
        let rect = Rect::from_min_max(pos2(30.0, 45.0), pos2(150.0, 90.0)) / ppp;
        for width in [RuleWidth::Hairline, RuleWidth::Points(2.0)] {
            let look = RuleLook { color: Color32::WHITE, width };
            for side in [RectSide::Top, RectSide::Bottom, RectSide::Left, RectSide::Right] {
                let shapes = Rule::edge(&look, rect, side, ppp);
                let [Shape::LineSegment { points: [from, to], stroke }] = shapes.as_slice() else {
                    panic!("one line segment for {side:?}");
                };
                let half = stroke.width / 2.0;
                let horizontal = matches!(side, RectSide::Top | RectSide::Bottom);
                let (at, low, high) = if horizontal { (from.y, rect.top(), rect.bottom()) } else { (from.x, rect.left(), rect.right()) };
                assert!(at - half >= low - EPS && at + half <= high + EPS, "{side:?} at {ppp}x: the band {at}±{half} leaves the rect");
                let edge = match side {
                    RectSide::Top => at - half - low,
                    RectSide::Left => at - half - low,
                    _ => high - (at + half),
                };
                assert!(edge.abs() < EPS, "{side:?} at {ppp}x: the band does not touch its side");
                let along = if horizontal { (from.x, to.x, rect.left(), rect.right()) } else { (from.y, to.y, rect.top(), rect.bottom()) };
                assert!((along.0 - along.2).abs() < EPS && (along.1 - along.3).abs() < EPS, "{side:?} runs the whole side");
            }
        }
    }
}
