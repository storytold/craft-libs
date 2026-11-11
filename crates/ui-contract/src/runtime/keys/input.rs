//! The input hook's key steps (H3): what it strips from each raw key event and records.

use super::route::layout_of;
use super::{AltTrack, Batch, MenuKey, NavKeys, Press, Shortcut, TabPress, current, with};
use crate::runtime::commands::{self, ChromeCommand, EditCommands, FirePhase};
use crate::runtime::egui::input::{self as raw_input, KeyEvent, RawInput};
use crate::runtime::env::Env;
use crate::runtime::focus::{self, EndKey, KeyUse};
use crate::runtime::frame::{ChromeToggle, request_chrome_toggle};
use crate::runtime::overlay;
use crate::runtime::sampler;
use craft_behavior::{EditCommand, OwnerClass};
use craft_host::target::Os;
use craft_keymap::{Chord, Key, KeyInput, Layout, Mods};
use egui::Context;
use std::sync::Arc;

/// What the strip of one event decided.
struct Strip<'a> {
    os: Os,
    layout: &'a Layout,
    keeps_tab: bool,
    pages: bool,
    /// The focused component reads every press as a chord (`OwnerClass::Recorder`).
    records: bool,
    claims: &'a [Chord],
    edits: &'a dyn Fn(EditCommand) -> bool,
    /// The chords bound to the edit commands.
    edit_chords: &'a [(EditCommand, Chord)],
    menu_bar: bool,
    /// A dialog keeps a canvas open and Space belongs to it: the press is the canvas's momentary command chord.
    canvas_space: bool,
}

impl Strip<'_> {
    /// Records `event` in `batch` and returns whether egui keeps it.
    ///
    /// `consumed` holds the keys whose press the strip took and whose release has not come yet: the release goes
    /// with the press, in this pass or a later one.
    fn event(&self, batch: &mut Batch, alt: &mut AltTrack, consumed: &mut Vec<Key>, event: KeyEvent) -> bool {
        match event {
            KeyEvent::Press { key, input, repeat } => {
                alt.other_key = true;
                batch.last_press = Some(input);
                if key == Key::Enter {
                    batch.enter_down = Some(true);
                }
                let kept = self.press(batch, key, &input, repeat);
                // The canvas's Space is a momentary chord: its release ends the command, so it stays readable.
                let held_by_canvas = key == Key::Space && self.canvas_space;
                if let Some(held) = input.physical.or(input.typed) {
                    consumed.retain(|k| *k != held);
                    if !kept && !held_by_canvas {
                        consumed.push(held);
                    }
                }
                kept
            }
            KeyEvent::Release { input } => {
                if let Some(at) = input.physical.or(input.typed).and_then(|held| consumed.iter().position(|k| *k == held)) {
                    consumed.swap_remove(at);
                    return false;
                }
                if input.physical.or(input.typed) == Some(Key::Enter) {
                    batch.enter_down = Some(false);
                }
                batch.shortcuts.push(Shortcut { input, phase: FirePhase::Release, typed: false, press: None, moved: false });
                true
            }
            KeyEvent::Text(text) => {
                alt.other_key = true;
                if let Some(press) = batch.shortcuts.last_mut().filter(|shortcut| shortcut.phase == FirePhase::Press) {
                    press.typed = true;
                }
                if let Some(input) = batch.last_press.take().filter(|_| !alt.right)
                    && let Some(physical) = input.physical
                {
                    batch.learned.push((physical, input.mods, text.clone()));
                }
                batch.texts.push(text);
                true
            }
            KeyEvent::Clip { command, text } if (self.edits)(command) => {
                batch.clips.push(command);
                if text.is_some() {
                    batch.paste = text;
                }
                false
            }
            KeyEvent::Clip { command, .. } => {
                // The platform sends the clipboard event in place of the chord: an unlisted one is the chord. The
                // web sends the chord's press as well.
                let key = match command {
                    EditCommand::Cut => Key::X,
                    EditCommand::Paste => Key::V,
                    _ => Key::C,
                };
                let input = KeyInput::new(Mods::CMD).with_typed(key);
                let sent = batch
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.phase == FirePhase::Press && shortcut.input.mods == Mods::CMD && shortcut.input.typed == Some(key));
                if !sent && self.records {
                    // A component that reads every chord gets the event as the chord's press, the way the key made it.
                    self.press(batch, key, &input, false);
                } else if !sent {
                    batch.shortcuts.push(Shortcut { input, phase: FirePhase::Press, typed: false, press: None, moved: false });
                }
                // The event stands for the chord, so the shortcut path is its only reader: egui does not copy or paste
                // on top of the command.
                false
            }
            KeyEvent::PointerPress => {
                alt.other_key = true;
                true
            }
            KeyEvent::RightAlt(down) => {
                alt.right = down;
                true
            }
            KeyEvent::Modifiers(held) => {
                self.alt_change(batch, alt, held.alt);
                true
            }
            KeyEvent::Access(request) => {
                batch.access.push(request);
                true
            }
            _ => true,
        }
    }

    /// A lone Alt press, pressed and released with no other key between, focuses the menu bar (9.7).
    fn alt_change(&self, batch: &mut Batch, alt: &mut AltTrack, held: bool) {
        if held && !alt.down {
            alt.other_key = false;
        }
        if !held && alt.down && !alt.other_key && self.menu_bar && self.os != Os::Mac {
            batch.menu.push(MenuKey::Focus);
        }
        alt.down = held;
    }

    fn press(&self, batch: &mut Batch, key: Key, input: &KeyInput, repeat: bool) -> bool {
        let mods = input.mods;
        let claimed = self.claims.iter().find(|chord| self.layout.matches(chord, input, self.os));
        let ctrl_tab = key == Key::Tab && (if self.os == Os::Mac { mods.ctrl && !mods.cmd } else { mods.cmd });
        let ctrl_tab = ctrl_tab && self.pages && !self.keeps_tab;
        let bare_tab = key == Key::Tab && !mods.cmd && !mods.ctrl && !mods.alt;
        let mnemonic = self.menu_bar && self.os != Os::Mac && mods == Mods::ALT && key.is_letter();
        let edit = self.edit_chords.iter().find(|(command, chord)| (self.edits)(*command) && self.layout.matches(chord, input, self.os));
        if let Some(chord) = claimed {
            batch.claimed.push(*chord);
            false
        } else if let Some((command, _)) = edit {
            batch.clips.push(*command);
            false
        } else if key == Key::Space && self.canvas_space {
            // The canvas has Space (`runtime::sampler`): only the shortcut dispatcher reads it, so the dialog's
            // focused control does not activate.
            if !repeat {
                batch.shortcuts.push(Shortcut { input: *input, phase: FirePhase::Press, typed: false, press: None, moved: false });
            }
            false
        } else if bare_tab && !self.keeps_tab {
            batch.tabs.push(TabPress { input: *input, repeat });
            false
        } else if ctrl_tab {
            batch.presses.push(Press { key, mods, repeat });
            false
        } else if key == Key::F6 && (mods.is_none() || mods == Mods::SHIFT) {
            batch.nav.panes.push(!mods.shift);
            false
        } else if key == Key::Escape && !mods.cmd && !mods.ctrl && !(self.records && !mods.is_none()) {
            // Alt and Shift ride along, as with Enter: a canvas tool reads them; Cmd and Ctrl chords are the app's. While
            // the focused stop records, Escape with any modifier held is a chord for the recorder; only plain Escape is a
            // scope escape.
            batch.escapes += 1;
            false
        } else if key == Key::F10 && mods.is_none() && self.menu_bar {
            batch.menu.push(MenuKey::Focus);
            false
        } else if mnemonic {
            batch.menu.push(MenuKey::Letter(key.name().chars().next().unwrap_or('A')));
            false
        } else {
            batch.presses.push(Press { key, mods, repeat });
            if !repeat {
                batch.shortcuts.push(Shortcut {
                    input: *input,
                    phase: FirePhase::Press,
                    typed: false,
                    press: Some(batch.presses.len() - 1),
                    moved: false,
                });
            }
            true
        }
    }
}

/// Whether a bare Tab press that a chrome command is bound to toggles the window's panels (spec 1.8) instead of
/// moving focus. `enabled` is `RootEnv.tab_toggles_chrome`; `canvas` is whether the window registered a document
/// canvas (a stop with `KeyUse::chrome_key`) in its last pass; `layer` is what the layer stack says about the focus
/// and `focused` is the `KeyUse` of the focused component, `None` when nothing is focused.
///
/// A modal or dialog layer never toggles: Tab moves focus inside it. In a window with a canvas the toggle is on when
/// nothing is focused or the canvas is; elsewhere it is on only when the canvas is focused. A component that keeps
/// Tab never reaches this function: the press is its own.
fn tab_toggles_chrome(enabled: bool, canvas: bool, layer: overlay::LayerKeys, focused: Option<&KeyUse>) -> bool {
    let closed = matches!(layer, overlay::LayerKeys::Blocked | overlay::LayerKeys::Class(OwnerClass::Dialog));
    if !enabled || closed {
        return false;
    }
    let on_canvas = focused.is_some_and(|keys| keys.chrome_key);
    if canvas { focused.is_none() || on_canvas } else { on_canvas }
}

/// The toggle of the chrome command whose bound chord `input` presses, when that command is enabled.
pub(super) fn chrome_toggle_of(ctx: &Context, input: &KeyInput) -> Option<ChromeToggle> {
    let os = crate::runtime::windows::os_of(ctx);
    let layout = layout_of(ctx);
    ChromeCommand::ALL.into_iter().find_map(|command| {
        let resolved = commands::resolve(ctx, command);
        let chord = resolved.chord?;
        (resolved.enabled && layout.matches(&chord, input, os)).then_some(command.toggle())
    })
}

/// The input hook's key steps (H3). Reads only `RawInput` and records keyed by `input.viewport_id`. Returns
/// whether it removed an event, so the hook asks for a repaint.
pub(crate) fn input_hook(ctx: &Context, input: &mut RawInput) -> bool {
    let viewport = input.viewport_id;
    // The canvas a modal layer keeps open takes the events its reach admits and nothing else (`runtime::sampler`).
    let (hole, trimmed) = sampler::hook(ctx, input);
    // A modal layer in another viewport blocks this one (app-modal, 15.5): nothing below reaches the key stage.
    if overlay::modal_elsewhere(ctx, viewport) {
        let pointer = ctx.input(|state| state.pointer.latest_pos());
        let (removed, press) = raw_input::drop_under_modal(input, hole.as_ref(), pointer);
        if press {
            overlay::note_dropped_press(ctx, viewport);
        }
        return removed || trimmed;
    }
    let os = crate::runtime::windows::os_of(ctx);
    let record = focus::record(ctx, viewport);
    let keys = record.keys;
    let menu_bar = record.menu_bar && keys.is_none_or(|keys| !matches!(keys.owner, OwnerClass::Text | OwnerClass::Recorder));
    let before = input.events.len();
    let edit_chords = if keys.is_some_and(|keys| EditCommands::ALL.iter().any(|command| command.edit().is_some_and(|edit| keys.edits.contains(edit))))
    {
        commands::edit_chords(ctx)
    } else {
        Arc::default()
    };
    let latin = input.latin_layout_chars.clone();
    let (batch, visible) = with(ctx, viewport, |stage| {
        let mut claims = record.claims.clone();
        claims.extend(stage.claims.iter().filter(|(owner, _)| Some(*owner) == record.focused).map(|(_, chord)| *chord));
        claims.extend(stage.captures.iter().map(|(_, chord)| *chord));
        // A bare End belongs to a stop that declares `EndKey::EndMode`: it reads the key as a claimed chord and toggles
        // its mode, so no collection navigation or scroll owner takes it as a move to the end.
        if keys.is_some_and(|keys| keys.end == EndKey::EndMode) {
            claims.push(Chord::plain(Key::End));
        }
        let edits = |command| keys.is_some_and(|keys| keys.edits.contains(command));
        stage.layout.set_latin(input.keyboard_layout_is_latin);
        let strip = Strip {
            os,
            layout: &stage.layout,
            keeps_tab: keys.is_some_and(|keys| keys.keep == focus::KeepKeys::Tab),
            pages: record.pages,
            records: keys.is_some_and(|keys| matches!(keys.owner, OwnerClass::Recorder)),
            claims: &claims,
            edits: &edits,
            edit_chords: &edit_chords,
            menu_bar,
            canvas_space: hole.as_ref().is_some_and(|hole| hole.space || hole.space_pressed),
        };
        let mut batch = Batch::default();
        let mut visible = None;
        input.events.retain(|event| {
            let read = raw_input::key_event(event, os, &latin);
            note_down(&mut stage.down, &read);
            visible = match &read {
                KeyEvent::PointerPress => Some(false),
                KeyEvent::Press { .. } | KeyEvent::Text(_) => Some(true),
                _ => visible,
            };
            strip.event(&mut batch, &mut stage.alt, &mut stage.consumed, read)
        });
        for (physical, mods, text) in std::mem::take(&mut batch.learned) {
            stage.layout.observe(physical, mods, &text);
        }
        (batch, visible)
    });
    if let Some(visible) = visible {
        focus::set_visible(ctx, viewport, visible);
    }
    with(ctx, viewport, |stage| {
        stage.batch = batch;
    });
    trimmed || input.events.len() != before
}

/// Updates the keys held after `event`: a press adds its key, a release removes it, and the window losing focus
/// (which also reads as the right Alt coming up) forgets them all, since their releases go to another window.
fn note_down(down: &mut Vec<Key>, event: &KeyEvent) {
    match event {
        KeyEvent::Press { input, .. } => {
            if let Some(key) = input.physical.or(input.typed).filter(|key| !down.contains(key)) {
                down.push(key);
            }
        }
        KeyEvent::Release { input } => down.retain(|key| Some(*key) != input.physical.or(input.typed)),
        KeyEvent::RightAlt(false) => down.clear(),
        _ => {}
    }
}

/// The keys down in the running viewport this pass, withheld presses included (a key a component claimed is still
/// down).
#[doc(hidden)]
pub fn down(ctx: &Context) -> Vec<Key> {
    current(ctx, |stage| stage.down.clone())
}

/// Takes the Tab and F6 moves the input hook recorded, for the focus service's begin hook. The pass's root env is
/// installed by now, so a bare Tab that passes the chrome gate and has a chrome command bound to it becomes that
/// command's toggle for the running viewport; any other Tab moves focus. A repeat of a toggling press is dropped: it
/// neither toggles again nor moves focus.
pub(crate) fn take_nav(ctx: &Context) -> NavKeys {
    let (mut nav, tabs) = current(ctx, |stage| (std::mem::take(&mut stage.batch.nav), std::mem::take(&mut stage.batch.tabs)));
    if tabs.is_empty() {
        return nav;
    }
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let enabled = Env::root_for(ctx, viewport).is_some_and(|env| env.tab_toggles_chrome);
    let focused = focus::focused_rec(ctx);
    let canvas = focus::with(ctx, |state| state.stops.iter().any(|rec| rec.stop.keys.chrome_key));
    let layer = overlay::key_layer(ctx, &focused.as_ref().map(|rec| focus::scope_chain(ctx, rec)).unwrap_or_default());
    let gate = tab_toggles_chrome(enabled, canvas, layer, focused.as_ref().map(|rec| &rec.stop.keys));
    for press in tabs {
        match gate.then(|| chrome_toggle_of(ctx, &press.input)).flatten() {
            Some(toggle) if !press.repeat => request_chrome_toggle(ctx, viewport, toggle),
            Some(_) => {}
            None => nav.tabs.push(!press.input.mods.shift),
        }
    }
    nav
}

/// The menu-bar keys of the pass (spec 9.7).
pub(crate) fn take_menu_keys(ctx: &Context) -> Vec<MenuKey> {
    current(ctx, |stage| std::mem::take(&mut stage.menu))
}
