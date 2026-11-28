//! Saved window placement: reading, writing and recording it in the placement store.

use super::host::{monitors, scale_of};
use super::{Geometry, Record, shared};
use crate::runtime::egui::viewports;
use crate::runtime::frame::PerViewport;
use craft_behavior::placement::{Placement, monitor_of};
use egui::{Context, Rect, ViewportId};

/// The placement saved under `key`.
pub(crate) fn saved_placement(ctx: &Context, key: &str) -> Option<Placement> {
    let store = shared(ctx, |shared| shared.store.clone())?;
    let store = store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    store.get(key)
}

/// Saves `placement` under `key`; the end hook writes the file.
pub(crate) fn save_placement(ctx: &Context, key: &str, placement: &Placement) {
    let Some(store) = shared(ctx, |shared| shared.store.clone()) else { return };
    store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).set(key, placement);
}

/// The interface zoom of the running pass, when the frame's window facts were measured at it: egui-winit measures them
/// before the frame, at the zoom then in force, and a zoom the frame sets applies from its first pass. `None` in the
/// frame of a change of zoom.
fn settled_zoom(ctx: &Context) -> Option<f32> {
    let zoom = ctx.zoom_factor();
    let measured = shared(ctx, |shared| shared.facts_zoom).unwrap_or(zoom);
    (measured == zoom).then_some(zoom)
}

/// Keeps the window's placement in the store, as [`placement_of`] records it, in the store's points at zoom 1
/// (`craft_behavior::placement`): egui's rects are points at the interface zoom. A frame whose window facts were
/// measured at another zoom than its own (the frame of a change of zoom) records nothing; the next one does. A
/// minimized window keeps the last placement it was shown at: Windows reports it as 0 × 0 far off screen, which would
/// reopen at egui's size floor. Where the platform tells no window position (Wayland), egui reports no window rect:
/// the size is then the pass's content rect, and the size and state are recorded while the position stays as it was.
pub(super) fn record_placement(ctx: &Context, viewport: ViewportId, key: &str, facts: &viewports::Facts) {
    if facts.minimized {
        return;
    }
    let Some(zoom) = settled_zoom(ctx) else { return };
    let mut facts = *facts;
    if facts.inner.is_none() {
        let content = ctx.input(|input| input.content_rect());
        facts.inner = Some(content).filter(|rect| rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0);
        facts.outer = None;
    }
    let Some(inner) = facts.inner else { return };
    let geometry = Geometry { inner, outer: facts.outer, scale: facts.native_scale, zoom, maximized: facts.maximized, fullscreen: facts.fullscreen };
    let unchanged = PerViewport::<Record>::with(ctx, viewport, |record| record.placed.replace(geometry) == Some(geometry));
    if unchanged {
        return;
    }
    let at_zoom_1 = viewports::Facts { inner: Some(inner * zoom), outer: facts.outer.map(|outer| outer * zoom), ..facts };
    if let Some(mut next) = placement_of(ctx, &at_zoom_1, saved_placement(ctx, key)) {
        next.zoom = zoom;
        save_placement(ctx, key, &next);
    }
}

/// `previous` (or a new record) updated to where the window with `facts` is: its scale, its state, its size and
/// outer position unless it is maximized or full screen (which keep the last normal ones), and the monitor showing
/// most of it, all in the points of `facts`. App windows record their placement with this at zoom 1
/// ([`record_placement`]); the dock model keeps its native windows' placements in egui points
/// (`WindowHost::placement`). `None` before the window has an inner rect and a scale.
pub(super) fn placement_of(ctx: &Context, facts: &viewports::Facts, previous: Option<Placement>) -> Option<Placement> {
    let inner = facts.inner?;
    let scale = scale_of(ctx, facts)?;
    let mut next = previous.unwrap_or_else(|| Placement::new(inner.size().into(), scale));
    next.scale = scale;
    next.maximized = facts.maximized;
    next.fullscreen = facts.fullscreen;
    if !facts.maximized && !facts.fullscreen {
        next.normal.size = inner.size().into();
        if let Some(outer) = facts.outer {
            next.normal.pos = [outer.min.x, outer.min.y];
            next.position_px = Some([(outer.min.x * scale).round() as i32, (outer.min.y * scale).round() as i32]);
            let outer_px = Rect::from_min_max(outer.min * scale, outer.max * scale);
            if let Some(monitor) = monitor_of(&monitors(ctx), outer_px) {
                next.monitor = Some(monitor.key.clone());
            }
        }
    }
    Some(next)
}
