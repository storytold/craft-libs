//! The input hook's key steps (H3): what it strips from each raw key event and records.

use super::route::layout_of;
use super::{AltTrack, Batch, LastPress, MenuKey, NavKeys, Press, Shortcut, Stage, TabPress, Typed, current, with};
use crate::runtime::commands::{self, EditCommands, FirePhase, NavCommand, WindowShortcut};
use crate::runtime::egui::input::{self as raw_input, Arrived, KeyEvent, RawInput};
use crate::runtime::env::Env;
use crate::runtime::focus::{self, EndKey, KeyUse};
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
    claims: &'a [Chord],
    edits: &'a dyn Fn(EditCommand) -> bool,
    /// The chords bound to the edit commands.
    edit_chords: &'a [(EditCommand, Chord)],
    /// The chords bound to the window's keys (`commands::window_shortcuts`).
    window_shortcuts: &'a [(WindowShortcut, Chord)],
    menu_bar: bool,
    /// The focused stop holds drafts (`KeyUse::holds_drafts`): a lone Alt alerts instead of focusing the menu bar.
    holds_drafts: bool,
    /// A dialog keeps a canvas open and Space belongs to it: the press is the canvas's momentary command chord.
    canvas_space: bool,
    /// An app's own egui widget or popup has the focus (`FocusedRecord::foreign`): Tab, Escape and the clipboard
    /// events are its own.
    foreign: bool,
    /// The keys of the window's canvas while nothing has the focus, neither a framework stop nor an app's own widget
    /// or popup: the canvas is then the key target, as the chrome gate takes it, and reads from egui the keys its
    /// `KeyUse` keeps (Tab with `KeepKeys::Tab`) and the clipboard events and edit chords of its edit set.
    canvas: Option<KeyUse>,
    /// egui holds no focus and the window registered no focus stop in its last pass: a framework focus move has
    /// nowhere to go, so a bare Tab is egui's. Focus on a component's widget without a stop is focus: egui would take
    /// a Tab from it as a move to the next widget it shows, in a later pass.
    no_stops: bool,
    /// egui holds a label text selection, which a copy or cut event copies.
    label_selection: bool,
}

impl Strip<'_> {
    /// Records `event` in `batch` and returns whether egui keeps it.
    ///
    /// `consumed` holds the keys whose press the strip took and whose release has not come yet: the release goes
    /// with the press, in this pass or a later one. `at` is the event's arrival time (`Arrived::at`).
    fn event(&self, batch: &mut Batch, alt: &mut AltTrack, consumed: &mut Vec<Key>, event: KeyEvent, at: Option<f64>) -> bool {
        if matches!(event, KeyEvent::Clip { .. }) {
            // A clipboard event stands for its chord's press, which types nothing.
            batch.last_press = None;
        }
        match event {
            KeyEvent::Press { key, input, repeat } => {
                alt.other_key = true;
                if key == Key::Enter {
                    batch.enter_down = Some(true);
                }
                let pushed = batch.presses.len();
                let claims = batch.claimed.len();
                let kept = self.press(batch, key, &input, repeat, at);
                // The canvas's Space keeps its text from every field, as a claim does.
                let canvas = batch.presses.get(pushed).is_some_and(|press| press.canvas);
                batch.last_press = Some(LastPress {
                    input,
                    pressed: (batch.presses.len() > pushed).then_some(pushed),
                    claimed: batch.claimed.len() > claims || canvas,
                });
                if kept {
                    if let Some(press) = batch.presses.get_mut(pushed) {
                        press.in_egui = Some(batch.egui_presses);
                        press.release = input.physical.or(input.typed);
                    }
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
                if key == Some(Key::Enter) {
                    batch.enter_down = Some(false);
                }
                batch.shortcuts.push(Shortcut { input, phase: FirePhase::Release, press: None, canvas: false, at });
                true
            }
            KeyEvent::Text(text) => {
                alt.other_key = true;
                let mut claimed = false;
                if let Some(last) = batch.last_press.take() {
                    if let Some(physical) = last.input.physical.filter(|_| !alt.right) {
                        batch.learned.push((physical, last.input.mods, text.clone()));
                    }
                    claimed = last.claimed;
                    if let Some(press) = last.pressed.and_then(|at| batch.presses.get_mut(at)) {
                        press.text = Some(batch.texts.len());
                    }
                }
                // Every text stays for now: the begin hook's routing decides which reader gets it (`route::begin`).
                batch.texts.push(Typed { text, claimed, in_egui: Some(batch.egui_texts) });
                batch.egui_texts += 1;
                true
            }
            KeyEvent::Clip { command, text } if (self.edits)(command) => {
                batch.clips.push(command);
                if text.is_some() {
                    batch.paste = text;
                }
                false
            }
            KeyEvent::Clip { .. } if self.foreign => true,
            KeyEvent::Clip { command, .. } if self.canvas.is_some_and(|canvas| canvas.edits.contains(command)) => true,
            KeyEvent::Clip { command: EditCommand::Copy | EditCommand::Cut, .. } if self.label_selection => true,
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
                    self.press(batch, key, &input, false, at);
                } else if !sent {
                    batch.shortcuts.push(Shortcut { input, phase: FirePhase::Press, press: None, canvas: false, at });
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

    /// A lone Alt press, pressed and released with no other key between, focuses the menu bar (9.7). In a field that
    /// holds drafts it only alerts, so the draft keeps its focus (L-52).
    fn alt_change(&self, batch: &mut Batch, alt: &mut AltTrack, held: bool) {
        if held && !alt.down {
            alt.other_key = false;
        }
        if !held && alt.down && !alt.other_key && self.os != Os::Mac {
            if self.holds_drafts {
                batch.refused = true;
            } else if self.menu_bar {
                batch.menu.push(MenuKey::Focus);
            }
        }
        alt.down = held;
    }

    /// What the key stage makes of a press of `key` with `input`.
    fn use_of(&self, key: Key, input: &KeyInput) -> PressUse {
        let mods = input.mods;
        let claimed = self.claims.iter().find(|chord| self.layout.matches(chord, input, self.os));
        let ctrl_tab = key == Key::Tab && (if self.os == Os::Mac { mods.ctrl && !mods.cmd } else { mods.cmd });
        let ctrl_tab = ctrl_tab && self.pages && !self.keeps_tab;
        let bare_tab = key == Key::Tab && !mods.cmd && !mods.ctrl && !mods.alt;
        // Alt and Shift ride along, as with Enter: a canvas tool reads them; Cmd and Ctrl chords are the app's. While
        // the focused stop records, Escape with any modifier held is a chord for the recorder; only plain Escape is a
        // scope escape.
        let escape = key == Key::Escape && !mods.cmd && !mods.ctrl && !(self.records && !mods.is_none());
        let mnemonic = self.menu_bar && self.os != Os::Mac && mods == Mods::ALT && key.is_letter();
        let edit = self.edit_chords.iter().find(|(command, chord)| (self.edits)(*command) && self.layout.matches(chord, input, self.os));
        let canvas_edit = self.canvas.is_some_and(|canvas| {
            self.edit_chords.iter().any(|(command, chord)| canvas.edits.contains(*command) && self.layout.matches(chord, input, self.os))
        });
        let canvas_tab = bare_tab && self.canvas.is_some_and(|canvas| canvas.keep == focus::KeepKeys::Tab);
        // The chrome keys are Tab's, which the bare-Tab arm and `take_nav` read.
        let window_shortcut = self
            .window_shortcuts
            .iter()
            .find(|(key, chord)| !matches!(key, WindowShortcut::Chrome(_)) && self.layout.matches(chord, input, self.os))
            .map(|(key, _)| *key);
        if let Some(chord) = claimed {
            PressUse::Claimed(*chord)
        } else if let Some((command, _)) = edit {
            PressUse::Edit(*command)
        } else if key == Key::Space && self.canvas_space {
            PressUse::CanvasSpace
        } else if (self.foreign && (bare_tab || escape)) || canvas_tab || canvas_edit {
            PressUse::Egui
        } else if bare_tab && self.no_stops {
            PressUse::TabToEgui
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
            PressUse::Claimed(chord) => {
                batch.claimed.push(chord);
                false
            }
            PressUse::Edit(command) => {
                batch.clips.push(command);
                false
            }
            PressUse::CanvasSpace => {
                // The canvas has Space while the pointer is over it (`runtime::sampler`): only the shortcut dispatcher reads
                // it, so the dialog's focused control does not activate, and a focused field neither types its space nor
                // keeps its momentary command from firing (production pans under a dialog whatever has focus).
                batch.presses.push(Press { key, mods, repeat, in_egui: None, release: None, text: None, canvas: true });
                if !repeat {
                    batch.shortcuts.push(Shortcut { input: *input, phase: FirePhase::Press, press: Some(batch.presses.len() - 1), canvas: true, at });
                }
                false
            }
            // The app's own focused widget or popup, or the canvas with nothing focused, reads the key from egui; the
            // runtime routes nothing for it, so no shortcut toggles the chrome or fires an edit command with it either.
            PressUse::Egui => true,
            PressUse::TabToEgui => {
                // egui moves its own focus with it, from nothing focused into the app's own widgets (`focus::begin`).
                batch.nav.tab_to_egui = true;
                true
            }
            PressUse::Tab => {
                batch.tabs.push(TabPress { input: *input, repeat });
                false
            }
            PressUse::CtrlTab => {
                batch.presses.push(Press { key, mods, repeat, in_egui: None, release: None, text: None, canvas: false });
                false
            }
            PressUse::Pane(forward) => {
                batch.nav.panes.push(forward);
                false
            }
            PressUse::Escape => {
                batch.escapes += 1;
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
                batch.presses.push(Press { key, mods, repeat, in_egui: None, release: None, text: None, canvas: false });
                if !repeat {
                    batch.shortcuts.push(Shortcut {
                        input: *input,
                        phase: FirePhase::Press,
                        press: Some(batch.presses.len() - 1),
                        canvas: false,
                        at,
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
        let typed = !batch.texts.is_empty() || !batch.presses.is_empty() || !batch.clips.is_empty() || !batch.claimed.is_empty() || batch.escapes > 0;
        typed && matches!(self.use_of(*key, input), PressUse::Tab | PressUse::Pane(_))
    }
}

/// What the key stage makes of one press (`Strip::use_of`).
enum PressUse {
    /// The focused component or a capture claims the chord.
    Claimed(Chord),
    /// The chord of an edit command the focused component lists.
    Edit(EditCommand),
    /// The Space of the canvas a dialog keeps open.
    CanvasSpace,
    /// The app's own widget or popup, or the canvas with nothing focused, reads the key from egui.
    Egui,
    /// A bare Tab egui moves its own focus with.
    TabToEgui,
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
/// component, its arena root and its menu level, and a bare End where it declares `EndKey::EndMode`.
pub(super) fn focused_claims(record: &focus::FocusedRecord, stage: &Stage) -> Vec<Chord> {
    let mut claims = record.claims.clone();
    let holds = |owner: Id| [record.focused, record.root, record.menu].contains(&Some(owner));
    claims.extend(stage.claims.iter().filter(|(owner, _)| holds(*owner)).map(|(_, chord)| *chord));
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
/// and pressing Tab hides the panels in Photoshop. Tab moves focus from a text field however it got the focus (Tab
/// commits a typed value and goes to the next field) and from a component the keyboard or a request focused. Elsewhere
/// it is on only when the canvas is focused. A component that keeps Tab never reaches this function: the press is its
/// own.
fn tab_toggles_chrome(enabled: bool, canvas: bool, layer: overlay::LayerKeys, focused: Option<&KeyUse>, pressed: bool) -> bool {
    let closed = matches!(layer, overlay::LayerKeys::Blocked | overlay::LayerKeys::Class(OwnerClass::Dialog));
    if !enabled || closed {
        return false;
    }
    let on_canvas = focused.is_some_and(|keys| keys.chrome_key);
    let pressed_control = pressed && focused.is_some_and(|keys| keys.owner != OwnerClass::Text);
    if canvas { focused.is_none() || on_canvas || pressed_control } else { on_canvas }
}

/// Whether a bare Tab pressed now in the running viewport would toggle its panels rather than move focus, by
/// [`tab_toggles_chrome`] over the viewport's root env, canvas, layers and focus.
pub(super) fn tab_toggles_now(ctx: &Context) -> bool {
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let enabled = Env::root_for(ctx, viewport).is_some_and(|env| env.tab_toggles_chrome);
    let focused = focus::focused_rec(ctx);
    let canvas = focus::canvas_keys(ctx, viewport).is_some();
    let layer = overlay::key_layer(ctx, &focused.as_ref().map(|rec| focus::scope_chain(ctx, rec)).unwrap_or_default(), None);
    let pressed = focused.as_ref().is_some_and(|rec| focus::origin(ctx, rec) == Some(focus::FocusOrigin::Press));
    tab_toggles_chrome(enabled, canvas, layer, focused.as_ref().map(|rec| &rec.stop.keys), pressed)
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
    // The canvas a modal layer keeps open takes the events its reach admits and nothing else (`runtime::sampler`).
    let (hole, trimmed) = sampler::hook(ctx, input, events);
    // A modal layer in another viewport blocks this one (app-modal, 15.5): nothing below reaches the key stage.
    if overlay::modal_elsewhere(ctx, viewport) {
        let pointer = ctx.input(|state| state.pointer.latest_pos());
        let (removed, press) = raw_input::drop_under_modal(events, hole.as_ref(), pointer);
        if press {
            overlay::note_dropped_press(ctx, viewport);
        }
        with(ctx, viewport, |stage| stage.keyed = false);
        return (removed || trimmed, Vec::new());
    }
    let os = crate::runtime::windows::os_of(ctx);
    let record = focus::record(ctx, viewport);
    let keys = record.keys;
    let menu_bar = record.menu_bar.is_some() && keys.is_none_or(|keys| !matches!(keys.owner, OwnerClass::Text | OwnerClass::Recorder));
    let before = events.len();
    let canvas = focus::canvas_keys(ctx, viewport).filter(|_| keys.is_none() && !record.foreign);
    let lists_edits = |keys: &KeyUse| EditCommands::ALL.iter().any(|command| command.edit().is_some_and(|edit| keys.edits.contains(edit)));
    let edit_chords = if keys.as_ref().or(canvas.as_ref()).is_some_and(lists_edits) { commands::edit_chords(ctx) } else { Arc::default() };
    let window_shortcuts = commands::window_shortcuts(ctx);
    let no_stops = keys.is_none() && !record.foreign && focus::tab_from_nothing(ctx, viewport);
    let latin = &input.latin_layout_chars;
    let (batch, visible, later) = with(ctx, viewport, |stage| {
        let mut claims = focused_claims(&record, stage);
        claims.extend(stage.captures.iter().map(|(_, chord)| *chord));
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
            window_shortcuts: &window_shortcuts,
            menu_bar,
            holds_drafts: keys.is_some_and(|keys| keys.holds_drafts),
            canvas_space: hole.as_ref().is_some_and(|hole| hole.space || hole.space_pressed),
            foreign: record.foreign,
            canvas,
            no_stops,
            label_selection: record.label_selection,
        };
        let mut batch = Batch::default();
        let mut visible = None;
        let mut later = Vec::new();
        stage.keyed = false;
        events.retain(|arrived| {
            let read = raw_input::key_event(&arrived.event, os, latin);
            if !later.is_empty() || strip.ends_pass(&batch, &read) {
                later.push(arrived.clone());
                return false;
            }
            note_down(&mut stage.down, &read);
            stage.keyed |= matches!(read, KeyEvent::Press { .. } | KeyEvent::Text(_) | KeyEvent::Clip { .. });
            visible = match &read {
                KeyEvent::PointerPress => Some(false),
                KeyEvent::Press { .. } | KeyEvent::Text(_) => Some(true),
                _ => visible,
            };
            strip.event(&mut batch, &mut stage.alt, &mut stage.consumed, read, arrived.at)
        });
        for (physical, mods, text) in std::mem::take(&mut batch.learned) {
            stage.layout.observe(physical, mods, &text);
        }
        (batch, visible, later)
    });
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

/// Whether the running viewport's input hook read a key press, typed text or a clipboard event in this pass, the ones
/// the key stage took from egui's input included: an app that redraws state after input asks this, not egui's events.
pub fn keyed(ctx: &Context) -> bool {
    current(ctx, |stage| stage.keyed)
}

/// Takes the Tab and F6 moves the input hook recorded, for the focus service's begin hook. The pass's root env is
/// installed by now, so a bare Tab that passes the chrome gate and has a chrome command bound to it becomes that
/// command's toggle for the running viewport; any other Tab moves focus. A repeat of a toggling press is dropped: it
/// neither toggles again nor moves focus.
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
    for press in tabs {
        match gate.then(|| chrome_toggle_of(ctx, &press.input)).flatten() {
            Some(toggle) if !press.repeat => request_chrome_toggle(ctx, viewport, toggle),
            Some(_) => {}
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
