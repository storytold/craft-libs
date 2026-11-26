//! The colour area's gamut shading: which cells of the 16 × 16 grid are shaded and where the edges run.

use craft_color_ui::AreaMode;
use craft_color_ui::ui::{AreaShape, ColorArea, Gamut};
use craft_theme::{ColorModel, HsbModel, Srgba};
use craft_ui::contract::Show as _;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Color32, Pos2, Rect, Shape, Ui, Vec2};

const GRID: f32 = 16.0;

fn area(ui: &mut Ui, data: &mut StoryData, gamut: Gamut<'static>) -> StoryOutput {
    let values = data.bind("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values).shape(AreaShape::Square).gamut(gamut).show(ui).into()
}

/// Moves the pale colours of the top-left quarter (saturation under a half, brightness over a half) to black.
fn top_left_quarter(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let to_black = |color: Srgba| {
        let (high, low) = (color.r.max(color.g).max(color.b), color.r.min(color.g).min(color.b));
        if high > 0.5 && low > high / 2.0 { Srgba::rgb(0.0, 0.0, 0.0) } else { color }
    };
    area(ui, data, Gamut::new(to_black))
}

fn moves_nothing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    area(ui, data, Gamut::new(|color: Srgba| color))
}

/// The fills that make up the shading: the largest group of rectangles of one fill and one size.
fn shaded_cells(shapes: &[Shape]) -> Vec<Rect> {
    let rects: Vec<(Color32, Rect)> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) => Some((rect.fill, rect.rect)),
            _ => None,
        })
        .collect();
    let same = |a: &(Color32, Rect), b: &(Color32, Rect)| a.0 == b.0 && (a.1.size() - b.1.size()).abs().max_elem() < 1e-3;
    let group = rects.iter().map(|pivot| rects.iter().filter(|other| same(pivot, other)).copied().collect::<Vec<_>>()).max_by_key(Vec::len);
    group.filter(|group| group.len() > 1).map(|group| group.into_iter().map(|(_, rect)| rect).collect()).unwrap_or_default()
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-2
}

#[trial]
fn a_gamut_shades_the_cells_it_moves_and_edges_them() {
    let mut fixture = Fixture::story(&Story::new("gamut", top_left_quarter));
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let cells = shaded_cells(&shapes);
    assert_eq!(cells.len(), 64, "the 8 × 8 cells of the top-left quarter are shaded");
    let min = cells.iter().fold(Pos2::new(f32::MAX, f32::MAX), |min, cell| min.min(cell.min));
    let size: Vec2 = cells[0].size();
    for row in 0..8 {
        for col in 0..8 {
            let at = min + Vec2::new(col as f32 * size.x, row as f32 * size.y);
            assert!(cells.iter().any(|shaded| near(shaded.min.x, at.x) && near(shaded.min.y, at.y)), "cell {col},{row} is shaded");
        }
    }

    // Edges lie on grid points of the field and only along the right and bottom side of the shaded block.
    let (right, bottom) = (min.x + 8.0 * size.x, min.y + 8.0 * size.y);
    let on_grid = |point: Pos2| {
        let (x, y) = ((point.x - min.x) / size.x, (point.y - min.y) / size.y);
        near(x, x.round()) && near(y, y.round()) && (0.0..=GRID + 0.01).contains(&x) && (0.0..=GRID + 0.01).contains(&y)
    };
    let edges: Vec<[Pos2; 2]> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::LineSegment { points, .. } if points.iter().all(|point| on_grid(*point)) => Some(*points),
            _ => None,
        })
        .collect();
    assert_eq!(edges.len(), 2 * 16, "a dark and a light line along each of the 16 cell sides of the boundary");
    for [from, to] in edges {
        let vertical = near(from.x, right) && near(to.x, right) && from.y > min.y - 1e-2 && to.y < bottom + 1e-2;
        let horizontal = near(from.y, bottom) && near(to.y, bottom) && from.x > min.x - 1e-2 && to.x < right + 1e-2;
        assert!(vertical || horizontal, "{from:?} to {to:?} lies on the boundary of the shaded block");
    }
}

#[trial]
fn a_gamut_that_moves_nothing_shades_nothing() {
    let mut fixture = Fixture::story(&Story::new("in gamut", moves_nothing));
    fixture.settle();
    assert!(shaded_cells(&fixture.frame_shapes()).is_empty());
}
