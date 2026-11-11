//! Step 9, update: activation, gestures and edit intents around `C::update`, and what it leaves untaken.

use super::label::scrub_press;
use super::scroll_owner::route_to_scroll_owner;
use crate::contract::cx::CxCore;
use crate::contract::cx::gesture_key;
use crate::contract::{Component, Cx, Resolved, ValueOf};
use crate::runtime::focus::{self};
use crate::runtime::keys;
use craft_behavior::{CancelReason, Gesture, GesturePhase, Input, Intent, PartRef, PressCancel};

/// Whether the component takes activations (a click, a key, AccessKit `Click`): a disabled or read-only one drops them.
pub(crate) fn takes_activation(resolved: &Resolved<'_>) -> bool {
    resolved.enabled && !resolved.read_only
}

/// A key or AccessKit activation in this pass, with the modifiers held in it: the press that routed
/// `Intent::Activate` carries them, and the pass's pointer frame reads them from the same input a pointer activation's
/// modifiers come from (Alt+Enter adds the way Alt+click does).
pub(crate) fn key_activation(ctx: &egui::Context) -> crate::contract::Activation {
    crate::contract::Activation { by_key: true, mods: crate::runtime::pointer::frame(ctx).mods }
}

/// Step 9, update, the part the library writes: the keyboard activation.
pub(crate) fn update_inputs(core: &mut CxCore<'_>) {
    if core.intents.take(craft_behavior::Intent::Activate) && takes_activation(core.resolved) {
        core.activation = Some(key_activation(&core.ctx));
    }
    // An AccessKit click on a part is a click on the component as a pointer click on the part is: the response
    // reports it, and a toggling part or a part reader takes it as it takes the pointer's.
    if core.intents.any_access_click() && takes_activation(core.resolved) {
        core.activation.get_or_insert(crate::contract::Activation { by_key: false, mods: craft_keymap::Mods::NONE });
        core.part_clicked = true;
    }
}

/// Step 9, before the component: a gesture cancelled under its press is live again once the press is released.
pub(crate) fn gesture_gate<C: Component>(comp: &C, cx: &mut Cx<'_, C>) {
    let held = cx.core.press_held() || scrub_press(comp, cx).is_some_and(|press| press.phase.button_down());
    let slot = gesture_key(cx.core.id);
    let gesture = cx.core.memory.shown::<Gesture<ValueOf<C>>>(slot);
    if gesture.dead && !held {
        gesture.dead = false;
    }
}

/// Step 9, the edit intents the library takes before `C::update`: Escape or a cancelled press ends a running gesture,
/// and a reset becomes a `Set` of the binding's reset value.
pub(crate) fn edit_intents<C: Component>(comp: &mut C, cx: &mut Cx<'_, C>) {
    use crate::contract::cx::Edit as _;
    if cx.gesture().is_some_and(|phase| matches!(phase, GesturePhase::Begin | GesturePhase::Update)) {
        let reason = if !cx.core.escape_to_list && cx.core.intents.take(Intent::Cancel) {
            Some(CancelReason::Escape)
        } else {
            cx.core.press_cancelled().map(|reason| match reason {
                PressCancel::Escape => CancelReason::Escape,
                PressCancel::Removed => CancelReason::Removed,
                _ => CancelReason::Blur,
            })
        };
        if let Some(reason) = reason {
            cx.cancel(reason);
        }
    }
    let reset = cx.core.intents.take_reset();
    let double = double_click(&cx.core).filter(|_| !matches!(C::INPUT, Input::Text | Input::Editor));
    if reset.is_some() || double.is_some() {
        let part = match reset {
            Some(whole) => cx.core.active.filter(|_| !whole),
            None => double.flatten(),
        };
        if let Some(part) = part.filter(|part| cx.core.own_reset_parts.contains(&(part.key, part.index))) {
            cx.core.reset_part = Some(part);
        } else if let Some(reset) = comp.bind().and_then(|bind| bind.reset_value().cloned()) {
            cx.set(reset);
        }
    }
}

/// A second press of a double click went down on the component's root (`Some(None)`) or one of its value parts
/// (`Some(Some(part))`) this pass (10.2).
fn double_click(core: &CxCore<'_>) -> Option<Option<PartRef>> {
    if core.press.double_pressed() {
        return Some(None);
    }
    core.value_parts
        .iter()
        .find(|key| core.results.get(key).is_some_and(|result| result.press.double_pressed()))
        .map(|&(key, index)| Some(PartRef { key, index }))
}

/// Step 9, after `update`: a secondary release on the component, or a `ContextMenu` intent it did not take, records a
/// context request that the overlay step and the popups shown after it read in this pass (9.2 rule 3).
pub(crate) fn record_context(core: &CxCore<'_>) {
    if core.intents.untaken(&Intent::ContextMenu)
        || (core.response.hovered() && crate::runtime::pointer::frame(&core.ctx).release == Some(craft_behavior::PointerButton::Secondary))
    {
        let part = focus::active_part(&core.ctx, core.id).and_then(|part| core.results.get(&(part.key, part.index)));
        keys::request_context(&core.ctx, core.id, part.map_or(core.rect, |part| part.rect));
    }
}

/// Writes back what the component left untaken, and its state-dependent claims for the next frames.
pub(crate) fn after_update(core: &mut CxCore<'_>) {
    route_to_scroll_owner(core);
    keys::settle(&core.ctx, Some(core.id), &core.intents);
    for chord in std::mem::take(&mut core.claims) {
        keys::claim(&core.ctx, core.id, chord);
    }
}
