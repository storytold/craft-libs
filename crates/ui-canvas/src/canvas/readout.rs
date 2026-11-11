//! The readout: values beside the pointer during a canvas gesture ("W 120 px", "Angle 15 deg").

use crate::contract::ghost;
use crate::primitives::{Draw, DrawEnv, PaintRecord, Span, TextRun};
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::StageAccess;
use crate::style::{Corners, Ink, SurfaceKind};
use craft_i18n::Direction;
use egui::{Context, Pos2, Rect, Vec2, vec2};
use std::cell::RefCell;

/// Paints rows of a label and a value beside the pointer at `at`, on the ghost layer above every popup, for this
/// pass only: a gesture calls it each pass it runs. The readout takes no input. It sits toward the lower end side
/// of the pointer (the lower right, mirrored under right-to-left), flips to the other side where the window has no
/// room and stays inside the window. Labels start at the reading start and values end at the reading end, the widest
/// label and the widest value a section gap apart. Labels are in the muted ink and values in the normal one. No rows
/// paint nothing.
pub fn readout(cx: &impl StageAccess, at: Pos2, rows: &[(Label, String)]) {
    paint_readout(cx.stage().ctx(), at, rows);
}

/// [`readout`] for a caller that has the context, such as the tool host.
pub(super) fn paint_readout(ctx: &Context, at: Pos2, rows: &[(Label, String)]) {
    if rows.is_empty() {
        return;
    }
    let env = Env::of_root(ctx);
    let painter = ghost::painter(ctx);
    let record = RefCell::new(PaintRecord::default());
    let draw = Draw::new(&painter, &record, DrawEnv { tokens: &env.tokens, direction: env.direction, ink: Ink::OnPanel });
    let view = env.view();
    let runs: Vec<(String, String)> = rows.iter().map(|(label, value)| (view.resolve(label).as_str().to_owned(), value.clone())).collect();
    let layouts: Vec<_> = runs
        .iter()
        .map(|(label, value)| {
            (draw.layout(&TextRun::spans(vec![Span::new(label.as_str()).muted()]), f32::INFINITY), draw.layout(&TextRun::new(value), f32::INFINITY))
        })
        .collect();
    let space = &env.tokens.space;
    let pad = vec2(space.gap.stack, space.gap.row);
    let label_width = layouts.iter().fold(0.0f32, |widest, (label, _)| widest.max(label.size().x));
    let value_width = layouts.iter().fold(0.0f32, |widest, (_, value)| widest.max(value.size().x));
    let width = 2.0 * pad.x + label_width + space.gap.section + value_width;
    let heights: Vec<f32> = layouts.iter().map(|(label, value)| label.size().y.max(value.size().y)).collect();
    let height = 2.0 * pad.y + heights.iter().sum::<f32>() + env.tokens.stroke.thin * (heights.len() - 1) as f32;
    let tip = env.tokens.size.pointer_tip;
    let offset = vec2(tip.x, tip.y);
    let rtl = env.direction == Direction::Rtl;
    let outer = place(at, offset, vec2(width, height), rtl, ctx.content_rect());

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
        y += row_height + env.tokens.stroke.thin;
    }
}

/// The readout's rect for a pointer at `at`: `offset` toward the reading end and down, on the other side of the pointer
/// where it would leave `window` across, and shifted to stay inside `window`.
fn place(at: Pos2, offset: Vec2, size: Vec2, rtl: bool, window: Rect) -> Rect {
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
