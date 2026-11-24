//! Step 8, sense: part states from the sensing core and the wheel takers.

use crate::runtime::env::WheelScope;
use crate::runtime::wheel;
use crate::style::{Flags, StateSet};
use craft_theme::{State, StateMix};
use egui::{Context, Id};

/// Registers a wheel taker for next pass's split (8.6): `sensed`, the taker the wheel bits of the component's sense
/// make (`WheelTaker::for_sense`). A component that senses no wheel bit takes no wheel, value control or not: the wheel
/// over a focused check box or text field scrolls the region around it, as native controls do, and a multi-line text
/// part whose text overflows registers its own scroll taker (`text_adapter`), which passes the wheel on at its ends.
/// Every value control that takes the wheel is a `WheelTaker::value` taker, in any scope: a wheel gesture it owns ends
/// with a `WheelPhase::End` share. A value control with a text part that steps by the wheel (a number field) is a field
/// (`WheelTaker::field`), which takes the plain wheel while focused wherever the pointer is. A value control takes the
/// wheel only while focused or armed, on the axes it senses; inside a `WheelScope::Free` scope one that senses the
/// wheel takes it without focus, unless it senses `Sense::WHEEL_Y_FOCUSED`, which no scope frees (a range whose wheel
/// steps its focused thumb). Inside a `WheelScope::Gated` scope every taker waits for focus or arming. A zoom zone that
/// takes no axis itself waits only for a gated scope: its wheel goes to the view around it.
pub(crate) fn register_wheel(ctx: &Context, sensed: Option<craft_behavior::WheelTaker<Id>>, value_control: bool, text: bool, scope: WheelScope) {
    let Some(taker) = sensed else { return };
    let free = !taker.gated && scope == WheelScope::Free;
    let zone = taker.zooms_view && !(taker.horizontal || taker.vertical || taker.zoom);
    wheel::register(
        ctx,
        taker.value(value_control).field(value_control && text).gated(taker.gated || scope == WheelScope::Gated || (value_control && !free && !zone)),
    );
}

/// The part states: each part's press and hover from the sensing core, with its own flags.
pub(crate) fn part_state(result: &crate::primitives::PartResult, enabled: bool, focus_visible: Option<bool>) -> StateSet {
    let mut flags = result.flags;
    if let Some(visible) = focus_visible {
        flags |= Flags::FOCUSED;
        if visible {
            flags |= Flags::FOCUS_VISIBLE;
        }
    }
    let pressed = result.pressed();
    let state = if !enabled || result.inactive {
        State::Disabled
    } else if pressed {
        State::Pressed
    } else if result.hovered {
        State::Hover
    } else {
        State::Rest
    };
    let on = flags.intersects(Flags::ON);
    StateSet::new(state, flags, StateMix::new(f32::from(u8::from(result.hovered)), f32::from(u8::from(pressed)), f32::from(u8::from(on))))
}
