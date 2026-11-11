//! Where a shown layer goes: its anchor, the region and screen it stays in, and the room its body has.

use super::host::{Layer, LayerMemory, memory_slot};
use super::opener::{Opening, opener_anchor};
use super::place::{Anchor, OverlayFit, Place, ViewportShare};
use super::props::Overlay;
use crate::runtime::egui::areas::{self};
use crate::runtime::env::Env;
use crate::runtime::memory::MemoryStage;
use crate::runtime::overlay::{self};
use craft_behavior::overlay::{PlaceRequest, Side};
use egui::{Context, Id, Pos2, Rect};

/// The anchor of `overlay` in screen space: the opener's rect through the opener's own layer, else `own` (the
/// owner's rect) or the given rect or point, in the space `own_to_screen` maps.
pub(crate) fn resolve_anchor(ctx: &Context, id: Id, overlay: &Overlay, own: Rect, own_to_screen: egui::emath::TSTransform) -> Rect {
    if let Some(first) = &overlay.opener {
        let (opener, opening) = first.active(ctx, stored_opening(ctx, id));
        let to_screen = areas::layer_to_screen(ctx, opener.layer);
        return match opener_anchor(opener, opening, &overlay.place) {
            Anchor::Rect(rect) => to_screen * rect,
            Anchor::Point(at) => to_screen * Rect::from_min_max(at, at),
            _ => to_screen * opener.rect,
        };
    }
    match &overlay.place {
        Place::Anchored { anchor: Anchor::Rect(rect), .. } => own_to_screen * *rect,
        Place::Anchored { anchor: Anchor::Point(at), .. } => own_to_screen * Rect::from_min_max(*at, *at),
        _ => own_to_screen * own,
    }
}

/// The overlay with the place's anchor that of the opener that opened it, for the anchors the host reads from the
/// place (`Pointer`, `Viewport`).
pub(super) fn opened_overlay(ctx: &Context, overlay: &Overlay, stored: Option<Opening>) -> Option<(Overlay, Opening)> {
    let first = overlay.opener.as_ref()?;
    let (opener, opening) = first.active(ctx, stored);
    let mut resolved = overlay.clone();
    if let Place::Anchored { anchor, .. } = &mut resolved.place {
        *anchor = opener_anchor(opener, opening, &overlay.place);
    }
    Some((resolved, opening))
}

/// The opening the layer `id` decided when it appeared, `None` before the host has run for it.
fn stored_opening(ctx: &Context, id: Id) -> Option<Opening> {
    MemoryStage::peek::<LayerMemory>(ctx, memory_slot(id)).opening
}

/// The region a modal or floating window layer opened in `env` is contained in: the region of the embedded shell it is
/// opened in (`AppShell::embedded`), `None` for a window's own layers and for popups, toasts and tooltips, which may
/// reach past it.
pub(crate) fn region_for(env: &Env, overlay: &Overlay) -> Option<Rect> {
    let contained = overlay.rank.is_modal() || matches!(overlay.rank, crate::runtime::overlay::Rank::Window);
    contained.then(|| env.get::<crate::runtime::env::ShellEntry>().0.and_then(|shell| shell.region())).flatten()
}

/// The screen rect a layer of `overlay` opened in `env` keeps inside: the safe rect, or the part of it a contained
/// layer's region leaves.
pub(crate) fn screen_for(ctx: &Context, env: &Env, overlay: &Overlay) -> Rect {
    let safe = areas::safe_rect(ctx);
    region_for(env, overlay).map_or(safe, |region| region.intersect(safe))
}

impl Overlay {
    /// The overlay as a layer contained in a region shows: its content may take the whole region, since the region is
    /// what the host has to give.
    pub(crate) fn in_region(&self) -> Overlay {
        let mut overlay = self.clone();
        if let OverlayFit::ContentCapped(_) = overlay.fit {
            overlay.fit = OverlayFit::ContentCapped(ViewportShare::new(1.0, 1.0));
        }
        overlay
    }
}

pub(super) fn anchor_rect(ctx: &Context, layer: &Layer<'_>, bounds: Rect, memory: &mut LayerMemory) -> Rect {
    resolved_anchor(ctx, layer.overlay, layer.anchor, bounds, layer.env.tokens.space.pad.regular, &mut memory.pointer_at)
}

/// The anchor rect of `overlay` in screen space, the one rule the host and the body's measuring run share: a
/// `Viewport` anchor from the safe rect `bounds` less `inset`, a `Pointer` anchor from `pointer_at` (the pointer where
/// the layer opened, taken from the pointer now when none is recorded), any other from `resolved`.
fn resolved_anchor(ctx: &Context, overlay: &Overlay, resolved: Rect, bounds: Rect, inset: f32, pointer_at: &mut Option<Pos2>) -> Rect {
    let point = |at: Pos2| Rect::from_min_max(at, at);
    match &overlay.place {
        Place::Anchored { anchor: Anchor::Viewport(at), .. } => point(at.pos_in_rect(&bounds.shrink(inset))),
        Place::Anchored { anchor: Anchor::Pointer, .. } => {
            point(*pointer_at.get_or_insert_with(|| areas::pointer_pos(ctx).unwrap_or(bounds.center())))
        }
        _ => resolved,
    }
}

/// The room a layer's body is measured in before the layer is placed: `content_room` for the anchor as the host
/// resolves it in this pass. `id` is the layer, whose memory holds the pointer where it opened. `own` and `own_to_screen` are the owner's rect and its layer's map to screen space.
pub(crate) fn measuring_room(
    ctx: &Context,
    id: Id,
    overlay: &Overlay,
    own: Rect,
    own_to_screen: egui::emath::TSTransform,
    inset: f32,
    rtl: bool,
) -> Option<egui::Vec2> {
    let opened = opened_overlay(ctx, overlay, stored_opening(ctx, id));
    let overlay = opened.as_ref().map_or(overlay, |(overlay, _)| overlay);
    if !matches!(overlay.place, Place::Anchored { .. }) {
        return None;
    }
    let bounds = areas::safe_rect(ctx);
    let resolved = resolve_anchor(ctx, id, overlay, own, own_to_screen);
    let anchor = resolved_anchor(ctx, overlay, resolved, bounds, inset, &mut MemoryStage::peek::<LayerMemory>(ctx, memory_slot(id)).pointer_at);
    Some(content_room(overlay, bounds, anchor, rtl))
}

/// The most a layer's body may take: the fit's share of the safe rect `bounds`, at least as wide as the layer's
/// least width, or the whole safe rect.
fn cap(overlay: &Overlay, bounds: Rect, anchor: Rect) -> egui::Vec2 {
    match overlay.fit {
        OverlayFit::ContentCapped(share) => {
            egui::Vec2::new((bounds.width() * share.x).max(overlay.min_width.points(anchor.width())), bounds.height() * share.y)
        }
        _ => bounds.size(),
    }
}

/// The room a layer offers its content before it is placed: the cap, and for an anchored layer the cap within the
/// larger side of its anchor along the side's axis. Content measured in it fits wherever `placement` puts it: on its
/// preferred side when it fits there, else on the larger side.
pub(crate) fn content_room(overlay: &Overlay, bounds: Rect, anchor: Rect, rtl: bool) -> egui::Vec2 {
    let cap = cap(overlay, bounds, anchor);
    let Place::Anchored { side, .. } = &overlay.place else { return cap.min(bounds.size()) };
    let request = PlaceRequest::new(anchor, cap, bounds).side(*side).rtl(rtl).min_width(overlay.min_width);
    let on = |side: Side| craft_behavior::overlay::place(&request.kept(Some(side))).rect.size();
    let (vertical, other) = match side {
        Side::Below => (true, Side::Above),
        Side::Above => (true, Side::Below),
        Side::Start => (false, Side::End),
        _ => (false, Side::Start),
    };
    let (preferred, other) = (on(*side), on(other));
    if vertical { egui::Vec2::new(preferred.x, preferred.y.max(other.y)) } else { egui::Vec2::new(preferred.x.max(other.x), preferred.y) }
}

/// Where the layer goes and the room its body has there.
pub(super) struct Placed {
    pub(super) at: Pos2,
    /// The side of the anchor an anchored layer took by its size; `None` while its size is unknown.
    pub(super) side: Option<Side>,
    /// The most the body may take from `at`: what the viewport leaves, within the fit's cap.
    pub(super) room: egui::Vec2,
}

/// The pure placement of the anchored forms; the viewport centre or the given position for floats, kept inside the
/// safe rect. An anchored layer takes its side by its size: below its anchor, above when the room below is too short
/// and the room above is larger, and keeps that side while it stays open and the content fits there or that side is
/// the larger. Before its size is known (a popup's first, hidden pass) it takes no side and its body is offered
/// `content_room`, so the size it measures decides the side.
pub(super) fn placement(ctx: &Context, layer: &Layer<'_>, size: Option<egui::Vec2>, bounds: Rect, anchor: Rect) -> Placed {
    let rtl = layer.env.direction.is_rtl();
    let cap = cap(layer.overlay, bounds, anchor);
    match &layer.overlay.place {
        Place::Anchored { side, align, .. } => {
            let Some(size) = size else {
                let room = content_room(layer.overlay, bounds, anchor, rtl);
                return Placed { at: anchor.left_bottom().clamp(bounds.min, bounds.max), side: None, room };
            };
            // The side taken in an earlier pass is the preferred one while the layer stays open: it stays while the
            // content fits there or that side is the larger, so a size measured later (a body that grew) still flips it.
            let taken = overlay::entry(ctx, layer.id).and_then(|entry| entry.side).unwrap_or(*side);
            let request = PlaceRequest::new(anchor, size, bounds).side(taken).align(*align).rtl(rtl).min_width(layer.overlay.min_width);
            let placed = craft_behavior::overlay::place(&request);
            // The room is what the side the layer took leaves for a body as large as the cap.
            let room = craft_behavior::overlay::place(&request.kept(Some(placed.side)).size(cap)).rect.size();
            Placed { at: placed.rect.min, side: Some(placed.side), room }
        }
        Place::Float(float) => {
            let size = size.unwrap_or(egui::Vec2::ZERO);
            let min = float.pos.unwrap_or_else(|| bounds.center() - size / 2.0);
            let max_min = Pos2::new((bounds.right() - size.x).max(bounds.left()), (bounds.bottom() - size.y).max(bounds.top()));
            let at = Pos2::new(min.x.clamp(bounds.left(), max_min.x), min.y.clamp(bounds.top(), max_min.y));
            Placed { at, side: None, room: (bounds.max - at).max(egui::Vec2::ZERO).min(cap) }
        }
    }
}
