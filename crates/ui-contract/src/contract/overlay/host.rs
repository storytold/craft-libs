//! The overlay host: shows a layer on an egui layer, remembers it while shown and closes its family.

use super::dismissal::dismissal;
use super::opener::Opening;
use super::place::{OverlayFit, Place};
use super::placement::{anchor_rect, opened_overlay, placement};
use super::props::{Outside, Overlay};
use crate::runtime::access::{self};
use crate::runtime::egui::areas::{self, LayerSpec};
use crate::runtime::env::Env;
use crate::runtime::focus::{self, FocusPolicy};
use crate::runtime::layout_memory;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::overlay::{self, Backdrop, CloseReason, OpenEntry, Rank};
use egui::{Context, Id, Pos2, Rect, Ui};

/// What the host reads for one layer.
pub(crate) struct Layer<'a> {
    /// The id of the layer's open state: the popup's or the container's. Its egui layer id derives from it.
    pub id: Id,
    /// The component whose memory stage holds the layer's bookkeeping: the opener or the container.
    pub owner: Id,
    pub overlay: &'a Overlay,
    /// The anchor in screen space.
    pub anchor: Rect,
    /// The opener's env; the layer's body starts from its capture (`Env::capture`).
    pub env: &'a Env,
    /// The layer is fading out.
    pub exiting: bool,
    /// The opacity the open and close motion gives.
    pub fade: f32,
    /// The size the layer's content takes in this pass, when its container measured it before the layer shows;
    /// a layer without one is placed by the size it took last pass.
    pub size: Option<egui::Vec2>,
    /// The screen rect the layer is contained in (see [`region_for`]): placement, scrim, clip and input blocking stop at it.
    pub region: Option<Rect>,
}

/// What the host reports.
pub(crate) struct LayerOut {
    /// Why the layer asks to close, decided after its body ran.
    pub dismissed: Option<CloseReason>,
}

/// What a layer remembers while it is shown, in its owner's memory stage: gone when the layer hides, and fresh in the
/// pass a layer appears (`overlay::appears`). Saved size and position live in the layer's save key, not here.
#[derive(Clone, Copy, Default)]
pub(super) struct LayerMemory {
    /// The pointer position when the layer first appeared, for `Anchor::Pointer`.
    pub(super) pointer_at: Option<Pos2>,
    /// How the layer's opener opened it, decided in the pass the layer appeared.
    pub(super) opening: Option<Opening>,
    /// The stop that was focused when the layer first appeared.
    opener_stop: Option<Option<focus::StopKey>>,
    /// The pass time the layer first appeared, for `Dismiss.timeout`.
    pub(super) since: Option<f64>,
    /// The focus was in the layer last time it was read: `Dismiss.focus_left` fires on a move from inside to outside.
    pub(super) focus_inside: bool,
    /// The layers opened from inside this one have been asked to close.
    family_closed: bool,
    /// The layer's focus scope has run in a pass that showed it: the scope takes its initial focus once per appearance.
    scope_entered: bool,
    /// The least width the layer's body took in the last pass: a content-sized collection in the layer is at least
    /// this wide, so its rows fill the layer.
    min_width: f32,
}

/// The least width the body of the layer `id` takes, as of the last pass; 0 before the layer's first pass.
pub(crate) fn floor_width(ctx: &Context, id: Id) -> f32 {
    MemoryStage::peek::<LayerMemory>(ctx, memory_slot(id)).min_width
}

pub(super) fn memory_slot(id: Id) -> Id {
    sub(id, "layer_memory")
}

fn recall(layer: &Layer<'_>, ctx: &Context) -> LayerMemory {
    MemoryStage::peek::<LayerMemory>(ctx, memory_slot(layer.id))
}

fn remember(layer: &Layer<'_>, ctx: &Context, memory: LayerMemory) {
    let mut stage = MemoryStage::open_owned(ctx, memory_slot(layer.id), layer.owner);
    *stage.shown::<LayerMemory>(memory_slot(layer.id)) = memory;
    stage.write_back();
}

/// The layer `id` is hidden: it forgets what it remembered, so its next appearance starts fresh. Every host caller
/// calls it when the layer's presence is hidden. A measuring run writes nothing; the real run forgets.
pub(crate) fn forget_layer(ctx: &Context, id: Id, owner: Id) {
    if crate::runtime::measure::active(ctx) {
        return;
    }
    let mut stage = MemoryStage::open_owned(ctx, memory_slot(id), owner);
    let taken = stage.take::<LayerMemory>(memory_slot(id));
    stage.write_back();
    if taken.is_some_and(|memory| !memory.family_closed) {
        overlay::close_descendants(ctx, id);
    }
}

/// Asks the layers opened from inside `layer` to close once the layer closes or starts to fade out, in the pass that
/// decided it.
fn close_family(ctx: &Context, layer: &Layer<'_>, closing: bool, memory: &mut LayerMemory) {
    if closing && !memory.family_closed {
        memory.family_closed = true;
        overlay::close_descendants(ctx, layer.id);
    }
}

/// The egui layer id of the layer `id`.
fn area_id(id: Id) -> Id {
    sub(id, "area")
}

/// The overlay host: shows `body` on an egui layer for `layer` and decides its dismissal.
pub(crate) fn host(ctx: &Context, layer: &Layer<'_>, body: &mut dyn FnMut(&mut Ui)) -> LayerOut {
    // A layer that appears starts with nothing it remembered while shown before, also when it opens again during its
    // exit fade or its host never saw it hide: the focus that was inside it does not close it as `focus_left`.
    let mut memory = if overlay::appears(ctx, layer.id, layer.exiting) { LayerMemory::default() } else { recall(layer, ctx) };
    let opened = opened_overlay(ctx, layer.overlay, memory.opening);
    if let Some((_, opening)) = &opened {
        memory.opening.get_or_insert(*opening);
    }
    let layer = &Layer { overlay: opened.as_ref().map_or(layer.overlay, |(overlay, _)| overlay), ..*layer };
    let overlay = layer.overlay;
    let bounds = layer.region.map_or_else(|| areas::safe_rect(ctx), |region| region.intersect(areas::safe_rect(ctx)));
    let anchor = anchor_rect(ctx, layer, bounds, &mut memory);
    let area = area_id(layer.id);
    // A container's layer is placed by the size its container measured for this pass, any other by the size it took last
    // pass. The layout memory entry under `size_id` marks that read, so a change of the size asks for
    // the discard pass that places the layer by its real size, under the rule every measured size follows: no ask when
    // the size also changed in the previous frame. A popup is placed by its anchor and opens from its opener's queued
    // request, which a discard pass would replay.
    let size_id = sub(layer.id, "layer_size");
    let sized = layer.owner == layer.id;
    if sized {
        let _ = layout_memory::read(ctx, size_id);
    }
    let size = layer.size.or_else(|| areas::layer_rect(ctx, area).map(|rect| rect.size()));
    let placed = placement(ctx, layer, size, bounds, anchor);
    let tier = overlay.rank.tier();
    if sized && areas::layer_rect(ctx, area).is_none() {
        areas::seed_area(ctx, areas::layer_of(tier, area), placed.at, placed.room);
    }
    // A modal's scrim; it takes the presses below the modal while the modal is open.
    let cover_id = sub(layer.id, "cover");
    let cover = match overlay.rank {
        Rank::Modal { backdrop } if backdrop != Backdrop::None => {
            let hole = match backdrop {
                Backdrop::ScrimWithHole(hole) => Some(hole),
                _ => None,
            };
            let scrim = crate::style::scrim_color(&layer.env.tokens).gamma_multiply(layer.fade);
            let through = crate::runtime::sampler::through(ctx, crate::runtime::egui::pass::current_viewport(ctx), layer.id);
            areas::show_cover_layer(ctx, areas::CoverLayer { id: cover_id, tier, screen: bounds, scrim, hole, through, absorbs: !layer.exiting });
            Some(areas::layer_of(tier, cover_id))
        }
        _ => None,
    };
    let parent = overlay::drawing_parent(ctx);
    let scope = (overlay.focus != FocusPolicy::NONE).then_some(overlay.focus);
    let min_width = overlay.min_width.points(anchor.width());
    let spec = LayerSpec { id: area, tier, at: placed.at, interactable: overlay.rank != Rank::Tooltip && !layer.exiting };
    // The opener of the layer's scope is the stop focused when the layer first appears: egui's modal gate takes focus
    // from the widgets below a modal layer by the first pass that shows the layer. A dropdown that keeps its anchor's key
    // focus belongs to the component that opened it: a press moves the key focus onto that component only at the end of
    // the frame that opens the layer, so the stop focused then is the previous one.
    let opener = *memory.opener_stop.get_or_insert_with(|| {
        let owned = (overlay.focus.initial == focus::Initial::KeepAnchor).then(|| focus::stop_rec(ctx, (layer.owner, None))).flatten();
        let pressed = overlay.opener.as_ref().filter(|first| first.fired && first.pressed).and_then(|_| focus::pressed_stop(ctx));
        owned.map(|rec| rec.stop.key()).or(pressed).or_else(|| focus::focused_stop(ctx))
    });
    overlay::begin_layer(ctx, layer.id, layer.exiting);
    let shown = areas::show_layer(ctx, &spec, |ui| {
        if let Some(region) = layer.region {
            ui.set_clip_rect(ui.clip_rect().intersect(region));
        }
        areas::fade(ui, layer.fade * overlay.opacity);
        // The layer is not inside its opener's arena, roving group, scroll or row: its content starts from the
        // opener's inherited env with the layer-local fields fresh.
        let installed = Env::install_for(ui, layer.env.capture().env());
        let real = !ui.is_sizing_pass();
        if real && (layer.exiting || matches!(overlay.rank, Rank::Popup | Rank::Modal { .. })) {
            access::report_layer(ctx, layer.id, ui.unique_id(), ui.max_rect(), overlay.rank.is_modal(), layer.exiting);
        }
        if let (Some(policy), true) = (scope, real) {
            let entering = !std::mem::replace(&mut memory.scope_entered, true);
            focus::push_scope_from(ctx, layer.id, policy, None, opener, layer.exiting, Some(entering));
        }
        areas::bound(ui, placed.room, min_width.min(placed.room.x));
        if overlay.fit == OverlayFit::Resizable {
            ui.set_min_size(overlay.size_limits.min.min(placed.room));
        }
        body(ui);
        if scope.is_some() && real {
            focus::pop_scope(ctx);
        }
        Env::restore(ctx, installed);
    });
    overlay::end_layer(ctx);
    if sized {
        layout_memory::record(ctx, size_id, shown.rect.size());
    }
    let mut entry = OpenEntry::new(layer.id, overlay.rank, overlay.keys, shown.rect, shown.layer);
    entry.parent = parent;
    entry.anchor = matches!(overlay.place, Place::Anchored { .. }).then_some(anchor);
    entry.side = placed.side;
    entry.exiting = layer.exiting;
    entry.cover = cover;
    entry.region = layer.region;
    entry.scoped = scope.is_some();
    entry.absorbs = overlay.dismiss.outside == Outside::Absorb;
    overlay::record(ctx, entry);
    memory.min_width = min_width.min(placed.room.x);
    let dismissed = if layer.exiting { None } else { dismissal(ctx, layer, shown.rect, anchor, scope, &mut memory) };
    close_family(ctx, layer, layer.exiting || dismissed.is_some(), &mut memory);
    remember(layer, ctx, memory);
    LayerOut { dismissed }
}

/// The layer-stack side of an OS-window mount: records the window as an entry of its viewport's stack (rank, key
/// class, scope; no egui layer) and decides its dismissal after the body ran. `rect` is the window's inner rect.
pub(crate) fn host_window(ctx: &Context, layer: &Layer<'_>, rect: Rect) -> LayerOut {
    let overlay = layer.overlay;
    let mut memory = recall(layer, ctx);
    let scope = (overlay.focus != FocusPolicy::NONE).then_some(overlay.focus);
    let mut entry = OpenEntry::new(layer.id, overlay.rank, overlay.keys, rect, areas::layer_of(overlay.rank.tier(), area_id(layer.id)));
    entry.scoped = scope.is_some();
    entry.window = true;
    overlay::record(ctx, entry);
    let dismissed = dismissal(ctx, layer, rect, Rect::ZERO, scope, &mut memory);
    close_family(ctx, layer, dismissed.is_some(), &mut memory);
    remember(layer, ctx, memory);
    LayerOut { dismissed }
}

/// Paints the ghost egui layer from runtime data: the drag preview and drop mark, and the painted cursor the cursor
/// service settled on. The end hook calls it once per pass, popup or no popup.
pub fn paint_frame_layers(ctx: &Context) {
    crate::contract::ghost::paint(ctx);
}
