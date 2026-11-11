//! Step 12, state: flags, eased state and the reset a control registers with its label.

use super::paint::surface_ink;
use crate::contract::run::ErasedPass;
use crate::contract::{Component, Cx, Validity};
use crate::runtime::access::scope;
use crate::runtime::motion::{Drives as _, spec};
use crate::style::{Flags, Ink, StateSet, StyleView};
use craft_behavior::Input;
use craft_theme::{State, StateMix};

/// Step 12: a control with a reset value tells its label scope whether its value differs from it (10.2).
pub(crate) fn register_reset<C: Component>(comp: &mut C, cx: &Cx<'_, C>) {
    if matches!(C::INPUT, Input::Text | Input::Editor) {
        return;
    }
    let Some(reset) = comp.bind().and_then(|bind| bind.reset_value().cloned()) else { return };
    let differs = cx.mixed || cx.shown.as_deref() != Some(&reset);
    scope::register_reset(&cx.core.ctx, cx.core.id, differs);
}

/// The eased hover, press and on values of a component, kept in its `Shown` record; a face shows its targets.
fn ease(p: &mut dyn ErasedPass<'_>, targets: [bool; 3]) -> StateMix {
    let [hover, press, on] = targets;
    if p.core().face_mode {
        return StateMix::new(f32::from(u8::from(hover)), f32::from(u8::from(press)), f32::from(u8::from(on)));
    }
    let mut tweens = p.shown_state().tweens.take();
    let [hover, press, on] = p.core().driver().state_anims(&mut tweens, [hover, press, on], [spec::HOVER, spec::HOVER, spec::CHECK]);
    p.shown_state().tweens = tweens;
    StateMix::new(hover, press, on)
}

/// The flags of step 12 that the component's type knows: its own `flags`, CHECKED from a shown `bool`, MIXED from a
/// mixed binding and the binding's validity.
pub(crate) fn value_flags<C: Component>(comp: &C, cx: &Cx<'_, C>) -> Flags {
    let mut flags = comp.flags();
    if cx.shown.as_deref().and_then(|value| (value as &dyn std::any::Any).downcast_ref::<bool>()) == Some(&true) {
        flags |= Flags::CHECKED;
    }
    if cx.mixed {
        flags |= Flags::MIXED;
    }
    match &cx.validity {
        Validity::Refuse(_) => flags |= Flags::INVALID,
        Validity::Warn(_) => flags |= Flags::WARN,
        _ => {}
    }
    flags
}

/// Step 12, state: flags first (`value_flags`, OPEN from a popup shown this pass, EXPANDED from the container's own open body, CHECKED from a command's checked state), then `state`; animates hover, press and on; the ink of the surface, else `inherited`.
pub(crate) fn state(p: &mut dyn ErasedPass<'_>, style: &dyn StyleView, open: bool, expanded: bool, inherited: Ink) {
    let mut flags = p.value_flags();
    let core = p.core();
    if open {
        flags |= Flags::OPEN;
    }
    if expanded {
        flags |= Flags::EXPANDED;
    }
    if core.resolved.read_only {
        flags |= Flags::READ_ONLY;
    }
    if core.resolved.selected == Some(true) {
        flags |= Flags::SELECTED;
    }
    if core.command.as_deref().and_then(|command| command.checked) == Some(true) {
        flags |= Flags::CHECKED;
    }
    if let Some(facts) = core.env.facts_of(core.id) {
        flags |= facts.flags;
    }
    if core.text_invalid {
        flags |= Flags::INVALID;
    }
    core.sync_focus();
    let (hovered, pressed) = (core.response.hovered(), core.press.phase.is_press());
    let (focused, visible) = (core.focus.focused, core.focus.visible);
    if focused {
        flags |= Flags::FOCUSED;
    }
    if visible {
        flags |= Flags::FOCUS_VISIBLE;
    }
    let state = if !core.resolved.enabled {
        State::Disabled
    } else if pressed || flags.contains(Flags::OPEN) {
        State::Pressed
    } else if hovered {
        State::Hover
    } else {
        State::Rest
    };
    p.shown_state().last_state = (hovered, visible);
    let t = ease(p, [hovered, pressed, flags.intersects(Flags::ON)]);
    let core = p.core();
    core.state = StateSet::new(state, flags, t);
    core.ink = surface_ink(style.surface(), &core.state, inherited);
    core.stated = true;
}
