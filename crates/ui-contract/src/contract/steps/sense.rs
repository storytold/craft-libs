//! Step 8, sense: part states from the sensing core and the wheel takers.

use crate::runtime::env::WheelScope;
use crate::runtime::wheel;
use crate::style::{Flags, StateSet};
use craft_behavior::Sense;
use craft_theme::{State, StateMix};
use egui::{Context, Id, Rect};

/// Registers a wheel taker for next pass's split (8.6) from the wheel bits of `sense` (`WheelTaker::for_sense`). A
/// value control takes the wheel only while focused or armed, on the axes it senses (the vertical one when it senses
/// none); inside a `WheelScope::Free` scope one that senses the wheel takes it without focus. Inside a
/// `WheelScope::Gated` scope every taker waits for focus or arming. A zoom zone that takes no axis itself waits only
/// for a gated scope: its wheel goes to the view around it.
pub(crate) fn register_wheel(ctx: &Context, id: Id, rect: Rect, sense: Sense, value_control: bool, scope: WheelScope) {
    let sensed = craft_behavior::WheelTaker::for_sense(id, rect, sense);
    let free = sensed.is_some() && scope == WheelScope::Free;
    let zone = sensed.is_some_and(|taker| taker.zooms_view && !(taker.horizontal || taker.vertical || taker.zoom));
    let taker = sensed.or_else(|| value_control.then(|| craft_behavior::WheelTaker::new(id, rect).vertical(true)));
    if let Some(taker) = taker {
        wheel::register(ctx, taker.gated(scope == WheelScope::Gated || (value_control && !free && !zone)));
    }
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
