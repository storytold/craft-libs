//! The `Styled` trait: state sets and resolved looks of a component and its parts.

use super::context::{Cx, CxCore};
use super::pointer::Pointer as _;
use crate::contract::Component;
use crate::primitives::PartResult;
use crate::runtime::focus::{self};
use crate::style::{Ink, Mix, StateSet, Stateful};
use craft_behavior::PartRef;
use craft_theme::Flags;

/// The resolved look (chapter 4); every method reads step 12's output.
pub trait Styled {
    /// The state set.
    fn state(&self) -> &StateSet;
    /// The root's own state: the hover and press of the root without those of its parts, for a control whose parts
    /// each paint their own state (a split button's main part).
    fn root_state(&self) -> StateSet;
    /// The ink the surface reads in.
    fn ink(&self) -> Ink;
    /// The value of `look` for the component's state set.
    fn resolve<T: Mix>(&self, look: &Stateful<T>) -> T;
    /// The value of `look` for the state set of `part`.
    fn resolve_part<T: Mix>(&self, part: impl Into<PartRef>, look: &Stateful<T>) -> T;
}

impl CxCore<'_> {
    fn assert_stated(&self) {
        debug_assert!(self.stated, "the state set is written at step 12; it is not readable in `update` or `overlay`");
    }

    /// The active part, read again when focus was written since step 8.
    pub(crate) fn active_now(&self) -> Option<PartRef> {
        if focus::writes() == self.focus.at {
            return self.active;
        }
        focus::facts(&self.ctx, self.id).part.filter(|part| self.results.contains_key(&(part.key, part.index)))
    }

    /// Reads the focus facts again when focus was written since they were read, so a request or a layer that took
    /// focus during this pass shows in the steps after it.
    #[doc(hidden)]
    pub fn sync_focus(&mut self) {
        if focus::writes() != self.focus.at {
            self.focus = focus::facts(&self.ctx, self.id);
            self.active = self.focus.part.filter(|part| self.results.contains_key(&(part.key, part.index)));
        }
    }

    /// The state set of `part`: its press and hover, its own flags, and the focus flags only when it is the
    /// active part of the focused component.
    pub fn part_state(&self, part: PartRef) -> StateSet {
        self.part_state_lit(part, false)
    }

    /// The state set of `part`, hovered as well when `lit`: a part the pointer is not over that a press at the pointer
    /// would act on.
    ///
    /// A part not declared this pass (one added by this pass's click, which `parts` declares next pass) reads idle.
    #[doc(hidden)]
    pub fn part_state_lit(&self, part: PartRef, lit: bool) -> StateSet {
        let result = self.part_if_declared(part).unwrap_or(&PartResult::IDLE);
        let hovered = result.hovered || lit;
        let mut flags = result.flags;
        if self.active_now() == Some(part) && focus::is_focused(&self.ctx, self.id) {
            flags |= Flags::FOCUSED;
            if focus::focus_visible(&self.ctx, self.id) {
                flags |= Flags::FOCUS_VISIBLE;
            }
        }
        let pressed = result.press.phase.is_press();
        let state = if !self.resolved.enabled || result.inactive {
            craft_theme::State::Disabled
        } else if pressed {
            craft_theme::State::Pressed
        } else if hovered {
            craft_theme::State::Hover
        } else {
            craft_theme::State::Rest
        };
        let on = flags.intersects(Flags::ON);
        let t = craft_theme::StateMix::new(f32::from(u8::from(hovered)), f32::from(u8::from(pressed)), f32::from(u8::from(on)));
        StateSet::new(state, flags, t)
    }
}

impl Styled for CxCore<'_> {
    fn state(&self) -> &StateSet {
        self.assert_stated();
        &self.state
    }

    fn root_state(&self) -> StateSet {
        self.assert_stated();
        let hovered = self.response.hovered() && !self.results.values().any(|result| result.hovered);
        let pressed = self.press.phase.is_press();
        let state = if !self.resolved.enabled {
            craft_theme::State::Disabled
        } else if pressed {
            craft_theme::State::Pressed
        } else if hovered {
            craft_theme::State::Hover
        } else {
            craft_theme::State::Rest
        };
        StateSet::new(
            state,
            self.state.flags,
            craft_theme::StateMix::new(f32::from(u8::from(hovered)), f32::from(u8::from(pressed)), self.state.t.on),
        )
    }

    fn ink(&self) -> Ink {
        self.assert_stated();
        self.ink
    }

    fn resolve<T: Mix>(&self, look: &Stateful<T>) -> T {
        self.assert_stated();
        self.state.resolve(look)
    }

    fn resolve_part<T: Mix>(&self, part: impl Into<PartRef>, look: &Stateful<T>) -> T {
        self.assert_stated();
        self.part_state(part.into()).resolve(look)
    }
}

impl<C: Component> Styled for Cx<'_, C> {
    fn state(&self) -> &StateSet {
        self.core.state()
    }

    fn root_state(&self) -> StateSet {
        self.core.root_state()
    }

    fn ink(&self) -> Ink {
        self.core.ink()
    }

    fn resolve<T: Mix>(&self, look: &Stateful<T>) -> T {
        self.core.resolve(look)
    }

    fn resolve_part<T: Mix>(&self, part: impl Into<PartRef>, look: &Stateful<T>) -> T {
        self.core.resolve_part(part, look)
    }
}
