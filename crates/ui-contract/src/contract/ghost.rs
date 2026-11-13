//! The ghost layer's content (8.4, 8.5): the running drag's preview beside the pointer, the mark of the drop target that
//! would take the payload, and the painted pointer the cursor service settled on, which goes to the window as its
//! cursor image instead where the window takes one. They come from the drag session's data and the cursor record; the
//! layer takes no input.

mod art;
mod pointers;

pub(crate) use art::Art;

use crate::primitives::{Draw, DrawEnv, IconSet, Mirror, PaintRecord, PaintShape, RuleKind, SvgIcon, TextRun, outline};
use crate::runtime::drag::{self, DragPreview, DropMark};
use crate::runtime::egui::areas;
use crate::runtime::env::Env;
use crate::runtime::{cursor, pointer};
use crate::style::{Corners, Ink, RuleWidth, SurfaceKind, rule_look};
use craft_behavior::Flow;
use egui::{Context, CornerRadius, Pos2, Rect, Vec2};
use std::cell::RefCell;

/// The end hook's painter of the ghost layer: pass it to `InstallSpec::frame_layers`.
pub fn paint(ctx: &Context) {
    let env = Env::of_root(ctx);
    let painter = painter(ctx);
    let record = RefCell::new(PaintRecord::default());
    let draw = Draw::new(&painter, &record, DrawEnv { tokens: &env.tokens, direction: env.direction, ink: Ink::OnPanel });
    paint_pointer(ctx, &env, &painter);
    if let Some(session) = drag::ghost(ctx) {
        paint_drag(ctx, &env, &painter, &draw, &session);
    }
}

/// The painter of the ghost layer, over the whole screen and above every popup. The layer takes no input.
#[doc(hidden)]
pub fn painter(ctx: &Context) -> egui::Painter {
    areas::ghost_painter(ctx)
}

/// The painted pointer the cursor service settled on: the window's cursor image where the window takes one at its
/// size, else painted at the pointer. A brush tip too large for an image hands its centre cross and badge to the
/// window and paints only its ring, so the pointer itself still moves with the hardware. A badged system cursor keeps
/// the system's arrow and has only its badge painted.
fn paint_pointer(ctx: &Context, env: &Env, painter: &egui::Painter) {
    use craft_behavior::{Cursor, PaintedCursor};
    let Some(at) = areas::pointer_pos(ctx) else { return };
    let Some(Cursor::Painted(cursor)) = cursor::current(ctx) else { return };
    let closed = pointer::frame(ctx).buttons.contains(craft_behavior::Buttons::PRIMARY);
    let target = cursor::image_target(ctx).filter(|_| !matches!(cursor, PaintedCursor::Badged { .. }));
    if let Some(target) = target {
        if cursor::show_image(ctx, &pointer_art(env, &cursor, closed, false), target) {
            return;
        }
        if matches!(cursor, PaintedCursor::Circle { .. } | PaintedCursor::Outline { .. })
            && cursor::show_image(ctx, &pointer_art(env, &cursor, closed, true), target)
        {
            pointer_art(env, &cursor.marked(false, None), closed, false).paint(painter, at);
            return;
        }
    }
    pointer_art(env, &cursor, closed, false).paint(painter, at);
}

/// The art of `cursor`: the hand as a fist while `closed`; with `marks_only`, a brush tip's centre cross and badge
/// without its ring.
fn pointer_art(env: &Env, cursor: &craft_behavior::PaintedCursor, closed: bool, marks_only: bool) -> Art {
    use craft_behavior::PaintedCursor;
    let overlay = &env.tokens.color.overlay;
    let pen = pointers::Pen {
        art: RefCell::new(Art::default()),
        fill: crate::style::rgba_color(overlay.stroke),
        halo: crate::style::rgba_color(overlay.halo),
        thin: env.tokens.stroke.thin,
    };
    let ink = crate::style::ink_color(&env.tokens, Ink::OnDark);
    let edge = env.tokens.stroke.thin;
    match cursor {
        PaintedCursor::Outline { rings, centre, badge, .. } => {
            if !marks_only {
                pointers::outline(&pen, rings);
            }
            pointers::tip_marks(&pen, *centre || marks_only, *badge, pen.thin, pen.fill);
        }
        PaintedCursor::Circle { radius, centre, badge, .. } => {
            if !marks_only {
                pointers::circle(&pen, *radius, edge, ink);
            }
            pointers::tip_marks(&pen, *centre || marks_only, *badge, edge, ink);
        }
        PaintedCursor::Crosshair => pointers::crosshair(&pen, env.tokens.size.icon.sm, edge, ink),
        PaintedCursor::Hand => pointers::hand(&pen, closed),
        PaintedCursor::Zoom { out, .. } => pointers::zoom(&pen, *out),
        PaintedCursor::Tool { icon, hotspot, badge, .. } => {
            let side = env.tokens.size.icon.md;
            let icon = SvgIcon::of(&env.tokens, env.direction, side, icon, IconSet::default(), Mirror::Auto);
            pointers::tool(&pen, icon, *hotspot, *badge, side);
        }
        PaintedCursor::Badged { badge, .. } => pointers::badge_beside(&pen, *badge),
        PaintedCursor::Turn { toward, .. } => pointers::turn(&pen, *toward),
        PaintedCursor::Distort { dir, .. } => pointers::distort(&pen, *dir),
        PaintedCursor::Skew { dir, .. } => pointers::skew(&pen, *dir),
        // `Hidden` has no marks.
        _ => {}
    }
    pen.art.into_inner()
}

/// The drag preview and the drop mark of the running drag.
fn paint_drag(ctx: &Context, env: &Env, painter: &egui::Painter, draw: &Draw<'_>, session: &drag::Ghost) {
    if let Some(mark) = session.mark {
        paint_mark(painter, env, mark);
    }
    let Some(at) = pointer::frame(ctx).pos else { return };
    match &session.preview {
        DragPreview::Badge(label) => {
            let label = env.view().resolve(label);
            paint_badge(draw, env, at, label.as_str());
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

fn paint_badge(draw: &Draw<'_>, env: &Env, at: Pos2, text: &str) {
    let space = &env.tokens.space;
    let run = TextRun::new(text);
    let layout = draw.layout(&run, f32::INFINITY);
    let pad = Vec2::splat(space.pad.control);
    let outer = Rect::from_min_size(at + Vec2::splat(space.gap.inline), layout.size() + pad * 2.0);
    draw.surface(outer, SurfaceKind::Popup, Corners::All);
    draw.galley(outer.min + pad, &layout);
}

/// The drop mark: bands of the drop rule on a halo of the panel surface one thin stroke wide, so the mark reads over
/// any content. An area mark is the same band along the inside of its rect.
fn paint_mark(painter: &egui::Painter, env: &Env, mark: DropMark) {
    let look = rule_look(&env.tokens, RuleKind::DropMark);
    let ppp = painter.ctx().pixels_per_point();
    let width = look.width.points(ppp);
    let halo = RuleWidth::Points(env.tokens.stroke.thin).points(ppp);
    let segments: Vec<(Pos2, Pos2)> = match mark {
        DropMark::Line { from, to } => vec![(from, to)],
        DropMark::Into(rect) | DropMark::Split(rect) => {
            let inside = rect.shrink(width / 2.0 + halo);
            let corners = [inside.left_top(), inside.right_top(), inside.right_bottom(), inside.left_bottom()];
            (0..corners.len()).map(|at| (corners[at], corners[(at + 1) % corners.len()])).collect()
        }
    };
    let band = |(from, to): (Pos2, Pos2), half: f32| Rect::from_two_pos(from, to).expand(half);
    // Every halo before any band, so the halo of one side never covers the band of the side before it.
    let panel = crate::style::rgba_color(env.tokens.color.surface.panel);
    for &segment in &segments {
        painter.rect_filled(band(segment, width / 2.0 + halo), CornerRadius::ZERO, panel);
    }
    for &segment in &segments {
        painter.rect_filled(band(segment, width / 2.0), CornerRadius::ZERO, look.color);
    }
}
