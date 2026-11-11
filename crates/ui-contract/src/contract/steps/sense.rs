//! Step 8, sense: part states from the sensing core and the wheel takers.

use crate::runtime::wheel;
use crate::style::{Flags, StateSet};
use craft_behavior::Sense;
use craft_theme::{State, StateMix};
use egui::{Context, Id, Rect};

/// Registers a wheel taker for next pass's split (8.6) from the wheel bits of `sense` (`WheelTaker::for_sense`); a
/// value control without them takes the vertical wheel only while focused or armed. Inside a wheel-gated scope
/// (`Overrides::wheel_gated`) every taker takes the wheel, on every axis, only while focused or armed.
pub(crate) fn register_wheel(ctx: &Context, id: Id, rect: Rect, sense: Sense, value_control: bool, gated: bool) {
    if let Some(taker) = craft_behavior::WheelTaker::for_sense(id, rect, sense) {
        wheel::register(ctx, taker.gated(taker.gated || gated));
    } else if value_control {
        wheel::register(ctx, craft_behavior::WheelTaker::new(id, rect).vertical(true).gated(true));
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
    let pressed = result.press.phase.is_press();
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
