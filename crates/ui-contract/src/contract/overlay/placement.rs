//! Where a shown layer goes: its anchor, the region and screen it stays in, and the room its body has.

use super::host::{Layer, LayerMemory, memory_slot};
use super::opener::{Opening, opener_anchor};
use super::place::{Anchor, OverlayFit, Place, ViewportShare};
use super::props::Overlay;
use crate::runtime::egui::areas::{self};
use crate::runtime::env::Env;
use crate::runtime::memory::MemoryStage;
use crate::runtime::overlay::{self};
use craft_behavior::OwnerClass;
use craft_behavior::overlay::{PlaceRequest, Side};
use egui::{Context, Id, Pos2, Rect};

/// The anchor of `overlay` in screen space: the opener's rect through the opener's own layer, else `own` (the
/// owner's rect) or the given rect or point, in the space `own_to_screen` maps.
pub(crate) fn resolve_anchor(ctx: &Context, id: Id, overlay: &Overlay, own: Rect, own_to_screen: egui::emath::TSTransform) -> Rect {
    if let Some(first) = &overlay.opener {
        let (opener, opening) = first.active(ctx, stored_opening(ctx, id));
        let to_screen = areas::layer_to_screen(ctx, opener.layer);
        return match opener_anchor(opener, opening, &overlay.place) {
            Anchor::Rect(rect) | Anchor::Corner(rect, _) => to_screen * rect,
            Anchor::Point(at) => to_screen * Rect::from_min_max(at, at),
            Anchor::Parent => parent_rect(ctx).unwrap_or(to_screen * opener.rect),
            _ => to_screen * opener.rect,
        };
    }
    match &overlay.place {
        Place::Anchored { anchor: Anchor::Rect(rect) | Anchor::Corner(rect, _), .. } => own_to_screen * *rect,
        Place::Anchored { anchor: Anchor::Point(at), .. } => own_to_screen * Rect::from_min_max(*at, *at),
        Place::Anchored { anchor: Anchor::Parent, .. } => parent_rect(ctx).unwrap_or(own_to_screen * own),
        _ => own_to_screen * own,
    }
}

/// The placed panel rect, in screen space, of the layer whose body is running: the parent a layer opened now has.
/// `None` outside a layer or before the parent has been placed.
fn parent_rect(ctx: &Context) -> Option<Rect> {
    let parent = overlay::drawing_parent(ctx)?;
    overlay::entry(ctx, parent).map(|entry| entry.rect)
}

/// The overlay with the place's anchor that of the opener that opened it, for the anchors the host reads from the
/// place (`Pointer`, `Corner`, `Parent`).
pub(super) fn opened_overlay(ctx: &Context, overlay: &Overlay, stored: Option<Opening>) -> Option<(Overlay, Opening)> {
    let first = overlay.opener.as_ref()?;
    let (opener, opening) = first.active(ctx, stored);
    let mut resolved = overlay.clone();
    if let Place::Anchored { anchor, .. } = &mut resolved.place {
        *anchor = opener_anchor(opener, opening, &overlay.place);
    }
    Some((resolved, opening))
}

/// Whether a layer of `overlay` may cover its anchor when it fits on neither side of it: a menu's layer (its key class
/// is [`OwnerClass::Menu`]) hung at the pointer, at a point, or at the rect a context request named. No other overlay
/// covers its anchor: a tooltip or a popover at the pointer stays off it, unless its place asks for it
/// ([`Place::covering_anchor`]). `stored` is the opening the layer decided.
pub(super) fn layer_covers_anchor(ctx: &Context, overlay: &Overlay, stored: Option<Opening>) -> bool {
    if matches!(overlay.place, Place::Anchored { covers: true, .. }) {
        return true;
    }
    if overlay.keys != OwnerClass::Menu {
        return false;
    }
    let (anchor, requested) = match &overlay.opener {
        Some(first) => {
            let (opener, opening) = first.active(ctx, stored);
            (opener_anchor(opener, opening, &overlay.place), opener.anchor.is_none() && matches!(opening, Opening::At(_)))
        }
        None => match &overlay.place {
            Place::Anchored { anchor, .. } => (anchor.clone(), false),
            Place::Float(_) => return false,
        },
    };
    match anchor {
        Anchor::Pointer | Anchor::Point(_) => true,
        // `opener_anchor` gives a requested rect only for a context request's opening.
        Anchor::Rect(_) => requested,
        _ => false,
    }
}

/// The opening the layer `id` decided when it appeared, `None` before the host has run for it.
fn stored_opening(ctx: &Context, id: Id) -> Option<Opening> {
    MemoryStage::peek::<LayerMemory>(ctx, memory_slot(id)).opening
}

/// The screen rect the popups opened in an env are contained in, and the popups opened from those: the menus of a menu
/// bar stay below the bar ([`OverlayUi::contain_popups_below`](super::OverlayUi::contain_popups_below)). `None`, the
/// root's, lets popups take the whole safe rect.
#[derive(Clone, Copy, Debug, Default, PartialEq, crate::contract::MemoProps)]
#[non_exhaustive]
pub struct PopupRegion(pub Option<Rect>);

impl PopupRegion {
    /// Popups contained in `region`; `None` lets them take the whole safe rect.
    pub fn new(region: Option<Rect>) -> Self {
        PopupRegion(region)
    }
}

impl crate::runtime::env::EnvValue for PopupRegion {
    fn root() -> Self {
        PopupRegion(None)
    }
}

/// The region a layer opened in `env` is contained in: for a modal or floating window layer the region of the
/// embedded shell it is opened in (`AppShell::embedded`), for a popup the env's [`PopupRegion`]; `None` for a window's
/// own layers and for toasts and tooltips, which may reach past it.
pub(crate) fn region_for(env: &Env, overlay: &Overlay) -> Option<Rect> {
    match overlay.rank {
        crate::runtime::overlay::Rank::Popup => env.get::<PopupRegion>().0,
        rank if rank.is_modal() || matches!(rank, crate::runtime::overlay::Rank::Window) => {
            env.get::<crate::runtime::env::ShellEntry>().0.and_then(|shell| shell.region())
        }
        _ => None,
    }
}

/// The room a layer opened in the running viewport may take, in its points: the work area around a palette window,
/// whose layers show in popup windows of their own (`windows::popup_room`), else the window's safe rect.
pub fn layer_room(ctx: &Context) -> Rect {
    crate::runtime::windows::popup_room(ctx).map_or_else(|| areas::safe_rect(ctx), |popup| popup.room)
}

/// The screen rect a layer of `overlay` opened in `env` keeps inside: the layer room ([`layer_room`]), or the part
/// of it a contained layer's region leaves.
pub(crate) fn screen_for(ctx: &Context, env: &Env, overlay: &Overlay) -> Rect {
    let safe = layer_room(ctx);
    region_for(env, overlay).map_or(safe, |region| region.intersect(safe))
}

impl Overlay {
    /// The overlay as a layer opened in `env` shows: a modal or window layer contained in an embedded shell's region
    /// may take the whole region, since the region is what the host has to give; a popup kept in a [`PopupRegion`]
    /// keeps its cap.
    pub(crate) fn opened_in(&self, env: &Env) -> Overlay {
        let shell = self.rank.is_modal() || matches!(self.rank, crate::runtime::overlay::Rank::Window);
        if shell && region_for(env, self).is_some() { self.in_region() } else { self.clone() }
    }

    /// The overlay as a layer contained in a region shows: its content may take the whole region.
    fn in_region(&self) -> Overlay {
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

/// The rect `layer` is placed inside: `kept_in` for its anchor as the host resolves it.
fn layer_bounds(layer: &Layer<'_>, bounds: Rect) -> Rect {
    kept_in(layer.overlay, layer.anchor, bounds, layer.env.tokens.space.pad.regular)
}

/// The part of the safe rect `bounds` a layer of `overlay` stays inside: within the overlay's own
/// [`bounds`](Overlay::bounds) when it sets them, in every placement kind. Bounds that miss the safe rect leave it whole.
fn within_bounds(overlay: &Overlay, bounds: Rect) -> Rect {
    overlay.bounds.map(|own| own.intersect(bounds)).filter(|both| !both.is_negative()).unwrap_or(bounds)
}

/// The rect a layer of `overlay` is placed and measured inside, the one rule the host and the body's measuring run
/// share: the safe rect `bounds` within the overlay's own bounds (`within_bounds`), and for a `Corner` anchor the part
/// of its rect (`resolved`, in screen space) within that, less `inset`, so a toast stays inside the body whose corner
/// it sits at.
fn kept_in(overlay: &Overlay, resolved: Rect, bounds: Rect, inset: f32) -> Rect {
    let bounds = within_bounds(overlay, bounds);
    match &overlay.place {
        Place::Anchored { anchor: Anchor::Corner(..), .. } => resolved.intersect(bounds).shrink(inset),
        _ => bounds,
    }
}

/// The anchor rect of `overlay` in screen space, the one rule the host and the body's measuring run share: a
/// `Corner` anchor the corner of the rect it is kept in (`kept_in`), a `Pointer` anchor from `pointer_at` (the pointer
/// where the layer opened, taken from the pointer now when none is recorded), any other from `resolved`.
fn resolved_anchor(ctx: &Context, overlay: &Overlay, resolved: Rect, bounds: Rect, inset: f32, pointer_at: &mut Option<Pos2>) -> Rect {
    let point = |at: Pos2| Rect::from_min_max(at, at);
    match &overlay.place {
        Place::Anchored { anchor: Anchor::Corner(_, at), .. } => point(at.pos_in_rect(&kept_in(overlay, resolved, bounds, inset))),
        Place::Anchored { anchor: Anchor::Pointer, .. } => {
            point(*pointer_at.get_or_insert_with(|| areas::pointer_pos(ctx).unwrap_or(bounds.center())))
        }
        _ => resolved,
    }
}

/// The room a layer's body is measured in before the layer is placed: `content_room` for the anchor as the host
/// resolves it in this pass, inside the screen rect the layer opened in `env` keeps inside.
/// `id` is the layer, whose memory holds the pointer where it opened. `own` and `own_to_screen` are the owner's rect and its layer's map to screen space.
#[expect(
    clippy::too_many_arguments,
    reason = "the measuring room reads the pass facts the caller holds as separate values; a struct would only restate them"
)]
pub(crate) fn measuring_room(
    ctx: &Context,
    env: &Env,
    id: Id,
    overlay: &Overlay,
    own: Rect,
    own_to_screen: egui::emath::TSTransform,
    inset: f32,
    rtl: bool,
) -> Option<egui::Vec2> {
    let stored = stored_opening(ctx, id);
    let covers = layer_covers_anchor(ctx, overlay, stored);
    let opened = opened_overlay(ctx, overlay, stored);
    let overlay = opened.as_ref().map_or(overlay, |(overlay, _)| overlay);
    if !matches!(overlay.place, Place::Anchored { .. }) {
        return None;
    }
    // The caller's bounds are in the owner's layer space, as an anchor rect is.
    let mapped = overlay.in_screen(own_to_screen);
    let overlay = &*mapped;
    let bounds = screen_for(ctx, env, overlay);
    let resolved = resolve_anchor(ctx, id, overlay, own, own_to_screen);
    let anchor = resolved_anchor(ctx, overlay, resolved, bounds, inset, &mut MemoryStage::peek::<LayerMemory>(ctx, memory_slot(id)).pointer_at);
    Some(content_room(overlay, covers, bounds, kept_in(overlay, resolved, bounds, inset), anchor, rtl))
}

/// The most a layer's body may take: the fit's share of the safe rect `bounds`, at least as wide as the layer's
/// least width, or the whole safe rect.
fn cap(overlay: &Overlay, bounds: Rect, anchor: Rect) -> egui::Vec2 {
    match overlay.fit {
        OverlayFit::ContentCapped(share) => {
            egui::Vec2::new((bounds.width() * share.x).max(overlay.least_width(anchor.width())), bounds.height() * share.y)
        }
        _ => bounds.size(),
    }
}

/// The room a layer offers its content before it is placed: the cap (a share of the safe rect `bounds`), and for an
/// anchored layer the cap within the larger side of its anchor along the side's axis, inside `kept` (`kept_in`), or
/// within `kept` for a layer that `covers` its anchor. Content measured in it fits wherever `placement` puts it: on its
/// preferred side when it fits there, else on the larger side, or over the anchor.
pub(crate) fn content_room(overlay: &Overlay, covers: bool, bounds: Rect, kept: Rect, anchor: Rect, rtl: bool) -> egui::Vec2 {
    let cap = cap(overlay, bounds, anchor);
    let Place::Anchored { side, .. } = &overlay.place else { return cap.min(bounds.size()) };
    let request = PlaceRequest::new(anchor, cap, kept).side(*side).rtl(rtl).min_width(overlay.min_width).covers_anchor(covers);
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
    /// The side the layer asked for when it took `side`.
    pub(super) asked: Option<Side>,
    /// The most the body may take from `at`: what the viewport leaves, within the fit's cap.
    pub(super) room: egui::Vec2,
}

/// The pure placement of the anchored forms; the centre or the given position for floats, kept inside the safe rect
/// within the overlay's bounds (`within_bounds`) by the keep rule moves follow. An anchored layer takes its side by its
/// size: below its anchor, above when the room below is too short and the room above is larger, and keeps that side
/// while it stays open, asks for the same side, and the content fits there or that side is the larger. A layer that `covers` its anchor
/// ([`layer_covers_anchor`]) and fits neither side keeps its height within the bounds and slides over the anchor.
/// Before its size is known (a popup's first, hidden pass) it takes no side and its body is offered `content_room`, so
/// the size it measures decides the side.
pub(super) fn placement(ctx: &Context, layer: &Layer<'_>, covers: bool, size: Option<egui::Vec2>, bounds: Rect, anchor: Rect) -> Placed {
    let rtl = layer.env.direction.is_rtl();
    let cap = cap(layer.overlay, bounds, anchor);
    match &layer.overlay.place {
        Place::Anchored { side, align, flips_across, offset, clear, .. } => {
            let kept = layer_bounds(layer, bounds);
            let Some(size) = size else {
                let room = content_room(layer.overlay, covers, bounds, kept, anchor, rtl);
                return Placed { at: anchor.left_bottom().clamp(kept.min, kept.max), side: None, asked: None, room };
            };
            // The side taken in an earlier pass is the preferred one while the layer stays open and asks for the same side:
            // it stays while the content fits there or that side is the larger, so a size measured later (a body that
            // grew) still flips it. A host that asks for another side gets that side's placement.
            let taken = overlay::entry(ctx, layer.id).filter(|entry| entry.asked == Some(*side)).and_then(|entry| entry.side).unwrap_or(*side);
            let request = PlaceRequest::new(anchor, size, kept)
                .side(taken)
                .align(*align)
                .rtl(rtl)
                .min_width(layer.overlay.min_width)
                .flips_across(*flips_across)
                .covers_anchor(covers)
                .offset(*offset)
                .clear(*clear);
            let placed = craft_behavior::overlay::place(&request);
            // The room is what the side the layer took leaves for a body as large as the cap, or for a layer that covers
            // its anchor, what the bounds leave.
            let room = craft_behavior::overlay::place(&request.kept(Some(placed.side)).size(cap)).rect.size();
            Placed { at: placed.rect.min, side: Some(placed.side), asked: Some(*side), room }
        }
        Place::Float(float) => {
            let kept = within_bounds(layer.overlay, bounds);
            let size = size.unwrap_or(egui::Vec2::ZERO);
            let min = float.top_left(size).unwrap_or_else(|| kept.center() - size / 2.0);
            let at = crate::contract::body::keep_inside(Rect::from_min_size(min, size), kept).min;
            Placed { at, side: None, asked: None, room: (kept.max - at).max(egui::Vec2::ZERO).min(cap) }
        }
    }
}
