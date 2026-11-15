//! The rows of the Paths panel: a thumbnail of the outline and the name.

use craft_behavior::ColumnKey;
use craft_ui::list::{ListRow, ThumbSize};
use craft_ui::primitives::textures::fit_rect;
use craft_ui::primitives::{ContentPainterCx, StrokeWidth};
use craft_ui::style::{Corners, SurfaceKind};
use egui::{Pos2, Rect, Shape, Stroke, vec2};

use super::PathRow;

/// The column that holds the thumbnail and the name.
pub(super) const NAME: ColumnKey = ColumnKey::new("name");

/// The row for `row`: a Medium thumbnail with its outline fitted to the `document`, and the name, italic for the
/// temporary kinds. The outline is the app's each pass, so nothing is cached.
pub(super) fn path_row<'r, K>(row: &'r PathRow<K>, document: kurbo::Size) -> ListRow<'r> {
    ListRow::new(row.name.clone())
        .italic(row.kind.italic())
        .thumb(ThumbSize::Medium, move |painter, rect| paint_outline(painter, rect, document, &row.outline))
}

/// Paints the document's page in `cell` at its aspect, and the outline on it.
fn paint_outline(painter: &ContentPainterCx<'_>, cell: Rect, document: kurbo::Size, outline: &kurbo::BezPath) {
    if !(document.width > 0.0 && document.height > 0.0) {
        return;
    }
    let page = fit_rect(cell, vec2(document.width as f32, document.height as f32));
    painter.with_draw(|draw| draw.surface(page, SurfaceKind::Well, Corners::None));
    let scale = f64::from(page.width()) / document.width;
    if scale <= 0.0 {
        return;
    }
    let at = |p: kurbo::Point| Pos2::new(page.min.x + (p.x * scale) as f32, page.min.y + (p.y * scale) as f32);
    let mut lines: Vec<Vec<Pos2>> = Vec::new();
    // Half a thumbnail pixel of flattening error, in document units.
    kurbo::flatten(outline.iter(), 0.5 / scale, |element| match element {
        kurbo::PathEl::MoveTo(p) => lines.push(vec![at(p)]),
        kurbo::PathEl::LineTo(p) => {
            if let Some(line) = lines.last_mut() {
                line.push(at(p));
            }
        }
        kurbo::PathEl::ClosePath => {
            if let Some(line) = lines.last_mut()
                && let Some(first) = line.first().copied()
            {
                line.push(first);
            }
        }
        _ => {}
    });
    let stroke = Stroke::new(painter.stroke_width(StrokeWidth::Hairline), painter.ink());
    for line in lines.into_iter().filter(|line| line.len() > 1) {
        painter.painter.add(Shape::line(line, stroke));
    }
}
