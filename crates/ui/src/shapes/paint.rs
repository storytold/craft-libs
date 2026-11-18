use super::ShapePreset;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};

/// Paint an original procedural preview, adapted from WordCraft's Insert Shapes menu (#2142).
/// Preview geometry is independent of document geometry: it does not insert/render editable
/// document objects. No external icon assets, fonts or application dependencies are needed.
/// Invalid/empty/excessive rectangles produce no paint; stroke widths are bounded to the icon.
pub fn paint_preset(painter: &Painter, rect: Rect, preset: ShapePreset, stroke: Stroke, fill: Color32) {
    if !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 || rect.size().max_elem() > 4096.0 {
        return;
    }
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    let scale = rect.width().min(rect.height()) / 20.0;
    let origin = rect.center() - vec2(10.0, 10.0) * scale;
    let point = |x: f32, y: f32| origin + vec2(x, y) * scale;
    let width = if stroke.width.is_finite() { stroke.width.clamp(0.0, rect.width().min(rect.height()) * 0.25) } else { 0.0 };
    let stroke = Stroke::new(width, stroke.color);
    let polygon = |points: &[(f32, f32)]| {
        painter.add(Shape::convex_polygon(points.iter().map(|&(x, y)| point(x, y)).collect(), fill, stroke));
    };
    let line = |points: &[(f32, f32)]| {
        painter.add(Shape::line(points.iter().map(|&(x, y)| point(x, y)).collect(), stroke));
    };
    let head = |tip: Pos2, previous: Pos2| {
        let direction = (tip - previous).normalized();
        let side = vec2(-direction.y, direction.x);
        painter.add(Shape::line(
            vec![tip - direction * (4.0 * scale) + side * (2.5 * scale), tip, tip - direction * (4.0 * scale) - side * (2.5 * scale)],
            stroke,
        ));
    };
    match preset {
        ShapePreset::Rectangle | ShapePreset::RoundedRectangle | ShapePreset::TextBox => {
            let shape = Rect::from_min_max(point(3.0, 5.0), point(17.0, 15.0));
            let radius = if preset == ShapePreset::RoundedRectangle { 3.0 * scale } else { 0.0 };
            painter.rect(shape, radius, fill, stroke, StrokeKind::Inside);
            if preset == ShapePreset::TextBox {
                for y in [8.0, 10.0, 12.0] {
                    line(&[(6.0, y), (14.0, y)]);
                }
            }
        }
        ShapePreset::Ellipse => {
            painter.add(Shape::ellipse_filled(rect.center(), vec2(7.5, 5.5) * scale, fill));
            painter.add(Shape::ellipse_stroke(rect.center(), vec2(7.5, 5.5) * scale, stroke));
        }
        ShapePreset::Triangle => polygon(&[(10.0, 3.5), (17.0, 16.5), (3.0, 16.5)]),
        ShapePreset::Diamond => polygon(&[(10.0, 3.0), (17.0, 10.0), (10.0, 17.0), (3.0, 10.0)]),
        ShapePreset::Line | ShapePreset::Arrow => {
            line(&[(4.0, 16.0), (16.0, 4.0)]);
            if preset.has_end_arrow() {
                head(point(16.0, 4.0), point(4.0, 16.0));
            }
        }
        ShapePreset::Star => {
            let points: Vec<_> = (0..10)
                .map(|i| {
                    let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    let radius = if i % 2 == 0 { 8.0 } else { 3.3 };
                    point(10.0 + radius * angle.cos(), 10.8 + radius * angle.sin())
                })
                .collect();
            fill_contour(&painter, point(10.0, 10.8), &points, fill);
            painter.add(Shape::closed_line(points, stroke));
        }
        ShapePreset::Heart => {
            let points: Vec<_> = (0..32)
                .map(|i| {
                    let t = i as f32 / 32.0 * std::f32::consts::TAU;
                    point(
                        10.0 + 16.0 * t.sin().powi(3) * 0.44,
                        9.6 - (13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos()) * 0.44,
                    )
                })
                .collect();
            fill_contour(&painter, point(10.0, 10.0), &points, fill);
            painter.add(Shape::closed_line(points, stroke));
        }
        _ => {
            let points: Vec<_> = match preset {
                ShapePreset::ElbowConnector | ShapePreset::ElbowArrowConnector => {
                    vec![point(4.0, 15.0), point(10.0, 15.0), point(10.0, 5.0), point(16.0, 5.0)]
                }
                ShapePreset::CurvedConnector | ShapePreset::CurvedArrowConnector => (0..=16)
                    .map(|i| {
                        let t = i as f32 / 16.0;
                        let m = 1.0 - t;
                        let w = [m * m * m, 3.0 * m * m * t, 3.0 * m * t * t, t * t * t];
                        point(w[0] * 4.0 + w[1] * 12.0 + w[2] * 8.0 + w[3] * 16.0, w[0] * 15.0 + w[1] * 15.0 + w[2] * 5.0 + w[3] * 5.0)
                    })
                    .collect(),
                _ => vec![point(4.0, 15.0), point(16.0, 5.0)],
            };
            painter.add(Shape::line(points.clone(), stroke));
            if let Some(&first) = points.first() {
                painter.circle_filled(first, 1.6 * scale, stroke.color);
            }
            if let [.., previous, tip] = points.as_slice() {
                if preset.has_end_arrow() {
                    head(*tip, *previous);
                } else {
                    painter.circle_filled(*tip, 1.6 * scale, stroke.color);
                }
            }
        }
    }
    // The painter's existing clip handles its host; all preview geometry stays in the icon box.
}

/// These two fixed contours are star-shaped around the supplied center: it lies in every
/// edge's interior half-plane. A triangle fan therefore fills their concavities without the
/// convex-polygon assumption. This is not a triangulator for arbitrary application paths.
fn fill_contour(painter: &Painter, center: Pos2, points: &[Pos2], fill: Color32) {
    if points.len() < 3 || fill == Color32::TRANSPARENT {
        return;
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, fill);
    for &point in points {
        mesh.colored_vertex(point, fill);
    }
    for index in 0..points.len() {
        mesh.add_triangle(0, 1 + index as u32, 1 + ((index + 1) % points.len()) as u32);
    }
    painter.add(Shape::mesh(mesh));
}
