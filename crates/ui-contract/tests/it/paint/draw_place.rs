//! Where `Draw` puts a stroke against a closed shape's edge, and a picture on a turned quad.

use craft_theme::{Contrast, Mode};
use craft_ui::primitives::{PaintShape, StrokePlace, StrokeWidth};
use craft_ui::runtime::env::Direction;
use craft_ui::style::Ink;
use craft_ui::testing::style::{paint_frame, tokens};
use craft_ui::testing::{Frames, trial};
use egui::epaint::{Mesh, TessellationOptions, Tessellator};
use egui::{CornerRadius, Pos2, Rect, Shape, pos2};

/// Slack for the polygon a tessellated circle is.
const EPS: f32 = 0.05;

/// The vertices `shapes` tessellate to, without feathering.
fn vertices(shapes: Vec<Shape>) -> Vec<Pos2> {
    let options = TessellationOptions { feathering: false, ..TessellationOptions::default() };
    let mut tessellator = Tessellator::new(1.0, options, [1, 1], vec![]);
    let mut mesh = Mesh::default();
    for shape in shapes {
        tessellator.tessellate_shape(shape, &mut mesh);
    }
    mesh.vertices.iter().map(|vertex| vertex.pos).collect()
}

/// An inside stroke stays within every closed shape's edge and an outside one stays out of it, whatever the shape;
/// a centred one crosses it.
#[trial]
fn a_placed_stroke_lies_on_its_side_of_every_closed_shapes_edge() {
    let tokens = tokens(Mode::Dark, Contrast::Standard);
    let rect = Rect::from_min_max(pos2(20.0, 20.0), pos2(80.0, 60.0));
    let centre = pos2(50.0, 50.0);
    let radius = 20.0;
    let square = vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let shapes = [PaintShape::Rect(rect, CornerRadius::ZERO), PaintShape::Polygon(square), PaintShape::Circle(centre, radius)];
    // How far a point lies outside the shape's edge (negative inside).
    let outside = |shape: &PaintShape, p: Pos2| match shape {
        PaintShape::Circle(..) => p.distance(centre) - radius,
        _ => {
            let dx = (rect.left() - p.x).max(p.x - rect.right());
            let dy = (rect.top() - p.y).max(p.y - rect.bottom());
            dx.max(dy)
        }
    };
    let mut frames = Frames::new();
    for shape in &shapes {
        for place in [StrokePlace::Inside, StrokePlace::Centre, StrokePlace::Outside] {
            let (painted, _) = paint_frame(&mut frames, &tokens, Direction::Ltr, Ink::OnPanel, |draw| {
                draw.stroke_placed(shape, place, StrokeWidth::Heavy, draw.role(|colors| colors.text.muted));
            });
            let points = vertices(painted);
            assert!(!points.is_empty(), "{shape:?} {place:?} paints");
            let (low, high) = points.iter().map(|p| outside(shape, *p)).fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
            match place {
                StrokePlace::Inside => assert!(high <= EPS, "{shape:?}: an inside stroke reaches {high} past the edge"),
                StrokePlace::Outside => assert!(low >= -EPS, "{shape:?}: an outside stroke reaches {low} into the shape"),
                StrokePlace::Centre => assert!(low < -EPS && high > EPS, "{shape:?}: a centred stroke crosses the edge ({low}..{high})"),
            }
        }
    }
}

/// A picture painted on a quarter-turned quad puts each corner of its part where the quad names it, so one cached
/// texture paints at any turn.
#[trial]
fn a_texture_quad_lands_each_corner_of_its_part_on_the_quads_corner() {
    let tokens = tokens(Mode::Dark, Contrast::Standard);
    let uv = Rect::from_min_max(pos2(0.25, 0.0), pos2(0.75, 1.0));
    // The rect (10, 10)–(50, 30) turned a quarter clockwise about its top left: its top left lands top right.
    let corners = [pos2(30.0, 10.0), pos2(30.0, 50.0), pos2(10.0, 50.0), pos2(10.0, 10.0)];
    let mut frames = Frames::new();
    let (painted, _) = paint_frame(&mut frames, &tokens, Direction::Ltr, Ink::OnPanel, |draw| {
        draw.texture_quad(corners, egui::TextureId::User(7), uv);
    });
    let [Shape::Mesh(mesh)] = painted.as_slice() else { panic!("one mesh, got {painted:?}") };
    assert_eq!(mesh.texture_id, egui::TextureId::User(7));
    for (want, at) in [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()].into_iter().zip(corners) {
        let vertex = mesh.vertices.iter().find(|v| v.uv == want).expect("a vertex per corner of the part");
        assert_eq!(vertex.pos, at, "the part's corner {want:?} lands on the quad's corner");
    }
    let area: f32 = mesh
        .indices
        .chunks(3)
        .map(|t| {
            let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[t[i] as usize].pos);
            ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0
        })
        .sum();
    assert!((area - 800.0).abs() < 1e-3, "the triangles cover the quad once: {area}");
}
