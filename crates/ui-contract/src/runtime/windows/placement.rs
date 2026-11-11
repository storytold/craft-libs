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

/// Keeps the window's placement in the store, as [`placement_of`] records it.
pub(super) fn record_placement(ctx: &Context, viewport: ViewportId, key: &str, facts: &viewports::Facts) {
    let Some(inner) = facts.inner else { return };
    let geometry = Geometry { inner, outer: facts.outer, scale: facts.native_scale, maximized: facts.maximized, fullscreen: facts.fullscreen };
    let unchanged = PerViewport::<Record>::with(ctx, viewport, |record| record.placed.replace(geometry) == Some(geometry));
    if unchanged {
        return;
    }
    if let Some(next) = placement_of(ctx, facts, saved_placement(ctx, key)) {
        save_placement(ctx, key, &next);
    }
}

/// `previous` (or a new record) updated to where the window with `facts` is: its scale, its state, its size and
/// outer position unless it is maximized or full screen (which keep the last normal ones), and the monitor showing
/// most of it. App windows and native dock windows record their placement with this. `None` before the window has
/// an inner rect and a scale.
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
