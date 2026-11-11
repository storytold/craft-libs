//! The pass's widget rects and AccessKit nodes: the widgets egui's hit test puts above a widget; for bypass
//! detection, what egui made interactive and which ids the runtime made on purpose; for the size cases, the scroll
//! regions the runtime showed.

use egui::{AreaState, Context, Id, Order, Rect};

/// The interact rects, in screen space, of the interactive egui widgets of the running pass that egui's hit test
/// puts above the widget `below`: those on a layer higher in its order, or later on `below`'s layer. A layer that is
/// not interactable is click-through and holds none. Widgets in `skip` are left out. Empty when `below` is not shown
/// in the pass.
pub(crate) fn above(ctx: &Context, below: Id, skip: &[Id]) -> Vec<Rect> {
    let (base, layers) = ctx.viewport(|viewport| {
        let widgets = &viewport.this_pass.widgets;
        let Some(base) = widgets.get(below).map(|rect| rect.layer_id) else { return (None, Vec::new()) };
        let mut layers = Vec::new();
        for (layer, rects) in widgets.layers().filter(|(layer, _)| layer.order.allow_interaction()) {
            let after = if *layer == base { rects.iter().position(|rect| rect.id == below).map_or(rects.len(), |at| at + 1) } else { 0 };
            let hits: Vec<Rect> =
                rects[after..].iter().filter(|rect| rect.sense.interactive() && !skip.contains(&rect.id)).map(|rect| rect.interact_rect).collect();
            if !hits.is_empty() {
                layers.push((*layer, hits));
            }
        }
        (Some(base), layers)
    });
    let Some(base) = base else { return Vec::new() };
    // egui's hit test sorts layers by `Order`, then by their place in the layer order; a layer the order does not
    // list yet stands lowest in its `Order`.
    let rank = |layer: egui::LayerId| -> (Order, Option<usize>) {
        (layer.order, ctx.memory(|memory| memory.layer_ids().position(|listed| listed == layer)))
    };
    let height = rank(base);
    let mut found = Vec::new();
    for (layer, hits) in layers {
        if layer != base && (rank(layer) <= height || !AreaState::load(ctx, layer.id).is_none_or(|state| state.interactable)) {
            continue;
        }
        let to_screen = ctx.layer_transform_to_global(layer).unwrap_or(egui::emath::TSTransform::IDENTITY);
        found.extend(hits.into_iter().map(|rect| to_screen * rect));
    }
    found
}

/// An egui widget the runtime makes for a purpose no component registers (a scrim that absorbs presses).
#[cfg(feature = "testing")]
pub(crate) fn note_widget(ctx: &Context, id: Id) {
    noted::with(ctx, |pass| {
        pass.widgets.insert(id);
    });
}

#[cfg(not(feature = "testing"))]
pub(crate) fn note_widget(_: &Context, _: Id) {}

/// A node the runtime wrote, so the pass-end walk knows it from a node a raw egui widget made.
#[cfg(feature = "testing")]
pub(crate) fn note_node(ctx: &Context, id: Id) {
    noted::with(ctx, |pass| {
        pass.nodes.insert(super::access::node_key(id));
    });
}

#[cfg(not(feature = "testing"))]
pub(crate) fn note_node(_: &Context, _: Id) {}

/// A scroll region the runtime showed: `view` is the rect that shows it and cuts what lies outside, `content` the rect
/// of what it holds, which scrolls through the view along the region's axes and lies within the view along the others.
#[cfg(feature = "testing")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScrollRegion {
    pub view: Rect,
    pub content: Rect,
}

/// Notes a scroll region of the running pass, for the suite's size cases.
#[cfg(feature = "testing")]
pub(crate) fn note_scroll_region(ctx: &Context, view: Rect, content: Rect) {
    noted::with(ctx, |pass| pass.scrolls.push(ScrollRegion { view, content }));
}

#[cfg(not(feature = "testing"))]
pub(crate) fn note_scroll_region(_: &Context, _: Rect, _: Rect) {}

/// The scroll regions the runtime noted in the root viewport's latest pass.
#[cfg(feature = "testing")]
pub(crate) fn scroll_regions(ctx: &Context) -> Vec<ScrollRegion> {
    noted::scrolls(ctx)
}

/// The ids the runtime noted in the root viewport's latest pass.
#[cfg(feature = "testing")]
pub(crate) fn noted_ids(ctx: &Context) -> (std::collections::HashSet<Id>, std::collections::HashSet<u64>) {
    noted::peek(ctx)
}

/// The interactive egui widgets of the root viewport's latest finished pass: those that sense a click or a drag
/// or take focus. A layer's own widget (an `Area`'s, the one egui keys `layer.id.with("move")`) is not one.
#[cfg(feature = "testing")]
pub(crate) fn interactive(ctx: &Context) -> Vec<Id> {
    ctx.viewport(|viewport| {
        let widgets = &viewport.prev_pass.widgets;
        let mut found = Vec::new();
        for (layer, rects) in widgets.layers() {
            let area = layer.id.with("move");
            found.extend(
                rects
                    .iter()
                    .filter(|rect| rect.id != layer.id && rect.id != area && (rect.sense.interactive() || rect.sense.is_focusable()))
                    .map(|rect| rect.id),
            );
        }
        found
    })
}

#[cfg(feature = "testing")]
mod noted {
    use crate::runtime::egui::memory;
    use crate::runtime::egui::pass;
    use egui::{Context, Id};
    use std::collections::HashSet;

    /// What the runtime noted in one pass; a new pass starts empty.
    #[derive(Default)]
    pub(super) struct Pass {
        pub(super) widgets: HashSet<Id>,
        pub(super) nodes: HashSet<u64>,
        pub(super) scrolls: Vec<super::ScrollRegion>,
    }

    #[derive(Default)]
    struct Noted {
        stamp: Option<u64>,
        pass: Pass,
    }

    /// The root viewport's notes of its latest pass.
    pub(super) fn peek(ctx: &Context) -> (HashSet<Id>, HashSet<u64>) {
        memory::with(ctx, |noted: &mut Noted| (noted.pass.widgets.clone(), noted.pass.nodes.clone()))
    }

    /// The root viewport's scroll regions of its latest pass.
    pub(super) fn scrolls(ctx: &Context) -> Vec<super::ScrollRegion> {
        memory::with(ctx, |noted: &mut Noted| noted.pass.scrolls.clone())
    }

    /// Runs `f` on the running pass's notes. Only the root viewport is kept: its pass number stamps the notes, and
    /// a child viewport's pass leaves them alone.
    pub(super) fn with<R>(ctx: &Context, f: impl FnOnce(&mut Pass) -> R) -> R {
        let viewport = pass::current_viewport(ctx);
        let nr = pass::pass_nr(ctx, egui::ViewportId::ROOT);
        memory::with(ctx, |noted: &mut Noted| {
            if viewport != egui::ViewportId::ROOT {
                return f(&mut Pass::default());
            }
            if noted.stamp != Some(nr) {
                noted.stamp = Some(nr);
                noted.pass = Pass::default();
            }
            f(&mut noted.pass)
        })
    }
}
