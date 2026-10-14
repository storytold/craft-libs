//! The app's side: events from the platform menu in, menu bars out.

use crate::chord::Chord;
use crate::layout::{Clash, Layout};
use crate::model::{MINIMIZE, MenuBar};

/// What the native menu reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An item was chosen with the mouse (or the keyboard inside an open menu), or by a key
    /// equivalent the app's key handling doesn't know ([`crate::Item::layout_shortcut`]). Only
    /// the item's id: a parameter the command needs goes in the id.
    Click(String),
    /// A key equivalent was pressed: the shortcut, to be handled as the key press it was.
    Key(Chord),
}

/// How ⌘C, ⌘X and ⌘V reach egui when the native menu takes them as key equivalents (an Edit
/// menu with Copy, Cut and Paste items).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClipboardKeys {
    /// As egui-winit makes them without a native menu: ⌘C is [`egui::Event::Copy`], ⌘X
    /// [`egui::Event::Cut`], and ⌘V a [`egui::ViewportCommand::RequestPaste`], which eframe answers
    /// with an [`egui::Event::Paste`] of the clipboard's text in the next frame. Text fields copy,
    /// cut and paste.
    #[default]
    Egui,
    /// Key presses like any other key equivalent, for an app that runs its clipboard commands
    /// from the keys itself.
    Raw,
}

/// A platform's native menu bar.
pub trait Backend {
    /// Bring the menu in step with `bar`: update labels, enabled and checked in place, or
    /// rebuild when its structure changed ([`crate::structure_key`], [`crate::window_key`]).
    fn sync(&mut self, bar: &MenuBar);
    /// Events since the last call. Platform items (Hide, Services, Zoom, …) are handled by the
    /// backend and not reported.
    fn drain(&mut self) -> Vec<Event>;
    /// Whether one of the native menus is open.
    fn tracking(&self) -> bool {
        false
    }
    /// Close the open native menu, if there is one.
    fn cancel_tracking(&mut self) {}
    /// Whether a text field has the keys ([`NativeMenu::set_text_editing`]), for
    /// the choice between reporting a key equivalent as a key press or a click.
    fn set_text_editing(&mut self, _editing: bool) {}
}

/// The native menu bar, while the app uses one (its in-window menus are hidden then).
pub struct NativeMenu {
    backend: Box<dyn Backend>,
    /// The state the menus were last built from.
    state: Option<u64>,
    clicks: Vec<String>,
    /// An event arrived (AppKit may have toggled a check item itself): re-sync.
    dirty: bool,
    clashes: Vec<Clash>,
    clipboard: ClipboardKeys,
    /// ⌘V arrived: [`NativeMenu::take_clicks`] asks eframe for the paste.
    paste: bool,
    /// Whether a click may be kept from egui as the one closing an open menu (see
    /// [`NativeMenu::raw_input`]).
    armed: bool,
}

impl NativeMenu {
    pub fn new(backend: Box<dyn Backend>) -> NativeMenu {
        NativeMenu {
            backend,
            state: None,
            clicks: Vec::new(),
            dirty: true,
            clashes: Vec::new(),
            clipboard: ClipboardKeys::default(),
            paste: false,
            armed: true,
        }
    }

    pub fn clipboard_keys(mut self, keys: ClipboardKeys) -> NativeMenu {
        self.clipboard = keys;
        self
    }

    /// Whether a text field has the keys (in egui, `ctx.wants_keyboard_input()`). Call it every
    /// frame, before the next [`NativeMenu::raw_input`]. While it is on, a key equivalent runs
    /// its item as an [`Event::Click`], as a Mac app's menu commands run while typing, except
    /// the text-editing keys the field handles itself: ⌘C, ⌘X, ⌘V, ⌘A, ⌘Z and ⇧⌘Z stay key
    /// presses (⌘C, ⌘X and ⌘V following [`ClipboardKeys`]).
    pub fn set_text_editing(&mut self, editing: bool) {
        self.backend.set_text_editing(editing);
    }

    /// Before egui sees this frame's input: key equivalents become the key presses they were,
    /// and clicks wait for [`NativeMenu::take_clicks`].
    ///
    /// A click or Escape in the window closes an open native menu. AppKit can deliver it to the
    /// window while the menu is still open (asynchronous menu presentation), so while the backend
    /// reports one open, the click or Escape is taken out of the input: it closes the menu
    /// without starting a canvas gesture or reaching the active tool. Pointer moves stay. Only
    /// one such click is kept from egui until the backend reports no menu open again, so a
    /// backend that wrongly keeps reporting one open costs one click, not every click.
    ///
    /// AppKit takes only the key-down: winit still delivers the key-up (it forwards key-ups while
    /// ⌘ is held). So a key equivalent adds just its press, ahead of its key-up when that is
    /// already queued. Adding a made-up release as well sends two key-ups; in PhotoCraft, which
    /// reads a ⌘V release without a press as an image paste, that made ⌘V paste three times.
    /// ⌘C, ⌘X and ⌘V follow [`ClipboardKeys`].
    pub fn raw_input(&mut self, raw: &mut egui::RawInput) {
        // The window's own input, before key equivalents are added to it.
        if let Some(kept) = dismiss_input(raw, || self.armed && self.backend.tracking()) {
            if kept {
                self.armed = false;
            }
            // Also without a menu reported open: an asynchronously presented menu can still be
            // on screen after it reports closing.
            self.backend.cancel_tracking();
        }
        if !self.armed && !self.backend.tracking() {
            self.armed = true;
        }
        for e in self.backend.drain() {
            match e {
                Event::Key(chord) => self.add_key(raw, &chord),
                Event::Click(id) => self.clicks.push(id),
            }
            self.dirty = true;
        }
    }

    fn add_key(&mut self, raw: &mut egui::RawInput, chord: &Chord) {
        let Some(press @ egui::Event::Key { key: pressed_key, .. }) = chord.key_event(true) else { return };
        let clipboard = self.clipboard == ClipboardKeys::Egui && chord.cmd && !(chord.ctrl || chord.alt || chord.shift);
        let event = match pressed_key {
            egui::Key::C if clipboard => egui::Event::Copy,
            egui::Key::X if clipboard => egui::Event::Cut,
            egui::Key::V if clipboard => {
                self.paste = true;
                return;
            }
            _ => press,
        };
        let at = raw
            .events
            .iter()
            .position(|e| matches!(e, egui::Event::Key { key, pressed: false, .. } if *key == pressed_key))
            .unwrap_or(raw.events.len());
        raw.events.insert(at, event);
    }

    /// The items clicked since the last call, for the app to run like in-window menu clicks.
    /// [`MINIMIZE`] (minimizes the window) and [`craft_quit::QUIT_MENU_ID`] (asks the window to
    /// close, see [`craft_quit::on_menu_event`]) are run here and left out. A ⌘V from the menu
    /// ([`ClipboardKeys::Egui`]) is asked of eframe here too.
    pub fn take_clicks(&mut self, ctx: &egui::Context) -> Vec<String> {
        if std::mem::take(&mut self.paste) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        let mut out = Vec::new();
        for id in std::mem::take(&mut self.clicks) {
            if id == MINIMIZE {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            } else if !craft_quit::on_menu_event(ctx, &id) {
                out.push(id);
            }
        }
        out
    }

    /// Bring the native menu up to date. `state` is a hash of what the menus show (see
    /// [`had_input`]); `build` lays them out ([`crate::mac_layout`]) and runs only when `state`
    /// changed or a menu event arrived since the last sync, since building the rows usually walks
    /// every command's enabled state.
    ///
    /// Returns the layout's clashes when they differ from the last ones returned, for the app to
    /// log; otherwise nothing.
    pub fn sync_with(&mut self, state: u64, build: impl FnOnce() -> Layout) -> Vec<Clash> {
        if self.state == Some(state) && !self.dirty {
            return Vec::new();
        }
        let layout = build();
        self.backend.sync(&layout.bar);
        self.state = Some(state);
        self.dirty = false;
        if layout.clashes == self.clashes {
            return Vec::new();
        }
        self.clashes = layout.clashes.clone();
        layout.clashes
    }
}

/// `Some` when `raw` has a press or Escape that closes an open menu: `Some(true)` when, with
/// `tracking` (a menu is open), those events were removed, since they belong to the menu.
pub(crate) fn dismiss_input(raw: &mut egui::RawInput, tracking: impl FnOnce() -> bool) -> Option<bool> {
    if !raw
        .events
        .iter()
        .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }))
    {
        return None;
    }
    let kept = tracking();
    if kept {
        raw.events.retain(|e| !matches!(e, egui::Event::PointerButton { .. } | egui::Event::Key { key: egui::Key::Escape, .. }));
    }
    Some(kept)
}

/// What a backend reports for an item chosen with `command`, `chord` (its shortcut) and
/// `layout_shortcut` ([`crate::Item::layout_shortcut`]). `key` holds the modifiers held (⌘, ⌃, ⌥,
/// ⇧) when a key-down chose it, `None` for a mouse click. `text_editing`: a text field has the
/// keys ([`NativeMenu::set_text_editing`]).
///
/// A key-down is the item's key equivalent, reported as the key press, unless it was ↩ or Space
/// inside the open menu: the item's shortcut is not a key equivalent at all (a bare tool key), or
/// needs ⌘ or ⌃ and neither is held. The key is the item's (AppKit matched on it); the modifiers
/// are the ones held, since AppKit may match ⌘R to a ⇧⌘R item. Minimize, craft-quit's Quit and
/// shortcuts the layout assigned are run by the driver or the app's click handling, not by its
/// key handling, so they stay clicks. While text editing, the app's key handling leaves shortcuts
/// to the field, so a key equivalent is a click too, except the field's own editing keys
/// ([`text_editing_key`]).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn chosen_event(command: &str, chord: Option<&Chord>, layout_shortcut: bool, key: Option<[bool; 4]>, text_editing: bool) -> Event {
    let runs_as_click = layout_shortcut || command == MINIMIZE || command == craft_quit::QUIT_MENU_ID;
    let pressed = key
        .zip(chord)
        .filter(|([cmd, ctrl, ..], c)| !runs_as_click && c.native_ok() && (*cmd || *ctrl || !(c.cmd || c.ctrl)))
        .map(|([cmd, ctrl, alt, shift], c)| Chord { cmd, ctrl, alt, shift, key: c.key.clone() })
        .filter(|c| !text_editing || text_editing_key(c));
    match pressed {
        Some(chord) => Event::Key(chord),
        None => Event::Click(command.to_string()),
    }
}

/// ⌘C, ⌘X, ⌘V, ⌘A, ⌘Z or ⇧⌘Z: keys a text field handles itself.
#[cfg(any(target_os = "macos", test))]
fn text_editing_key(c: &Chord) -> bool {
    let is = |k: &str| c.key.eq_ignore_ascii_case(k);
    c.cmd && !c.ctrl && !c.alt && (if c.shift { is("Z") } else { ["C", "X", "V", "A", "Z"].iter().any(|k| is(k)) })
}

/// A key press or click this frame. It may have changed state an app's menu hash doesn't list,
/// so apps fold it into the `state` passed to [`NativeMenu::sync_with`], for example as
/// `had_input(ctx).then_some(ctx.cumulative_frame_nr())`.
pub fn had_input(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: false, .. })))
}
