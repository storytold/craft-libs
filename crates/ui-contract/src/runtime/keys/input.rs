//! The input hook's key steps (H3): what it strips from each raw key event and records.

use super::route::layout_of;
use super::{AltTrack, Batch, EscapePress, LastPress, MenuKey, NavKeys, Press, Shortcut, Stage, TabPress, Typed, current, with};
use crate::runtime::commands::{self, EditCommands, FirePhase, NavCommand, WindowShortcut};
use crate::runtime::egui::input::{self as raw_input, Arrived, KeyEvent, RawInput};
use crate::runtime::env::Env;
use crate::runtime::focus::{self, EditSet, EndKey, KeyUse};
use crate::runtime::frame::{ChromeToggle, request_chrome_toggle};
use crate::runtime::overlay;
use crate::runtime::sampler;
use craft_behavior::{EditCommand, OwnerClass};
use craft_host::target::Os;
use craft_keymap::{Chord, Key, KeyInput, Layout, Mods};
use egui::{Context, Id, ViewportId};
use std::sync::Arc;

/// What the strip of one event decided.
struct Strip<'a> {
    os: Os,
    layout: &'a Layout,
    keeps_tab: bool,
    pages: bool,
    /// The focused component reads every press as a chord (`OwnerClass::Recorder`).
    records: bool,
    /// A press or a drag session is open: an Escape cancels it whatever the modifiers.
    press_captured: bool,
    claims: &'a Claims,
    /// The edit commands the focused framework component takes as `Intent::Edit`.
    edits: &'a dyn Fn(EditCommand) -> bool,
    /// The chords bound to the edit commands that are the platform's text-editing chords.
    edit_chords: &'a [(EditCommand, Chord)],
    /// The chords bound to the window's keys (`commands::window_shortcuts`).
    window_shortcuts: &'a [(WindowShortcut, Chord)],
    menu_bar: bool,
    /// The registered bar takes only the Alt+letter chords it claims (`FocusedRecord::menu_bar_by_claim`): every other
    /// Alt+letter is an ordinary press.
    menu_bar_by_claim: bool,
    /// The registered bar holds the focus (F10 or a lone Alt gave it a title) with no menu open.
    bar_focused: bool,
    /// The focused stop holds drafts (`KeyUse::holds_drafts`): a lone Alt alerts instead of focusing the menu bar.
    holds_drafts: bool,
    /// A dialog keeps a canvas open and Space belongs to it: the press is the canvas's momentary command chord.
    canvas_space: bool,
    /// An app's own egui widget or popup has the focus (`FocusedRecord::foreign`): Escape, the plain Enter and the
    /// clipboard events are its own, and no window intent comes of them.
    foreign: bool,
    /// A bare Tab is egui's: an app's own popup is open, or the app's own focused widget keeps Tab by its egui key
    /// filter (`FocusedRecord::tab_in_egui`), or the window has no stop to move the focus to and nothing focused
    /// (`focus::tab_has_no_stop`), so the Tab is the app's own binding's.
    tab_in_egui: bool,
    /// The focused stop is the app's own egui widget (`Stop::host`, `FocusedRecord::host`), which reads its keys from
    /// egui: its Escape stays there, as an app's own focused widget's does. Tab and the arrows follow its `KeyUse`, as
    /// the framework's navigation reads it.
    host: bool,
    /// The edit set whose clipboard events and edit chords stay in egui's input: the focused stop's when it is the
    /// app's own widget (`host`), else the window canvas's with nothing focused (`target_canvas`).
    egui_edits: EditSet,
    /// egui holds a label text selection, which a copy or cut event copies.
    label_selection: bool,
    /// A tool host claimed the lone Alt tap: it goes to the claimer on every OS, not to the menu bar.
    alt_claimed: bool,
    /// The focus is in a menu (`FocusedRecord::menu`).
    in_menu: bool,
    /// An Alt release in this input ended an Alt+numpad code (`raw_input::take_alt_code_ended`): the code's digits
    /// never reach this stage, so without this the release would read as a lone Alt.
    alt_code_ended: bool,
    /// Whether the window's keymap binds a chord to a command that is enabled now (`commands::runs_now`).
    bound: &'a dyn Fn(&Chord) -> bool,
    /// A held key's repeats are shortcuts too (`RootEnv::shortcuts_repeat`).
    repeats: bool,
}

/// How the strip reads one raw event beside its key reading: when it arrived, and what its neighbours say of it.
#[derive(Clone, Copy)]
struct Arrival {
    /// The event's arrival time (`Arrived::at`).
    at: Option<f64>,
    /// An input method's commit follows the event: a press it follows typed through the input method.
    ime_next: bool,
    /// A key press follows the event: a text before it is that press's (the web sends a press's text first).
    press_next: bool,
}

/// The key a typed `text` stands for: one character that names a key, or the US base key of a shifted character no key
/// names (`~` is typed on Backtick).
fn text_key(text: &str) -> Option<Key> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    Key::from_char(c).or(match c {
        '~' => Some(Key::Backtick),
        _ => None,
    })
}

/// The chord a clipboard event of `command` stands for with `mods` held: Cmd and the command's key, with the modifiers
/// held beside Cmd (Cmd+Shift+V pastes in place).
fn clip_chord(command: EditCommand, mods: Mods) -> (Key, KeyInput) {
    let key = match command {
        EditCommand::Cut => Key::X,
        EditCommand::Paste => Key::V,
        _ => Key::C,
    };
    let mut chord_mods = mods;
    chord_mods.cmd = true;
    (key, KeyInput::new(chord_mods).with_typed(key))
}

/// Whether a clipboard event with `mods` held is the focused editor's edit: the platform sends it for the edit's own
/// chord, and for Cmd with Shift or Alt too (Cmd+Shift+X), which is no text-editing chord but the app's (a binding of it
/// to the edit still reaches the editor, through the command).
fn clip_is_edit(mods: Mods) -> bool {
    !((mods.cmd || mods.ctrl) && (mods.shift || mods.alt))
}

impl Strip<'_> {
    /// Whether the text the platform sends with the press `input` stands for is a shortcut's own and belongs to no
    /// field: Alt is held and the keymap binds the press to a command that is enabled, so it fires (S-L177). A platform
    /// sends its key's text with an Alt shortcut (Alt+Q on Linux). A bound Alt chord whose command is disabled does
    /// nothing, so its text is typed, as Word types Option+Shift+-'s em dash where Collapse does not apply. A plain key's
    /// text is typing, which a focused field or list takes before any shortcut (`route::holds_press`), and Cmd and Ctrl
    /// chords type nothing.
    fn shortcut_text(&self, input: &KeyInput) -> bool {
        input.mods.alt && self.layout.candidates(input).iter().any(|chord| (self.bound)(chord))
    }

    /// Records `event` in `batch` and returns whether egui keeps it.
    ///
    /// `consumed` holds the keys whose press the strip took and whose release has not come yet: the release goes
    /// with the press, in this pass or a later one. `down` holds the keys down before the event; a press of a key
    /// already down is a repeat, as egui reads it, whatever the platform said.
    fn event(&self, batch: &mut Batch, alt: &mut AltTrack, consumed: &mut Vec<Key>, down: &[Key], event: KeyEvent, arrival: Arrival) -> bool {
        let at = arrival.at;
        if matches!(event, KeyEvent::Clip { .. }) {
            // A clipboard event stands for its chord's press, which types nothing.
            batch.last_press = None;
        }
        match event {
            KeyEvent::Press { key, input, repeat } => {
                alt.other_key = true;
                let repeat = repeat || input.physical.or(input.typed).is_some_and(|held| down.contains(&held));
                let pushed = batch.presses.len();
                let claims = batch.claimed.len();
                let menu_keys = batch.menu.len();
                // Text an input method commits for the press is typing, whatever the press's chord binds.
                let shortcut_text = self.shortcut_text(&input) && !arrival.ime_next;
                let kept = self.press(batch, key, &input, repeat, at);
                // The canvas's Space keeps its text from every field, as a claim does.
                let canvas = batch.presses.get(pushed).is_some_and(|press| press.canvas);
                batch.last_press = Some(LastPress {
                    input,
                    pressed: (batch.presses.len() > pushed).then_some(pushed),
                    // A mnemonic's text (a plain letter on the focused bar) is no field's typing.
                    claimed: batch.claimed.len() > claims || canvas || shortcut_text || batch.menu.len() > menu_keys,
                });
                // The release key is kept for withheld presses too: the held-over rule matches repeats by it.
                if let Some(press) = batch.presses.get_mut(pushed) {
                    press.release = input.physical.or(input.typed);
                    press.ime = arrival.ime_next;
                    press.held = down.iter().copied().filter(|key| Some(*key) != press.release).collect();
                    if kept {
                        press.in_egui = Some(batch.egui_presses);
                    }
                }
                if kept {
                    batch.egui_presses += 1;
                }
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
                // The press's text comes before its release: a text after it is no press's.
                let key = input.physical.or(input.typed);
                if batch.last_press.as_ref().is_some_and(|last| last.input.physical.or(last.input.typed) == key) {
                    batch.last_press = None;
                }
                if let Some(at) = key.and_then(|held| consumed.iter().position(|k| *k == held)) {
                    consumed.swap_remove(at);
                    return false;
                }
                batch.shortcuts.push(Shortcut { input, phase: FirePhase::Release, press: None, canvas: false, at, place: batch.place });
                true
            }
            KeyEvent::Text(text) => {
                alt.other_key = true;
                let mut claimed = false;
                let last = batch.last_press.take();
                if let Some(last) = &last {
                    if let Some(physical) = last.input.physical.filter(|_| !alt.right) {
                        batch.learned.push((physical, last.input.mods, text.clone()));
                    }
                    claimed = last.claimed;
                    if let Some(press) = last.pressed.and_then(|at| batch.presses.get_mut(at)) {
                        press.text = Some(batch.texts.len());
                    }
                }
                claimed |= self.text_as_key(batch, last.as_ref(), &text, alt, arrival);
                // Every text stays for now: the begin hook's routing decides which reader gets it (`route::begin`).
                batch.texts.push(Typed { text, claimed, in_egui: Some(batch.egui_texts), place: batch.place });
                batch.egui_texts += 1;
                true
            }
            KeyEvent::Clip { command, pasted, mods } if (self.edits)(command) && clip_is_edit(mods) => {
                batch.clips.push((command, batch.place));
                // A paste of text and one of an image in the same pass are one paste of both.
                if let Some(pasted) = pasted {
                    let content = batch.paste.get_or_insert_default();
                    content.text = pasted.text.or(content.text.take());
                    content.image = pasted.image.or(content.image.take());
                }
                false
            }
            KeyEvent::Clip { .. } if self.foreign => true,
            KeyEvent::Clip { command, .. } if self.egui_edits.contains(command) => true,
            KeyEvent::Clip { command: EditCommand::Copy | EditCommand::Cut, .. } if self.label_selection => true,
            // A clipboard event behind an open menu reaches nothing: the menu keeps it, as a platform menu does. The
            // chord's own key press still runs its command, which closes the menu.
            KeyEvent::Clip { .. } if self.in_menu => false,
            KeyEvent::Clip { command, pasted, mods } => {
                // The platform sends the clipboard event in place of the chord: an unlisted one is the chord. The
                // web sends the chord's press as well. A paste's content goes with the command the chord fires.
                if let Some(pasted) = pasted.filter(|_| command == EditCommand::Paste) {
                    let content = batch.app_paste.get_or_insert_default();
                    content.text = pasted.text.or(content.text.take());
                    content.image = pasted.image.or(content.image.take());
                }
                let (key, input) = clip_chord(command, mods);
                let sent = batch
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.phase == FirePhase::Press && shortcut.input.mods == input.mods && shortcut.input.typed == Some(key));
                if !sent && self.records {
                    // A component that reads every chord gets the event as the chord's press, the way the key made it.
                    self.press(batch, key, &input, false, at);
                } else if !sent {
                    batch.shortcuts.push(Shortcut { input, phase: FirePhase::Press, press: None, canvas: false, at, place: batch.place });
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
                // AltGr types characters: its Alt is no lone Alt, whichever of this event and the modifier change
                // arrives first.
                if down {
                    alt.other_key = true;
                }
                true
            }
            KeyEvent::Modifiers(held) => {
                let last = alt.last.unwrap_or(Mods::NONE);
                if (held.cmd, held.ctrl, held.alt) != (last.cmd, last.ctrl, last.alt) {
                    batch.mods.push((batch.shortcuts.len(), held));
                }
                self.alt_change(batch, alt, held);
                alt.last = Some(held);
                true
            }
            KeyEvent::Access(request) => {
                batch.access.push(request);
                true
            }
            _ => true,
        }
    }

    /// Reads a one-character `text` as a press of the key it stands for (`text_key`) where no press typed it. A text
    /// with no press before it (`last`) or after it is a tap of that key with the held modifiers, which a claim takes or
    /// the shortcut dispatcher reads (Shift+~ arrives as `~` alone). A text Alt typed with a press no claim took is its
    /// key's for a claim (Option+8 types `[` on a Turkish Mac). Cmd and Ctrl chords type nothing, and the Alt that typed
    /// the character is no part of its key's chord. Returns whether a claim or a mnemonic took the text.
    fn text_as_key(&self, batch: &mut Batch, last: Option<&LastPress>, text: &str, alt: &AltTrack, arrival: Arrival) -> bool {
        let Some(key) = text_key(text) else { return false };
        let lone = last.is_none() && !arrival.press_next;
        let mods = match last {
            None if lone => alt.last.unwrap_or(Mods::NONE),
            Some(last) if !last.claimed && last.input.mods.alt => last.input.mods,
            _ => return false,
        };
        if mods.cmd || mods.ctrl {
            return false;
        }
        let input = KeyInput::new(Mods { alt: false, ..mods }).with_typed(key);
        if !lone {
            let PressUse::Claimed { chord, .. } = self.use_of(key, &input) else { return false };
            batch.claimed.push(chord);
            return true;
        }
        let (pushed, claims, menu_keys) = (batch.presses.len(), batch.claimed.len(), batch.menu.len());
        self.press(batch, key, &input, false, arrival.at);
        if let Some(press) = batch.presses.get_mut(pushed) {
            press.text = Some(batch.texts.len());
            // The tap has no release event: it ends where it began.
            if !press.canvas {
                batch.shortcuts.push(Shortcut { input, phase: FirePhase::Release, press: None, canvas: false, at: arrival.at, place: batch.place });
            }
        }
        batch.claimed.len() > claims || batch.menu.len() > menu_keys
    }

    /// A lone Alt press, pressed and released with no other key, pointer press, wheel turn, touch or Shift, Ctrl or Cmd
    /// between, goes to the tool that claims it on every OS, else focuses the menu bar off a Mac (9.7). In a field that
    /// holds drafts it only alerts, so the draft keeps its focus (L-52). A release that ended an Alt+numpad code is no
    /// lone Alt: the code's digits were the keys between.
    fn alt_change(&self, batch: &mut Batch, alt: &mut AltTrack, held: Mods) {
        if held.alt && !alt.down {
            alt.other_key = alt.right;
        }
        // Alt with Shift, Ctrl or Cmd is a chord (Windows reads Alt+Shift as the input-language switch).
        if held.alt && (held.shift || held.ctrl || held.cmd) {
            alt.other_key = true;
        }
        if !held.alt && alt.down && !alt.other_key && !self.alt_code_ended && (self.alt_claimed || self.os != Os::Mac) {
            if self.holds_drafts {
                batch.refused = true;
            } else if self.alt_claimed {
                batch.alt_tap = true;
            } else if self.menu_bar {
                batch.menu.push(MenuKey::Focus);
            }
        }
        alt.down = held.alt;
    }

    /// What the key stage makes of a press of `key` with `input`.
    fn use_of(&self, key: Key, input: &KeyInput) -> PressUse {
        let mods = input.mods;
        let matches = |claims: &[Chord]| claims.iter().find(|chord| self.layout.matches(chord, input, self.os)).copied();
        let claimed = matches(&self.claims.framework).map(|chord| (chord, false)).or_else(|| matches(&self.claims.host).map(|chord| (chord, true)));
        let ctrl_tab = key == Key::Tab && (if self.os == Os::Mac { mods.ctrl && !mods.cmd } else { mods.cmd });
        let ctrl_tab = ctrl_tab && self.pages && !self.keeps_tab;
        let bare_tab = key == Key::Tab && !mods.cmd && !mods.ctrl && !mods.alt;
        let plain_enter = key == Key::Enter && mods.is_none();
        // Alt and Shift ride along, as with Enter: a canvas tool reads them; Cmd and Ctrl chords are the app's. While
        // the focused stop records, Escape with any modifier held is a chord for the recorder; only plain Escape is a
        // scope escape. Over an open press any Escape is the press's cancel: a borrowed tool's drag holds Cmd. Shift or
        // Alt with Escape that the keymap binds to an enabled command is that command's chord.
        let bound = || !mods.is_none() && self.layout.candidates(input).iter().any(|chord| (self.bound)(chord));
        let escape = key == Key::Escape && (self.press_captured || (!mods.cmd && !mods.ctrl && !(self.records && !mods.is_none()) && !bound()));
        // A shortcut comes before a mnemonic: an Alt+letter the keymap binds to an enabled command runs it. On the
        // focused bar a letter without Alt, Ctrl or Cmd is a title's access key, and one no title holds goes nowhere
        // (the bar's access mode), so it runs no shortcut either.
        let on_bar = self.bar_focused && !mods.alt && !mods.ctrl && !mods.cmd;
        let mnemonic = self.menu_bar
            && !self.menu_bar_by_claim
            && self.os != Os::Mac
            && key.is_letter()
            && ((mods == Mods::ALT && !self.shortcut_text(input)) || on_bar);
        let edit = self.edit_chords.iter().find(|(command, chord)| (self.edits)(*command) && self.layout.matches(chord, input, self.os));
        let egui_edit =
            self.edit_chords.iter().any(|(command, chord)| self.egui_edits.contains(*command) && self.layout.matches(chord, input, self.os));
        // The chrome keys are Tab's, which the bare-Tab arm and `take_nav` read.
        let window_shortcut = self
            .window_shortcuts
            .iter()
            .find(|(key, chord)| !matches!(key, WindowShortcut::Chrome(_)) && self.layout.matches(chord, input, self.os))
            .map(|(key, _)| *key);
        if let Some((chord, egui)) = claimed {
            PressUse::Claimed { chord, egui }
        } else if let Some((command, _)) = edit {
            PressUse::Edit(*command)
        } else if key == Key::Space && self.canvas_space {
            PressUse::CanvasSpace
        } else if (self.foreign && (escape || plain_enter))
            || (bare_tab && self.tab_in_egui)
            || (self.host && escape && !self.press_captured)
            || egui_edit
        {
            PressUse::Egui
        } else if bare_tab && !self.keeps_tab {
            PressUse::Tab
        } else if ctrl_tab {
            PressUse::CtrlTab
        } else if let Some(WindowShortcut::Pane { forward }) = window_shortcut {
            PressUse::Pane(forward)
        } else if escape {
            PressUse::Escape
        } else if window_shortcut == Some(WindowShortcut::MenuBar) && self.menu_bar {
            PressUse::MenuBar
        } else if mnemonic {
            PressUse::Mnemonic(key.name().chars().next().unwrap_or('A'))
        } else {
            PressUse::Press
        }
    }

    fn press(&self, batch: &mut Batch, key: Key, input: &KeyInput, repeat: bool, at: Option<f64>) -> bool {
        let mods = input.mods;
        match self.use_of(key, input) {
            PressUse::Claimed { chord, egui } => {
                batch.claimed.push(chord);
                egui
            }
            PressUse::Edit(command) => {
                batch.clips.push((command, batch.place));
                false
            }
            PressUse::CanvasSpace => {
                // The canvas has Space while the pointer is over it (`runtime::sampler`): only the shortcut dispatcher reads
                // it, so the dialog's focused control does not activate, and a focused field neither types its space nor
                // keeps its momentary command from firing (production pans under a dialog whatever has focus).
                batch.presses.push(Press {
                    key,
                    mods,
                    repeat,
                    in_egui: None,
                    release: None,
                    text: None,
                    canvas: true,
                    place: batch.place,
                    held: Vec::new(),
                    ime: false,
                });
                if !repeat {
                    batch.shortcuts.push(Shortcut {
                        input: *input,
                        phase: FirePhase::Press,
                        press: Some(batch.presses.len() - 1),
                        canvas: true,
                        at,
                        place: batch.place,
                    });
                }
                false
            }
            // The app's own focused widget (a host stop included) or popup, or the canvas with nothing focused, reads
            // the key from egui; the runtime routes nothing for it, so no shortcut toggles the chrome or fires an edit
            // command with it either.
            PressUse::Egui => true,
            PressUse::Tab => {
                batch.tabs.push(TabPress { input: *input, repeat, at, place: batch.place });
                false
            }
            PressUse::CtrlTab => {
                batch.presses.push(Press {
                    key,
                    mods,
                    repeat,
                    in_egui: None,
                    release: None,
                    text: None,
                    canvas: false,
                    place: batch.place,
                    held: Vec::new(),
                    ime: false,
                });
                false
            }
            PressUse::Pane(forward) => {
                batch.nav.panes.push(forward);
                false
            }
            PressUse::Escape => {
                batch.escapes.push(EscapePress { mods, repeat });
                false
            }
            PressUse::MenuBar => {
                batch.menu.push(MenuKey::Focus);
                false
            }
            PressUse::Mnemonic(letter) => {
                batch.menu.push(MenuKey::Letter(letter));
                false
            }
            PressUse::Press => {
                batch.presses.push(Press {
                    key,
                    mods,
                    repeat,
                    in_egui: None,
                    release: None,
                    text: None,
                    canvas: false,
                    place: batch.place,
                    held: Vec::new(),
                    ime: false,
                });
                if !repeat || self.repeats {
                    batch.shortcuts.push(Shortcut {
                        input: *input,
                        phase: if repeat { FirePhase::Repeat } else { FirePhase::Press },
                        press: Some(batch.presses.len() - 1),
                        canvas: false,
                        at,
                        place: batch.place,
                    });
                }
                true
            }
        }
    }

    /// Whether `event` ends the pass's key events: a press that moves focus in the begin hook (Tab, a pane key) after
    /// key input of this pass, which the focused component reads in the pass. The begin hook moves focus before any
    /// component runs, so the press and every event after it wait for the next frame (`HeldInput`): the component
    /// reads what came before the move, and the move and what follows it apply in the order they arrived.
    fn ends_pass(&self, batch: &Batch, event: &KeyEvent) -> bool {
        let KeyEvent::Press { key, input, .. } = event else { return false };
        let typed =
            !batch.texts.is_empty() || !batch.presses.is_empty() || !batch.clips.is_empty() || !batch.claimed.is_empty() || !batch.escapes.is_empty();
        typed && matches!(self.use_of(*key, input), PressUse::Tab | PressUse::Pane(_))
    }
}

/// What the key stage makes of one press (`Strip::use_of`).
enum PressUse {
    /// The key target or a capture claims the chord. `egui`: the claim is an app's own egui widget's
    /// (`Stop::host`), which reads the press from egui's input, so egui keeps it.
    Claimed {
        chord: Chord,
        egui: bool,
    },
    /// The chord of an edit command the focused component lists.
    Edit(EditCommand),
    /// The Space of the canvas a dialog keeps open.
    CanvasSpace,
    /// The app's own widget or popup, or the canvas with nothing focused, reads the key from egui.
    Egui,
    /// A bare Tab: a focus move or a chrome toggle, which `take_nav` decides.
    Tab,
    /// The page switch of the focused stop.
    CtrlTab,
    /// A pane move, forward when `true`.
    Pane(bool),
    Escape,
    /// The window's key that focuses the menu bar.
    MenuBar,
    /// Alt and a letter: the menu bar's mnemonic.
    Mnemonic(char),
    /// Any other press: routing and the shortcut dispatcher read it.
    Press,
}

/// The chords the focused component of `record` claims: its stop's, the state-dependent ones (`keys::claim`) of the
/// component, its arena root and its menu level, and a bare End where it declares `EndKey::EndMode`. With nothing
/// focused and no modal layer open, the window's document canvas's state-dependent claims hold instead, except Tab and
/// Shift+Tab.
pub(super) fn focused_claims(record: &focus::FocusedRecord, stage: &Stage) -> Vec<Chord> {
    let mut claims = record.claims.clone();
    claims.extend(state_claims(record, stage));
    claims
}

/// The chords of `focused_claims` that the focused stop does not declare: the state-dependent ones (the unfocused
/// canvas's included) and the End of `EndKey::EndMode`.
fn state_claims(record: &focus::FocusedRecord, stage: &Stage) -> Vec<Chord> {
    let mut claims = Vec::new();
    let canvas = record.canvas.filter(|_| !record.foreign);
    let holds = |owner: Id| [record.focused, record.root, record.menu, record.menu_bar].contains(&Some(owner));
    // The unfocused canvas's tool never takes Tab or Shift+Tab: with nothing focused they move focus into the window,
    // so a tool that claims the plain keys cannot trap keyboard navigation.
    let canvas_holds =
        |owner: Id, chord: &Chord| Some(owner) == canvas && !(chord.key == Key::Tab && (chord.mods == Mods::NONE || chord.mods == Mods::SHIFT));
    claims.extend(stage.claims.iter().filter(|(owner, chord)| holds(*owner) || canvas_holds(*owner, chord)).map(|(_, chord)| *chord));
    // A bare End belongs to a stop that declares `EndKey::EndMode`: it reads the key as a claimed chord and toggles its
    // mode, so no collection navigation or scroll owner takes it as a move to the end.
    if record.keys.is_some_and(|keys| keys.end == EndKey::EndMode) {
        claims.push(Chord::plain(Key::End));
    }
    claims
}

/// Whether a bare Tab press that a chrome command is bound to toggles the window's panels (spec 1.8) instead of
/// moving focus. `enabled` is `RootEnv.tab_toggles_chrome`; `canvas` is whether the window registered a document
/// canvas (a stop with `KeyUse::chrome_key`) in its last pass; `layer` is what the layer stack says about the focus
/// and `focused` is the `KeyUse` of the focused component, `None` when nothing is focused.
///
/// `pressed` is whether a pointer press moved focus onto the focused component (`focus::origin`).
///
/// A modal or dialog layer never toggles: Tab moves focus inside it. In a window with a canvas the toggle is on when
/// nothing is focused, the canvas is, or a press focused a component other than a text field, as clicking a panel's row
/// and pressing Tab hides the panels in Photoshop; read-only text a press focused to select from (a stop only a press
/// reaches, a label's selection part) is no text field. Tab moves focus from a text field however it got the focus (Tab
/// commits a typed value and goes to the next field) and from a component the keyboard or a request focused. Elsewhere
/// it is on only when the canvas is focused. A component that keeps Tab never reaches this function: the press is its
/// own.
fn tab_toggles_chrome(enabled: bool, canvas: bool, layer: overlay::LayerKeys, focused: Option<&focus::Stop>, pressed: bool) -> bool {
    let closed = matches!(layer, overlay::LayerKeys::Blocked | overlay::LayerKeys::Class(OwnerClass::Dialog));
    if !enabled || closed {
        return false;
    }
    let on_canvas = focused.is_some_and(|stop| stop.keys.chrome_key);
    let pressed_control = pressed && focused.is_some_and(|stop| stop.keys.owner != OwnerClass::Text || stop.press_only);
    if canvas { focused.is_none() || on_canvas || pressed_control } else { on_canvas }
}

/// Whether a bare Tab pressed now in the running viewport would toggle its panels rather than move focus, by
/// [`tab_toggles_chrome`] over the viewport's root env, canvas, layers and focus.
pub(super) fn tab_toggles_now(ctx: &Context) -> bool {
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let enabled = Env::root_for(ctx, viewport).is_some_and(|env| env.tab_toggles_chrome);
    let focused = focus::focused_rec(ctx);
    let canvas = focus::canvas_stop(ctx, viewport).is_some();
    let layer = overlay::key_layer(ctx, &focused.as_ref().map(|rec| focus::scope_chain(ctx, rec)).unwrap_or_default(), None);
    let pressed = focused.as_ref().is_some_and(|rec| focus::origin(ctx, rec) == Some(focus::FocusOrigin::Press));
    tab_toggles_chrome(enabled, canvas, layer, focused.as_ref().map(|rec| &rec.stop), pressed)
}

/// Whether `chord` is the key of a chrome command (`ChromeCommand`), as the window's keymap binds it.
pub(super) fn is_chrome_chord(ctx: &Context, chord: &Chord) -> bool {
    commands::window_shortcuts(ctx).iter().any(|(key, bound)| matches!(key, WindowShortcut::Chrome(_)) && bound == chord)
}

/// The toggle of the chrome command whose bound chord `input` presses, when that command is enabled.
pub(super) fn chrome_toggle_of(ctx: &Context, input: &KeyInput) -> Option<ChromeToggle> {
    let os = crate::runtime::windows::os_of(ctx);
    let layout = layout_of(ctx);
    let command = commands::window_shortcuts(ctx).iter().find_map(|(key, chord)| match key {
        WindowShortcut::Chrome(command) if layout.matches(chord, input, os) => Some(*command),
        _ => None,
    })?;
    commands::resolve(ctx, command).enabled.then_some(command.toggle())
}

/// The input hook's key steps (H3) on `events`, the input's events with their arrival times. Reads only `RawInput`
/// and records keyed by `input.viewport_id`. Returns whether it removed an event, so the hook asks for a repaint, and
/// the events that wait for the next frame (`Strip::ends_pass`), in order.
pub(crate) fn input_hook(ctx: &Context, input: &RawInput, events: &mut Vec<Arrived>) -> (bool, Vec<Arrived>) {
    let viewport = input.viewport_id;
    // Taken first, before any early return: the raw-input step set it in this hook call, and it must not reach a later
    // one.
    let alt_code_ended = raw_input::take_alt_code_ended(ctx, viewport);
    // The canvas a modal layer keeps open takes the events its reach admits and nothing else (`runtime::sampler`).
    let (hole, trimmed) = sampler::hook(ctx, input, events);
    // A modal layer in another viewport blocks this one (app-modal, 15.5): nothing below reaches the key stage.
    if overlay::modal_elsewhere(ctx, viewport) {
        let pointer = ctx.input(|state| state.pointer.latest_pos());
        let (removed, press) = raw_input::drop_under_modal(events, hole.as_ref(), pointer);
        if press {
            overlay::note_dropped_press(ctx, viewport);
        }
        with(ctx, viewport, |stage| {
            stage.keyed = false;
            stage.stirred = false;
            stage.textless_paste = false;
        });
        return (removed || trimmed, Vec::new());
    }
    let os = crate::runtime::windows::os_of(ctx);
    let record = focus::record(ctx, viewport);
    let keys = record.keys;
    let menu_bar = record.menu_bar.is_some() && keys.is_none_or(|keys| !focus::takes_letters(&keys));
    let before = events.len();
    let target_canvas = target_canvas(ctx, viewport, &record);
    let canvas = target_canvas.as_ref().map(|stop| stop.keys);
    let lists_edits = |keys: &KeyUse| EditCommands::ALL.iter().any(|command| command.edit().is_some_and(|edit| keys.edits.contains(edit)));
    let edit_chords = if keys.as_ref().or(canvas.as_ref()).is_some_and(lists_edits) { commands::edit_chords(ctx) } else { Arc::default() };
    let window_shortcuts = commands::window_shortcuts(ctx);
    let latin = &input.latin_layout_chars;
    let requested_paste = commands::board::has_pending_paste(ctx, viewport);
    let mut requested_content = None::<commands::ClipContent>;
    // S-L177: an Alt chord's text is withheld only when the press can run its command; a bound command that is
    // disabled (Word's Alt+Shift+- Collapse outside Outline) leaves the text typed (Option+Shift+- is an em dash).
    let bound = |chord: &Chord| commands::runs_now(ctx, viewport, chord);
    let press_captured = crate::runtime::pointer::press_captured_in(ctx, viewport) || crate::runtime::drag::running(ctx);
    let repeats = Env::root_for(ctx, viewport).is_some_and(|env| env.shortcuts_repeat);
    let tab_in_egui = record.tab_in_egui || focus::tab_has_no_stop(ctx, viewport);
    let (batch, visible, later) = with(ctx, viewport, |stage| {
        let claims = stage_claims(&record, target_canvas.as_ref(), stage);
        let edits = |command| !record.host && keys.is_some_and(|keys| keys.edits.contains(command));
        stage.layout.set_latin(input.keyboard_layout_is_latin);
        let strip = Strip {
            os,
            layout: &stage.layout,
            keeps_tab: keys.is_some_and(|keys| keys.keep == focus::KeepKeys::Tab),
            pages: record.pages,
            records: keys.is_some_and(|keys| matches!(keys.owner, OwnerClass::Recorder)),
            press_captured,
            claims: &claims,
            edits: &edits,
            edit_chords: &edit_chords,
            window_shortcuts: &window_shortcuts,
            menu_bar,
            menu_bar_by_claim: record.menu_bar_by_claim,
            bar_focused: record.menu_bar.is_some() && record.focused == record.menu_bar,
            holds_drafts: keys.is_some_and(|keys| keys.holds_drafts),
            canvas_space: hole.as_ref().is_some_and(|hole| hole.space || hole.space_pressed),
            foreign: record.foreign,
            tab_in_egui,
            host: record.host,
            egui_edits: keys.filter(|_| record.host).or(canvas).map_or(EditSet::NONE, |keys| keys.edits),
            label_selection: record.label_selection,
            alt_claimed: !stage.alt_claims.is_empty(),
            in_menu: record.menu.is_some(),
            alt_code_ended,
            bound: &bound,
            repeats,
        };
        let mut batch = Batch::default();
        let mut visible = None;
        let mut later = Vec::new();
        let mut released = false;
        stage.keyed = false;
        stage.stirred = false;
        stage.textless_paste = false;
        let mut place = 0;
        let ime_presses = std::mem::take(&mut stage.ime_presses);
        let press_next: Vec<bool> = (0..events.len())
            .map(|at| matches!(events.get(at + 1).map(|arrived| &arrived.event), Some(egui::Event::Key { pressed: true, .. })))
            .collect();
        events.retain(|arrived| {
            // Every event the IME engine left counts, as the engine counts them (`ime::take_placed`).
            batch.place = place;
            place += 1;
            let read = raw_input::key_event(&arrived.event, os, latin, stage.alt.last.unwrap_or(Mods::NONE));
            // Sensing commits a release during the pass, after the key stage. Let it finish before a later Escape;
            // otherwise Escape would cancel a gesture that the user had already released.
            let escape_after_release =
                released && matches!(&read, KeyEvent::Press { key, input, .. } if matches!(strip.use_of(*key, input), PressUse::Escape));
            if !later.is_empty() || escape_after_release || strip.ends_pass(&batch, &read) {
                later.push(arrived.clone());
                return false;
            }
            released |= matches!(arrived.event, egui::Event::PointerButton { pressed: false, .. });
            // Read from the event the pass applies, so a held-back one is not read twice.
            batch.focus_lost |= matches!(arrived.event, egui::Event::WindowFocused(false));
            if requested_paste && let KeyEvent::Clip { command: EditCommand::Paste, pasted: Some(content), .. } = &read {
                let combined = requested_content.get_or_insert_default();
                combined.text = content.text.clone().or(combined.text.take());
                combined.image = content.image.clone().or(combined.image.take());
                return false;
            }
            stage.keyed |= matches!(read, KeyEvent::Press { .. } | KeyEvent::Text(_) | KeyEvent::Clip { .. });
            stage.stirred |= matches!(arrived.event, egui::Event::Key { repeat: false, .. });
            stage.textless_paste |=
                matches!(&read, KeyEvent::Clip { command: EditCommand::Paste, pasted, .. } if pasted.as_ref().is_none_or(|c| c.text.is_none()));
            visible = match &read {
                KeyEvent::PointerPress => Some(false),
                // An accessibility activation is not the preceding key: a menu it opens must wait for its own navigation.
                KeyEvent::Access(action) if matches!(action.kind(), raw_input::ActionKind::Click | raw_input::ActionKind::ShowContextMenu) => {
                    Some(false)
                }
                KeyEvent::Press { .. } | KeyEvent::Text(_) => Some(true),
                _ => visible,
            };
            let down = stage.down.clone();
            note_down(&mut stage.down, &read);
            let arrival = Arrival {
                at: arrived.at,
                ime_next: ime_presses.contains(&arrived.event),
                press_next: press_next.get(batch.place).copied().unwrap_or(false),
            };
            strip.event(&mut batch, &mut stage.alt, &mut stage.consumed, &down, read, arrival)
        });
        for (physical, mods, text) in std::mem::take(&mut batch.learned) {
            stage.layout.observe(physical, mods, &text);
        }
        (batch, visible, later)
    });
    if let Some(content) = requested_content {
        commands::board::deliver_requested_paste(ctx, viewport, content);
    }
    if let Some(visible) = visible {
        focus::set_visible(ctx, viewport, visible);
    }
    if batch.refused {
        crate::runtime::frame::refuse(ctx);
    }
    with(ctx, viewport, |stage| {
        stage.batch = batch;
    });
    (trimmed || events.len() != before, later)
}

/// The chords the key stage takes as claimed (`PressUse::Claimed`).
struct Claims {
    /// The captures', the menu level's and the menu bar's, and the key target's when it is a framework component.
    framework: Vec<Chord>,
    /// The ones the key target declares (`KeyUse::claims`) when it is an app's own egui widget (`Stop::host`): egui
    /// keeps their presses, where the app reads them.
    host: Vec<Chord>,
}

/// The window's document canvas as the key target of `viewport`'s input: nothing has the focus, neither a framework
/// stop nor an app's own widget or popup (`Strip::canvas`).
fn target_canvas(ctx: &Context, viewport: ViewportId, record: &focus::FocusedRecord) -> Option<focus::Stop> {
    focus::canvas_stop(ctx, viewport).filter(|_| record.keys.is_none() && !record.foreign)
}

/// The claims of the key stage: the focused component's (`focused_claims`) and every capture's. With nothing focused
/// the window's document canvas `canvas` (`target_canvas`) stands for the focused component with the chords it
/// declares, as it does for a fired command (`claims::fire_claimed`).
fn stage_claims(record: &focus::FocusedRecord, canvas: Option<&focus::Stop>, stage: &Stage) -> Claims {
    let (mut framework, mut host) =
        if record.host { (state_claims(record, stage), record.claims.clone()) } else { (focused_claims(record, stage), Vec::new()) };
    framework.extend(stage.captures.iter().map(|(_, chord)| *chord));
    if let Some(canvas) = canvas {
        let to = if canvas.host { &mut host } else { &mut framework };
        to.extend_from_slice(canvas.keys.claims);
    }
    Claims { framework, host }
}

/// Whether the key stage of `input`'s viewport will take a press as a claimed chord (`PressUse::Claimed`), read before
/// the stage runs in the same hook call: the raw-input step leaves such an Alt+digit to its owner instead of folding it
/// into an Alt code (`alt_code::fold`).
pub(crate) fn claims_press(ctx: &Context, input: &RawInput) -> impl Fn(&egui::Event) -> bool + use<> {
    let viewport = input.viewport_id;
    let os = crate::runtime::windows::os_of(ctx);
    let record = focus::record(ctx, viewport);
    let canvas = target_canvas(ctx, viewport, &record);
    let (claims, mut layout) = with(ctx, viewport, |stage| (stage_claims(&record, canvas.as_ref(), stage), stage.layout.clone()));
    layout.set_latin(input.keyboard_layout_is_latin);
    let latin = input.latin_layout_chars.clone();
    // Only a press is read, and it carries its own modifiers.
    move |event: &egui::Event| match raw_input::key_event(event, os, &latin, Mods::NONE) {
        KeyEvent::Press { input: pressed, .. } => claims.framework.iter().chain(&claims.host).any(|chord| layout.matches(chord, &pressed, os)),
        _ => false,
    }
}

/// Notes the key presses of `events` that an input method's commit follows, before the IME stage takes the commits out
/// of the input: the key stage reads such a press as typing through the input method (`Press::ime`).
pub(crate) fn note_ime_presses(ctx: &Context, viewport: ViewportId, events: &[Arrived]) {
    let commit = |event: &egui::Event| matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty());
    let presses = events
        .windows(2)
        .filter(|pair| matches!(pair[0].event, egui::Event::Key { pressed: true, .. }) && commit(&pair[1].event))
        .map(|pair| pair[0].event.clone())
        .collect();
    with(ctx, viewport, |stage| stage.ime_presses = presses);
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

/// The keys down in the running viewport this pass: the key stage's `down` (withheld presses included), plus each key
/// automation holds there that is not already listed. It gives the same answer on every pass of a frame, and is a
/// guard, never an event.
// PLAN: the contract says `Vec<egui::Key>`, but the stage and `injected_holds` use `craft_keymap::Key`.
pub fn held(ctx: &Context) -> Vec<Key> {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keys);
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let mut down = current(ctx, |stage| stage.down.clone());
    for key in crate::runtime::windows::WindowHost::of(ctx).injected_holds(viewport) {
        if !down.contains(&key) {
            down.push(key);
        }
    }
    down
}

/// The key state, as the tracked input `Keys` compares it: the keys down, whether a key arrived, whether a paste with no
/// text arrived and Caps Lock.
pub(crate) fn keys_state(ctx: &Context) -> (Vec<Key>, bool, bool, bool) {
    let (down, keyed, textless_paste) = current(ctx, |stage| (stage.down.clone(), stage.keyed, stage.textless_paste));
    (down, keyed, textless_paste, crate::runtime::lock_keys::caps_on(ctx))
}

/// Counts an input of `viewport` whose `events` hold one sent to the focus: a key, text, IME, clipboard or AccessKit
/// action event (`Input::Sent`). Read before any stage takes events, so the IME's and the ones a shortcut takes count.
pub(crate) fn count_sent(ctx: &Context, viewport: ViewportId, events: &[Arrived]) {
    let sent = events.iter().any(|arrived| {
        matches!(
            arrived.event,
            egui::Event::Key { .. }
                | egui::Event::Text(_)
                | egui::Event::Ime(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Paste(_)
                | egui::Event::PasteImage(_)
                | egui::Event::MiddleClickPaste { .. }
                | egui::Event::AccessKitActionRequest(_)
        )
    });
    if sent {
        with(ctx, viewport, |stage| stage.sent += 1);
    }
}

/// How many inputs of the running viewport held an event sent to the focus: its value moves with each one.
pub(crate) fn sent(ctx: &Context) -> u64 {
    current(ctx, |stage| stage.sent)
}

/// Whether the running viewport's input hook read a key press, typed text or a clipboard event in this pass, the ones
/// the key stage took from egui's input included: an app that redraws state after input asks this, not egui's events.
pub fn keyed(ctx: &Context) -> bool {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keys);
    current(ctx, |stage| stage.keyed)
}

/// Whether a key went down or up in the running viewport's pass, repeats aside: a press the key stage took from egui
/// (a claimed chord, the canvas's Space) and its release count, so a held pointer reads the change.
pub(crate) fn stirred(ctx: &Context) -> bool {
    current(ctx, |stage| stage.stirred)
}

/// Whether the running viewport's input hook read a paste with no text in this pass: the paste chord came while the
/// clipboard held no text (a picture, a PDF), so the platform sent no text to paste. A focused text field takes the
/// event, so an app asks this, not egui's events.
pub fn textless_paste(ctx: &Context) -> bool {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keys);
    current(ctx, |stage| stage.textless_paste)
}

/// Takes the Tab and F6 moves the input hook recorded, for the focus service's begin hook. The pass's root env is
/// installed by now, so a bare Tab that passes the chrome gate and has a chrome command bound to it becomes that
/// command's toggle for the running viewport; one the keymap binds to another enabled command instead (an app command
/// the chrome command yielded the chord to) is that command's press, which the routing hook after this one hands the
/// dispatcher; any other Tab moves focus. A repeat of a toggling or firing press is dropped: it neither acts again nor
/// moves focus.
pub(crate) fn take_nav(ctx: &Context) -> NavKeys {
    let (mut nav, tabs) = current(ctx, |stage| {
        let mut nav = std::mem::take(&mut stage.batch.nav);
        nav.panes.append(&mut stage.fired_panes);
        (nav, std::mem::take(&mut stage.batch.tabs))
    });
    if tabs.is_empty() {
        return nav;
    }
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let gate = tab_toggles_now(ctx);
    let layout = layout_of(ctx);
    let fires = |input: &KeyInput| layout.candidates(input).iter().any(|chord| commands::runs_now(ctx, viewport, chord));
    for press in tabs {
        match gate.then(|| chrome_toggle_of(ctx, &press.input)).flatten() {
            Some(toggle) if !press.repeat => request_chrome_toggle(ctx, viewport, toggle),
            Some(_) => {}
            None if gate && fires(&press.input) => {
                if !press.repeat {
                    let TabPress { input, at, place, .. } = press;
                    let shortcut = Shortcut { input, phase: FirePhase::Press, press: None, canvas: false, at, place };
                    current(ctx, |stage| stage.batch.shortcuts.push(shortcut));
                }
            }
            None => nav.tabs.push(!press.input.mods.shift),
        }
    }
    nav
}

/// Queues what a menu, palette or automation fire of `command` asks of `viewport`: a pane move, or the menu bar's
/// focus, in its next pass. A key fire moved already in the key stage.
pub(crate) fn queue_nav(ctx: &Context, viewport: ViewportId, command: NavCommand) {
    with(ctx, viewport, |stage| match command {
        NavCommand::NextPane => stage.fired_panes.push(true),
        NavCommand::PreviousPane => stage.fired_panes.push(false),
        NavCommand::FocusMenuBar => stage.fired_menu = true,
    });
    crate::runtime::egui::repaint::request_of(ctx, viewport);
}

/// The menu-bar keys of the pass (spec 9.7).
pub(crate) fn take_menu_keys(ctx: &Context) -> Vec<MenuKey> {
    current(ctx, |stage| std::mem::take(&mut stage.menu))
}
