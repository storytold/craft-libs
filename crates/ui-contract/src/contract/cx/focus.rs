//! The `Focus` trait: focus facts and requests of a component and its parts.

use super::context::{Cx, CxCore};
use crate::contract::Component;
use crate::runtime::focus::{self};
use crate::runtime::keys::{Intents, MenuKey};
use craft_behavior::{PartRef, PressPhase};
use craft_keymap::Chord;
use egui::Rect;

/// Focus and keys (chapter 9).
pub trait Focus {
    /// The intents routed to this component this pass: `cx.intents().take(Intent::Activate)`.
    fn intents(&mut self) -> &mut Intents;
    /// Claims `chord` for the next frames while this component is focused.
    fn claim(&mut self, chord: Chord);
    /// Whether `chord` arrived this pass for this component, consuming it.
    fn take_claimed(&mut self, chord: Chord) -> bool;
    /// The focused part while the component holds focus.
    fn active_part(&self) -> Option<PartRef>;
    /// The part focus moved onto in this frame (Tab or an arrow into a roving group, a focus request, a click), for
    /// every pass of the frame; `None` when focus stayed on its part, left the component, or sits on the component
    /// itself.
    fn focus_arrived(&self) -> Option<PartRef>;
    /// [`focus_arrived`](Focus::focus_arrived) when the part is not under a press: focus came by the keyboard or a
    /// focus request (Photoshop: Tab into a field selects its text, a click does not). A press on the part reports
    /// none; the press selects by its own rule.
    fn focus_arrived_by_key(&self) -> Option<PartRef>;
    /// The component holds key focus.
    fn is_focused(&self) -> bool;
    /// The component holds focus and shows it.
    fn focus_visible(&self) -> bool;
    /// Asks the nearest scroll owner to bring `rect` into view.
    fn reveal(&mut self, rect: Rect);
    /// Registers that this component is a menu bar that takes the menu-bar keys of the next input hook (9.7).
    fn register_menu_bar(&mut self);
    /// The menu-bar keys of the pass: F10 or a lone Alt press focus the bar, Alt and a letter open a menu.
    fn take_menu_keys(&mut self) -> Vec<MenuKey>;
}

impl Focus for CxCore<'_> {
    fn intents(&mut self) -> &mut Intents {
        &mut self.intents
    }

    fn claim(&mut self, chord: Chord) {
        self.claims.push(chord);
    }

    fn take_claimed(&mut self, chord: Chord) -> bool {
        crate::runtime::keys::take_claimed(&self.ctx, self.id, chord)
    }

    fn active_part(&self) -> Option<PartRef> {
        self.active_now()
    }

    fn focus_arrived(&self) -> Option<PartRef> {
        let facts = if focus::writes() == self.focus.at { self.focus } else { focus::facts(&self.ctx, self.id) };
        facts.arrived.filter(|part| self.results.contains_key(&(part.key, part.index)))
    }

    fn focus_arrived_by_key(&self) -> Option<PartRef> {
        self.focus_arrived().filter(|part| {
            self.results.get(&(part.key, part.index)).is_some_and(|result| matches!(result.press.phase, PressPhase::Idle | PressPhase::Hover))
        })
    }

    fn is_focused(&self) -> bool {
        focus::is_focused(&self.ctx, self.id)
    }

    fn focus_visible(&self) -> bool {
        focus::focus_visible(&self.ctx, self.id)
    }

    fn reveal(&mut self, rect: Rect) {
        self.reveal = Some(rect);
    }

    fn register_menu_bar(&mut self) {
        focus::register_menu_bar(&self.ctx);
    }

    fn take_menu_keys(&mut self) -> Vec<MenuKey> {
        crate::runtime::keys::take_menu_keys(&self.ctx)
    }
}

impl<C: Component> Focus for Cx<'_, C> {
    fn intents(&mut self) -> &mut Intents {
        self.core.intents()
    }

    fn claim(&mut self, chord: Chord) {
        self.core.claim(chord);
    }

    fn take_claimed(&mut self, chord: Chord) -> bool {
        self.core.take_claimed(chord)
    }

    fn active_part(&self) -> Option<PartRef> {
        self.core.active_part()
    }

    fn focus_arrived(&self) -> Option<PartRef> {
        self.core.focus_arrived()
    }

    fn focus_arrived_by_key(&self) -> Option<PartRef> {
        self.core.focus_arrived_by_key()
    }

    fn is_focused(&self) -> bool {
        self.core.is_focused()
    }

    fn focus_visible(&self) -> bool {
        self.core.focus_visible()
    }

    fn reveal(&mut self, rect: Rect) {
        self.core.reveal(rect);
    }

    fn register_menu_bar(&mut self) {
        self.core.register_menu_bar();
    }

    fn take_menu_keys(&mut self) -> Vec<MenuKey> {
        self.core.take_menu_keys()
    }
}
