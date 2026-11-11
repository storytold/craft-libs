//! When a layer closes by itself (15.4): presses outside, Escape, and activations that keep it open.

use super::host::{Layer, LayerMemory};
use super::props::Outside;
use crate::contract::{Component, Cx};
use crate::runtime::commands::FireSource;
use crate::runtime::focus::{self, FocusPolicy, Initial};
use crate::runtime::overlay::{self, CloseReason, Rank};
use crate::runtime::{frame, keys, pointer};
use craft_behavior::CancelRule;
use craft_keymap::Mods;
use egui::{Context, Rect};

/// The dismissal rules of 15.4, decided after the body ran so a field inside still commits in the press pass.
pub(super) fn dismissal(
    ctx: &Context,
    layer: &Layer<'_>,
    rect: Rect,
    anchor: Rect,
    scope: Option<FocusPolicy>,
    memory: &mut LayerMemory,
) -> Option<CloseReason> {
    let dismiss = layer.overlay.dismiss;
    // Both the Escape rule and the focus rule read the same fact: whether the focus is in the layer.
    let inside = focus::focus_in_layer(ctx, layer.id);
    let was_inside = memory.focus_inside;
    if let Some(now) = inside {
        memory.focus_inside = now;
    }
    if let Some(reason) = escape(ctx, layer, scope, inside) {
        return Some(reason);
    }
    if dismiss.anchor_gone && layer.overlay.opener.as_ref().is_some_and(|first| !first.active(ctx, None).0.enabled) {
        return Some(CloseReason::AnchorGone);
    }
    // The input hook removed the press and recorded it; presses inside the family and its anchors never get here.
    if dismiss.outside == Outside::Absorb {
        if overlay::absorbed_press(ctx).is_some() {
            return Some(CloseReason::Outside);
        }
    } else if press_outside(ctx, layer, rect, anchor) {
        if dismiss.outside == Outside::Close {
            return Some(CloseReason::Outside);
        }
        overlay::note_pressed_outside(ctx, layer.id);
    }
    if dismiss.on_activate && overlay::activated_inside(ctx, layer.id) {
        // An activation closes the menus as a row's run does, and focus leaves them the same way.
        focus::return_from_menus(ctx);
        return Some(CloseReason::Activated);
    }
    if dismiss.enter && scope.is_some() && overlay::top(ctx) == Some(layer.id) && keys::take_submit(ctx) {
        return Some(CloseReason::Enter);
    }
    if dismiss.focus_left
        && let Some(policy) = scope
        && policy.kind != craft_behavior::ScopeKind::Group
        && was_inside
        && inside == Some(false)
        && overlay::is_open(ctx, layer.id)
    {
        return Some(CloseReason::FocusLeft);
    }
    if let Some(timeout) = dismiss.timeout {
        let now = frame::clock(ctx);
        let since = *memory.since.get_or_insert(now);
        let due = since + f64::from(timeout);
        if now >= due {
            return Some(CloseReason::Timeout);
        }
        crate::runtime::egui::repaint::request_after(ctx, crate::runtime::egui::pass::current_viewport(ctx), due - now);
    }
    None
}

/// Whether a primary press landed outside the layer, its anchor and the layers opened from inside it.
fn press_outside(ctx: &Context, layer: &Layer<'_>, rect: Rect, anchor: Rect) -> bool {
    let Some(press) = pointer::frame(ctx).press.filter(|press| press.button == craft_behavior::PointerButton::Primary).map(|press| press.origin)
    else {
        return false;
    };
    let inside_family = overlay::family(ctx, layer.id).iter().filter_map(|id| overlay::entry(ctx, *id)).any(|entry| entry.rect.contains(press));
    !rect.contains(press) && !anchor.contains(press) && !inside_family
}

/// Whether the focused stop sits in no scope that belongs to a shown layer.
fn focus_in_no_layer(ctx: &Context) -> bool {
    focus::focused_rec(ctx).is_none_or(|rec| focus::scope_chain(ctx, &rec).iter().all(|scope| overlay::entry(ctx, *scope).is_none()))
}

/// Escape reaches only the top layer, and only when focus is inside its scope (a modal also when it is in no layer):
/// a component inside that takes the Escape (a draft, a gesture) keeps it, and a captured press goes first.
fn escape(ctx: &Context, layer: &Layer<'_>, scope: Option<FocusPolicy>, inside: Option<bool>) -> Option<CloseReason> {
    let policy = scope?;
    if policy.cancel == CancelRule::Ignore || overlay::top(ctx) != Some(layer.id) {
        return None;
    }
    // A modal on top also owns Escape while the focus is in no layer (nothing focused, or a stop of the window
    // below): its card may have nothing to focus. So does a popup, whose body may have nothing to focus, and a
    // keep-anchor popup, which leaves the focus where it was.
    let owns_loose = layer.overlay.rank.is_modal() || layer.overlay.rank == Rank::Popup || policy.initial == Initial::KeepAnchor;
    if inside != Some(true) && !(owns_loose && focus_in_no_layer(ctx)) {
        return None;
    }
    let step = keys::scope_escape(ctx, policy.cancel, pointer::press_captured(ctx), false)?;
    matches!(step, craft_behavior::focus::EscapeStep::CloseScope).then_some(CloseReason::Escape)
}

impl<C: Component> Cx<'_, C> {
    /// This activation runs but does not close the layer chain it happened in (`Dismiss::on_activate`).
    pub fn keep_open(&mut self) {
        self.core.keep_open = true;
    }

    /// Injects an activation for this component, from `source`: the pipeline runs its bound command with that
    /// source, reports it to `Component::activated` readers and closes the layer chain as for a click.
    pub fn activate(&mut self, source: FireSource) {
        self.core.injected = Some(source);
    }

    /// The modifiers held this frame, Alt alone included.
    pub fn modifiers(&self) -> Mods {
        pointer::frame(&self.core.ctx).mods
    }
}
