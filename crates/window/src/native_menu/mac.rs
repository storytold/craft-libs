//! The macOS menu bar, through muda (AppKit menus behind a safe API; no GTK with default features
//! off). This module only builds and updates the `NSMenu`s.
//!
//! - A radio item is a check item: AppKit menus have no radio look.
//! - A structure change rebuilds the menu bar; labels, enabled and checked are updated in place.
//!   A change of the windows key rebuilds only the Windows menu: AppKit adds its own items to that
//!   menu (window tiling, the window list), so muda can't insert rows there by position.
//! - A chosen item is a click or a key equivalent: AppKit's current event says which, and the
//!   handler gets whether a key press chose it and the modifiers held, for a click too (⌥ on a
//!   row with an alternate; AppKit may match ⌘R to a ⇧⌘R item).
//! - The process has one muda menu event handler, set by the first [`Installed::set_handler`] or
//!   [`install_app_menu`]; each call points it at its own handler.
//! - A check or radio item: AppKit (muda) flips its state when it is chosen. The next `apply` sets
//!   it back to the app's state.
//! - The platform items (Services, Show All, Minimize, Zoom, Bring All to Front, Close Window) are
//!   AppKit's predefined ones, titled by the caller.
//! - While a menu is open, changes wait until it closes: replacing or changing an `NSMenu` while
//!   AppKit tracks it can leave the displayed menu stranded.
//! - While a native panel has the keys (a Save or Open panel, the colour panel), an item with the
//!   standard Undo, Redo, Cut, Copy, Paste or Select All shortcut acts on the panel's text field,
//!   through a hidden menu of AppKit's own editing items ([`PANEL_EDIT_MENU`]), and the handler
//!   doesn't hear of it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType, NSMenuItem, NSPanel};
use objc2_foundation::{NSBundle, NSLocale, NSObjectProtocol, NSString};

use super::{Bar, Chosen, Handler, Held, Item, Menu, MenuKind, Node, Options, Platform, Shortcut, Wake};

/// Set by [`Installed::install`]: there is one full menu bar per process.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// A menu is tracking: set when AppKit posts `NSMenuDidBeginTracking`, cleared on `NSMenuDidEndTracking`. An atomic,
/// so it reads right off the main thread too; AppKit can deliver window input while a menu still tracks
/// (asynchronous menu presentation).
static TRACKING: AtomicBool = AtomicBool::new(false);

/// A menu began tracking since the last [`take_menu_began`]; set on begin only.
static BEGAN: AtomicBool = AtomicBool::new(false);

pub(super) fn menu_tracking() -> bool {
    TRACKING.load(Ordering::Relaxed)
}

pub(super) fn take_menu_began() -> bool {
    BEGAN.swap(false, Ordering::Relaxed)
}

/// Keep [`TRACKING`] and [`BEGAN`] in step with AppKit's notifications, and call `wake` when tracking ends so the
/// app applies the changes it held back while the menu was open.
fn observe_tracking(wake: Wake) {
    craft_appkit::observe_menu_tracking(Arc::new(move |tracking| {
        TRACKING.store(tracking, Ordering::Relaxed);
        if tracking {
            BEGAN.store(true, Ordering::Relaxed);
        } else {
            wake();
        }
    }));
}

/// Where the menu event handler sends events: the handler set last.
static ROUTE: Mutex<Option<Handler>> = Mutex::new(None);

/// muda keeps the first menu event handler set and ignores later ones, so it is set once.
static HANDLER: OnceLock<()> = OnceLock::new();

/// The items whose shortcut is a text field's own (⌘Z, ⌘C, …), by id: chosen while a native panel
/// has the keys, they act on its field ([`panel_takes_edit`]). Published from the installed
/// entries ([`publish_field_edits`]) only once a build has succeeded, so a failed rebuild, which
/// keeps the old menu, keeps its routing too.
static FIELD_EDITS: Mutex<Option<HashMap<String, FieldEdit>>> = Mutex::new(None);

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
    // Without a current event the item was chosen by a call, not by the user: a click, no modifiers.
    let chosen = MainThreadMarker::new()
        .and_then(|mtm| NSApplication::sharedApplication(mtm).currentEvent())
        .map(|ev| {
            let flags = ev.modifierFlags();
            Chosen {
                key_down: ev.r#type() == NSEventType::KeyDown,
                held: Held {
                    command: flags.contains(NSEventModifierFlags::Command),
                    control: flags.contains(NSEventModifierFlags::Control),
                    option: flags.contains(NSEventModifierFlags::Option),
                    shift: flags.contains(NSEventModifierFlags::Shift),
                },
            }
        })
        .unwrap_or_default();
    // Undo, Copy, Paste, … while a native panel has the keys are its text field's.
    let edit = lock(&FIELD_EDITS).as_ref().and_then(|edits| edits.get(e.id.as_ref()).copied());
    if edit.is_some_and(panel_takes_edit) {
        return;
    }
    // A copy, so nothing is locked while the app's handler runs.
    let handler = lock(&ROUTE).clone();
    if let Some(handler) = handler {
        handler(e.id.as_ref(), chosen);
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
    /// The text-field command the item's shortcut stands for in a native panel, if any.
    edit: Option<FieldEdit>,
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
    /// The last [`Installed::apply`] had to rebuild menus and could not.
    failed: bool,
}

impl Installed {
    pub(super) fn install(bar: &Bar, options: Options, wake: Wake) -> Option<Installed> {
        MainThreadMarker::new()?;
        if INSTALLED.swap(true, Ordering::SeqCst) {
            return None;
        }
        let mut installed = Installed { menu: None, entries: HashMap::new(), structure: 0, window: None, options, failed: false };
        if !installed.build(bar) {
            INSTALLED.store(false, Ordering::SeqCst);
            return None;
        }
        observe_tracking(wake);
        Some(installed)
    }

    pub(super) fn set_handler(&self, handler: Handler) {
        route_events(handler);
    }

    /// Build the menu bar for `bar` and make it NSApp's. On an error or a panic in muda or AppKit
    /// the menus stay as they were (never crash for a menu), [`Installed::failed`] reports it and
    /// this returns false.
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
            menu.append(&panel_edit_menu()?)?;
            menu.init_for_nsapp();
            if let Some(item) = MainThreadMarker::new().and_then(panel_edit_item) {
                item.setHidden(true);
            }
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
                publish_field_edits(&self.entries);
                self.window = window;
                self.structure = bar.structure;
                self.failed = false;
                true
            }
            _ => {
                self.failed = true;
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
                publish_field_edits(&self.entries);
                self.window = Some(window);
                true
            }
            _ => self.build(bar),
        }
    }

    pub(super) fn apply(&mut self, bar: &Bar) -> bool {
        // An update in place has nothing to fail; a rebuild below sets it again.
        self.failed = false;
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

    pub(super) fn failed(&self) -> bool {
        self.failed
    }

    pub(super) fn tracking(&self) -> bool {
        menu_tracking()
    }

    pub(super) fn take_began(&self) -> bool {
        take_menu_began()
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
                let edit = it.shortcut.as_ref().and_then(field_edit);
                entries.insert(it.id.clone(), Entry { handle, label: it.label.clone(), enabled: it.enabled, edit });
            }
        }
    }
    Ok(())
}

/// Make the installed entries' text-field shortcuts the ones [`handle_event`] routes to a panel.
fn publish_field_edits(entries: &Entries) {
    let edits = entries.iter().filter_map(|(id, e)| Some((id.clone(), e.edit?))).collect();
    *lock(&FIELD_EDITS) = Some(edits);
}

/// The text-editing commands a native panel's text field takes from the menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldEdit {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// The field command for an item with one of the Mac's standard editing shortcuts: ⌘Z, ⇧⌘Z, ⌘X,
/// ⌘C, ⌘V or ⌘A.
fn field_edit(s: &Shortcut) -> Option<FieldEdit> {
    let h = s.held;
    if !h.command || h.control || h.option {
        return None;
    }
    match (s.code, h.shift) {
        ("KeyZ", false) => Some(FieldEdit::Undo),
        ("KeyZ", true) => Some(FieldEdit::Redo),
        ("KeyX", false) => Some(FieldEdit::Cut),
        ("KeyC", false) => Some(FieldEdit::Copy),
        ("KeyV", false) => Some(FieldEdit::Paste),
        ("KeyA", false) => Some(FieldEdit::SelectAll),
        _ => None,
    }
}

/// The title of a hidden menu of AppKit's standard Undo, Redo, Cut, Copy, Paste and Select All
/// items ([`PANEL_EDITS`], in that order). Their actions (`undo:`, `copy:`, …) have no target, so
/// AppKit sends them to the key window's first responder: the text field of a native panel, also
/// when the panel runs in another process, as Save and Open panels do. The menu is hidden, so it
/// is never shown and takes no key equivalents.
const PANEL_EDIT_MENU: &str = "craft-window panel edit actions";

/// The hidden menu's items, in order.
const PANEL_EDITS: [FieldEdit; 6] = [FieldEdit::Undo, FieldEdit::Redo, FieldEdit::Cut, FieldEdit::Copy, FieldEdit::Paste, FieldEdit::SelectAll];

/// The hidden menu of standard editing items, to append to the menu bar before it is NSApp's.
fn panel_edit_menu() -> muda::Result<Submenu> {
    let menu = Submenu::new(PANEL_EDIT_MENU, true);
    menu.append_items(&[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ])?;
    Ok(menu)
}

/// The installed hidden menu's item in the menu bar ([`PANEL_EDIT_MENU`]).
fn panel_edit_item(mtm: MainThreadMarker) -> Option<Retained<NSMenuItem>> {
    NSApplication::sharedApplication(mtm).mainMenu()?.itemWithTitle(&NSString::from_str(PANEL_EDIT_MENU))
}

/// Whether a native panel (a Save or Open panel, the colour panel) has the keys and took `edit`.
///
/// AppKit offers ⌘Z, ⌘C, ⌘V and the like to the menu bar before the panel's text field sees them.
/// A standard Edit menu sends them on to the field; the app's own items run the app's command, so
/// typing a file name in the Save panel and pressing ⌘Z undid the document behind it. So the field
/// gets the command instead, through the matching item of the hidden standard menu
/// ([`PANEL_EDIT_MENU`]). The app doesn't hear of it even when the field has nothing to do, as a
/// standard menu's greyed item would do nothing.
fn panel_takes_edit(edit: FieldEdit) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    // The app's own windows are never panels.
    if !NSApplication::sharedApplication(mtm).keyWindow().is_some_and(|w| w.isKindOfClass(NSPanel::class())) {
        return false;
    }
    let index = PANEL_EDITS.iter().position(|e| *e == edit).and_then(|i| isize::try_from(i).ok());
    if let (Some(menu), Some(index)) = (panel_edit_item(mtm).and_then(|item| item.submenu()), index) {
        menu.performActionForItemAtIndex(index);
    }
    true
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
