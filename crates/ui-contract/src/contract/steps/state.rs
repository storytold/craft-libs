//! Step 12, state: flags, eased state and the reset a control registers with its label.

use super::paint::surface_ink;
use crate::contract::cx::CxCore;
use crate::contract::{Component, Cx, Validity};
use crate::runtime::access::scope;
use crate::runtime::memory::sub;
use crate::runtime::motion::{Drives as _, spec};
use crate::style::{ComponentStyle, Flags, Ink, StateSet};
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

/// The `(hovered, focus_visible)` a component showed last pass, which `PartsCx::hovered` and `focus_visible` read.
#[derive(Clone, Copy, Default)]
pub(crate) struct LastState(pub bool, pub bool);

/// The eased hover, press and on values of a component; a face shows its targets.
fn ease(core: &CxCore<'_>, targets: [bool; 3]) -> StateMix {
    let [hover, press, on] = targets;
    if core.face_mode {
        return StateMix::new(f32::from(u8::from(hover)), f32::from(u8::from(press)), f32::from(u8::from(on)));
    }
    let driver = core.driver();
    let [hover, press, on] = driver.state_anims([hover, press, on], [spec::HOVER, spec::HOVER, spec::CHECK]);
    StateMix::new(hover, press, on)
}

/// Step 12, state: flags first (OPEN from a popup shown this pass, EXPANDED from the container's own open body; CHECKED from a shown `bool` or a command's checked state, MIXED from a mixed binding), then `state`; animates hover, press and on; the ink of the surface, else `inherited`.
pub(crate) fn state<C: Component>(comp: &C, cx: &mut Cx<'_, C>, open: bool, expanded: bool, inherited: Ink) {
    let mut flags = comp.flags();
    if open {
        flags |= Flags::OPEN;
    }
    if expanded {
        flags |= Flags::EXPANDED;
    }
    if cx.core.resolved.read_only {
        flags |= Flags::READ_ONLY;
    }
    if cx.core.resolved.selected == Some(true) {
        flags |= Flags::SELECTED;
    }
    let shown_on = cx.shown.as_deref().and_then(|value| (value as &dyn std::any::Any).downcast_ref::<bool>()).copied();
    let command_on = cx.core.command.as_deref().and_then(|command| command.checked);
    if shown_on == Some(true) || command_on == Some(true) {
        flags |= Flags::CHECKED;
    }
    if let Some(facts) = cx.core.env.facts_of(cx.core.id) {
        flags |= facts.flags;
    }
    if cx.mixed {
        flags |= Flags::MIXED;
    }
    if cx.core.text_invalid {
        flags |= Flags::INVALID;
    }
    match &cx.validity {
        Validity::Refuse(_) => flags |= Flags::INVALID,
        Validity::Warn(_) => flags |= Flags::WARN,
        _ => {}
    }
    let core = &mut cx.core;
    core.sync_focus();
    let (hovered, pressed) = (core.response.hovered(), core.press.phase.is_press());
    let (focused, visible) = (core.focus.focused, core.focus.visible);
    if focused {
        flags |= Flags::FOCUSED;
    }
    if visible {
        flags |= Flags::FOCUS_VISIBLE;
    }
    let id = core.id;
    *core.memory.shown::<LastState>(sub(id, "last_state")) = LastState(hovered, visible);
    let state = if !core.resolved.enabled {
        State::Disabled
    } else if pressed || flags.contains(Flags::OPEN) {
        State::Pressed
    } else if hovered {
        State::Hover
    } else {
        State::Rest
    };
    let t = ease(core, [hovered, pressed, flags.intersects(Flags::ON)]);
    core.state = StateSet::new(state, flags, t);
    core.ink = surface_ink(cx.style.surface(), &core.state, inherited);
    core.stated = true;
}
