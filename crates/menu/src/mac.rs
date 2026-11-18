//! The macOS menu bar, through muda (AppKit menus behind a safe API; no GTK with default features
//! off). This module only builds and updates the `NSMenu`s; the layout and how keys reach egui are
//! in the platform-free part of the crate.
//!
//! - An item's muda id is its command's [`craft_keymap::CommandRef::key`]. muda ids must be
//!   unique: a command shown twice (Keyboard Shortcuts in Edit and Window) has `key` for its first
//!   item and `key#2` for its second, and both report the command.
//! - A radio item is a check item: AppKit menus have no radio look.
//! - A structure change ([`crate::structure_key`]: a new Open Recent file, a language switch)
//!   rebuilds the menu bar; labels, enabled and checked are updated in place. Opening or closing a
//!   document ([`crate::window_key`]) rebuilds only the Window menu: AppKit adds its own items to
//!   that menu (window tiling, the window list), so muda can't insert rows there by position.
//! - A chosen item is a click or a key equivalent: AppKit's current event says which. A key
//!   equivalent goes back to egui as the key press that was made (the event's modifiers, not the
//!   item's: AppKit may match ⌘R to a ⇧⌘R item), so the app's key rules still apply. A shortcut
//!   the layout assigned ([`crate::Item::layout_shortcut`]) stays a click: the app's keys don't
//!   know it. While a text field has the keys, a key equivalent is a click unless the field
//!   handles it ([`NativeMenu::set_gate`]).
//! - Only shortcuts with ⌘ or ⌃ become key equivalents. The others (`B`, `⇧B`, `⌥B`, `F5`) are not
//!   shown in the native menu: muda 0.21 has no handle on the `NSMenuItem`, and its
//!   `set_attributed_title` needs an `NSAttributedString` with a font and a tab stop, which
//!   objc2 builds only with `unsafe` calls (the workspace forbids unsafe code).
//! - The process has one muda menu event handler, set by the first [`install`] or
//!   [`install_app_menu`]; each install points it at its own menu.
//! - A check or radio item: AppKit (muda) flips its state when it is chosen. The next sync sets it
//!   back to the app's state; the driver re-syncs after every menu event.
//! - Hide ([`crate::Builtin::Hide`]) and Hide Others ([`crate::Builtin::HideOthers`]) are custom items calling
//!   `NSApplication hide:` and `hideOtherApplications:`, because the predefined ones always take ⌘H
//!   and ⌥⌘H.
//! - While a menu is open, changes wait until it closes: replacing or changing an `NSMenu` while
//!   AppKit tracks it can leave the displayed menu stranded.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType};

use craft_keymap::{Chord, CommandRef, Key, Mods};

use crate::driver::{Backend, Dispatch, Event, NativeMenu, native_ok};
use crate::model::{Builtin, Item, ItemKind, Menu, MenuBar, MenuRole, Node, Standard, native_id, structure_key, window_key};

/// Set by [`install`]: there is one full menu bar per process.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Where the menu event handler sends events: the menu installed last.
static ROUTE: Mutex<Option<Route>> = Mutex::new(None);

/// muda keeps the first menu event handler set and ignores later ones, so it is set once.
static HANDLER: OnceLock<()> = OnceLock::new();

thread_local! {
    /// The minimal app menu ([`install_app_menu`]). muda's items must outlive the native menu
    /// that shows them.
    static APP_MENU: RefCell<Option<muda::Menu>> = const { RefCell::new(None) };
}

/// Install the native menu bar for `bar` (a [`crate::mac_layout`] result) and return the driver
/// for it. Call it once, on the main thread, in eframe's creator closure, so winit's default menu
/// doesn't stay up. `None` when AppKit refused the menus (the app keeps its in-window menus), off
/// the main thread, or when a menu bar is already installed. It replaces an app menu from
/// [`install_app_menu`].
///
/// craft-menu owns muda's menu event handler. muda keeps the first one set, so an app calls this
/// or the deprecated `craft_quit::install_app_menu`, never both: called first, that one leaves
/// this menu without events; called after, it replaces this menu bar with its app menu.
pub fn install(ctx: &egui::Context, bar: &MenuBar) -> Option<NativeMenu> {
    install_with(ctx, bar, MacOptions::default())
}

/// How [`install_with`] sets up the native menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MacOptions {
    /// Make the [`MenuRole::Window`] menu AppKit's Windows menu, which lists the app's windows
    /// after the menu's own items. On by default. An app that lists its documents itself
    /// in the Window menu ([`MenuBar::documents`]) and keeps them in one window turns it off: AppKit
    /// would list that window as well.
    pub window_list: bool,
}

impl Default for MacOptions {
    fn default() -> MacOptions {
        MacOptions { window_list: true }
    }
}

impl MacOptions {
    pub fn window_list(mut self, on: bool) -> MacOptions {
        self.window_list = on;
        self
    }
}

/// [`install`] with `options`.
pub fn install_with(ctx: &egui::Context, bar: &MenuBar, options: MacOptions) -> Option<NativeMenu> {
    MainThreadMarker::new()?;
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return None;
    }
    let mut menu = MacMenu::new(ctx, options);
    if !menu.build(bar) {
        INSTALLED.store(false, Ordering::SeqCst);
        return None;
    }
    // `NativeMenu::new` attaches the driver's dispatch, which points the handler at this menu.
    Some(NativeMenu::new(Box::new(menu)))
}

/// Replace winit's default macOS app menu with a minimal one whose Quit (⌘Q) asks the app first,
/// for an app without a [`MenuBar`] of its own: About, Services, Hide, Hide Others, Show All and
/// Quit, labelled in English with `app_name` ("Quit WordCraft").
///
/// winit's Quit ends the process without asking, so unsaved work is lost. This Quit has the id
/// [`craft_quit::QUIT_MENU_ID`] and runs [`craft_quit::on_menu_event`]: it asks the root window to
/// close, the app's unsaved-changes check runs as for the close button, and
/// [`craft_quit::on_close_request`] then quits. Call it once, from the eframe app creator: winit
/// builds its default menu before that and would replace one installed earlier.
///
/// Does nothing when this menu or a full menu bar ([`install`]) is already installed. On error
/// winit's menu stays: the app still runs, but ⌘Q quits without asking.
pub fn install_app_menu(ctx: &egui::Context, app_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if MainThreadMarker::new().is_none() {
        return Err("craft_menu::mac::install_app_menu runs on the main thread".into());
    }
    if INSTALLED.load(Ordering::SeqCst) || APP_MENU.with_borrow(Option::is_some) {
        return Ok(());
    }
    let quit = MenuItem::with_id(craft_quit::QUIT_MENU_ID, format!("Quit {app_name}"), true, Some(Accelerator::new(Modifiers::META, Code::KeyQ)));
    let app_menu = Submenu::new(app_name, true);
    app_menu.append_items(&[
        &PredefinedMenuItem::about(Some(&format!("About {app_name}")), None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(Some(&format!("Hide {app_name}"))),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let menu = muda::Menu::new();
    menu.append(&app_menu)?;
    menu.init_for_nsapp();
    route_events(Route::AppMenu(ctx.clone()));
    APP_MENU.set(Some(menu));
    Ok(())
}

/// The installed menu the handler reports to.
#[derive(Clone)]
enum Route {
    /// [`install_app_menu`]'s: only Quit reports, to craft-quit.
    AppMenu(egui::Context),
    /// A full menu bar's.
    Menu(MenuRoute),
}

#[derive(Clone)]
struct MenuRoute {
    ctx: egui::Context,
    targets: Targets,
    dispatch: Dispatch,
    tx: Sender<Event>,
}

/// What each native item runs, by muda id, for the handler.
type Targets = Arc<Mutex<HashMap<String, Target>>>;

/// Point the process's menu event handler at `route`, setting the handler the first time.
fn route_events(route: Route) {
    *lock(&ROUTE) = Some(route);
    HANDLER.get_or_init(|| MenuEvent::set_event_handler(Some(handle_event)));
}

/// The menu event handler. AppKit calls it on the main thread, from the event that chose the
/// item: a key equivalent is a key-down, a click a mouse-up (or ↩ inside the open menu).
fn handle_event(e: MenuEvent) {
    let held = MainThreadMarker::new()
        .and_then(|mtm| NSApplication::sharedApplication(mtm).currentEvent())
        .filter(|ev| ev.r#type() == NSEventType::KeyDown)
        .map(|ev| {
            let flags = ev.modifierFlags();
            Mods {
                cmd: flags.contains(NSEventModifierFlags::Command),
                ctrl: flags.contains(NSEventModifierFlags::Control),
                alt: flags.contains(NSEventModifierFlags::Option),
                shift: flags.contains(NSEventModifierFlags::Shift),
            }
        });
    // A copy, so nothing is locked while the app's hook runs.
    let route = lock(&ROUTE).clone();
    let ctx = match route {
        None => return,
        Some(Route::AppMenu(ctx)) => {
            craft_quit::on_menu_event(&ctx, e.id.as_ref());
            ctx
        }
        Some(Route::Menu(r)) => {
            let target = lock(&r.targets).get(e.id.as_ref()).cloned();
            // Ids that aren't ours are AppKit's own items (Services, Zoom, …), already handled.
            if let Some(t) = target {
                let event = r.dispatch.chosen(&t.command, t.chord.as_ref(), t.layout_shortcut, held);
                // The receiver goes away only with the app.
                let _ = r.tx.send(event);
            }
            r.ctx
        }
    };
    // Choosing an item is not window input, so nothing else wakes egui: without this the item
    // runs only with the next input event, 8 to 90 seconds later in measurements.
    ctx.request_repaint();
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
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
    target: Target,
    label: String,
    enabled: bool,
}

/// What choosing an item reports.
#[derive(Clone)]
struct Target {
    /// The command (several items can share one).
    command: CommandRef,
    chord: Option<Chord>,
    layout_shortcut: bool,
}

/// The native items by muda id.
type Entries = HashMap<String, Entry>;

/// The Window menu, the muda ids of its items, and its [`window_key`].
type WindowMenu = (Submenu, Vec<String>, u64);

struct MacMenu {
    ctx: egui::Context,
    menu: Option<muda::Menu>,
    entries: Entries,
    /// The entries' targets, for the menu event handler.
    targets: Targets,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    structure: u64,
    window: Option<WindowMenu>,
    /// The latest menus, waiting for the open menu to close.
    pending: Option<MenuBar>,
    options: MacOptions,
}

impl MacMenu {
    fn new(ctx: &egui::Context, options: MacOptions) -> MacMenu {
        let (tx, rx) = channel();
        MacMenu {
            ctx: ctx.clone(),
            menu: None,
            entries: HashMap::new(),
            targets: Targets::default(),
            tx,
            rx,
            structure: 0,
            window: None,
            pending: None,
            options,
        }
    }

    /// Hand the entries' targets to the menu event handler.
    fn publish(&self) {
        *lock(&self.targets) = self.entries.iter().map(|(id, e)| (id.clone(), e.target.clone())).collect();
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
                if m.role == MenuRole::Help || (m.role == MenuRole::Window && self.options.window_list) {
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
                self.publish();
                true
            }
            _ => {
                // The new menu may have been made NSApp's before the failure.
                if let Some(old) = &self.menu {
                    let _ = catch_unwind(AssertUnwindSafe(|| {
                        old.init_for_nsapp();
                        if let Some((sub, ..)) = &self.window
                            && self.options.window_list
                        {
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
            if self.options.window_list {
                sub.set_as_windows_menu_for_nsapp();
            }
            Ok((sub, ids, window_key(bar)))
        }));
        match replaced {
            Ok(Ok(window)) => {
                self.entries = entries;
                self.window = Some(window);
                self.publish();
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
            let id = native_id(&it.command, |id| items.contains_key(id));
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
            // Compare with AppKit's own state: muda flips a check item when it's chosen, and the
            // app's state wins (a radio item chosen again stays selected).
            if let (Handle::Check(h), Some(c)) = (&e.handle, it.kind.checked())
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
                let id = native_id(&it.command, |id| entries.contains_key(id));
                let chord = it.shortcut;
                let accel = chord.as_ref().filter(|c| native_ok(c)).and_then(accelerator);
                let handle = match it.kind {
                    ItemKind::Check(c) | ItemKind::Radio(c) => {
                        let h = CheckMenuItem::with_id(id.clone(), escape(&it.label), it.enabled, c, accel);
                        sub.append(&h)?;
                        Handle::Check(h)
                    }
                    ItemKind::Plain => {
                        let h = MenuItem::with_id(id.clone(), escape(&it.label), it.enabled, accel);
                        sub.append(&h)?;
                        Handle::Plain(h)
                    }
                };
                let target = Target { command: it.command.clone(), chord, layout_shortcut: it.layout_shortcut };
                let entry = Entry { handle, target, label: it.label.clone(), enabled: it.enabled };
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
        while let Ok(event) = self.rx.try_recv() {
            // With asynchronous menu presentation the menu can stay on screen after its item ran.
            self.cancel_tracking();
            // Hide and Hide Others run here. While a shortcut is recorded their key equivalents
            // arrive as key presses instead, for the recorder.
            if let Event::Click(command) = &event
                && let Some(hide @ (Builtin::Hide | Builtin::HideOthers)) = Builtin::of(command)
            {
                if let Some(mtm) = MainThreadMarker::new() {
                    let app = NSApplication::sharedApplication(mtm);
                    if hide == Builtin::Hide { app.hide(None) } else { app.hideOtherApplications(None) }
                }
                continue;
            }
            out.push(event);
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

    fn attach(&mut self, dispatch: Dispatch) {
        route_events(Route::Menu(MenuRoute { ctx: self.ctx.clone(), targets: self.targets.clone(), dispatch, tx: self.tx.clone() }));
    }

    fn cancel_tracking(&mut self) {
        if let Some(menu) = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).mainMenu()) {
            menu.cancelTracking();
        }
    }
}

/// `&` marks a mnemonic in muda labels.
fn escape(s: &str) -> String {
    s.replace('&', "&&")
}

/// A key equivalent for AppKit. `cmd` is ⌘ (`META`) and `ctrl` is ⌃: never folded together.
fn accelerator(c: &Chord) -> Option<Accelerator> {
    let m = c.mods;
    let mut mods = Modifiers::empty();
    for (on, flag) in [(m.cmd, Modifiers::META), (m.ctrl, Modifiers::CONTROL), (m.alt, Modifiers::ALT), (m.shift, Modifiers::SHIFT)] {
        if on {
            mods |= flag;
        }
    }
    let code = match c.key {
        Key::A => Code::KeyA,
        Key::B => Code::KeyB,
        Key::C => Code::KeyC,
        Key::D => Code::KeyD,
        Key::E => Code::KeyE,
        Key::F => Code::KeyF,
        Key::G => Code::KeyG,
        Key::H => Code::KeyH,
        Key::I => Code::KeyI,
        Key::J => Code::KeyJ,
        Key::K => Code::KeyK,
        Key::L => Code::KeyL,
        Key::M => Code::KeyM,
        Key::N => Code::KeyN,
        Key::O => Code::KeyO,
        Key::P => Code::KeyP,
        Key::Q => Code::KeyQ,
        Key::R => Code::KeyR,
        Key::S => Code::KeyS,
        Key::T => Code::KeyT,
        Key::U => Code::KeyU,
        Key::V => Code::KeyV,
        Key::W => Code::KeyW,
        Key::X => Code::KeyX,
        Key::Y => Code::KeyY,
        Key::Z => Code::KeyZ,
        Key::Num0 => Code::Digit0,
        Key::Num1 => Code::Digit1,
        Key::Num2 => Code::Digit2,
        Key::Num3 => Code::Digit3,
        Key::Num4 => Code::Digit4,
        Key::Num5 => Code::Digit5,
        Key::Num6 => Code::Digit6,
        Key::Num7 => Code::Digit7,
        Key::Num8 => Code::Digit8,
        Key::Num9 => Code::Digit9,
        Key::F1 => Code::F1,
        Key::F2 => Code::F2,
        Key::F3 => Code::F3,
        Key::F4 => Code::F4,
        Key::F5 => Code::F5,
        Key::F6 => Code::F6,
        Key::F7 => Code::F7,
        Key::F8 => Code::F8,
        Key::F9 => Code::F9,
        Key::F10 => Code::F10,
        Key::F11 => Code::F11,
        Key::F12 => Code::F12,
        Key::F13 => Code::F13,
        Key::F14 => Code::F14,
        Key::F15 => Code::F15,
        Key::F16 => Code::F16,
        Key::F17 => Code::F17,
        Key::F18 => Code::F18,
        Key::F19 => Code::F19,
        Key::F20 => Code::F20,
        Key::F21 => Code::F21,
        Key::F22 => Code::F22,
        Key::F23 => Code::F23,
        Key::F24 => Code::F24,
        Key::OpenBracket => Code::BracketLeft,
        Key::CloseBracket => Code::BracketRight,
        Key::Backslash => Code::Backslash,
        Key::Slash => Code::Slash,
        // `+` is ⇧= on the keyboards AppKit matches by key: Zoom In shows ⌘=.
        Key::Equals | Key::Plus => Code::Equal,
        Key::Minus => Code::Minus,
        Key::Quote => Code::Quote,
        Key::Semicolon => Code::Semicolon,
        Key::Comma => Code::Comma,
        Key::Period => Code::Period,
        Key::Backtick => Code::Backquote,
        // `Delete` is forward delete (egui's `Key::Delete`), `Backspace` the key Macs label
        // delete. muda writes them as U+007F and U+0008; which physical key AppKit matches to
        // each is not yet checked on a Mac.
        Key::Delete => Code::Delete,
        Key::Backspace => Code::Backspace,
        Key::Insert => Code::Insert,
        Key::Left => Code::ArrowLeft,
        Key::Right => Code::ArrowRight,
        Key::Up => Code::ArrowUp,
        Key::Down => Code::ArrowDown,
        Key::Home => Code::Home,
        Key::End => Code::End,
        Key::PageUp => Code::PageUp,
        Key::PageDown => Code::PageDown,
        Key::Escape if !mods.is_empty() => Code::Escape,
        Key::Enter if !mods.is_empty() => Code::Enter,
        Key::Tab if !mods.is_empty() => Code::Tab,
        Key::Space if !mods.is_empty() => Code::Space,
        _ => return None,
    };
    Some(Accelerator::new(mods, code))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::driver::key_event;

    /// The shortcuts the layout writes itself, and common app ones, become key equivalents.
    #[test]
    fn shortcuts_map_to_key_equivalents() {
        #[rustfmt::skip]
        let shortcuts = [
            "Cmd+Q", "Cmd+H", "Cmd+Alt+H", "Ctrl+Cmd+H", "Ctrl+Cmd+M", "Cmd+,", "Cmd+Shift+Z", "Cmd+1", "Cmd+[", "Cmd+=", "Cmd++",
            "Cmd+-", "Cmd+Delete", "Cmd+Backspace", "Cmd+Left", "Alt+Cmd+ArrowDown", "Cmd+Escape", "Cmd+Home", "Cmd+End",
            "Cmd+PageUp", "Cmd+PageDown", "Cmd+Enter", "Ctrl+F7",
        ];
        for s in shortcuts {
            let c = Chord::parse(s).unwrap();
            assert!(native_ok(&c) && accelerator(&c).is_some(), "{s} has no key equivalent");
            assert!(matches!(key_event(&c, true), egui::Event::Key { .. }), "{s} has no egui key press");
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
}
