//! The cursor service (8.5): requests of a pass collect here, the end hook picks the winner and records it, and a
//! painted winner reaches the window as its cursor image where the window shows one.

use crate::runtime::egui::areas::LayerKey;
use crate::runtime::egui::{output, pass, viewports};
use crate::runtime::frame::PerViewport;
use crate::runtime::windows;
use craft_behavior::cursor::pick_index;
use craft_behavior::{Cursor, CursorIcon, CursorPriority, PaintedCursor};
use craft_host::target::{Caps, Os};
use egui::{Context, Rect, ViewportId};
use std::any::Any;
use std::sync::Arc;

/// The largest side of a cursor image in device pixels on Linux, where X servers and hardware cursor planes commonly
/// cap cursors at 256 and fall back to slow software cursors, or reject them, past that.
const MAX_SIDE_LINUX: u16 = 255;
/// The largest side elsewhere: winit's limit; AppKit sets none on an `NSCursor` image.
pub(crate) const MAX_SIDE: u16 = 2048;

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

/// Where a cursor's footprint may show: a layer and a clip in screen space. A brush tip outline is a mark on its
/// canvas, not a pointer, so it shows only inside the canvas's view and under every layer above the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CursorRegion {
    /// The layer the requester paints on.
    pub layer: LayerKey,
    /// The part of the screen the footprint may cover.
    pub clip: Rect,
}

impl CursorRegion {
    /// The region of `clip` on `layer`.
    pub fn new(layer: LayerKey, clip: Rect) -> Self {
        CursorRegion { layer, clip }
    }
}

#[derive(Default)]
pub(crate) struct State {
    requests: Vec<(Cursor, CursorPriority)>,
    /// The region of each request, in the order of `requests`.
    regions: Vec<Option<CursorRegion>>,
    record: Option<Cursor>,
    /// The region of the recorded winner.
    region: Option<CursorRegion>,
    /// The last image handed to the window with the art and scale it was drawn from. egui-winit uploads an image to
    /// the OS whenever its `Arc` changes, so an unchanged pointer hands over the same one.
    image: Option<(Box<dyn Any + Send + Sync>, f32, CursorBitmap)>,
}

/// Requests `cursor` for the running pass of this viewport.
pub fn request(ctx: &Context, cursor: Cursor, priority: CursorPriority) {
    push(ctx, cursor, priority, None);
}

/// Requests `cursor` for the running pass of this viewport, its footprint (a brush tip outline) shown only in
/// `region`: the overlay host paints a tip outline that leaves the region, or that a layer above the region's covers,
/// on the region's layer clipped to it, and gives the window only its marks.
#[doc(hidden)]
pub fn request_within(ctx: &Context, cursor: Cursor, priority: CursorPriority, region: CursorRegion) {
    push(ctx, cursor, priority, Some(region));
}

fn push(ctx: &Context, cursor: Cursor, priority: CursorPriority, region: Option<CursorRegion>) {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        state.requests.push((cursor, priority));
        state.regions.push(region);
    });
}

/// The end hook: records the pass's winner, which in a window an app-modal layer in another window blocks is the
/// blocked cursor. A system cursor goes out through the viewport's output; a painted cursor hides the system one and
/// is left for the overlay host, which paints it or hands it back as an image ([`show_image`]), except a badged
/// system cursor, which keeps its base and has only the badge painted. The window's image from the last pass is
/// cleared: egui keeps it until it is replaced. Over the resize areas of a window the OS resizes from its own frame
/// the cursor is the default whatever was requested: the OS shows its resize cursor there, and any other fights it.
/// Where the shell cannot set the system cursor ([`paints`]) a painted winner leaves the system pointer showing.
pub(crate) fn end_pass(ctx: &Context) {
    let blocked = windows::blocking_modal(ctx).is_some();
    let viewport = pass::current_viewport(ctx);
    let to_os = !blocked && over_os_resize_area(ctx, viewport);
    let hides = paints(ctx);
    let won = PerViewport::<State>::with(ctx, viewport, |state| {
        let (won, region) = match pick_index(&state.requests) {
            _ if blocked => (Some(Cursor::System(CursorIcon::NotAllowed)), None),
            _ if to_os => (Some(Cursor::System(CursorIcon::Default)), None),
            Some(at) => (state.requests.get(at).map(|(cursor, _)| cursor.clone()), state.regions.get(at).copied().flatten()),
            None => (None, None),
        };
        state.requests.clear();
        state.regions.clear();
        state.region = region;
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
        Some(_) if !hides => output::set_cursor(ctx, CursorIcon::Default),
        Some(_) => output::set_cursor(ctx, CursorIcon::None),
        None => {}
    }
}

/// Whether a painted cursor shows in place of the system pointer: only where the shell sets the system cursor
/// ([`Caps::SYSTEM_CURSOR`]). A host without it (Wayland with no cursor theme, where a hidden pointer never comes
/// back) keeps the system pointer, and no painted cursor is drawn or handed to the window.
pub(crate) fn paints(ctx: &Context) -> bool {
    windows::base_shell(ctx).caps.contains(Caps::SYSTEM_CURSOR)
}

/// Whether the pointer lies in the resize areas of `viewport`'s window where the OS resizes it from its own frame
/// (`windows::os_resizes`).
fn over_os_resize_area(ctx: &Context, viewport: ViewportId) -> bool {
    if !windows::os_resizes(ctx, viewport) {
        return false;
    }
    let Some(hover) = crate::runtime::pointer::pass_pointer(ctx).hover else { return false };
    let content = pass::content_rect(ctx);
    let at = hover - content.min;
    craft_window::mac::in_resize_area([content.width(), content.height()], [at.x, at.y])
}

/// The cursor the last pass of the running viewport settled on; `None` is the default arrow.
pub fn current(ctx: &Context) -> Option<Cursor> {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.record.clone())
}

/// The region of the cursor the last pass settled on, when its requester gave one ([`request_within`]).
pub(crate) fn current_region(ctx: &Context) -> Option<CursorRegion> {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.region)
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
    // A host that keeps its cursors at their size ignores the interface zoom.
    let follows_zoom = crate::runtime::env::Env::of_root(ctx).cursor_follows_zoom;
    let scale = match (shell.os == Os::Mac, follows_zoom) {
        (_, false) => 1.0,
        (true, true) => zoom,
        (false, true) => pixels_per_point,
    };
    Some(ImageTarget { scale, max_side: if shell.os == Os::Linux { MAX_SIDE_LINUX } else { MAX_SIDE } })
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
