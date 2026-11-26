//! The ghost layer's content (8.4, 8.5): the running drag's preview beside the pointer, the mark of the drop target that
//! would take the payload (with the payload's name inside a region mark, for a drag that names it), and the painted
//! pointer the cursor service settled on, which goes to the window as its cursor image instead where the window takes
//! one. They come from the drag session's data and the cursor record; the layer takes no input. Value readouts beside
//! the pointer during a gesture paint here too, for the pass that asks.

use crate::primitives::pointer_art::Art;
use crate::primitives::pointer_art::pointer_art;
use crate::primitives::{Draw, DrawEnv, PaintRecord, PaintShape, RuleKind, Span, TextRun, outline};
use crate::runtime::drag::{self, DragPreview, DropMark};
use crate::runtime::egui::areas;
use crate::runtime::env::{Env, EnvView, Label};
use crate::runtime::{cursor, pointer};
use crate::style::{Corners, Ink, RuleWidth, SurfaceKind, UiTokens, rule_look};
use craft_behavior::Flow;
use egui::{Context, CornerRadius, Pos2, Rect, Vec2, vec2};
use std::cell::RefCell;

/// The end hook's painter of the ghost layer: pass it to `InstallSpec::frame_layers`. The drag preview paints in the
/// env of the region the drag started in and the drop mark in the tokens of the target's region; the pointer, which
/// belongs to no region, in the root env's.
pub fn paint(ctx: &Context) {
    let root = Env::of_root(ctx);
    let painter = painter(ctx);
    paint_pointer(ctx, &root, &painter);
    if let Some(session) = drag::ghost(ctx) {
        paint_drag(ctx, session.env.as_ref().unwrap_or(&root), &painter, &session);
    }
}

/// Paints rows of a label and a value beside the pointer at `at`, on the ghost layer, in `env`'s tokens, words and
/// direction: the env of the region that asks, for this pass only. It sits toward the lower end side of the pointer
/// (the lower right, mirrored under right-to-left), flips to the other side where the window has no room and stays
/// inside the window. Labels start at the reading start and values end at the reading end, the widest label and the
/// widest value a section gap apart. Labels are in the muted ink and values in the normal one. No rows paint nothing.
pub fn readout(ctx: &Context, env: &EnvView<'_>, at: Pos2, rows: &[(Label, String)]) {
    if rows.is_empty() {
        return;
    }
    let tokens = env.shared_tokens();
    let tokens = &*tokens;
    let direction = env.direction();
    let painter = painter(ctx);
    let record = RefCell::new(PaintRecord::default());
    let draw = Draw::new(&painter, &record, DrawEnv { tokens, direction, ink: Ink::OnPanel });
    let runs: Vec<(String, String)> = rows.iter().map(|(label, value)| (env.resolve(label).as_str().to_owned(), value.clone())).collect();
    let layouts: Vec<_> = runs
        .iter()
        .map(|(label, value)| {
            (draw.layout(&TextRun::spans(vec![Span::new(label.as_str()).muted()]), f32::INFINITY), draw.layout(&TextRun::new(value), f32::INFINITY))
        })
        .collect();
    let space = &tokens.space;
    let pad = vec2(space.gap.stack, space.gap.row);
    let label_width = layouts.iter().fold(0.0f32, |widest, (label, _)| widest.max(label.size().x));
    let value_width = layouts.iter().fold(0.0f32, |widest, (_, value)| widest.max(value.size().x));
    let width = 2.0 * pad.x + label_width + space.gap.section + value_width;
    let heights: Vec<f32> = layouts.iter().map(|(label, value)| label.size().y.max(value.size().y)).collect();
    let height = 2.0 * pad.y + heights.iter().sum::<f32>() + tokens.stroke.thin * (heights.len() - 1) as f32;
    let tip = tokens.size.pointer_tip;
    let rtl = direction.is_rtl();
    let outer = readout_place(at, vec2(tip.x, tip.y), vec2(width, height), rtl, ctx.content_rect());
    draw.surface(outer, SurfaceKind::Popup, Corners::All);
    let mut y = outer.min.y + pad.y;
    for ((label, value), row_height) in layouts.iter().zip(&heights) {
        let (label_x, value_x) = if rtl {
            (outer.max.x - pad.x - label.size().x, outer.min.x + pad.x)
        } else {
            (outer.min.x + pad.x, outer.max.x - pad.x - value.size().x)
        };
        draw.galley(Pos2::new(label_x, y), label);
        draw.galley(Pos2::new(value_x, y), value);
        y += row_height + tokens.stroke.thin;
    }
}

/// The readout's rect for a pointer at `at`: `offset` toward the reading end and down, on the other side of the pointer
/// where it would leave `window` across, and shifted to stay inside `window`.
fn readout_place(at: Pos2, offset: Vec2, size: Vec2, rtl: bool, window: Rect) -> Rect {
    let beside = |toward_end: bool| {
        let left = if toward_end != rtl { at.x + offset.x } else { at.x - offset.x - size.x };
        Rect::from_min_size(Pos2::new(left, at.y + offset.y), size)
    };
    let fits = |rect: Rect| rect.min.x >= window.min.x && rect.max.x <= window.max.x;
    let rect = if fits(beside(true)) || !fits(beside(false)) { beside(true) } else { beside(false) };
    let dx = (window.min.x - rect.min.x).max(0.0) - (rect.max.x - window.max.x).max(0.0);
    let dy = (window.min.y - rect.min.y).max(0.0) - (rect.max.y - window.max.y).max(0.0);
    rect.translate(vec2(dx, dy))
}

/// The painter of the ghost layer, over the whole screen and above every popup. The layer takes no input.
#[doc(hidden)]
pub fn painter(ctx: &Context) -> egui::Painter {
    areas::ghost_painter(ctx)
}

/// The painted pointer the cursor service settled on: the window's cursor image where the window takes one at its
/// size, else painted at the pointer. A brush tip too large for an image hands its centre cross and badge to the
/// window and paints only its ring, so the pointer itself still moves with the hardware. A badged system cursor keeps
/// the system's arrow and has only its badge painted. Where the shell cannot set the system cursor nothing is painted.
///
/// A tip outline requested within a region (`cursor::request_within`: a canvas's view on the canvas's layer) is a
/// mark on that canvas: the whole outline goes to the window only while it lies inside the region with no layer above
/// the region's over it. Otherwise its ring paints on the region's layer clipped to the region, under the panels,
/// menus and dialogs above it, and the window takes its marks as a too large tip's.
fn paint_pointer(ctx: &Context, env: &Env, painter: &egui::Painter) {
    use craft_behavior::{Cursor, PaintedCursor};
    let Some(at) = areas::pointer_pos(ctx) else { return };
    let Some(Cursor::Painted(cursor)) = cursor::current(ctx).filter(|_| cursor::paints(ctx)) else { return };
    let closed = pointer::frame_raw(ctx).buttons.contains(craft_behavior::Buttons::PRIMARY);
    let tip = matches!(cursor, PaintedCursor::Circle { .. } | PaintedCursor::Outline { .. });
    let whole = pointer_art(env, &cursor, closed, false);
    let confined = cursor::current_region(ctx).filter(|region| tip && !shows_whole(ctx, region, &whole, at));
    let ring = |painter: &egui::Painter| {
        let art = pointer_art(env, &cursor.clone().marked(false, None), closed, false);
        match confined {
            Some(region) => paint_in_region(ctx, region, &art, at),
            None => art.paint(painter, at),
        }
    };
    let target = cursor::image_target(ctx).filter(|_| !matches!(cursor, PaintedCursor::Badged { .. }));
    if let Some(target) = target {
        if confined.is_none() && cursor::show_image(ctx, &whole, target) {
            return;
        }
        if tip && cursor::show_image(ctx, &pointer_art(env, &cursor, closed, true), target) {
            ring(painter);
            return;
        }
    }
    match confined {
        Some(_) => {
            ring(painter);
            pointer_art(env, &cursor, closed, true).paint(painter, at);
        }
        None => whole.paint(painter, at),
    }
}

/// Whether `art` at the pointer `at` lies inside `region` with no layer above the region's layer over it.
fn shows_whole(ctx: &Context, region: &cursor::CursorRegion, art: &Art, at: Pos2) -> bool {
    let Some(bounds) = art.bounds().map(|bounds| bounds.translate(at.to_vec2())) else { return true };
    region.clip.contains_rect(bounds) && !areas::rects_over(ctx, region.layer, None).iter().any(|over| over.intersects(bounds))
}

/// Paints `art` at the pointer `at` (screen space) on `region`'s layer, clipped to the region.
fn paint_in_region(ctx: &Context, region: cursor::CursorRegion, art: &Art, at: Pos2) {
    let to_layer = areas::layer_to_screen(ctx, region.layer).inverse();
    let painter = egui::Painter::new(ctx.clone(), region.layer, to_layer * region.clip);
    art.paint(&painter, to_layer * at);
}

/// The drag preview, in `env`, the source's, and the drop mark of the running drag, with the payload's name in it.
fn paint_drag(ctx: &Context, env: &Env, painter: &egui::Painter, session: &drag::Ghost) {
    if let Some((mark, tokens)) = &session.mark {
        paint_mark(painter, tokens, *mark);
        if let (DropMark::Region { area, .. }, DragPreview::Named(name)) = (*mark, &session.preview) {
            paint_region_name(painter, tokens, env, area, name);
        }
    }
    let record = RefCell::new(PaintRecord::default());
    let draw = &Draw::new(painter, &record, DrawEnv { tokens: &env.tokens, direction: env.direction, ink: Ink::OnPanel });
    let Some(at) = pointer::frame_raw(ctx).pos else { return };
    match &session.preview {
        DragPreview::Badge(label) => {
            let label = env.view().resolve(label);
            paint_drag_badge(draw, env, at, label.as_str());
        }
        DragPreview::Frame { size, label } => {
            let frame = Rect::from_center_size(at, *size);
            painter.extend(outline(&env.tokens, &PaintShape::Rect(frame, CornerRadius::ZERO)));
            if let Some(label) = label {
                let label = env.view().resolve(label);
                // The label starts at the reading start of the frame's padded rect and wraps inside it.
                let inner = frame.shrink(env.tokens.space.pad.control);
                let layout = draw.layout(&TextRun::new(label.as_str()), inner.width());
                let (at, _) = Flow::new(inner, env.direction.is_rtl()).take_start(layout.size().x);
                draw.galley(at.min, &layout);
            }
        }
        _ => {}
    }
}

/// The payload's name inside the region `area` a drop would take: at the reading start of its top, inside the mark's
/// band, on one line elided to the area's width. A region too small for the line shows no name. It paints in the mark's
/// `tokens`, with the words and direction of `env`, the source's.
fn paint_region_name(painter: &egui::Painter, tokens: &UiTokens, env: &Env, area: Rect, name: &Label) {
    let name = env.view().resolve(name);
    let inner = area.shrink(tokens.space.pad.control);
    if inner.width() <= 0.0 {
        return;
    }
    let record = RefCell::new(PaintRecord::default());
    let draw = Draw::new(painter, &record, DrawEnv { tokens, direction: env.direction, ink: Ink::OnPanel });
    let layout = draw.layout(&TextRun::new(name.as_str()).elide(), inner.width());
    if layout.size().y > inner.height() {
        return;
    }
    let (at, _) = Flow::new(inner, env.direction.is_rtl()).take_start(layout.size().x.min(inner.width()));
    draw.galley(at.min, &layout);
}

fn paint_drag_badge(draw: &Draw<'_>, env: &Env, at: Pos2, text: &str) {
    let space = &env.tokens.space;
    let run = TextRun::new(text);
    let layout = draw.layout(&run, f32::INFINITY);
    let pad = Vec2::splat(space.pad.control);
    let outer = Rect::from_min_size(at + Vec2::splat(space.gap.inline), layout.size() + pad * 2.0);
    draw.surface(outer, SurfaceKind::Popup, Corners::All);
    draw.galley(outer.min + pad, &layout);
}

/// The drop mark: bands of the drop rule on a halo of the panel surface one thin stroke wide, so the mark reads over
/// any content. An area mark is the same band along the inside of its rect; a region is also washed with the rule's
/// colour, so the area the payload takes reads as one piece, and carries its line band.
fn paint_mark(painter: &egui::Painter, tokens: &UiTokens, mark: DropMark) {
    let look = rule_look(tokens, RuleKind::DropMark);
    let ppp = painter.ctx().pixels_per_point();
    let width = look.width.points(ppp);
    let halo = RuleWidth::Points(tokens.stroke.thin).points(ppp);
    let outline = |rect: Rect| {
        let inside = rect.shrink(width / 2.0 + halo);
        let corners = [inside.left_top(), inside.right_top(), inside.right_bottom(), inside.left_bottom()];
        (0..corners.len()).map(move |at| (corners[at], corners[(at + 1) % corners.len()]))
    };
    let segments: Vec<(Pos2, Pos2)> = match mark {
        DropMark::Line { from, to } => vec![(from, to)],
        DropMark::Into(rect) => outline(rect).collect(),
        DropMark::Region { area, line } => {
            painter.rect_filled(area, CornerRadius::ZERO, look.color.gamma_multiply(craft_theme::Opacity::WASH));
            outline(area).chain(line.map(|[from, to]| (from, to))).collect()
        }
    };
    let band = |(from, to): (Pos2, Pos2), half: f32| Rect::from_two_pos(from, to).expand(half);
    // Every halo before any band, so the halo of one side never covers the band of the side before it.
    let panel = crate::style::rgba_color(tokens.color.surface.panel);
    for &segment in &segments {
        painter.rect_filled(band(segment, width / 2.0 + halo), CornerRadius::ZERO, panel);
    }
    for &segment in &segments {
        painter.rect_filled(band(segment, width / 2.0), CornerRadius::ZERO, look.color);
    }
}
