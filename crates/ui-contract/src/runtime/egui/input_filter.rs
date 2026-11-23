//! The input filter: the one owner of edits to the raw input a viewport's pass reads.
//!
//! A family crate hands the filter what it learned outside the window (a key equivalent the platform's menu
//! took) and the filter puts it into the viewport's input in the hook, ahead of the stages that read keys, so it
//! reaches the application as the key press the user made. No family crate touches `RawInput` or `egui::Event`.

use std::collections::HashMap;

use craft_keymap::Chord;
use egui::{Context, Event, ViewportId};

use super::injected::{egui_modifiers, key_press, key_release};
use super::memory::Service;
use super::repaint;
use crate::runtime::windows::KeyEquivalent;

#[derive(Default)]
struct Pending {
    by_viewport: HashMap<ViewportId, Vec<KeyEquivalent>>,
}

/// Queues `equivalent` for the next input of `viewport` and asks for a pass of it. When a pass of `viewport` is
/// running already, its input is taken: the pass is discarded, and the repeat reads the equivalent in its input.
pub(crate) fn add_key_equivalent(ctx: &Context, viewport: ViewportId, equivalent: KeyEquivalent) {
    Service::<Pending>::run(ctx, |pending| pending.by_viewport.entry(viewport).or_default().push(equivalent));
    repaint::request_of(ctx, viewport);
    repeat_running_pass(ctx, viewport, "craft_ui: a key equivalent reached a pass after its input was taken");
}

/// Discards the pass that is running for `viewport`, so the input hook runs again for it with what the filter holds
/// now. Nothing happens outside a pass of `viewport`, where the next input hook reads it.
fn repeat_running_pass(ctx: &Context, viewport: ViewportId, reason: &'static str) {
    if crate::runtime::frame::running(ctx).is_some() && super::pass::current_viewport(ctx) == viewport {
        super::sizing::request_discard(ctx, super::sizing::Rerun::Input(reason));
    }
}

/// The input hook's filter step: adds the queued key equivalents of `input.viewport_id` to `input.events`.
pub(crate) fn input_hook(ctx: &Context, input: &mut egui::RawInput) {
    dismiss_input(ctx, input);
    let queued = Service::<Pending>::run(ctx, |pending| pending.by_viewport.remove(&input.viewport_id));
    let Some(queued) = queued else { return };
    let platform = craft_keymap::egui::platform(ctx);
    for equivalent in queued {
        match equivalent {
            KeyEquivalent::Copy => input.events.push(Event::Copy),
            KeyEquivalent::Cut => input.events.push(Event::Cut),
            KeyEquivalent::Press(chord) => insert_press(input, chord, platform, false),
            KeyEquivalent::Tap(chord) => insert_press(input, chord, platform, true),
        }
    }
}

/// Puts the press of `chord` ahead of its key-up when that is already in the input, else last; `tap` adds the
/// release right after it.
fn insert_press(input: &mut egui::RawInput, chord: Chord, platform: craft_keymap::Platform, tap: bool) {
    let (key, modifiers) = (chord.key.to_egui(), egui_modifiers(chord.mods, platform));
    let at = input.events.iter().position(|e| matches!(e, Event::Key { key: up, pressed: false, .. } if *up == key)).unwrap_or(input.events.len());
    input.events.insert(at, key_press(key, modifiers));
    if tap {
        input.events.insert(at + 1, key_release(key, modifiers));
    }
}

/// What the platform's menu reports about itself, per viewport: whether a menu is open, whether the next closing
/// press may still be taken from the window, and whether the platform is to close its menu.
#[derive(Clone, Copy)]
struct MenuState {
    open: bool,
    armed: bool,
    close: bool,
}

impl Default for MenuState {
    fn default() -> Self {
        MenuState { open: false, armed: true, close: false }
    }
}

#[derive(Default)]
struct Menus {
    by_viewport: HashMap<ViewportId, MenuState>,
}

/// Records whether the platform's menu of `viewport` is open. A report of "closed" re-arms the filter: a menu that
/// keeps reporting itself open costs the window one press, not every press.
pub(crate) fn set_menu_open(ctx: &Context, viewport: ViewportId, open: bool) {
    let opened = Service::<Menus>::run(ctx, |menus| {
        let state = menus.by_viewport.entry(viewport).or_default();
        let opened = open && !state.open;
        state.open = open;
        state.armed |= !open;
        opened
    });
    // A menu that opens after the pass took its input: the repeat drops the click that belongs to the menu.
    if opened {
        repeat_running_pass(ctx, viewport, "craft_ui: the platform's menu opened after the pass took its input");
    }
}

/// Whether the filter saw a press or Escape since the last call: the platform's menu is to close.
pub(crate) fn take_menu_close(ctx: &Context, viewport: ViewportId) -> bool {
    Service::<Menus>::run(ctx, |menus| menus.by_viewport.get_mut(&viewport).is_some_and(|state| std::mem::take(&mut state.close)))
}

/// The input hook's dismiss step: a press or Escape in `input` asks the platform's menu to close, and while one is
/// open (and not already taken from the window once) the pointer and Escape events are dropped, since they belong
/// to the menu. Pointer moves stay.
fn dismiss_input(ctx: &Context, input: &mut egui::RawInput) {
    let dismissing = input
        .events
        .iter()
        .any(|e| matches!(e, Event::PointerButton { pressed: true, .. } | Event::Key { key: egui::Key::Escape, pressed: true, .. }));
    if !dismissing {
        return;
    }
    let drop = Service::<Menus>::run(ctx, |menus| {
        let state = menus.by_viewport.entry(input.viewport_id).or_default();
        state.close = true;
        let drop = state.open && state.armed;
        state.armed &= !drop;
        drop
    });
    if drop {
        input.events.retain(|e| !matches!(e, Event::PointerButton { .. } | Event::Key { key: egui::Key::Escape, .. }));
    }
}
