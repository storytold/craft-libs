//! The macOS menu bar, through muda (AppKit menus behind a safe API; no GTK with default features
//! off). This module only builds and updates the `NSMenu`s; the layout and how keys reach egui are
//! in the platform-free part of the crate.
//!
//! - muda ids must be unique: a command shown twice (Keyboard Shortcuts in Edit and Window) has
//!   `id` for its first item and `id#2` for its second.
//! - A structure change ([`crate::structure_key`]: a new Open Recent file, a language switch)
//!   rebuilds the menu bar; labels, enabled and checked are updated in place. Opening or closing a
//!   document ([`crate::window_key`]) rebuilds only the Window menu: AppKit adds its own items to
//!   that menu (window tiling, the window list), so muda can't insert rows there by position.
//! - A chosen item is a click or a key equivalent: AppKit's current event says which. A key
//!   equivalent goes back to egui as the key press that was made (the event's modifiers, not the
//!   item's: AppKit may match ⌘R to a ⇧⌘R item), so the app's key rules still apply. A shortcut
//!   the layout assigned ([`crate::Item::layout_shortcut`]) stays a click: the app's keys don't
//!   know it. While a text field has the keys, a key equivalent is a click unless the field
//!   handles it ([`NativeMenu::set_text_editing`]).
//! - Hide ([`crate::HIDE`]) and Hide Others ([`crate::HIDE_OTHERS`]) are custom items calling
//!   `NSApplication hide:` and `hideOtherApplications:`, because the predefined ones always take ⌘H
//!   and ⌥⌘H.
//! - While a menu is open, changes wait until it closes: replacing or changing an `NSMenu` while
//!   AppKit tracks it can leave the displayed menu stranded.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType};

use crate::chord::Chord;
use crate::driver::{Backend, Event, NativeMenu, chosen_event};
use crate::model::{HIDE, HIDE_OTHERS, Item, Menu, MenuBar, MenuRole, Node, Standard, structure_key, window_key};

/// Set by the first [`install`]: muda keeps the first menu event handler and ignores later ones.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the native menu bar for `bar` (a [`crate::mac_layout`] result) and return the driver
/// for it. Call it once, on the main thread, in eframe's creator closure, so winit's default menu
/// doesn't stay up. `None` when AppKit refused the menus: the app keeps its in-window menus.
///
/// The menu takes over muda's menu event handler. There is one per process and the first one set
/// stays, so a second call returns `None`, and this and `craft_quit::install_app_menu` are
/// alternatives: an app calls one of them, once.
pub fn install(ctx: &egui::Context, bar: &MenuBar) -> Option<NativeMenu> {
    MainThreadMarker::new()?;
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return None;
    }
    let mut menu = MacMenu::new(ctx);
    menu.build(bar).then(|| NativeMenu::new(Box::new(menu)))
}

#[derive(Clone)]
enum Handle {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

/// One native item.
#[derive(Clone)]
struct Entry {
    handle: Handle,
    /// The command id (several entries can share one).
    command: String,
    chord: Option<Chord>,
    layout_shortcut: bool,
    label: String,
    enabled: bool,
}

/// How an item was chosen, read from AppKit's current event while it calls the handler.
struct Chosen {
    id: String,
    /// For a key press (not a mouse click), the modifiers held: ⌘, ⌃, ⌥, ⇧.
    key: Option<[bool; 4]>,
}

/// The native items by muda id.
type Entries = HashMap<String, Entry>;

/// The Window menu, the muda ids of its items, and its [`window_key`].
type WindowMenu = (Submenu, Vec<String>, u64);

struct MacMenu {
    ctx: egui::Context,
    menu: Option<muda::Menu>,
    entries: Entries,
    rx: Receiver<Chosen>,
    structure: u64,
    window: Option<WindowMenu>,
    /// The latest menus, waiting for the open menu to close.
    pending: Option<MenuBar>,
    /// A text field has the keys ([`NativeMenu::set_text_editing`]).
    text_editing: bool,
}

impl MacMenu {
    fn new(ctx: &egui::Context) -> MacMenu {
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            // AppKit calls this on the main thread, from the event that chose the item: a key
            // equivalent is a key-down, a click a mouse-up (or ↩ inside the open menu).
            let event = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).currentEvent());
            let key = event.filter(|ev| ev.r#type() == NSEventType::KeyDown).map(|ev| {
                let flags = ev.modifierFlags();
                [NSEventModifierFlags::Command, NSEventModifierFlags::Control, NSEventModifierFlags::Option, NSEventModifierFlags::Shift]
                    .map(|m| flags.contains(m))
            });
            // The receiver goes away only with the app.
            let _ = tx.send(Chosen { id: e.id.0, key });
            // Choosing an item is not window input, so nothing else wakes egui: without this the
            // item runs only with the next input event, 8 to 90 seconds later in measurements.
            repaint.request_repaint();
        }));
        MacMenu { ctx: ctx.clone(), menu: None, entries: HashMap::new(), rx, structure: 0, window: None, pending: None, text_editing: false }
    }

    /// Build the menu bar for `bar` and make it NSApp's. On an error or a panic in muda or AppKit
    /// the menus stay as they were (never crash for a menu) and this returns false.
    fn build(&mut self, bar: &MenuBar) -> bool {
        let built = catch_unwind(AssertUnwindSafe(|| -> muda::Result<(muda::Menu, Entries, Option<WindowMenu>)> {
            let mut entries = Entries::new();
            let mut window = None;
            let menu = muda::Menu::new();
            let mut special = Vec::new();
            for m in &bar.menus {
                let (sub, ids) = submenu(&mut entries, m)?;
                menu.append(&sub)?;
                if m.role == MenuRole::Window {
                    window = Some((sub.clone(), ids, window_key(bar)));
                }
                if matches!(m.role, MenuRole::Window | MenuRole::Help) {
                    special.push((m.role, sub));
                }
            }
            menu.init_for_nsapp();
            // Only once the menu is NSApp's: muda looks the submenus up in the installed main menu.
            for (role, sub) in special {
                match role {
                    MenuRole::Window => sub.set_as_windows_menu_for_nsapp(),
                    _ => sub.set_as_help_menu_for_nsapp(),
                }
            }
            Ok((menu, entries, window))
        }));
        match built {
            Ok(Ok((menu, entries, window))) => {
                self.menu = Some(menu);
                self.entries = entries;
                self.window = window;
                self.structure = structure_key(bar);
                true
            }
            _ => {
                // The new menu may have been made NSApp's before the failure.
                if let Some(old) = &self.menu {
                    let _ = catch_unwind(AssertUnwindSafe(|| {
                        old.init_for_nsapp();
                        if let Some((sub, ..)) = &self.window {
                            sub.set_as_windows_menu_for_nsapp();
                        }
                    }));
                }
                false
            }
        }
    }

    /// Replace the Window menu (a document opened or closed), leaving the other menus as they are.
    /// Falls back to a full build when that fails.
    fn rebuild_window(&mut self, bar: &MenuBar) {
        let (Some(menu), Some((old, old_ids, _))) = (self.menu.clone(), self.window.clone()) else {
            self.build(bar);
            return;
        };
        let Some((at, spec)) = bar.menus.iter().enumerate().find(|(_, m)| m.role == MenuRole::Window) else {
            self.build(bar);
            return;
        };
        let mut entries = self.entries.clone();
        for id in &old_ids {
            entries.remove(id);
        }
        let replaced = catch_unwind(AssertUnwindSafe(|| -> muda::Result<WindowMenu> {
            let (sub, ids) = submenu(&mut entries, spec)?;
            menu.remove(&old)?;
            if let Err(e) = menu.insert(&sub, at) {
                let _ = menu.insert(&old, at);
                return Err(e);
            }
            sub.set_as_windows_menu_for_nsapp();
            Ok((sub, ids, window_key(bar)))
        }));
        match replaced {
            Ok(Ok(window)) => {
                self.entries = entries;
                self.window = Some(window);
            }
            _ => {
                self.build(bar);
            }
        }
    }

    fn update(&mut self, bar: &MenuBar) {
        if self.menu.is_none() || structure_key(bar) != self.structure {
            self.build(bar);
            return;
        }
        if self.window.as_ref().map(|w| w.2) != Some(window_key(bar)) {
            self.rebuild_window(bar);
        }
        // Items by muda id: the n-th item of a command, depth first, is its n-th native item.
        let mut items: HashMap<String, &Item> = HashMap::new();
        for it in bar.items() {
            let id = unique_id(&it.id, |id| items.contains_key(id));
            items.insert(id, it);
        }
        for (id, e) in &mut self.entries {
            let Some(it) = items.get(id) else { continue };
            if e.label != it.label {
                let text = escape(&it.label);
                match &e.handle {
                    Handle::Plain(h) => h.set_text(text),
                    Handle::Check(h) => h.set_text(text),
                }
                e.label = it.label.clone();
            }
            if e.enabled != it.enabled {
                match &e.handle {
                    Handle::Plain(h) => h.set_enabled(it.enabled),
                    Handle::Check(h) => h.set_enabled(it.enabled),
                }
                e.enabled = it.enabled;
            }
            // Compare with AppKit's own state: muda toggles a check item when it's clicked.
            if let (Handle::Check(h), Some(c)) = (&e.handle, it.checked)
                && h.is_checked() != c
            {
                h.set_checked(c);
            }
        }
    }

    /// Nothing else may wake egui when the menu closes (Escape goes to the menu), so look again.
    fn retry_later(&mut self, bar: MenuBar) {
        self.pending = Some(bar);
        self.ctx.request_repaint_after(Duration::from_millis(250));
    }
}

/// A top-level menu, and the muda ids of the items it added to `entries`.
fn submenu(entries: &mut Entries, m: &Menu) -> muda::Result<(Submenu, Vec<String>)> {
    let before: HashSet<String> = entries.keys().cloned().collect();
    let sub = Submenu::new(escape(&m.title), true);
    append(entries, &sub, &m.children)?;
    let ids = entries.keys().filter(|id| !before.contains(*id)).cloned().collect();
    Ok((sub, ids))
}

fn append(entries: &mut Entries, sub: &Submenu, nodes: &[Node]) -> muda::Result<()> {
    for n in nodes {
        match n {
            Node::Separator => sub.append(&PredefinedMenuItem::separator())?,
            Node::Submenu { label, children, .. } => {
                let s = Submenu::new(escape(label), !children.is_empty());
                append(entries, &s, children)?;
                sub.append(&s)?;
            }
            Node::Standard { kind, label } => {
                let text = Some(label.as_str());
                let p = match kind {
                    Standard::Services => PredefinedMenuItem::services(text),
                    Standard::ShowAll => PredefinedMenuItem::show_all(text),
                    Standard::Minimize => PredefinedMenuItem::minimize(text),
                    Standard::Zoom => PredefinedMenuItem::maximize(text),
                    Standard::BringAllToFront => PredefinedMenuItem::bring_all_to_front(text),
                    Standard::CloseWindow => PredefinedMenuItem::close_window(text),
                };
                sub.append(&p)?;
            }
            Node::Item(it) => {
                let id = unique_id(&it.id, |id| entries.contains_key(id));
                let chord = it.shortcut.as_deref().and_then(Chord::parse);
                let accel = chord.as_ref().filter(|c| c.native_ok()).and_then(accelerator);
                let handle = match it.checked {
                    Some(c) => {
                        let h = CheckMenuItem::with_id(id.clone(), escape(&it.label), it.enabled, c, accel);
                        sub.append(&h)?;
                        Handle::Check(h)
                    }
                    None => {
                        let h = MenuItem::with_id(id.clone(), escape(&it.label), it.enabled, accel);
                        sub.append(&h)?;
                        Handle::Plain(h)
                    }
                };
                let entry = Entry {
                    handle,
                    command: it.id.clone(),
                    chord,
                    layout_shortcut: it.layout_shortcut,
                    label: it.label.clone(),
                    enabled: it.enabled,
                };
                entries.insert(id, entry);
            }
        }
    }
    Ok(())
}

impl Backend for MacMenu {
    fn sync(&mut self, bar: &MenuBar) {
        if self.tracking() {
            // The driver counts this sync as done and may not send these menus again.
            self.retry_later(bar.clone());
            return;
        }
        self.pending = None;
        self.update(bar);
    }

    fn drain(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(chosen) = self.rx.try_recv() {
            // With asynchronous menu presentation the menu can stay on screen after its item ran.
            self.cancel_tracking();
            // Ids that aren't ours are AppKit's own items (Services, Zoom, …), already handled.
            let Some(e) = self.entries.get(&chosen.id) else { continue };
            if e.command == HIDE || e.command == HIDE_OTHERS {
                if let Some(mtm) = MainThreadMarker::new() {
                    let app = NSApplication::sharedApplication(mtm);
                    if e.command == HIDE { app.hide(None) } else { app.hideOtherApplications(None) }
                }
                continue;
            }
            out.push(chosen_event(&e.command, e.chord.as_ref(), e.layout_shortcut, chosen.key, self.text_editing));
        }
        // After the events: they were chosen from the menus as they were.
        if let Some(bar) = self.pending.take() {
            if self.tracking() {
                self.retry_later(bar);
            } else {
                self.update(&bar);
            }
        }
        out
    }

    /// A menu bar menu is open while one of its titles is highlighted.
    ///
    /// This assumes AppKit sets the main menu's `highlightedItem` while one of its menus is open
    /// and clears it when the menu closes; not yet confirmed on every macOS version. If it stayed
    /// set, menu updates would wait (retried every 250 ms) and the driver would keep only one
    /// window click from egui ([`NativeMenu::raw_input`]).
    fn tracking(&self) -> bool {
        MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).mainMenu()).is_some_and(|m| m.highlightedItem().is_some())
    }

    fn set_text_editing(&mut self, editing: bool) {
        self.text_editing = editing;
    }

    fn cancel_tracking(&mut self) {
        if let Some(menu) = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).mainMenu()) {
            menu.cancelTracking();
        }
    }
}

/// `id`, or `id#2`, `id#3`, … when `taken`.
fn unique_id(id: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut unique = id.to_string();
    let mut n = 2;
    while taken(&unique) {
        unique = format!("{id}#{n}");
        n += 1;
    }
    unique
}

/// `&` marks a mnemonic in muda labels.
fn escape(s: &str) -> String {
    s.replace('&', "&&")
}

/// A key equivalent for AppKit. `Cmd` is ⌘ (`META`) and `Ctrl` is ⌃: never folded together.
fn accelerator(c: &Chord) -> Option<Accelerator> {
    let mut mods = Modifiers::empty();
    for (on, m) in [(c.cmd, Modifiers::META), (c.ctrl, Modifiers::CONTROL), (c.alt, Modifiers::ALT), (c.shift, Modifiers::SHIFT)] {
        if on {
            mods |= m;
        }
    }
    const LETTERS: [Code; 26] = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    const DIGITS: [Code; 10] =
        [Code::Digit0, Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4, Code::Digit5, Code::Digit6, Code::Digit7, Code::Digit8, Code::Digit9];
    const F: [Code; 24] = [
        Code::F1,
        Code::F2,
        Code::F3,
        Code::F4,
        Code::F5,
        Code::F6,
        Code::F7,
        Code::F8,
        Code::F9,
        Code::F10,
        Code::F11,
        Code::F12,
        Code::F13,
        Code::F14,
        Code::F15,
        Code::F16,
        Code::F17,
        Code::F18,
        Code::F19,
        Code::F20,
        Code::F21,
        Code::F22,
        Code::F23,
        Code::F24,
    ];
    let k = c.key.as_str();
    let mut chars = k.chars();
    let code = match (chars.next(), chars.next()) {
        (Some(ch), None) if ch.is_ascii_alphabetic() => *LETTERS.get((ch.to_ascii_uppercase() as usize).checked_sub('A' as usize)?)?,
        (Some(ch), None) if ch.is_ascii_digit() => *DIGITS.get((ch as usize).checked_sub('0' as usize)?)?,
        _ if c.is_function_key() => *F.get(k.get(1..)?.parse::<usize>().ok()?.checked_sub(1)?)?,
        _ => match k {
            "[" => Code::BracketLeft,
            "]" => Code::BracketRight,
            "\\" => Code::Backslash,
            "/" => Code::Slash,
            // `+` is ⇧= on the keyboards AppKit matches by key: Zoom In shows ⌘=.
            "=" | "+" => Code::Equal,
            "-" => Code::Minus,
            "'" => Code::Quote,
            ";" => Code::Semicolon,
            "," => Code::Comma,
            "." => Code::Period,
            "`" => Code::Backquote,
            // keyboard-types names: `Delete` is forward delete (egui's `Key::Delete`), `Backspace`
            // the key Macs label delete. muda writes them as U+007F and U+0008; which physical key
            // AppKit matches to each is not yet checked on a Mac.
            "Delete" => Code::Delete,
            "Backspace" => Code::Backspace,
            "Left" | "ArrowLeft" => Code::ArrowLeft,
            "Right" | "ArrowRight" => Code::ArrowRight,
            "Up" | "ArrowUp" => Code::ArrowUp,
            "Down" | "ArrowDown" => Code::ArrowDown,
            "Home" => Code::Home,
            "End" => Code::End,
            "PageUp" => Code::PageUp,
            "PageDown" => Code::PageDown,
            "Escape" | "Esc" if !mods.is_empty() => Code::Escape,
            "Enter" | "Return" if !mods.is_empty() => Code::Enter,
            "Tab" if !mods.is_empty() => Code::Tab,
            "Space" if !mods.is_empty() => Code::Space,
            _ => return None,
        },
    };
    Some(Accelerator::new(mods, code))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The shortcuts the layout writes itself, and common app ones, become key equivalents.
    #[test]
    fn shortcuts_map_to_key_equivalents() {
        #[rustfmt::skip]
        let shortcuts = [
            "Cmd+Q", "Cmd+H", "Cmd+Alt+H", "Ctrl+Cmd+H", "Ctrl+Cmd+M", "Cmd+,", "Cmd+Shift+Z", "Cmd+1", "Cmd+[", "Cmd+=", "Cmd++",
            "Cmd+-", "Cmd+Delete", "Cmd+Backspace", "Cmd+Left", "Alt+Cmd+ArrowDown", "Cmd+Escape", "Cmd+Home", "Cmd+End",
            "Cmd+PageUp", "Cmd+PageDown", "Cmd+Enter", "F7", "Alt+F12",
        ];
        for s in shortcuts {
            let c = Chord::parse(s).unwrap();
            assert!(c.native_ok() && accelerator(&c).is_some(), "{s} has no key equivalent");
            assert!(c.key_event(true).is_some(), "{s} has no egui key press");
        }
        let code = |s: &str| accelerator(&Chord::parse(s).unwrap()).unwrap();
        assert_eq!(code("Cmd+Delete"), Accelerator::new(Modifiers::META, Code::Delete), "forward delete, as egui's Key::Delete");
        assert_eq!(code("Cmd+Backspace"), Accelerator::new(Modifiers::META, Code::Backspace));
        assert_eq!(code("Cmd++"), code("Cmd+="), "⌘+ is the ⌘= key");
    }

    #[test]
    fn control_and_command_stay_distinct() {
        let a = accelerator(&Chord::parse("Cmd+M").unwrap()).unwrap();
        let b = accelerator(&Chord::parse("Ctrl+Cmd+M").unwrap()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn a_command_shown_twice_gets_a_second_id() {
        let mut taken = vec!["edit.keys".to_string()];
        let second = unique_id("edit.keys", |id| taken.iter().any(|t| t == id));
        taken.push(second.clone());
        assert_eq!((second.as_str(), unique_id("edit.keys", |id| taken.iter().any(|t| t == id)).as_str()), ("edit.keys#2", "edit.keys#3"));
    }
}
