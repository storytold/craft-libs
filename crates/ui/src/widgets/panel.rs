//! Panel chrome helpers: accent drop lines on a dragged-over edge, collapsible section headers,
//! and a group header's painted anchor positions. All of them name sides in reading order
//! through [`crate::direction`]: a vertical drop line's `after` is the right edge left-to-right
//! and the left edge right-to-left; horizontal lines are never mirrored (rows stack top-down in
//! both directions). A section header's chevron and title lead from the row's reading-start edge.

use egui::{Align2, Id, InnerResponse, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::direction::{self, HasDirection, is_rtl, start_align2, start_x};

/// An accent insertion line on one edge of `r` while a drag hovers it. Vertical lines stand on
/// `r`'s left or right edge, where `after` is in reading order (the right edge left-to-right, the
/// left edge right-to-left); horizontal lines are never mirrored — rows stack top-down in both
/// directions — so there `after` keeps its physical meaning (bottom, else top).
pub fn drop_line(ui: &Ui, d: &impl HasDirection, r: Rect, after: bool, vertical: bool, stroke: Stroke) {
    if vertical {
        let after = after != is_rtl(d);
        let x = if after { r.right() } else { r.left() };
        ui.painter().line_segment([pos2(x, r.top() + 2.0), pos2(x, r.bottom() - 2.0)], stroke);
    } else {
        let y = if after { r.bottom() } else { r.top() };
        ui.painter().line_segment([pos2(r.left() + 4.0, y), pos2(r.right() - 4.0, y)], stroke);
    }
}

/// A group header's painted anchors at `indent` from the row's reading-start edge: chevron,
/// folder icon, name and count x, and the name's alignment.
#[derive(Clone, Copy, Debug)]
pub struct HeaderAnchors {
    pub chevron_x: f32,
    pub folder_x: f32,
    pub name_x: f32,
    pub count_x: f32,
    pub halign: Align2,
}

/// [`HeaderAnchors`] for a header row `r` indented `indent` from its reading-start edge.
pub fn header_anchors(d: &impl HasDirection, r: Rect, indent: f32) -> HeaderAnchors {
    let sx = start_x(d, r) + direction::forward(d) * indent;
    let rtl = is_rtl(d);
    HeaderAnchors {
        chevron_x: if rtl { sx - 14.0 } else { sx },
        folder_x: if rtl { sx - 34.0 } else { sx + 18.0 },
        name_x: if rtl { sx - 40.0 } else { sx + 40.0 },
        count_x: direction::end_x(d, r) - direction::forward(d) * 8.0,
        halign: start_align2(d),
    }
}

/// The chevron's rect of a [`section_header`] row: 12×12, 7 pt in from the reading-start edge.
pub fn section_chevron_rect(d: &impl HasDirection, r: Rect) -> Rect {
    Rect::from_center_size(pos2(start_x(d, r) + direction::forward(d) * 7.0, r.center().y), vec2(12.0, 12.0))
}

/// The x a [`section_header`]'s title anchors at: 18 pt in from the reading-start edge.
pub fn section_title_anchor(d: &impl HasDirection, r: Rect) -> (f32, Align2) {
    (start_x(d, r) + direction::forward(d) * 18.0, start_align2(d))
}

/// A collapsible section header: a clickable full-width row with a chevron and the title leading
/// at the reading start (the right edge in a right-to-left interface). Returns the row's response
/// and whether the section is open. `id` keys the open state. `paint_chevron` draws the chevron
/// (down when open, else towards the reading start) into the chevron rect.
pub fn section_header(
    ui: &mut Ui,
    d: &impl HasDirection,
    id: &str,
    height: f32,
    paint_chevron: impl FnOnce(&Ui, Rect, bool),
    paint_title: impl FnOnce(&Ui, Rect, f32, Align2),
) -> InnerResponse<bool> {
    let key = ui.scope_id().with(("section-header", id));
    let mut open = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(true);
    let width = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_chevron(ui, section_chevron_rect(d, r), open);
    let (x, anchor) = section_title_anchor(d, r);
    paint_title(ui, r, x, anchor);
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, ui.is_enabled(), open, ""));
    InnerResponse::new(open, resp)
}

/// The id a [`section_header`] keeps its open state under, for tests and callers that reuse it.
pub fn section_header_key(ui: &Ui, id: &str) -> Id {
    ui.scope_id().with(("section-header", id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use egui::{Color32, Pos2, RawInput};

    fn drop_line_start(dir: Direction, after: bool, vertical: bool) -> egui::Pos2 {
        let ctx = egui::Context::default();
        direction::apply(&ctx, dir);
        let r = Rect::from_min_size(pos2(100.0, 50.0), vec2(80.0, 24.0));
        let mut p = egui::Pos2::new(f32::NAN, f32::NAN);
        let mut out = ctx.run_ui(RawInput::default(), |ui| {
            drop_line(ui, &dir, r, after, vertical, Stroke::new(2.0, Color32::RED));
        });
        for shape in &out.shapes {
            if let egui::Shape::LineSegment { points, .. } = &shape.shape {
                p = points[0];
            }
        }
        out.textures_delta.clear();
        p
    }

    /// A vertical drop line's `after` is in reading order: the right edge left-to-right, the
    /// left edge right-to-left; a horizontal line never mirrors.
    #[test]
    fn drop_line_after_is_in_reading_order() {
        assert!((drop_line_start(Direction::Ltr, true, true).x - 180.0).abs() < 1e-4);
        assert!((drop_line_start(Direction::Ltr, false, true).x - 100.0).abs() < 1e-4);
        assert!((drop_line_start(Direction::Rtl, true, true).x - 100.0).abs() < 1e-4, "rtl: after = the left edge");
        assert!((drop_line_start(Direction::Rtl, false, true).x - 180.0).abs() < 1e-4, "rtl: before = the right edge");
        // Horizontal lines stack top-down in both directions.
        assert!((drop_line_start(Direction::Ltr, true, false).y - 74.0).abs() < 1e-4);
        assert!((drop_line_start(Direction::Rtl, true, false).y - 74.0).abs() < 1e-4);
        assert!((drop_line_start(Direction::Rtl, false, false).y - 50.0).abs() < 1e-4);
    }

    /// `header_anchors` reproduces the hand-mirrored group-header table in both directions.
    #[test]
    fn header_anchors_lead_from_the_reading_start() {
        let r = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, 24.0));
        let a = header_anchors(&Direction::Ltr, r, 4.0);
        assert!(
            (a.chevron_x - 104.0).abs() < 1e-4
                && (a.folder_x - 122.0).abs() < 1e-4
                && (a.name_x - 144.0).abs() < 1e-4
                && (a.count_x - 292.0).abs() < 1e-4
                && a.halign == Align2::LEFT_CENTER,
            "ltr: {a:?}"
        );
        let a = header_anchors(&Direction::Rtl, r, 4.0);
        assert!(
            (a.chevron_x - 282.0).abs() < 1e-4
                && (a.folder_x - 262.0).abs() < 1e-4
                && (a.name_x - 256.0).abs() < 1e-4
                && (a.count_x - 108.0).abs() < 1e-4
                && a.halign == Align2::RIGHT_CENTER,
            "rtl: {a:?}"
        );
    }

    /// Draw a `section_header` in `dir`; return its row rect and the title galley's rect.
    fn section_header_rects(dir: Direction) -> (Rect, Rect) {
        let ctx = egui::Context::default();
        direction::apply(&ctx, dir);
        let mut row = Rect::NOTHING;
        let mut out = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() }, |ui| {
            row = section_header(
                ui,
                &dir,
                "sec",
                24.0,
                |ui, r, _open| {
                    ui.painter().rect_filled(r, 2.0, Color32::GRAY);
                },
                |ui, row, x, anchor| {
                    ui.painter().text(pos2(x, row.center().y), anchor, "Brushes", egui::FontId::default(), Color32::WHITE);
                },
            )
            .response
            .rect;
        });
        let mut title = Rect::NOTHING;
        for shape in &out.shapes {
            if let egui::Shape::Text(ts) = &shape.shape
                && ts.galley.text().contains("Brushes")
            {
                title = shape.shape.visual_bounding_rect();
            }
        }
        out.textures_delta.clear();
        (row, title)
    }

    /// A section header's title leads at the reading start: 18 pt from the left edge
    /// left-to-right, hugging the right edge right-to-left.
    #[test]
    fn section_header_title_hugs_the_reading_start() {
        let (lrow, ltitle) = section_header_rects(Direction::Ltr);
        assert!(ltitle.is_positive() && (ltitle.left() - (lrow.left() + 18.0)).abs() < 2.0, "ltr: {ltitle:?} in {lrow:?}");
        let (rrow, rtitle) = section_header_rects(Direction::Rtl);
        assert!(rtitle.is_positive() && (rrow.right() - 18.0 - rtitle.right()).abs() < 2.0, "rtl: {rtitle:?} in {rrow:?}");
    }
}
