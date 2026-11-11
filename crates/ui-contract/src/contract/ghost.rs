//! The ghost layer's content (8.4, 8.5): the running drag's preview beside the pointer, the mark of the drop target that
//! would take the payload, and the painted pointer the cursor service settled on. They come from the drag session's
//! data and the cursor record; the layer takes no input.

mod pointers;

use crate::primitives::{Draw, DrawEnv, PaintRecord, PaintShape, RuleKind, TextRun, outline};
use crate::runtime::drag::{self, DragPreview, DropMark};
use crate::runtime::egui::areas;
use crate::runtime::env::Env;
use crate::runtime::{cursor, pointer};
use crate::style::{Corners, Ink, SurfaceKind};
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

/// The painted pointer the cursor service settled on. The hand shows its fist while the primary button is held.
fn paint_pointer(ctx: &Context, env: &Env, painter: &egui::Painter) {
    use craft_behavior::{Cursor, PaintedCursor};
    let Some(at) = areas::pointer_pos(ctx) else { return };
    let Some(Cursor::Painted(cursor)) = cursor::current(ctx) else { return };
    let overlay = &env.tokens.color.overlay;
    let pen = pointers::Pen {
        painter,
        at,
        fill: crate::style::rgba_color(overlay.stroke),
        halo: crate::style::rgba_color(overlay.halo),
        thin: env.tokens.stroke.thin,
    };
    let ink = crate::style::ink_color(&env.tokens, Ink::OnDark);
    let edge = crate::style::rule_look(&env.tokens, RuleKind::Edge).width;
    match &cursor {
        PaintedCursor::Outline { rings, .. } => pointers::outline(&pen, rings),
        PaintedCursor::Circle { radius, .. } => pointers::circle(&pen, *radius, edge, ink),
        PaintedCursor::Crosshair => pointers::crosshair(&pen, env.tokens.size.icon.sm, edge, ink),
        PaintedCursor::Hand => pointers::hand(&pen, pointer::frame(ctx).buttons.contains(craft_behavior::Buttons::PRIMARY)),
        PaintedCursor::Zoom { out, .. } => pointers::zoom(&pen, *out),
        PaintedCursor::Tool { icon, hotspot, badge, .. } => {
            let record = RefCell::new(PaintRecord::default());
            let on_dark = Draw::new(painter, &record, DrawEnv { tokens: &env.tokens, direction: env.direction, ink: Ink::OnDark });
            pointers::tool(&pen, &on_dark, icon, *hotspot, *badge, env.tokens.size.icon.md);
        }
        PaintedCursor::Badged { badge, .. } => pointers::badge_beside(&pen, *badge),
        PaintedCursor::Turn { toward, .. } => pointers::turn(&pen, *toward),
        PaintedCursor::Distort { dir, .. } => pointers::distort(&pen, *dir),
        PaintedCursor::Skew { dir, .. } => pointers::skew(&pen, *dir),
        // `Hidden` paints nothing.
        _ => {}
    }
}

/// The drag preview and the drop mark of the running drag.
fn paint_drag(ctx: &Context, env: &Env, painter: &egui::Painter, draw: &Draw<'_>, session: &drag::Ghost) {
    if let Some(mark) = session.mark {
        paint_mark(draw, mark);
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

fn paint_mark(draw: &Draw<'_>, mark: DropMark) {
    match mark {
        DropMark::Line { from, to } => draw.rule(RuleKind::DropMark, from, to),
        DropMark::Into(rect) | DropMark::Split(rect) => {
            let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
            for (at, from) in corners.iter().enumerate() {
                draw.rule(RuleKind::DropMark, *from, corners[(at + 1) % corners.len()]);
            }
        }
    }
}
