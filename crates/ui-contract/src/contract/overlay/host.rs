//! The overlay host: shows a layer on an egui layer, remembers it while shown and closes its family.

use super::dismissal::{dismissal, popup_dismissal};
use super::opener::{Opening, Trigger};
use super::place::{OverlayFit, Place};
use super::placement::{anchor_rect, layer_covers_anchor, opened_overlay, placement};
use super::props::{Outside, Overlay};
use crate::runtime::access::{self};
use crate::runtime::egui::areas::{self, LayerSpec};
use crate::runtime::env::Env;
use crate::runtime::focus::{self, FocusPolicy};
use crate::runtime::layout_memory;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::motion::{self, Driver};
use crate::runtime::overlay::{self, Backdrop, CloseReason, OpenEntry, Rank};
use crate::runtime::windows::{self, PopupOpen, PopupRoom, WindowSpec};
use craft_behavior::{MotionKind, Playback};
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};

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
    /// The size the layer's content takes in this pass, as its container measured it before the layer shows. A
    /// toast or a tooltip, which has none, is placed by the size it took last pass.
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
}

pub(super) fn memory_slot(id: Id) -> Id {
    sub(id, "layer_memory")
}

fn recall(layer: &Layer<'_>, ctx: &Context) -> LayerMemory {
    MemoryStage::peek::<LayerMemory>(ctx, memory_slot(layer.id))
}

/// What the layer remembers as its host's pass starts. A layer that `appears` starts with nothing it remembered while
/// shown before, also when it opens again during its exit fade or its host never saw it hide: the focus that was inside
/// it does not close it as `focus_left`. A right-click on a context opener while its layer is open hangs the layer at
/// the new pointer.
fn recall_shown(layer: &Layer<'_>, ctx: &Context, appears: bool) -> LayerMemory {
    if appears {
        return LayerMemory::default();
    }
    let mut memory = recall(layer, ctx);
    let pointed = layer
        .overlay
        .opener
        .as_ref()
        .is_some_and(|first| first.all().any(|opener| opener.fired && opener.trigger == Trigger::Context && opener.opening == Opening::Pointer));
    if pointed {
        memory.opening = None;
        memory.pointer_at = None;
    }
    memory
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
    windows::close_popup(ctx, id);
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

/// The overlay host: shows `body` on an egui layer for `layer` and decides its dismissal. A layer opened in a palette
/// window shows in a popup window of its own ([`host_in_popup`]); a full-window layer covers its own window.
pub(crate) fn host(ctx: &Context, layer: &Layer<'_>, body: &mut dyn FnMut(&mut Ui)) -> LayerOut {
    if !layer.overlay.is_full_window()
        && let Some(popup) = windows::popup_room(ctx)
    {
        return host_in_popup(ctx, layer, popup, body);
    }
    let appears = overlay::appears(ctx, layer.id, layer.exiting);
    let mut memory = recall_shown(layer, ctx, appears);
    let opened = opened_overlay(ctx, layer.overlay, memory.opening);
    if let Some((_, opening)) = &opened {
        memory.opening.get_or_insert(*opening);
    }
    let covers = layer_covers_anchor(ctx, layer.overlay, memory.opening);
    let layer = &Layer { overlay: opened.as_ref().map_or(layer.overlay, |(overlay, _)| overlay), ..*layer };
    let overlay = layer.overlay;
    let bounds = layer.region.map_or_else(|| areas::safe_rect(ctx), |region| region.intersect(areas::safe_rect(ctx)));
    let anchor = anchor_rect(ctx, layer, bounds, &mut memory);
    let area = area_id(layer.id);
    let min_width = overlay.least_width(anchor.width());
    // A container's layer is placed by the size its container measured for this pass, at least its least width, so
    // its shown size is already the one it is placed by. A toast or a tooltip is placed by the size it took last pass:
    // for such a layer that owns its memory, the layout memory entry under `size_id` marks that read, so a change of
    // the shown size asks for the discard pass that places the layer by it, under the rule every measured size
    // follows: no ask when the size also changed in the previous frame. A popup's layer, whose memory its opener owns,
    // asks none: it opens from its opener's queued request, which a discard pass would replay.
    let size_id = sub(layer.id, "layer_size");
    let sized = layer.owner == layer.id && layer.size.is_none();
    if sized {
        let _ = layout_memory::read(ctx, size_id);
    }
    let size = match layer.size {
        Some(size) => Some(egui::vec2(size.x.max(min_width), size.y)),
        None => areas::layer_rect(ctx, area).map(|rect| rect.size()),
    };
    let mut placed = placement(ctx, layer, covers, size, bounds, anchor);
    let nudge = Nudge::of(ctx, layer, appears);
    if let Some(nudge) = &nudge {
        placed.at += nudge.offset();
    }
    let tier = overlay::layer_tier(ctx, overlay.rank);
    if (sized || layer.size.is_some()) && areas::layer_rect(ctx, area).is_none() {
        areas::seed_area(ctx, areas::layer_of(tier, area), placed.at, placed.room);
    }
    // A modal's cover: it takes the presses below the modal while the modal is open, and paints the scrim if it has one.
    let cover_id = sub(layer.id, "cover");
    let cover = match overlay.rank {
        Rank::Modal { backdrop } => {
            let scrim = match backdrop {
                Backdrop::Scrim => crate::style::scrim_color(&layer.env.tokens).gamma_multiply(layer.fade),
                Backdrop::None => egui::Color32::TRANSPARENT,
            };
            let through = crate::runtime::sampler::through(ctx, crate::runtime::egui::pass::current_viewport(ctx), layer.id);
            areas::show_cover_layer(ctx, areas::CoverLayer { id: cover_id, tier, screen: bounds, scrim, through, absorbs: !layer.exiting });
            Some(areas::layer_of(tier, cover_id))
        }
        _ => None,
    };
    let parent = overlay::drawing_parent(ctx);
    let scope = (overlay.focus != FocusPolicy::NONE).then_some(overlay.focus);
    let mut spec = LayerSpec { id: area, tier, at: placed.at, interactable: overlay.rank != Rank::Tooltip && !layer.exiting, root: None };
    // A popup's or a modal's layer, and any layer while it fades out, has a root node, made before its `Area` shows so
    // egui hangs the layer's content below it. A sizing pass of the `Area` reports nothing.
    if layer.exiting || matches!(overlay.rank, Rank::Popup | Rank::Modal { .. }) {
        spec.root = areas::content_rect(ctx, &spec).map(|rect| access::report_layer(ctx, layer.id, rect, layer.exiting));
    }
    // The opener of the layer's scope is the stop focused when the layer first appears: egui's modal gate takes focus
    // from the widgets below a modal layer by the first pass that shows the layer. A dropdown that keeps its anchor's key
    // focus belongs to the component that opened it: a press moves the key focus onto that component only at the end of
    // the frame that opens the layer, so the stop focused then is the previous one. A layer the app opens from a recorded
    // opener (a button whose click the app answers) returns focus to that opener's stop.
    let opener = *memory.opener_stop.get_or_insert_with(|| {
        let owned = (overlay.focus.initial == focus::Initial::KeepAnchor).then(|| focus::stop_rec(ctx, (layer.owner, None))).flatten();
        let pressed = overlay.opener.as_ref().filter(|first| first.fired && first.pressed).and_then(|_| focus::pressed_stop(ctx));
        let recorded = overlay.opener.as_ref().and_then(|first| focus::stop_rec(ctx, (first.id, None))).map(|rec| rec.stop.key());
        owned.map(|rec| rec.stop.key()).or(pressed).or(recorded).or_else(|| focus::focused_stop(ctx))
    });
    // A modal closes the popups beneath it in the pass it first shows; `appears` is false in a discard pass that repeats
    // the frame, so the close is queued once. A popup opened above it later is not touched.
    if overlay.rank.is_modal() && appears {
        overlay::close_popups_outside(ctx, &[layer.id]);
    }
    overlay::begin_layer(ctx, layer.id, tier, layer.exiting, placed.side);
    let shown = areas::show_layer(ctx, &spec, |ui| {
        if let Some(region) = layer.region {
            ui.set_clip_rect(ui.clip_rect().intersect(region));
        }
        areas::fade(ui, layer.fade * overlay.opacity);
        // The layer is not inside its opener's arena, roving group, scroll, row or label scope: its content starts
        // from the opener's inherited env with the layer-local fields fresh, and outside the opener's scopes.
        let installed = Env::install_for(ui, layer.env.capture().env());
        let real = !ui.is_sizing_pass();
        if spec.root.is_some() && layer.exiting {
            access::hide_exiting_content(ctx, ui.unique_id());
        }
        // Every layer is a focus scope, one without a policy (`FocusPolicy::NONE`) included: its stops are the layer's,
        // not those of the scope it was opened in, so a trap around it leaves them out (a menu's companion bar).
        if real {
            let entering = !std::mem::replace(&mut memory.scope_entered, true);
            focus::push_scope_from(ctx, layer.id, overlay.focus, None, opener, layer.exiting, Some(entering));
        }
        areas::bound(ui, placed.room, min_width.min(placed.room.x));
        if overlay.fit == OverlayFit::Resizable {
            ui.set_min_size(overlay.size_limits.min.min(placed.room));
        }
        // The layer's content is not part of the flow of the body it was shown from (a tooltip shown at step 16 of a
        // control inside a body's content): no enclosing transcript takes its measure calls and no enclosing body
        // checks them. Without the boundary a tooltip's text was checked against the body's kept transcript, which
        // asked for a discard pass every other frame while the tooltip showed.
        access::scope::outside(ctx, || layout_memory::boundary(ctx, true, || body(ui)));
        if real && overlay.rank.is_modal() {
            access::report_modal(ctx, layer.id);
        }
        if real {
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
    entry.asked = placed.asked;
    entry.exiting = layer.exiting;
    entry.cover = cover;
    entry.region = layer.region;
    entry.scoped = scope.is_some();
    entry.absorbs = overlay.dismiss.outside == Outside::Absorb;
    entry.closes_on_commands = overlay.dismiss.commands;
    overlay::record(ctx, entry);
    let dismissed = if layer.exiting { None } else { dismissal(ctx, layer, shown.rect, anchor, scope, &mut memory) };
    // The layer closes after its body ran: the run shows it open, so a record of the run is not replayed.
    if dismissed.is_some() {
        crate::runtime::replay::note_unsettled(ctx);
    }
    if let Some(nudge) = &nudge {
        // `dismissal` above records the press beside the layer (`dismissal.rs`, `note_pressed_outside`), so the nudge
        // starts after it and moves the layer from the next pass on.
        nudge.start_on_press(ctx, layer);
        nudge.flash(ctx, &shown, layer.env);
    }
    close_family(ctx, layer, layer.exiting || dismissed.is_some(), &mut memory);
    remember(layer, ctx, memory);
    LayerOut { dismissed }
}

/// A layer opened in a palette window, shown in a popup window of its own (`WindowSpec::popup`): placed in the
/// opener's points within the work area around it, the window at that place in desktop points.
///
/// The layer's bookkeeping is split between the two windows. The popup's pass shows the layer at the window's origin
/// and records it in the popup's stack, with its focus scope and the dismissal its own input decides (Escape, Enter,
/// activations, focus, timeout). The opener's pass records an entry that stands for the window (`OpenEntry::popup`),
/// so its family, its anchor and the presses in the opener are judged where they happen; a press in any other window
/// of the app, and for a popup the app losing focus, close it too ([`popup_dismissal`]).
///
/// A layer whose size its container measured is placed before the window opens. A layer without one (a tooltip) is
/// laid out in the popup's pass first, in the room its unplaced form has, and placed by the size it took there.
/// A popup that takes keys (a menu, a flyout) takes the OS focus; a tooltip takes none and lets the pointer through.
fn host_in_popup(ctx: &Context, layer: &Layer<'_>, popup: PopupRoom, body: &mut dyn FnMut(&mut Ui)) -> LayerOut {
    let appears = overlay::note_appeared(ctx, layer.id, layer.exiting);
    let mut memory = recall_shown(layer, ctx, appears);
    let opened = opened_overlay(ctx, layer.overlay, memory.opening);
    if let Some((_, opening)) = &opened {
        memory.opening.get_or_insert(*opening);
    }
    let covers = layer_covers_anchor(ctx, layer.overlay, memory.opening);
    // The popup window lies outside any region of the opener: the work area is its room.
    let layer = &Layer { overlay: opened.as_ref().map_or(layer.overlay, |(overlay, _)| overlay), region: None, ..*layer };
    let overlay = layer.overlay;
    let bounds = popup.room;
    let anchor = anchor_rect(ctx, layer, bounds, &mut memory);
    let min_width = overlay.least_width(anchor.width());
    let measured = layer.size.map(|size| egui::vec2(size.x.max(min_width), size.y));
    let unplaced = placement(ctx, layer, covers, measured, bounds, anchor);
    let parent = overlay::drawing_parent(ctx);
    let tier = overlay::layer_tier(ctx, overlay.rank);
    let scope = (overlay.focus != FocusPolicy::NONE).then_some(overlay.focus);
    if overlay.rank.is_modal() && appears {
        overlay::close_popups_outside(ctx, &[layer.id]);
    }
    // The body reports in its parent's space: the popup's points are the opener's screen less the layer's place.
    let to_parent = overlay::parent_space(ctx, layer.id);
    let tooltip = overlay.rank == Rank::Tooltip;
    let spec = WindowSpec::popup().takes_focus(!tooltip && (scope.is_some() || overlay.rank.takes_keys())).passes_pointer(tooltip);
    let known = measured.map(|size| Rect::from_min_size(unplaced.at, size));
    let open = PopupOpen { id: layer.id, spec: &spec, rect: known.map(|rect| rect.translate(popup.origin.to_vec2())) };
    let mut shown = None;
    windows::show_popup(ctx, &open, |ui| {
        let at = unplaced.at.to_vec2();
        overlay::note_parent_space(ui.ctx(), layer.id, egui::emath::TSTransform::from_translation(-at) * to_parent);
        shown = Some(show_in_popup(ui, layer, &unplaced, min_width, scope, &mut memory, &mut *body));
    });
    let (size, dismissed) = shown.unwrap_or((measured.unwrap_or_default(), None));
    let placed = if known.is_some() { unplaced } else { placement(ctx, layer, covers, Some(size), bounds, anchor) };
    let rect = Rect::from_min_size(placed.at, size);
    if known.is_none() {
        windows::place_popup(ctx, layer.id, rect.translate(popup.origin.to_vec2()));
    }
    let mut entry = OpenEntry::new(layer.id, overlay.rank, overlay.keys, rect, areas::layer_of(tier, area_id(layer.id)));
    entry.parent = parent;
    entry.anchor = matches!(overlay.place, Place::Anchored { .. }).then_some(anchor);
    entry.side = placed.side;
    entry.asked = placed.asked;
    entry.exiting = layer.exiting;
    entry.scoped = scope.is_some();
    entry.absorbs = overlay.dismiss.outside == Outside::Absorb;
    entry.closes_on_commands = overlay.dismiss.commands;
    entry.window = true;
    entry.popup = true;
    overlay::record(ctx, entry);
    let viewport = crate::runtime::egui::viewports::viewport_id(layer.id);
    let dismissed = if layer.exiting { None } else { dismissed.or_else(|| popup_dismissal(ctx, layer, rect, anchor, viewport)) };
    close_family(ctx, layer, layer.exiting || dismissed.is_some(), &mut memory);
    remember(layer, ctx, memory);
    LayerOut { dismissed }
}

/// The popup window's pass of [`host_in_popup`]: the layer at the window's origin, offered the room `placed` leaves,
/// its entry in the popup's stack and the dismissal the popup's own input decides. Returns the size the layer took
/// and that dismissal.
fn show_in_popup(
    ui: &mut Ui,
    layer: &Layer<'_>,
    placed: &super::placement::Placed,
    min_width: f32,
    scope: Option<FocusPolicy>,
    memory: &mut LayerMemory,
    body: &mut dyn FnMut(&mut Ui),
) -> (Vec2, Option<CloseReason>) {
    let ctx = ui.ctx().clone();
    let overlay = layer.overlay;
    let window = ctx.content_rect();
    let area = area_id(layer.id);
    let tier = overlay::layer_tier(&ctx, overlay.rank);
    let cover = match overlay.rank {
        Rank::Modal { backdrop } => {
            let scrim = match backdrop {
                Backdrop::Scrim => crate::style::scrim_color(&layer.env.tokens).gamma_multiply(layer.fade),
                Backdrop::None => egui::Color32::TRANSPARENT,
            };
            let cover_id = sub(layer.id, "cover");
            let through = crate::runtime::sampler::through(&ctx, crate::runtime::egui::pass::current_viewport(&ctx), layer.id);
            areas::show_cover_layer(&ctx, areas::CoverLayer { id: cover_id, tier, screen: window, scrim, through, absorbs: !layer.exiting });
            Some(areas::layer_of(tier, cover_id))
        }
        _ => None,
    };
    // The layer's content starts from its opener's env, on the popup window's own root: its scale and window facts.
    let opener = layer.env.capture().env();
    let env = Env::root_of(&ctx).map_or_else(|| opener.clone(), |root| opener.rebase(&root));
    let mut spec = LayerSpec { id: area, tier, at: window.min, interactable: overlay.rank != Rank::Tooltip && !layer.exiting, root: None };
    // The layer's root node, made before its `Area` shows, as in `host`.
    if layer.exiting || matches!(overlay.rank, Rank::Popup | Rank::Modal { .. }) {
        spec.root = areas::content_rect(&ctx, &spec).map(|rect| access::report_layer(&ctx, layer.id, rect, layer.exiting));
    }
    overlay::begin_layer(&ctx, layer.id, tier, layer.exiting, placed.side);
    let shown = areas::show_layer(&ctx, &spec, |ui| {
        areas::fade(ui, layer.fade * overlay.opacity);
        let installed = Env::install_for(ui, env.clone());
        let real = !ui.is_sizing_pass();
        if spec.root.is_some() && layer.exiting {
            access::hide_exiting_content(&ctx, ui.unique_id());
        }
        // The focused stop the scope returns to is in the opener's window; the opener's window takes the OS focus
        // back when the popup closes (`windows::close_popup`).
        if real {
            let entering = !std::mem::replace(&mut memory.scope_entered, true);
            focus::push_scope_from(&ctx, layer.id, overlay.focus, None, None, layer.exiting, Some(entering));
        }
        areas::bound(ui, placed.room, min_width.min(placed.room.x));
        if overlay.fit == OverlayFit::Resizable {
            ui.set_min_size(overlay.size_limits.min.min(placed.room));
        }
        access::scope::outside(&ctx, || layout_memory::boundary(&ctx, true, || body(ui)));
        if real && overlay.rank.is_modal() {
            access::report_modal(&ctx, layer.id);
        }
        if real {
            focus::pop_scope(&ctx);
        }
        Env::restore(&ctx, installed);
    });
    overlay::end_layer(&ctx);
    let mut entry = OpenEntry::new(layer.id, overlay.rank, overlay.keys, shown.rect, shown.layer);
    entry.side = placed.side;
    entry.asked = placed.asked;
    entry.exiting = layer.exiting;
    entry.cover = cover;
    entry.scoped = scope.is_some();
    overlay::record(&ctx, entry);
    let dismissed = if layer.exiting { None } else { dismissal(&ctx, layer, shown.rect, Rect::NOTHING, scope, memory) };
    (shown.rect.size(), dismissed)
}

/// The motion slot of a layer's nudge, under the layer's id.
// PLAN: the decision names a `NUDGE_SPEC`; `MotionSpec::new` (`runtime/motion.rs:39`) is a private `const fn` of a file outside this set, so
// the nudge runs on `spec::OPEN_FADE` (a fade, which reduced motion still runs) over the open token's duration.
const NUDGE: &str = "nudge";

/// The swings of a nudge's shake either way, dying away.
const NUDGE_SWINGS: f32 = 3.0;

/// The edge flashes of a nudge under reduced motion.
const NUDGE_FLASHES: f32 = 2.0;

/// A press beside a modal layer that stays open (`Outside::Keep`) nudges it, so the user sees why nothing behind it
/// reacts: a short damped shake, or two accent edge flashes when the motion policy snaps travel. `left` runs from 1 at
/// the press to 0 at rest on the motion clock, in the layer's motion memory, which goes with the layer; a layer that
/// appears starts at rest. With motion off the fade the nudge steps by snaps too, so nothing shows.
struct Nudge {
    /// How much of the nudge is left: 1 at the press, 0 at rest.
    left: f32,
    /// The layer shakes; otherwise its edge flashes.
    shakes: bool,
    /// The widest swing of the shake: the gap between neighbouring controls.
    amplitude: f32,
}

impl Nudge {
    /// The nudge of `layer` in this pass, stepped on the pass clock; `None` for a layer that never nudges. A second
    /// pass of one frame reads the same clock, so it repeats the first pass's value.
    fn of(ctx: &Context, layer: &Layer<'_>, appears: bool) -> Option<Nudge> {
        if !layer.overlay.rank.is_modal() || layer.overlay.dismiss.outside != Outside::Keep || layer.exiting {
            return None;
        }
        let policy = layer.env.motion;
        let stage = MemoryStage::open_owned(ctx, sub(layer.id, NUDGE), layer.owner);
        let left = Nudge::driven(stage, layer, |driver| {
            if appears {
                driver.jump(NUDGE, 0.0_f32);
            }
            driver.anim(NUDGE, 0.0_f32, motion::spec::OPEN_FADE)
        });
        let shakes = policy.mode.playback(MotionKind::Travel) == Playback::Run;
        Some(Nudge { left, shakes, amplitude: layer.env.tokens.space.gap.item })
    }

    /// Runs `f` with the motion driver of `layer`'s nudge in `stage`, which its owner keeps, and writes the stage back.
    fn driven<R>(stage: MemoryStage, layer: &Layer<'_>, f: impl FnOnce(&Driver<'_>) -> R) -> R {
        let result = f(&Driver { stage: &stage, base: layer.id, policy: layer.env.motion });
        stage.write_back();
        result
    }

    /// How far through the nudge it is, 0 at the press to 1 at rest.
    fn through(&self) -> f32 {
        1.0 - self.left.clamp(0.0, 1.0)
    }

    /// The layer's sideways offset: the shake's swing, none at rest and none when the edge flashes instead.
    fn offset(&self) -> Vec2 {
        if !self.shakes || self.left <= 0.0 {
            return Vec2::ZERO;
        }
        let swing = (self.through() * NUDGE_SWINGS * std::f32::consts::TAU).sin() * self.left;
        Vec2::new(self.amplitude * swing, 0.0)
    }

    /// Starts the nudge when a press landed beside the layer in this pass and none runs; a press while one runs leaves
    /// it alone. A later pass of the frame gets no press (`pressed_outside` is per pass), so it starts nothing.
    fn start_on_press(&self, ctx: &Context, layer: &Layer<'_>) {
        if self.left > 0.0 || !overlay::pressed_outside(ctx, layer.id) {
            return;
        }
        // `of` claimed the nudge's stage in this pass and wrote it back, so this one claims nothing.
        let stage = MemoryStage::open_after(ctx, sub(layer.id, NUDGE), layer.owner);
        Nudge::driven(stage, layer, |driver| {
            driver.jump(NUDGE, 1.0_f32);
            // Aims it at rest from this pass's clock, so it runs and asks for the passes that step it.
            driver.anim(NUDGE, 0.0_f32, motion::spec::OPEN_FADE);
        });
    }

    /// Paints the edge flash on the layer's rect while the nudge runs without the shake: on in the first and third
    /// quarters, so it flashes twice.
    fn flash(&self, ctx: &Context, shown: &areas::LayerShown, env: &Env) {
        if self.shakes || self.left <= 0.0 || (self.through() * NUDGE_FLASHES * 2.0).floor() % 2.0 != 0.0 {
            return;
        }
        let stroke = egui::Stroke::new(env.tokens.stroke.medium, crate::style::rgba_color(env.tokens.color.accent.solid));
        let radius = egui::CornerRadius::from(env.tokens.shape.radius.window);
        ctx.layer_painter(shown.layer).rect_stroke(shown.rect, radius, stroke, egui::StrokeKind::Inside);
    }
}

/// The layer-stack side of an OS-window mount: records the window as an entry of its viewport's stack (rank, key
/// class, scope; no egui layer) and decides its dismissal after the body ran. `rect` is the window's inner rect.
pub(crate) fn host_window(ctx: &Context, layer: &Layer<'_>, rect: Rect) -> LayerOut {
    // The entry stands for a window of another viewport, whose pass no record of this one holds.
    crate::runtime::replay::block(ctx);
    let overlay = layer.overlay;
    let mut memory = recall(layer, ctx);
    let scope = (overlay.focus != FocusPolicy::NONE).then_some(overlay.focus);
    let mut entry =
        OpenEntry::new(layer.id, overlay.rank, overlay.keys, rect, areas::layer_of(overlay::layer_tier(ctx, overlay.rank), area_id(layer.id)));
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
