//! The macOS menu bar, through muda (AppKit menus behind a safe API; no GTK with default features
//! off). This module only builds and updates the `NSMenu`s.
//!
//! - A radio item is a check item: AppKit menus have no radio look.
//! - A structure change rebuilds the menu bar; labels, enabled and checked are updated in place.
//!   A change of the windows key rebuilds only the Windows menu: AppKit adds its own items to that
//!   menu (window tiling, the window list), so muda can't insert rows there by position.
//! - A chosen item is a click or a key equivalent: AppKit's current event says which, and the
//!   handler gets the modifiers held for a key-down (AppKit may match ⌘R to a ⇧⌘R item).
//! - The process has one muda menu event handler, set by the first [`Installed::set_handler`] or
//!   [`install_app_menu`]; each call points it at its own handler.
//! - A check or radio item: AppKit (muda) flips its state when it is chosen. The next `apply` sets
//!   it back to the app's state.
//! - The platform items (Services, Show All, Minimize, Zoom, Bring All to Front, Close Window) are
//!   AppKit's predefined ones, titled by the caller.
//! - While a menu is open, changes wait until it closes: replacing or changing an `NSMenu` while
//!   AppKit tracks it can leave the displayed menu stranded.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType};
use objc2_foundation::{NSBundle, NSLocale, NSObjectProtocol, NSString};

use super::{Bar, Handler, Held, Item, Menu, MenuKind, Node, Options, Platform, Shortcut};

/// Set by [`Installed::install`]: there is one full menu bar per process.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Where the menu event handler sends events: the handler set last.
static ROUTE: Mutex<Option<Handler>> = Mutex::new(None);

/// muda keeps the first menu event handler set and ignores later ones, so it is set once.
static HANDLER: OnceLock<()> = OnceLock::new();

thread_local! {
    /// The minimal app menu ([`install_app_menu`]). muda's items must outlive the native menu
    /// that shows them.
    static APP_MENU: RefCell<Option<muda::Menu>> = const { RefCell::new(None) };
}

/// Point the process's menu event handler at `handler`, setting the handler the first time.
fn route_events(handler: Handler) {
    *lock(&ROUTE) = Some(handler);
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
            Held {
                command: flags.contains(NSEventModifierFlags::Command),
                control: flags.contains(NSEventModifierFlags::Control),
                option: flags.contains(NSEventModifierFlags::Option),
                shift: flags.contains(NSEventModifierFlags::Shift),
            }
        });
    // A copy, so nothing is locked while the app's handler runs.
    let handler = lock(&ROUTE).clone();
    if let Some(handler) = handler {
        handler(e.id.as_ref(), held);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A minimal application menu whose Quit item `quit_id` (⌘Q) reports to `handler`: About,
/// Services, Hide, Hide Others, Show All and Quit, labelled in English with `app_name`
/// ("Quit WordCraft"). Replaces the default menu of the windowing library; call it once, early.
///
/// Does nothing when this menu or a full menu bar ([`Installed::install`]) is already installed.
pub fn install_app_menu(app_name: &str, quit_id: &str, handler: Handler) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if MainThreadMarker::new().is_none() {
        return Err("the application menu is installed on the main thread".into());
    }
    if INSTALLED.load(Ordering::SeqCst) || APP_MENU.with_borrow(Option::is_some) {
        return Ok(());
    }
    let quit = MenuItem::with_id(quit_id, format!("Quit {app_name}"), true, Some(Accelerator::new(Modifiers::META, Code::KeyQ)));
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
    route_events(handler);
    APP_MENU.set(Some(menu));
    Ok(())
}

/// Hide the application (`NSApplication hide:`).
pub fn hide_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).hide(None);
    }
}

/// Hide the other applications (`NSApplication hideOtherApplications:`).
pub fn hide_other_apps() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).hideOtherApplications(None);
    }
}

/// AppKit's own title for `english`: the `MenuCommands` table of the AppKit bundle in the user's
/// language, `english` when the table has no such key. Cached per title; the user's language
/// does not change while the app runs.
pub fn system_title(english: &str) -> String {
    thread_local! {
        static TITLES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }
    TITLES.with(|titles| {
        titles
            .borrow_mut()
            .entry(english.to_owned())
            .or_insert_with(|| {
                let Some(bundle) = NSBundle::bundleWithIdentifier(&NSString::from_str("com.apple.AppKit")) else {
                    return english.to_owned();
                };
                let value = NSString::from_str(english);
                let table = NSString::from_str("MenuCommands");
                // A framework's strings follow the main bundle's localization, which is English
                // for an app without .lproj folders; ask in the user's languages instead
                // (macOS 15 and later).
                if bundle.respondsToSelector(sel!(localizedStringForKey:value:table:localizations:)) {
                    let languages = NSLocale::preferredLanguages();
                    bundle.localizedStringForKey_value_table_localizations(&value, Some(&value), Some(&table), &languages).to_string()
                } else {
                    bundle.localizedStringForKey_value_table(&value, Some(&value), Some(&table)).to_string()
                }
            })
            .clone()
    })
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
    label: String,
    enabled: bool,
}

/// The native items by muda id.
type Entries = HashMap<String, Entry>;

/// The Windows menu, the muda ids of its items, and the bar's `windows` key.
type WindowMenu = (Submenu, Vec<String>, u64);

pub(super) struct Installed {
    menu: Option<muda::Menu>,
    entries: Entries,
    structure: u64,
    window: Option<WindowMenu>,
    options: Options,
}

impl Installed {
    pub(super) fn install(bar: &Bar, options: Options) -> Option<Installed> {
        MainThreadMarker::new()?;
        if INSTALLED.swap(true, Ordering::SeqCst) {
            return None;
        }
        let mut installed = Installed { menu: None, entries: HashMap::new(), structure: 0, window: None, options };
        if !installed.build(bar) {
            INSTALLED.store(false, Ordering::SeqCst);
            return None;
        }
        Some(installed)
    }

    pub(super) fn set_handler(&self, handler: Handler) {
        route_events(handler);
    }

    /// Build the menu bar for `bar` and make it NSApp's. On an error or a panic in muda or AppKit
    /// the menus stay as they were (never crash for a menu) and this returns false.
    fn build(&mut self, bar: &Bar) -> bool {
        let built = catch_unwind(AssertUnwindSafe(|| -> muda::Result<(muda::Menu, Entries, Option<WindowMenu>)> {
            let mut entries = Entries::new();
            let mut window = None;
            let menu = muda::Menu::new();
            let mut special = Vec::new();
            for m in &bar.menus {
                let (sub, ids) = submenu(&mut entries, m)?;
                menu.append(&sub)?;
                if m.kind == MenuKind::Windows {
                    window = Some((sub.clone(), ids, bar.windows));
                }
                if m.kind == MenuKind::Help || (m.kind == MenuKind::Windows && self.options.window_list) {
                    special.push((m.kind, sub));
                }
            }
            menu.init_for_nsapp();
            // Only once the menu is NSApp's: muda looks the submenus up in the installed main menu.
            for (kind, sub) in special {
                match kind {
                    MenuKind::Windows => sub.set_as_windows_menu_for_nsapp(),
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
                self.structure = bar.structure;
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

    /// Replace the Windows menu (a document opened or closed), leaving the other menus as they are.
    /// Falls back to a full build when that fails. Returns whether menus were rebuilt.
    fn rebuild_window(&mut self, bar: &Bar) -> bool {
        let (Some(menu), Some((old, old_ids, _))) = (self.menu.clone(), self.window.clone()) else {
            return self.build(bar);
        };
        let Some((at, spec)) = bar.menus.iter().enumerate().find(|(_, m)| m.kind == MenuKind::Windows) else {
            return self.build(bar);
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
            Ok((sub, ids, bar.windows))
        }));
        match replaced {
            Ok(Ok(window)) => {
                self.entries = entries;
                self.window = Some(window);
                true
            }
            _ => self.build(bar),
        }
    }

    pub(super) fn apply(&mut self, bar: &Bar) -> bool {
        if self.menu.is_none() || bar.structure != self.structure {
            return self.build(bar);
        }
        let mut rebuilt = false;
        if self.window.as_ref().map(|w| w.2) != Some(bar.windows) {
            rebuilt = self.rebuild_window(bar);
        }
        let items: HashMap<&str, &Item> = bar.items().into_iter().map(|it| (it.id.as_str(), it)).collect();
        for (id, e) in &mut self.entries {
            let Some(it) = items.get(id.as_str()) else { continue };
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
            if let (Handle::Check(h), Some(c)) = (&e.handle, it.checked)
                && h.is_checked() != c
            {
                h.set_checked(c);
            }
        }
        rebuilt
    }

    /// A menu bar menu is open while one of its titles is highlighted.
    ///
    /// This assumes AppKit sets the main menu's `highlightedItem` while one of its menus is open
    /// and clears it when the menu closes; not yet confirmed on every macOS version. If it stayed
    /// set, a caller that waits for the menu to close would wait for good.
    pub(super) fn tracking(&self) -> bool {
        MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).mainMenu()).is_some_and(|m| m.highlightedItem().is_some())
    }

    pub(super) fn cancel_tracking(&self) {
        if let Some(menu) = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).mainMenu()) {
            menu.cancelTracking();
        }
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
            Node::Submenu { label, children } => {
                let s = Submenu::new(escape(label), !children.is_empty());
                append(entries, &s, children)?;
                sub.append(&s)?;
            }
            Node::Platform { kind, title } => {
                let text = Some(title.as_str());
                let p = match kind {
                    Platform::Services => PredefinedMenuItem::services(text),
                    Platform::ShowAll => PredefinedMenuItem::show_all(text),
                    Platform::Minimize => PredefinedMenuItem::minimize(text),
                    Platform::Zoom => PredefinedMenuItem::maximize(text),
                    Platform::BringAllToFront => PredefinedMenuItem::bring_all_to_front(text),
                    Platform::CloseWindow => PredefinedMenuItem::close_window(text),
                };
                sub.append(&p)?;
            }
            Node::Item(it) => {
                let accel = it.shortcut.as_ref().and_then(accelerator);
                let handle = match it.checked {
                    Some(c) => {
                        let h = CheckMenuItem::with_id(it.id.clone(), escape(&it.label), it.enabled, c, accel);
                        sub.append(&h)?;
                        Handle::Check(h)
                    }
                    None => {
                        let h = MenuItem::with_id(it.id.clone(), escape(&it.label), it.enabled, accel);
                        sub.append(&h)?;
                        Handle::Plain(h)
                    }
                };
                entries.insert(it.id.clone(), Entry { handle, label: it.label.clone(), enabled: it.enabled });
            }
        }
    }
    Ok(())
}

/// `&` marks a mnemonic in muda labels.
fn escape(s: &str) -> String {
    s.replace('&', "&&")
}

/// A key equivalent for AppKit. `command` is ⌘ (`META`) and `control` is ⌃: never folded together.
fn accelerator(s: &Shortcut) -> Option<Accelerator> {
    let mut mods = Modifiers::empty();
    for (on, flag) in
        [(s.held.command, Modifiers::META), (s.held.control, Modifiers::CONTROL), (s.held.option, Modifiers::ALT), (s.held.shift, Modifiers::SHIFT)]
    {
        if on {
            mods |= flag;
        }
    }
    Some(Accelerator::new(mods, Code::from_str(s.code).ok()?))
}
