//! The macOS menu bar. This module lowers the platform-free menu model to the plain description
//! [`craft_window::native_menu`] shows, and turns the platform's events into the driver's
//! [`Event`]s. AppKit itself is in craft-window; off macOS [`install`] returns `None`.
//!
//! - An item's native id is its command's [`craft_keymap::CommandRef::key`]. Native ids must be
//!   unique: a command shown twice (Keyboard Shortcuts in Edit and Window) has `key` for its first
//!   item and `key#2` for its second, and both report the command.
//! - A radio item is a check item: AppKit menus have no radio look.
//! - A structure change ([`crate::structure_key`]: a new Open Recent file, a language switch)
//!   rebuilds the menu bar; labels, enabled and checked are updated in place. Opening or closing a
//!   document ([`crate::window_key`]) rebuilds only the Window menu.
//! - A chosen item is a click or a key equivalent: the platform says which. A key equivalent goes
//!   back to egui as the key press that was made (the event's modifiers, not the item's: AppKit may
//!   match ⌘R to a ⇧⌘R item), so the app's key rules still apply. A shortcut the layout assigned
//!   ([`crate::Item::layout_shortcut`]) stays a click: the app's keys don't know it. While a text
//!   field has the keys, a key equivalent is a click unless the field handles it
//!   ([`NativeMenu::set_gate`]).
//! - Only shortcuts with ⌘ or ⌃ become key equivalents. The others (`B`, `⇧B`, `⌥B`, `F5`) are not
//!   shown in the native menu: muda 0.21 has no handle on the `NSMenuItem`, and its
//!   `set_attributed_title` needs an `NSAttributedString` with a font and a tab stop, which
//!   objc2 builds only with `unsafe` calls (the workspace forbids unsafe code).
//! - Hide ([`crate::Builtin::Hide`]) and Hide Others ([`crate::Builtin::HideOthers`]) are custom items
//!   calling `NSApplication hide:` and `hideOtherApplications:`, because the predefined ones always
//!   take ⌘H and ⌥⌘H.
//! - Standard items (Services, Show All, Minimize, Zoom, Bring All to Front, Close Window) and the
//!   Hide Others and Minimize built-ins use the system's localized titles when the app supplies no
//!   translation, that is when the label is still the English name.
//! - While a menu is open, changes wait until it closes.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use craft_keymap::{Chord, CommandRef, Key, Mods};
use craft_window::native_menu as native;

use crate::driver::{Backend, Dispatch, Event, NativeMenu, native_ok};
use crate::model::{Builtin, Item, ItemKind, Menu, MenuBar, MenuRole, Node, Standard, label_or_system, native_id, structure_key, window_key};

/// Install the native menu bar for `bar` (a [`crate::mac_layout`] result) and return the driver
/// for it. Call it once, on the main thread, in eframe's creator closure, so winit's default menu
/// doesn't stay up. `None` when AppKit refused the menus (the app keeps its in-window menus), off
/// the main thread, when a menu bar is already installed, and on platforms without an application
/// menu bar. It replaces an app menu from [`install_app_menu`].
///
/// craft-menu owns the process's menu event handler.
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
    /// Set [`MacOptions::window_list`].
    pub fn window_list(mut self, on: bool) -> MacOptions {
        self.window_list = on;
        self
    }
}

/// [`install`] with `options`.
pub fn install_with(ctx: &egui::Context, bar: &MenuBar, options: MacOptions) -> Option<NativeMenu> {
    let (spec, targets) = lower(bar);
    let installed = native::NativeBar::install(&spec, native::Options::default().window_list(options.window_list))?;
    let (tx, rx) = channel();
    let backend = Native { ctx: ctx.clone(), bar: installed, targets: Arc::new(Mutex::new(targets)), tx, rx, pending: None };
    // `NativeMenu::new` attaches the driver's dispatch, which points the handler at this menu.
    Some(NativeMenu::new(Box::new(backend)))
}

/// Replace winit's default macOS app menu with a minimal one whose Quit (⌘Q) asks the app first,
/// for an app without a [`MenuBar`] of its own: About, Services, Hide, Hide Others, Show All and
/// Quit, labelled in English with `app_name` ("Quit WordCraft").
///
/// winit's Quit ends the process without asking, so unsaved work is lost. This Quit has the id
/// [`craft_quit::QUIT_MENU_ID`] and runs [`craft_quit::on_menu_event`]: it asks the root window to
/// close, and the app's unsaved-changes check runs as for the close button. Call it once from the eframe app creator: winit
/// builds its default menu before that and would replace one installed earlier.
///
/// Does nothing when this menu or a full menu bar ([`install`]) is already installed, and on
/// platforms without an application menu bar. On error winit's menu stays: the app still runs, but
/// ⌘Q quits without asking.
pub fn install_app_menu(ctx: &egui::Context, app_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ctx = ctx.clone();
    let handler = Arc::new(move |id: &str, _: Option<native::Held>| {
        craft_quit::on_menu_event(&ctx, id);
        // Choosing an item is not window input, so nothing else wakes egui.
        ctx.request_repaint();
    });
    native::install_app_menu(app_name, craft_quit::QUIT_MENU_ID, handler)
}

/// What each native item runs, by native id, for the handler.
type Targets = Arc<Mutex<HashMap<String, Target>>>;

/// What choosing an item reports.
#[derive(Clone)]
struct Target {
    /// The command (several items can share one).
    command: CommandRef,
    chord: Option<Chord>,
    layout_shortcut: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Native {
    ctx: egui::Context,
    bar: native::NativeBar,
    /// The items' targets, for the menu event handler.
    targets: Targets,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    /// The latest menus, waiting for the open menu to close.
    pending: Option<MenuBar>,
}

impl Native {
    fn update(&mut self, bar: &MenuBar) {
        let (spec, targets) = lower(bar);
        if self.bar.apply(&spec) {
            *lock(&self.targets) = targets;
        }
    }

    /// Nothing else may wake egui when the menu closes (Escape goes to the menu), so look again.
    fn retry_later(&mut self, bar: MenuBar) {
        self.pending = Some(bar);
        craft_ui::runtime::frame::request_repaint_after(&self.ctx, 0.25);
    }
}

/// The plain description of `bar` and what each of its native ids runs. The n-th item of a
/// command, depth first, has the n-th native id of that command.
fn lower(bar: &MenuBar) -> (native::Bar, HashMap<String, Target>) {
    let mut targets = HashMap::new();
    let menus = bar.menus.iter().map(|m| lower_menu(m, &mut targets)).collect();
    (native::Bar::new(menus, structure_key(bar), window_key(bar)), targets)
}

fn lower_menu(m: &Menu, targets: &mut HashMap<String, Target>) -> native::Menu {
    let kind = match m.role {
        MenuRole::Window => native::MenuKind::Windows,
        MenuRole::Help => native::MenuKind::Help,
        _ => native::MenuKind::Plain,
    };
    native::Menu::new(m.title.clone(), kind, lower_nodes(&m.children, targets))
}

fn lower_nodes(nodes: &[Node], targets: &mut HashMap<String, Target>) -> Vec<native::Node> {
    nodes
        .iter()
        .map(|n| match n {
            Node::Separator => native::Node::Separator,
            Node::Submenu { label, children, .. } => native::Node::Submenu { label: label.clone(), children: lower_nodes(children, targets) },
            Node::Standard { kind, label } => native::Node::Platform {
                kind: match kind {
                    Standard::Services => native::Platform::Services,
                    Standard::ShowAll => native::Platform::ShowAll,
                    Standard::Minimize => native::Platform::Minimize,
                    Standard::Zoom => native::Platform::Zoom,
                    Standard::BringAllToFront => native::Platform::BringAllToFront,
                    Standard::CloseWindow => native::Platform::CloseWindow,
                },
                title: label_or_system(label, kind.english(), native::system_title),
            },
            Node::Item(it) => {
                let id = native_id(&it.command, |id| targets.contains_key(id));
                let shortcut = it.shortcut.as_ref().filter(|c| native_ok(c)).and_then(shortcut);
                let checked = match it.kind {
                    ItemKind::Check(c) | ItemKind::Radio(c) => Some(c),
                    ItemKind::Plain => None,
                };
                let item = native::Item::new(id.clone(), native_label(it)).enabled(it.enabled).checked(checked).shortcut(shortcut);
                targets.insert(id, Target { command: it.command.clone(), chord: it.shortcut, layout_shortcut: it.layout_shortcut });
                native::Node::Item(item)
            }
        })
        .collect()
}

/// The label shown for `it`: Hide Others and Minimize take the system title when the app has no
/// translation.
fn native_label(it: &Item) -> String {
    match Builtin::of(&it.command) {
        Some(Builtin::HideOthers) => label_or_system(&it.label, "Hide Others", native::system_title),
        Some(Builtin::Minimize) => label_or_system(&it.label, Standard::Minimize.english(), native::system_title),
        _ => it.label.clone(),
    }
}

impl Backend for Native {
    fn sync(&mut self, bar: &MenuBar) {
        if self.bar.tracking() {
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
            self.bar.cancel_tracking();
            // Hide and Hide Others run here. While a shortcut is recorded their key equivalents
            // arrive as key presses instead, for the recorder.
            if let Event::Click(command) = &event
                && let Some(hide @ (Builtin::Hide | Builtin::HideOthers)) = Builtin::of(command)
            {
                if hide == Builtin::Hide {
                    native::hide_app()
                } else {
                    native::hide_other_apps()
                }
                continue;
            }
            out.push(event);
        }
        // After the events: they were chosen from the menus as they were.
        if let Some(bar) = self.pending.take() {
            if self.bar.tracking() {
                self.retry_later(bar);
            } else {
                self.update(&bar);
            }
        }
        out
    }

    fn tracking(&self) -> bool {
        self.bar.tracking()
    }

    fn attach(&mut self, dispatch: Dispatch) {
        let (ctx, targets, tx) = (self.ctx.clone(), self.targets.clone(), self.tx.clone());
        self.bar.set_handler(Arc::new(move |id: &str, held: Option<native::Held>| {
            let target = lock(&targets).get(id).cloned();
            // Ids that aren't ours are AppKit's own items (Services, Zoom, …), already handled.
            if let Some(t) = target {
                let held = held.map(|h| Mods { cmd: h.command, ctrl: h.control, alt: h.option, shift: h.shift });
                let event = dispatch.chosen(&t.command, t.chord.as_ref(), t.layout_shortcut, held);
                // The receiver goes away only with the app.
                let _ = tx.send(event);
            }
            // Choosing an item is not window input, so nothing else wakes egui: without this the item
            // runs only with the next input event, 8 to 90 seconds later in measurements.
            ctx.request_repaint();
        }));
    }

    fn cancel_tracking(&mut self) {
        self.bar.cancel_tracking();
    }
}

/// The key equivalent for a shortcut: `command` is ⌘ and `control` is ⌃, never folded together.
fn shortcut(c: &Chord) -> Option<native::Shortcut> {
    let m = c.mods;
    let mods_empty = !(m.cmd || m.ctrl || m.alt || m.shift);
    // The shifted symbols name the key they are typed on, and a menu item that wants the shift writes it as a
    // modifier; AppKit has no key equivalent for the context-menu key, the extra key of an ISO keyboard or F25 and up.
    let code: &'static str = match c.key {
        Key::Menu | Key::IntlBackslash => return None,
        Key::OpenCurlyBracket | Key::CloseCurlyBracket | Key::Pipe | Key::Colon | Key::Questionmark | Key::Exclamationmark => return None,
        Key::F25 | Key::F26 | Key::F27 | Key::F28 | Key::F29 | Key::F30 | Key::F31 | Key::F32 | Key::F33 | Key::F34 | Key::F35 => return None,
        Key::Escape | Key::Enter | Key::Tab | Key::Space if mods_empty => return None,
        key => key.code_name(),
    };
    Some(native::Shortcut { held: native::Held { command: m.cmd, control: m.ctrl, option: m.alt, shift: m.shift }, code })
}
