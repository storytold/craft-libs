//! A painted pointer's art as data: marks in points around the hotspot, drawn either on the ghost painter or into the
//! image a window shows as its cursor. Both come from the one list, so the two look the same.

use crate::primitives::SvgIcon;
use crate::runtime::cursor::CursorBitmap;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use resvg::tiny_skia;
use std::sync::Arc;

/// How far past a mark's geometry its antialiasing reaches, in device pixels.
const FRINGE_PX: f32 = 1.0;
/// The share of a quarter circle's radius a cubic Bézier's control points lie along its tangents.
const ARC_HANDLE: f32 = 0.552_284_8;

/// One mark, in points from the hotspot.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Mark {
    /// A straight line with square-cut ends.
    Line(Vec2, Vec2, Stroke),
    /// A circle's outline.
    Ring(Vec2, f32, Stroke),
    /// A rectangle's outline, centred on its edges.
    Frame(Rect, Stroke),
    /// A filled circle.
    Disc(Vec2, f32, Color32),
    /// A filled rectangle with corners rounded by the radius.
    Block(Rect, f32, Color32),
    /// A filled convex polygon with its outline.
    Polygon(Vec<Vec2>, Color32, Stroke),
    /// A line through the points, closed when the flag is set.
    Path(Vec<Vec2>, bool, Stroke),
    /// A catalog icon in a colour, centred in a square.
    Icon(Rect, SvgIcon, Color32),
}

impl Mark {
    /// The rect the mark covers, its line included.
    fn bounds(&self) -> Rect {
        let around = |points: &[Vec2], stroke: &Stroke| Rect::from_points(&points.iter().map(|p| p.to_pos2()).collect::<Vec<_>>()).expand(stroke.width / 2.0);
        match self {
            Mark::Line(a, b, stroke) => around(&[*a, *b], stroke),
            Mark::Ring(centre, radius, stroke) => Rect::from_center_size(centre.to_pos2(), Vec2::splat(2.0 * radius + stroke.width)),
            Mark::Frame(rect, stroke) => rect.expand(stroke.width / 2.0),
            Mark::Disc(centre, radius, _) => Rect::from_center_size(centre.to_pos2(), Vec2::splat(2.0 * radius)),
            Mark::Block(rect, ..) | Mark::Icon(rect, ..) => *rect,
            Mark::Polygon(points, _, stroke) | Mark::Path(points, _, stroke) => around(points, stroke),
        }
    }
}

/// A pointer's marks, drawn in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Art {
    marks: Vec<Mark>,
}

impl Art {
    /// Adds `mark` above the marks so far.
    pub(crate) fn push(&mut self, mark: Mark) {
        self.marks.push(mark);
    }

    /// Paints the art with its hotspot at `at`.
    pub(crate) fn paint(&self, painter: &Painter, at: Pos2) {
        let path = |points: &[Vec2]| points.iter().map(|p| at + *p).collect::<Vec<_>>();
        for mark in &self.marks {
            match mark {
                Mark::Line(a, b, stroke) => {
                    painter.line_segment([at + *a, at + *b], *stroke);
                }
                Mark::Ring(centre, radius, stroke) => {
                    painter.circle_stroke(at + *centre, *radius, *stroke);
                }
                Mark::Frame(rect, stroke) => {
                    painter.rect_stroke(rect.translate(at.to_vec2()), 0.0, *stroke, egui::StrokeKind::Middle);
                }
                Mark::Disc(centre, radius, color) => {
                    painter.circle_filled(at + *centre, *radius, *color);
                }
                Mark::Block(rect, round, color) => {
                    painter.rect_filled(rect.translate(at.to_vec2()), *round, *color);
                }
                Mark::Polygon(points, fill, stroke) => {
                    painter.add(Shape::convex_polygon(path(points), *fill, *stroke));
                }
                Mark::Path(points, closed, stroke) => {
                    painter.add(if *closed { Shape::closed_line(path(points), *stroke) } else { Shape::line(path(points), *stroke) });
                }
                Mark::Icon(rect, icon, color) => icon.paint(painter, rect.translate(at.to_vec2()), *color),
            }
        }
    }

    /// The art as a cursor image at `scale` device pixels per point, its hotspot on the centre pixel of an odd square;
    /// `None` for art with no marks, with a size that is not finite, or wider than `max_side` pixels. The size is
    /// checked before anything is allocated.
    pub(crate) fn rasterise(&self, scale: f32, max_side: u16) -> Option<CursorBitmap> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let bounds = self.marks.iter().map(Mark::bounds).reduce(|a, b| a.union(b))?;
        let reach = [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y].into_iter().map(f32::abs).fold(0.0, f32::max);
        let half = (reach * scale + FRINGE_PX).ceil();
        if !half.is_finite() || half > f32::from(max_side.saturating_sub(1) / 2) {
            return None;
        }
        let hot = half as u16;
        let side = hot * 2 + 1;
        let mut pixmap = tiny_skia::Pixmap::new(u32::from(side), u32::from(side))?;
        // Point (0, 0) lands on the centre of the hotspot pixel.
        let origin = f32::from(hot) + 0.5;
        let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, origin, origin);
        for mark in &self.marks {
            draw(&mut pixmap, mark, transform, scale, origin);
        }
        let rgba: Vec<u8> = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        Some(CursorBitmap { rgba: Arc::from(rgba), size: [side, side], hotspot: [hot, hot] })
    }
}

fn paint_of(color: Color32) -> tiny_skia::Paint<'static> {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let mut paint = tiny_skia::Paint { anti_alias: true, ..tiny_skia::Paint::default() };
    paint.set_color_rgba8(r, g, b, a);
    paint
}

fn stroke_path(pixmap: &mut tiny_skia::Pixmap, path: Option<tiny_skia::Path>, stroke: Stroke, transform: tiny_skia::Transform) {
    if let Some(path) = path
        && stroke.width > 0.0
        && stroke.color.a() > 0
    {
        let line = tiny_skia::Stroke { width: stroke.width, ..tiny_skia::Stroke::default() };
        pixmap.stroke_path(&path, &paint_of(stroke.color), &line, transform, None);
    }
}

fn fill_path(pixmap: &mut tiny_skia::Pixmap, path: Option<tiny_skia::Path>, color: Color32, transform: tiny_skia::Transform) {
    if let Some(path) = path
        && color.a() > 0
    {
        pixmap.fill_path(&path, &paint_of(color), tiny_skia::FillRule::Winding, transform, None);
    }
}

fn polyline(points: &[Vec2], closed: bool) -> Option<tiny_skia::Path> {
    let (first, rest) = points.split_first()?;
    let mut builder = tiny_skia::PathBuilder::new();
    builder.move_to(first.x, first.y);
    for p in rest {
        builder.line_to(p.x, p.y);
    }
    if closed {
        builder.close();
    }
    builder.finish()
}

fn rect_path(rect: Rect) -> Option<tiny_skia::Path> {
    tiny_skia::Rect::from_ltrb(rect.min.x, rect.min.y, rect.max.x, rect.max.y).map(tiny_skia::PathBuilder::from_rect)
}

/// A rect with corners rounded by `round` (at most half its shorter side), as egui rounds a filled rect.
fn rounded_path(rect: Rect, round: f32) -> Option<tiny_skia::Path> {
    let r = round.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    if r <= 0.0 {
        return rect_path(rect);
    }
    let k = r * (1.0 - ARC_HANDLE);
    let (l, t, rr, b) = (rect.min.x, rect.min.y, rect.max.x, rect.max.y);
    let mut builder = tiny_skia::PathBuilder::new();
    builder.move_to(l + r, t);
    builder.line_to(rr - r, t);
    builder.cubic_to(rr - k, t, rr, t + k, rr, t + r);
    builder.line_to(rr, b - r);
    builder.cubic_to(rr, b - k, rr - k, b, rr - r, b);
    builder.line_to(l + r, b);
    builder.cubic_to(l + k, b, l, b - k, l, b - r);
    builder.line_to(l, t + r);
    builder.cubic_to(l, t + k, l + k, t, l + r, t);
    builder.close();
    builder.finish()
}

fn draw(pixmap: &mut tiny_skia::Pixmap, mark: &Mark, transform: tiny_skia::Transform, scale: f32, origin: f32) {
    match mark {
        Mark::Line(a, b, stroke) => stroke_path(pixmap, polyline(&[*a, *b], false), *stroke, transform),
        Mark::Ring(centre, radius, stroke) => stroke_path(pixmap, tiny_skia::PathBuilder::from_circle(centre.x, centre.y, *radius), *stroke, transform),
        Mark::Frame(rect, stroke) => stroke_path(pixmap, rect_path(*rect), *stroke, transform),
        Mark::Disc(centre, radius, color) => fill_path(pixmap, tiny_skia::PathBuilder::from_circle(centre.x, centre.y, *radius), *color, transform),
        Mark::Block(rect, round, color) => fill_path(pixmap, rounded_path(*rect, *round), *color, transform),
        Mark::Polygon(points, fill, stroke) => {
            fill_path(pixmap, polyline(points, true), *fill, transform);
            stroke_path(pixmap, polyline(points, true), *stroke, transform);
        }
        Mark::Path(points, closed, stroke) => stroke_path(pixmap, polyline(points, *closed), *stroke, transform),
        Mark::Icon(rect, icon, color) => {
            let side = rect.width().min(rect.height());
            let mask = icon.mask(side, scale);
            let [w, h] = mask.size;
            // On whole pixels, centred as the painted icon is.
            let left = (origin + rect.center().x * scale - w as f32 / 2.0).round() as i64;
            let top = (origin + rect.center().y * scale - h as f32 / 2.0).round() as i64;
            composite(pixmap, &mask, left, top, *color);
        }
    }
}

/// `mask`'s alpha in `color` over the pixmap, its top left at pixel (`left`, `top`).
fn composite(pixmap: &mut tiny_skia::Pixmap, mask: &egui::ColorImage, left: i64, top: i64, color: Color32) {
    let (width, height) = (i64::from(pixmap.width()), i64::from(pixmap.height()));
    let [mw, mh] = mask.size;
    let pixels = pixmap.pixels_mut();
    let src = color.to_array();
    for my in 0..mh {
        let y = top + my as i64;
        if !(0..height).contains(&y) {
            continue;
        }
        for mx in 0..mw {
            let x = left + mx as i64;
            let cover = u32::from(mask.pixels[my * mw + mx].a());
            if cover == 0 || !(0..width).contains(&x) {
                continue;
            }
            let [r, g, b, a] = src.map(|c| (u32::from(c) * cover + 127) / 255);
            let dst = &mut pixels[(y * width + x) as usize];
            let keep = 255 - a;
            let over = |s: u32, d: u8| (s + (u32::from(d) * keep + 127) / 255).min(255) as u8;
            let alpha = over(a, dst.alpha());
            let channel = |s: u32, d: u8| over(s, d).min(alpha);
            *dst = tiny_skia::PremultipliedColorU8::from_rgba(channel(r, dst.red()), channel(g, dst.green()), channel(b, dst.blue()), alpha)
                .unwrap_or(*dst);
        }
    }
}
