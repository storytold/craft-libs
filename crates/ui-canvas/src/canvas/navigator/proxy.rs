//! The Navigator's geometry: where the document sits in the preview box, the proxy of the view, and the view a click
//! or a Cmd/Ctrl-drag asks for. Pure functions, for tests and for apps that draw a preview of their own (an
//! artboards-only preview).

use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{CanvasView, ZoomLadder};
use egui::{Pos2, Rect, pos2, vec2};

/// A zoom rectangle under this many document units on a side steps one rung in instead.
const MIN_ZOOM_BOX: f64 = 2.0;

/// The least room the document gets inside the inset, in points on a side.
const LEAST_INNER: f32 = 1.0;

/// The document of `size` (document units) fitted inside `frame` with `inset` points on every side, centred and
/// keeping its aspect: the rect the preview picture is drawn into.
///
/// The space left after the inset is at least 1 by 1 point; an empty size counts as 1 by 1.
pub fn preview_rect(frame: Rect, size: Size, inset: f32) -> Rect {
    let room = frame.size() - vec2(2.0 * inset, 2.0 * inset);
    let (inner_x, inner_y) = (room.x.max(LEAST_INNER), room.y.max(LEAST_INNER));
    let (width, height) = (size.width.max(1.0) as f32, size.height.max(1.0) as f32);
    let scale = (inner_x / width).min(inner_y / height);
    Rect::from_center_size(frame.center(), vec2(width * scale, height * scale))
}

/// The proxy of `view` in the preview: the canvas rectangle `view.rect` mapped into document space and from there into
/// `preview` (the rect from [`preview_rect`] for `size`), as four corners in the canvas's order (top-left, top-right,
/// bottom-right, bottom-left on screen).
///
/// On an upright, unmirrored view this is the canvas rectangle's size over the zoom, divided by the pixel aspect
/// horizontally, times the preview scale, centred on the preview point of `view.center`. On a rotated or flipped view
/// it is the quad [`CanvasView::to_doc`] gives for the canvas corners, mapped the same way. The quad is not clipped.
pub fn proxy_quad(view: &CanvasView, size: Size, preview: Rect) -> [Pos2; 4] {
    let scale = preview_scale(preview, size);
    let screen = view.rect;
    [screen.left_top(), screen.right_top(), screen.right_bottom(), screen.left_bottom()].map(|corner| {
        let doc = view.to_doc(corner);
        pos2(preview.min.x + doc.x as f32 * scale, preview.min.y + doc.y as f32 * scale)
    })
}

/// Zooms `view` so the document rectangle `doc_rect` (document units) fills the canvas `view.rect`, centred on it, as a
/// Navigator Cmd/Ctrl-drag does. Returns whether the view changed.
///
/// Rules, in order:
/// - Any edge of `doc_rect` that is not finite: nothing happens. This is checked before clipping, since `min` and `max`
///   would turn a NaN edge into a document edge.
/// - `doc_rect` is intersected with the document `(0, 0)..size`; a negative width or height after that (a rectangle
///   off the document) does nothing.
/// - Under 2 document units on a side, or an empty canvas: one ladder rung in ([`ZoomLadder::step`] with `dir` 1).
/// - Otherwise the point zoom is `min(canvas.w / (r.w * aspect), canvas.h / r.h)`, converted to the ladder's device
///   zoom with `pixels_per_point` and clamped with [`ZoomLadder::clamp`]; a zoom that is not finite does nothing.
/// - The centre becomes the clipped rectangle's centre.
pub fn zoom_to_document_rect(view: &mut CanvasView, doc_rect: DocRect, size: Size, pixels_per_point: f32) -> bool {
    let edges = [doc_rect.x0, doc_rect.y0, doc_rect.x1, doc_rect.y1];
    if !edges.iter().all(|edge| edge.is_finite()) {
        return false;
    }
    // Not `kurbo::Rect::intersect`, which clamps a rectangle off the document to zero size.
    let (x0, y0) = (doc_rect.x0.max(0.0), doc_rect.y0.max(0.0));
    let (x1, y1) = (doc_rect.x1.min(size.width), doc_rect.y1.min(size.height));
    let (width, height) = (x1 - x0, y1 - y0);
    if width < 0.0 || height < 0.0 {
        return false;
    }
    let aspect = if view.aspect.is_finite() && view.aspect > 0.0 { view.aspect } else { 1.0 };
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let canvas = view.rect.size();
    let device = if width < MIN_ZOOM_BOX || height < MIN_ZOOM_BOX || canvas.x <= 0.0 || canvas.y <= 0.0 {
        ZoomLadder::step(zoom::device(view, ppp), 1, size)
    } else {
        let point_zoom = (f64::from(canvas.x) / (width * aspect)).min(f64::from(canvas.y) / height);
        point_zoom * f64::from(ppp)
    };
    if !device.is_finite() {
        return false;
    }
    let before = *view;
    view.zoom = CanvasView::points_per_unit(ZoomLadder::clamp(device, size), ppp);
    view.center = Point::new(x0 + width / 2.0, y0 + height / 2.0);
    *view != before
}

/// The document point under `at` in a preview drawn into `preview` for a document of `size`, clamped to the document
/// `(0, 0)..=size`: the view centre a click or a drag at `at` asks for.
pub fn view_center_for(preview: Rect, size: Size, at: Pos2) -> Point {
    let scale = f64::from(preview_scale(preview, size));
    Point::new((f64::from(at.x - preview.min.x) / scale).clamp(0.0, size.width), (f64::from(at.y - preview.min.y) / scale).clamp(0.0, size.height))
}

/// Preview points per document unit.
fn preview_scale(preview: Rect, size: Size) -> f32 {
    preview.width() / size.width.max(1.0) as f32
}
