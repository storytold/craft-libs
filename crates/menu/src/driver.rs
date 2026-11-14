//! The app's side: events from the platform menu in, menu bars out.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use craft_keymap::{Chord, CommandRef, Key, Mods};

use crate::layout::{Clash, Layout};
use crate::model::{Builtin, Item, ItemKind, MenuBar, Node};

/// What the native menu reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An item was chosen with the mouse (or the keyboard inside an open menu), or by a key
    /// equivalent the app's key handling doesn't know ([`crate::Item::layout_shortcut`]): the
    /// item's command.
    Click(CommandRef),
    /// A key equivalent was pressed: the shortcut, to be handled as the key press it was.
    Key(Chord),
    /// An item whose shortcut a text field handles itself (Edit › Copy, Undo) was clicked while a
    /// text field has the keys: the shortcut, to be handled as a press and release of it.
    Tap(Chord),
    /// A click the app's [`NativeMenu::on_command`] hook ran; [`NativeMenu::take_clicks`] leaves it
    /// out.
    Handled(CommandRef),
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

/// What may run from the menus this frame, set by the app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    /// A modal dialog is open.
    pub modal: bool,
    /// The shortcut editor is recording a chord.
    pub recording_shortcut: bool,
    /// A text field has the keys.
    pub text_editing: bool,
}

impl Gate {
    /// Whether `item` may run now: while a modal dialog is open or a shortcut is being recorded,
    /// only the built-in items (Hide, Hide Others, Minimize, Quit) do. The built-in Quit stays
    /// because it asks the window to close, and the app's close guard knows about its dialogs; an
    /// app's own Quit item ([`crate::ItemRole::Quit`]) is greyed with its other commands. A text field having the keys doesn't stop a click: menu commands run while
    /// typing, as in a Mac app, and the keys a field keeps are handled before the menus see them.
    pub fn allows(&self, item: &Item) -> bool {
        self.allows_command(&item.command)
    }

    /// [`Gate::allows`] for an item that runs `command`.
    pub fn allows_command(&self, command: &CommandRef) -> bool {
        !(self.modal || self.recording_shortcut) || Builtin::of(command).is_some()
    }

    /// `bar` with every item the gate forbids disabled, so the menus show them greyed, but the
    /// items whose shortcut a text field handles itself (Undo, Copy, Select All, …).
    pub(crate) fn grey<'a>(&self, bar: &'a MenuBar) -> Cow<'a, MenuBar> {
        fn walk(nodes: &mut [Node], gate: &Gate) {
            for n in nodes {
                match n {
                    // The field's own keys stay enabled: AppKit doesn't send a disabled item's key
                    // equivalent on, so greyed Copy or Undo would leave ⌘C and ⌘Z dead in a
                    // dialog's text fields. Their clicks are still dropped (`take_clicks`).
                    Node::Item(it) if !gate.allows(it) && !it.shortcut.as_ref().is_some_and(text_editing_key) => it.enabled = false,
                    Node::Submenu { children, .. } => walk(children, gate),
                    _ => {}
                }
            }
        }
        if !(self.modal || self.recording_shortcut) {
            return Cow::Borrowed(bar);
        }
        let mut bar = bar.clone();
        for m in &mut bar.menus {
            walk(&mut m.children, self);
        }
        Cow::Owned(bar)
    }
}

/// An app's [`NativeMenu::on_command`] hook: `true` when it ran the command.
pub type CommandHook = Box<dyn Fn(&CommandRef) -> bool + Send + Sync>;

/// What a backend's menu event handler needs from the driver when an item is chosen: the
/// current [`Gate`] and the app's [`NativeMenu::on_command`] hook. The platform calls the handler
/// between frames, from the event that chose the item, so this is shared with the driver rather
/// than borrowed from it. [`NativeMenu::new`] hands it to the backend ([`Backend::attach`]).
#[derive(Clone, Default)]
pub struct Dispatch(Arc<Mutex<Shared>>);

type SharedHook = Arc<dyn Fn(&CommandRef) -> bool + Send + Sync>;

#[derive(Default)]
struct Shared {
    gate: Gate,
    hook: Option<SharedHook>,
}

impl Dispatch {
    /// The event for an item with `command`, its `shortcut` and [`Item::layout_shortcut`],
    /// chosen with the modifiers `held` for a key-down (⌘, ⌃, ⌥, ⇧ as AppKit reports them) or
    /// `None` for a mouse click, under the current gate. A click the gate allows on an app command
    /// (not a [`Builtin`]) also goes to the app's [`NativeMenu::on_command`] hook, before this
    /// returns; when the hook ran it, the event is [`Event::Handled`].
    pub fn chosen(&self, command: &CommandRef, shortcut: Option<&Chord>, layout_shortcut: bool, held: Option<Mods>) -> Event {
        let (gate, hook) = {
            let shared = self.lock();
            (shared.gate, shared.hook.clone())
        };
        let event = chosen_event(command, shortcut, layout_shortcut, held, gate);
        if let (Event::Click(c), Some(hook)) = (&event, hook)
            && gate.allows_command(c)
            && Builtin::of(c).is_none()
            && hook(c)
        {
            return Event::Handled(c.clone());
        }
        event
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
    /// Keep the driver's [`Dispatch`]: the backend's menu event handler turns every chosen item
    /// into an [`Event`] through [`Dispatch::chosen`]. [`NativeMenu::new`] calls it once.
    fn attach(&mut self, dispatch: Dispatch);
}

/// The native menu bar, while the app uses one (its in-window menus are hidden then).
pub struct NativeMenu {
    backend: Box<dyn Backend>,
    /// The state the menus were last built from.
    state: Option<u64>,
    clicks: Vec<CommandRef>,
    /// An event arrived (AppKit may have toggled a check item itself): re-sync.
    dirty: bool,
    clashes: Vec<Clash>,
    clipboard: ClipboardKeys,
    /// ⌘V arrived: [`NativeMenu::take_clicks`] asks eframe for the paste.
    paste: bool,
    /// Whether a click may be kept from egui as the one closing an open menu (see
    /// [`NativeMenu::raw_input`]).
    armed: bool,
    gate: Gate,
    dispatch: Dispatch,
    /// The menus last laid out, before the gate greyed anything, and the gate they were last
    /// synced with: a gate change re-syncs them without laying them out again.
    bar: Option<MenuBar>,
    synced_gate: Option<Gate>,
    /// The commands whose items were check items in a layout so far: they stay check items, so
    /// that `checked` going from `Some` to `None` doesn't rebuild the menus.
    checkable: HashSet<String>,
}

impl NativeMenu {
    pub fn new(mut backend: Box<dyn Backend>) -> NativeMenu {
        let dispatch = Dispatch::default();
        backend.attach(dispatch.clone());
        NativeMenu {
            backend,
            state: None,
            clicks: Vec::new(),
            dirty: true,
            clashes: Vec::new(),
            clipboard: ClipboardKeys::default(),
            paste: false,
            armed: true,
            gate: Gate::default(),
            dispatch,
            bar: None,
            synced_gate: None,
            checkable: HashSet::new(),
        }
    }

    pub fn clipboard_keys(mut self, keys: ClipboardKeys) -> NativeMenu {
        self.clipboard = keys;
        self
    }

    /// What may run from the menus. Call it every frame, before [`NativeMenu::take_clicks`]; it
    /// also holds for the menu events that arrive until the next call.
    ///
    /// - While [`Gate::modal`] or [`Gate::recording_shortcut`] is on, the app's items are greyed
    ///   (updated in place at the next [`NativeMenu::sync_with`]) and clicks on them are dropped,
    ///   also a click that arrived before the gate closed and is still waiting for
    ///   [`NativeMenu::take_clicks`]. The built-in items (Hide, Hide Others, Minimize, Quit) stay.
    ///   A greyed item's key equivalent goes to the window, so the app's key handling sees it.
    /// - While [`Gate::recording_shortcut`] is on, every key equivalent reaches egui as the plain
    ///   key press that was made (⌘Q, ⌘H and ⌘C included), for the shortcut recorder.
    /// - While [`Gate::text_editing`] is on (in egui, `ctx.wants_keyboard_input()`), a key
    ///   equivalent runs its item as an [`Event::Click`], as a Mac app's menu commands run while
    ///   typing, except the keys the field handles itself: ⌘C, ⌘X and ⌘V (following
    ///   [`ClipboardKeys`]) and [`craft_keymap::text_editing_chords`] (⌘A, ⌘Z, ⇧⌘Z, ⌘←, …) stay
    ///   key presses.
    pub fn set_gate(&mut self, gate: Gate) {
        self.gate = gate;
        self.dispatch.lock().gate = gate;
    }

    /// Run `hook` with each command clicked in the menus, from the menu event handler, as the
    /// click arrives: on macOS on the main thread, between frames, also while egui is idle. Only
    /// app commands the current gate ([`NativeMenu::set_gate`]) allows reach it, and key
    /// equivalents reported as key presses don't. It must return quickly; it is for waking the
    /// app's own loop or a worker.
    ///
    /// The hook returns `true` when it ran the command: [`NativeMenu::take_clicks`] then leaves
    /// it out. A command it returned `false` for comes back from `take_clicks`, after the gate as
    /// it is then.
    pub fn on_command(&mut self, hook: CommandHook) {
        self.dispatch.lock().hook = Some(Arc::from(hook));
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
                Event::Key(chord) => self.add_key(raw, &chord, false),
                Event::Tap(chord) => self.add_key(raw, &chord, true),
                Event::Click(command) => self.clicks.push(command),
                Event::Handled(_) => {}
            }
            self.dirty = true;
        }
    }

    /// `tap`: a press and its release, for a click that stands for the key (no key-up follows).
    fn add_key(&mut self, raw: &mut egui::RawInput, chord: &Chord, tap: bool) {
        let pressed_key = chord.key.to_egui();
        let clipboard = self.clipboard == ClipboardKeys::Egui && chord.mods == Mods::CMD && !self.gate.recording_shortcut;
        let event = match chord.key {
            Key::C if clipboard => egui::Event::Copy,
            Key::X if clipboard => egui::Event::Cut,
            Key::V if clipboard => {
                self.paste = true;
                return;
            }
            _ => key_event(chord, true),
        };
        let at = raw
            .events
            .iter()
            .position(|e| matches!(e, egui::Event::Key { key, pressed: false, .. } if *key == pressed_key))
            .unwrap_or(raw.events.len());
        let press = matches!(event, egui::Event::Key { .. });
        raw.events.insert(at, event);
        if tap && press {
            raw.events.insert(at + 1, key_event(chord, false));
        }
    }

    /// The commands clicked since the last call that the gate ([`NativeMenu::set_gate`]) allows
    /// now, for the app to run like in-window menu clicks; the others are dropped.
    /// [`Builtin::Minimize`] (minimizes the window) and [`Builtin::Quit`] (asks the window to
    /// close, see [`craft_quit::on_menu_event`]) are run here and left out. A ⌘V from the menu
    /// ([`ClipboardKeys::Egui`]) is asked of eframe here too.
    pub fn take_clicks(&mut self, ctx: &egui::Context) -> Vec<CommandRef> {
        if std::mem::take(&mut self.paste) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        let mut out = Vec::new();
        for command in std::mem::take(&mut self.clicks) {
            if !self.gate.allows_command(&command) {
                continue;
            }
            match Builtin::of(&command) {
                Some(Builtin::Minimize) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                Some(Builtin::Quit) => {
                    craft_quit::on_menu_event(ctx, Builtin::Quit.id());
                }
                _ => out.push(command),
            }
        }
        out
    }

    /// Bring the native menu up to date. `state` is a hash of what the menus show (see
    /// [`had_input`]); `build` lays them out ([`crate::mac_layout`]) and runs only when `state`
    /// changed or a menu event arrived since the last sync, since building the rows usually walks
    /// every command's enabled state. A menu event re-syncs because AppKit flips a check or radio
    /// item's state itself when it is chosen: the sync sets it back to the app's state. A gate
    /// change ([`NativeMenu::set_gate`]) re-syncs the menus last laid out, greyed or not, without
    /// calling `build`.
    ///
    /// `state` must cover everything the menus show. A change that comes without input, such as
    /// a load or save finishing on a worker or a timer, leaves the menus stale until the next
    /// input or menu event unless it is in the hash. An app that can't keep the hash complete
    /// uses [`NativeMenu::sync`] instead.
    ///
    /// Returns the layout's clashes when they differ from the last ones returned, for the app to
    /// log; otherwise nothing.
    pub fn sync_with(&mut self, state: u64, build: impl FnOnce() -> Layout) -> Vec<Clash> {
        let relayout = self.state != Some(state) || self.dirty;
        if !relayout && self.synced_gate == Some(self.gate) {
            return Vec::new();
        }
        let mut clashes = Vec::new();
        if relayout {
            let mut layout = build();
            self.keep_kinds(&mut layout.bar);
            clashes = self.keep(layout);
            self.state = Some(state);
        }
        self.push();
        clashes
    }

    /// [`NativeMenu::sync_with`] without a state hash: the app lays the menus out every frame
    /// and they are synced when the layout differs from the last one (or a menu event or gate
    /// change calls for it), so they never go stale. It costs a layout per frame.
    pub fn sync(&mut self, mut layout: Layout) -> Vec<Clash> {
        self.keep_kinds(&mut layout.bar);
        if !self.dirty && self.bar.as_ref() == Some(&layout.bar) && self.clashes == layout.clashes && self.synced_gate == Some(self.gate) {
            return Vec::new();
        }
        let clashes = self.keep(layout);
        // A later `sync_with` lays the menus out again.
        self.state = None;
        self.push();
        clashes
    }

    /// An item that was a check item once stays one, unchecked: switching its kind would change
    /// [`crate::structure_key`] and rebuild the menus.
    fn keep_kinds(&mut self, bar: &mut MenuBar) {
        let checkable = &mut self.checkable;
        bar.for_each_item_mut(|it| match it.kind {
            ItemKind::Check(_) => {
                checkable.insert(it.command.key());
            }
            ItemKind::Plain if checkable.contains(&it.command.key()) => it.kind = ItemKind::Check(false),
            _ => {}
        });
    }

    /// Keep `layout` as the menus to show; its clashes when they differ from the last ones.
    fn keep(&mut self, layout: Layout) -> Vec<Clash> {
        self.dirty = false;
        self.bar = Some(layout.bar);
        if layout.clashes == self.clashes {
            return Vec::new();
        }
        self.clashes = layout.clashes.clone();
        layout.clashes
    }

    /// Sync the backend with the kept menus, greyed by the gate.
    fn push(&mut self) {
        if let Some(bar) = &self.bar {
            self.backend.sync(&self.gate.grey(bar));
        }
        self.synced_gate = Some(self.gate);
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
/// `layout_shortcut` ([`crate::Item::layout_shortcut`]) under `gate`. `held` holds the modifiers
/// held when a key-down chose it, `None` for a mouse click.
///
/// A key-down is the item's key equivalent, reported as the key press, unless it was ↩ or Space
/// inside the open menu: the item's shortcut is not a key equivalent at all ([`native_ok`]), or
/// neither ⌘ nor ⌃ is held. The key is the item's (AppKit matched on it); the modifiers are the
/// ones held, since AppKit may match ⌘R to a ⇧⌘R item. Minimize, craft-quit's Quit and shortcuts
/// the layout assigned are run by the driver or the app's click handling, not by its key
/// handling, so they stay clicks. While text editing, the app's key handling leaves shortcuts to
/// the field, so a key equivalent is a click too, except the field's own editing keys
/// ([`text_editing_key`]); a mouse click on one of those items (Edit › Copy, Undo) is a tap of
/// its shortcut ([`Event::Tap`]), so it acts on the field rather than the document. While recording a shortcut every key equivalent is a key press, for the
/// recorder.
pub(crate) fn chosen_event(command: &CommandRef, chord: Option<&Chord>, layout_shortcut: bool, held: Option<Mods>, gate: Gate) -> Event {
    let recording = gate.recording_shortcut;
    let runs_as_click = !recording && (layout_shortcut || matches!(Builtin::of(command), Some(Builtin::Minimize | Builtin::Quit)));
    let Some(held) = held else {
        return match chord {
            Some(c) if !recording && gate.text_editing && text_editing_key(c) => Event::Tap(*c),
            _ => Event::Click(command.clone()),
        };
    };
    let pressed = chord
        .filter(|c| !runs_as_click && native_ok(c) && (held.cmd || held.ctrl))
        .map(|c| Chord::new(held, c.key))
        .filter(|c| recording || !gate.text_editing || text_editing_key(c));
    match pressed {
        Some(chord) => Event::Key(chord),
        None => Event::Click(command.clone()),
    }
}

/// A chord a text field handles itself: the clipboard keys and craft-keymap's text-editing
/// chords (undo, redo, select all, moving by word and line).
fn text_editing_key(c: &Chord) -> bool {
    const CLIPBOARD: [Chord; 3] = [Chord::new(Mods::CMD, Key::C), Chord::new(Mods::CMD, Key::X), Chord::new(Mods::CMD, Key::V)];
    CLIPBOARD.contains(c) || craft_keymap::text_editing_chords(craft_keymap::Platform::Mac).contains(c)
}

/// May the native menu make this a key equivalent? Only with ⌘ or ⌃. Other shortcuts (a bare
/// letter, ⇧ or ⌥ alone, a bare function key) are tool keys: as key equivalents AppKit would take
/// a `b` typed into a text field, and the app's key handling may bind them per tool or context.
/// They are shown but not made key equivalents.
pub(crate) fn native_ok(c: &Chord) -> bool {
    c.has_command()
}

/// The key press egui would have received for `chord` on macOS: `cmd` is ⌘, `ctrl` the Control
/// key.
pub(crate) fn key_event(chord: &Chord, pressed: bool) -> egui::Event {
    let m = chord.mods;
    let key = chord.key.to_egui();
    let modifiers = egui::Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: m.cmd, command: m.cmd };
    egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

/// A key press or click this frame. It may have changed state an app's menu hash doesn't list,
/// so apps fold it into the `state` passed to [`NativeMenu::sync_with`], for example as
/// `had_input(ctx).then_some(ctx.cumulative_frame_nr())`.
pub fn had_input(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: false, .. })))
}
