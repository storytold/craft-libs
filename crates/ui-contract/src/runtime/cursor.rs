//! The cursor service (8.5): requests of a pass collect here, the end hook picks the winner and records it, and a
//! painted winner reaches the window as its cursor image where the window shows one.

use crate::primitives::pointer_art::{self as pointers, Art};
use crate::primitives::{IconSet, Mirror, SvgIcon};
use crate::runtime::egui::{output, pass, viewports};
use crate::runtime::env::Env;
use crate::runtime::frame::PerViewport;
use crate::runtime::windows;
use crate::style::Ink;
use craft_behavior::cursor::pick;
use craft_behavior::{Cursor, CursorIcon, CursorPriority, PaintedCursor};
use craft_host::target::Os;
use egui::{Context, ViewportId};
use std::any::Any;
use std::cell::RefCell;
use std::sync::Arc;

/// The largest side of a cursor image in device pixels on Linux, where X servers and hardware cursor planes commonly
/// cap cursors at 256 and fall back to slow software cursors, or reject them, past that.
const MAX_SIDE_LINUX: u16 = 255;
/// The largest side elsewhere: winit's limit; AppKit sets none on an `NSCursor` image.
const MAX_SIDE: u16 = 2048;

/// A cursor image in straight (not premultiplied) RGBA, row by row, its hotspot on the centre pixel.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CursorBitmap {
    /// Straight RGBA, row by row.
    pub rgba: Arc<[u8]>,
    /// Width and height in pixels; odd, so the hotspot lies on the centre pixel at every scale.
    pub size: [u16; 2],
    /// The hotspot in pixels from the top left.
    pub hotspot: [u16; 2],
}

/// How the running viewport's window takes a cursor image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageTarget {
    /// Image pixels per point.
    pub scale: f32,
    /// The largest side in pixels; a pointer that needs more is painted.
    pub max_side: u16,
}

#[derive(Default)]
pub(crate) struct State {
    requests: Vec<(Cursor, CursorPriority)>,
    record: Option<Cursor>,
    /// The last image handed to the window with the art and scale it was drawn from. egui-winit uploads an image to
    /// the OS whenever its `Arc` changes, so an unchanged pointer hands over the same one.
    image: Option<(Box<dyn Any + Send + Sync>, f32, CursorBitmap)>,
}

/// The window image of the painted pointer `cursor` at `scale` device pixels per point, rasterised from the art the
/// ghost paints; `None` when the art does not fit a cursor image.
pub fn painted_image(env: &Env, cursor: &PaintedCursor, scale: f32) -> Option<CursorBitmap> {
    pointer_art(env, cursor, false, false).rasterise(scale, MAX_SIDE)
}

/// The art of `cursor`: the hand as a fist while `closed`; with `marks_only`, a brush tip's centre cross and badge
/// without its ring. The ghost paints it and the window's cursor image is rasterised from it, so both look the same.
pub(crate) fn pointer_art(env: &Env, cursor: &PaintedCursor, closed: bool, marks_only: bool) -> Art {
    use craft_behavior::PointerGlyph;
    let overlay = &env.tokens.color.overlay;
    let pen = pointers::Pen {
        art: RefCell::new(Art::default()),
        fill: crate::style::rgba_color(overlay.stroke),
        halo: crate::style::rgba_color(overlay.halo),
        thin: env.tokens.stroke.thin,
        rtl: env.direction.is_rtl(),
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
        PaintedCursor::TypeOn { path, .. } => pointers::type_on(&pen, *path),
        PaintedCursor::IBeam => pointers::i_beam_pointer(&pen),
        PaintedCursor::Locator => pointers::locator(&pen),
        PaintedCursor::Arrow { hollow, toward, .. } => pointers::arrow(&pen, *hollow, *toward),
        PaintedCursor::PathText { start, end, .. } => pointers::path_text(&pen, *start, *end),
        PaintedCursor::PointerTool { pointer, icon, .. } => {
            // The arrow's icon sits under its body, so it takes the larger size to show past the arrow.
            let side = if *pointer == PointerGlyph::Arrow { env.tokens.size.icon.md } else { env.tokens.size.icon.sm };
            let icon = SvgIcon::of(&env.tokens, env.direction, side, icon, IconSet::default(), Mirror::Auto);
            pointers::pointer_tool(&pen, *pointer, icon, side, edge, ink);
        }
        // `Hidden` has no marks.
        _ => {}
    }
    pen.art.into_inner()
}

/// Requests `cursor` for the running pass of this viewport.
#[doc(hidden)]
pub fn request(ctx: &Context, cursor: Cursor, priority: CursorPriority) {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.requests.push((cursor, priority)));
}

/// The end hook: records the pass's winner, which in a window an app-modal layer in another window blocks is the
/// blocked cursor. A system cursor goes out through the viewport's output; a painted cursor hides the system one and
/// is left for the overlay host, which paints it or hands it back as an image ([`show_image`]), except a badged
/// system cursor, which keeps its base and has only the badge painted. The window's image from the last pass is
/// cleared: egui keeps it until it is replaced.
pub(crate) fn end_pass(ctx: &Context) {
    let blocked = windows::blocking_modal(ctx).is_some();
    let won = PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        let won = if blocked { Some(Cursor::System(CursorIcon::NotAllowed)) } else { pick(&state.requests).cloned() };
        state.requests.clear();
        if !matches!(won, Some(Cursor::Painted(_))) {
            state.image = None;
        }
        state.record = won.clone();
        won
    });
    output::set_cursor_image(ctx, None);
    match won {
        Some(Cursor::System(icon)) => output::set_cursor(ctx, icon),
        Some(Cursor::Painted(PaintedCursor::Badged { base, .. })) => output::set_cursor(ctx, base),
        Some(_) => output::set_cursor(ctx, CursorIcon::None),
        None => {}
    }
}

/// The cursor the last pass of the running viewport settled on; `None` is the default arrow.
pub fn current(ctx: &Context) -> Option<Cursor> {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.record.clone())
}

/// How the running viewport's window takes a cursor image, or `None` where it takes none: only the root viewport (the
/// framework's other windows are egui's immediate viewports, which hand no image to the OS), and only where the shell
/// shows images at the window's scale. The image is in points on macOS, where AppKit sizes an `NSCursor` in points,
/// so it scales with the interface zoom only; in device pixels elsewhere.
pub(crate) fn image_target(ctx: &Context) -> Option<ImageTarget> {
    if pass::current_viewport(ctx) != ViewportId::ROOT {
        return None;
    }
    let shell = windows::base_shell(ctx);
    let zoom = ctx.zoom_factor();
    let pixels_per_point = ctx.pixels_per_point();
    let native = viewports::current(ctx).native_scale.unwrap_or(pixels_per_point / zoom);
    if !shell.cursor_images(native) {
        return None;
    }
    Some(ImageTarget {
        scale: if shell.os == Os::Mac { zoom } else { pixels_per_point },
        max_side: if shell.os == Os::Linux { MAX_SIDE_LINUX } else { MAX_SIDE },
    })
}

/// A painted pointer the window can take as its cursor image: drawn at a scale, compared with the last one handed over.
pub(crate) trait CursorArt: Any + Clone + PartialEq + Send + Sync {
    /// The image at `scale` device pixels per point, `None` where the art has none at that size.
    fn rasterise(&self, scale: f32, max_side: u16) -> Option<CursorBitmap>;
}

/// Hands `art` to the window as its cursor image at `target`, drawing it only when the art or the scale changed since
/// the last image; the crosshair is the system cursor an integration that cannot take the image falls back to.
/// Returns false, and hands nothing over, when the art has no image at that size.
pub(crate) fn show_image<A: CursorArt>(ctx: &Context, art: &A, target: ImageTarget) -> bool {
    let bitmap = PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        if let Some((last, scale, bitmap)) = &state.image
            && last.downcast_ref::<A>() == Some(art)
            && *scale == target.scale
        {
            return Some(bitmap.clone());
        }
        let bitmap = art.rasterise(target.scale, target.max_side)?;
        state.image = Some((Box::new(art.clone()), target.scale, bitmap.clone()));
        Some(bitmap)
    });
    let Some(bitmap) = bitmap else { return false };
    output::set_cursor_image(ctx, Some(&bitmap));
    output::set_cursor(ctx, CursorIcon::Crosshair);
    true
}
