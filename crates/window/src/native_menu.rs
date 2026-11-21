//! The platform's own menu bar for an app, from a plain description of its menus.
//!
//! The app describes its menus as a [`Bar`] (titles, items with a unique `id`, labels, check state
//! and key equivalents). [`NativeBar::install`] makes it the application's menu bar and
//! [`NativeBar::apply`] keeps it in step with later descriptions. A chosen item reaches the app's
//! [`Handler`] with the item's `id`, whether a key press chose it and the modifiers held. Nothing here knows
//! what an item runs.
//!
//! Only macOS has an application menu bar (with the `appkit` feature, built on `muda`). On other
//! platforms and without the feature [`NativeBar::install`] and [`install_app_menu`] do nothing, so
//! callers need no platform check.
//!
//! - A check item is flipped by the platform when it is chosen; the next [`NativeBar::apply`] sets
//!   it back to the described state.
//! - A change of [`Bar::structure`] rebuilds every menu; a change of [`Bar::windows`] rebuilds only
//!   the [`MenuKind::Windows`] menu, because AppKit adds its own items there. Anything else is
//!   updated in place (label, enabled, checked).
//! - While a menu is open ([`NativeBar::tracking`]) the caller waits to apply changes: replacing a
//!   menu the platform is tracking can leave it stranded on screen.
//! - Only items with a [`Shortcut`] become key equivalents; the caller decides which shortcuts
//!   those are.

use std::sync::Arc;

#[cfg(all(target_os = "macos", feature = "appkit"))]
mod mac;
#[cfg(all(target_os = "macos", feature = "appkit"))]
use mac as imp;
#[cfg(not(all(target_os = "macos", feature = "appkit")))]
mod other;
#[cfg(not(all(target_os = "macos", feature = "appkit")))]
use other as imp;

pub use imp::{hide_app, hide_other_apps, install_app_menu, system_title};

/// Called when an item is chosen: the item's `id` and how it was chosen ([`Chosen`]: key press or
/// click, and the modifiers held either way). The platform calls it between frames from the event
/// that chose the item, on the main thread. An id the app does not know belongs to a platform item (Services, Zoom, …), which the
/// platform already handled.
pub type Handler = Arc<dyn Fn(&str, Chosen) + Send + Sync>;

/// Called from any thread when menu tracking ends, so the app repaints and applies the changes it held back.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// How a menu item was chosen: by a key equivalent or a click, and the modifiers held then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Chosen {
    /// A key-down chose the item.
    pub key_down: bool,
    /// The modifiers held when it was chosen.
    pub held: Held,
}

/// The modifier keys held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// Command (⌘).
    pub command: bool,
    /// Control (⌃).
    pub control: bool,
    /// Option (⌥).
    pub option: bool,
    /// Shift (⇧).
    pub shift: bool,
}

/// A key equivalent: a W3C key code name (`"KeyS"`, `"Digit1"`, `"BracketLeft"`, `"ArrowLeft"`,
/// `"F5"`, …) with modifiers. A code name the platform does not know leaves the item without one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shortcut {
    /// The modifiers; ⌘ and ⌃ stay distinct.
    pub held: Held,
    /// The key.
    pub code: &'static str,
}

/// The items the platform supplies and localises itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Platform {
    /// The Services submenu.
    Services,
    /// Show All.
    ShowAll,
    /// Minimize.
    Minimize,
    /// Zoom.
    Zoom,
    /// Bring All to Front.
    BringAllToFront,
    /// Close Window.
    CloseWindow,
}

/// What the platform treats specially in a top-level menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuKind {
    /// An ordinary menu.
    #[default]
    Plain,
    /// The menu the platform lists the app's windows in ([`Options::window_list`]).
    Windows,
    /// The menu the platform adds its search field to.
    Help,
}

/// One item the user can choose.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Item {
    /// Unique in the bar; reported to the [`Handler`].
    pub id: String,
    /// The text shown. `&` is literal.
    pub label: String,
    /// Whether the item can be chosen.
    pub enabled: bool,
    /// `Some` makes a check item with this state.
    pub checked: Option<bool>,
    /// The key equivalent.
    pub shortcut: Option<Shortcut>,
}

impl Item {
    /// An enabled plain item.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Item {
        Item { id: id.into(), label: label.into(), enabled: true, checked: None, shortcut: None }
    }

    /// Whether the item can be chosen.
    pub fn enabled(mut self, enabled: bool) -> Item {
        self.enabled = enabled;
        self
    }

    /// Make it a check item with this state.
    pub fn checked(mut self, checked: Option<bool>) -> Item {
        self.checked = checked;
        self
    }

    /// Its key equivalent.
    pub fn shortcut(mut self, shortcut: Option<Shortcut>) -> Item {
        self.shortcut = shortcut;
        self
    }
}

/// An entry of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Node {
    /// A dividing line.
    Separator,
    /// A submenu, disabled while it has no children.
    Submenu {
        /// The title.
        label: String,
        /// Its entries.
        children: Vec<Node>,
    },
    /// An item the platform supplies, with the title to show.
    Platform {
        /// Which one.
        kind: Platform,
        /// The title.
        title: String,
    },
    /// An item the app handles.
    Item(Item),
}

/// A top-level menu.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Menu {
    /// The title.
    pub title: String,
    /// What the platform does with it.
    pub kind: MenuKind,
    /// Its entries.
    pub children: Vec<Node>,
}

impl Menu {
    /// A menu of `kind`.
    pub fn new(title: impl Into<String>, kind: MenuKind, children: Vec<Node>) -> Menu {
        Menu { title: title.into(), kind, children }
    }
}

/// The whole menu bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bar {
    /// The menus, left to right.
    pub menus: Vec<Menu>,
    /// Changes when the set of menus, submenus, items, their kinds or shortcuts change; a changed
    /// value rebuilds the bar.
    pub structure: u64,
    /// Changes when the [`MenuKind::Windows`] menu's items change; a changed value rebuilds only
    /// that menu.
    pub windows: u64,
}

impl Bar {
    /// A bar of `menus` with the two change keys.
    pub fn new(menus: Vec<Menu>, structure: u64, windows: u64) -> Bar {
        Bar { menus, structure, windows }
    }

    /// Every item, depth first.
    pub fn items(&self) -> Vec<&Item> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Item>) {
            for n in nodes {
                match n {
                    Node::Item(i) => out.push(i),
                    Node::Submenu { children, .. } => walk(children, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for m in &self.menus {
            walk(&m.children, &mut out);
        }
        out
    }
}

/// How [`NativeBar::install`] sets the bar up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// Make the [`MenuKind::Windows`] menu the platform's window list, which adds the app's
    /// windows after the menu's own items.
    pub window_list: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { window_list: true }
    }
}

impl Options {
    /// Set [`Options::window_list`].
    pub fn window_list(mut self, on: bool) -> Options {
        self.window_list = on;
        self
    }
}

/// Whether a menu is open. Safe from any thread; false where there is no application menu bar.
pub fn menu_tracking() -> bool {
    imp::menu_tracking()
}

/// Whether a menu began tracking since the last call, which clears it. Safe from any thread; false where there is
/// no application menu bar.
pub fn take_menu_began() -> bool {
    imp::take_menu_began()
}

/// The installed menu bar.
pub struct NativeBar(imp::Installed);

impl NativeBar {
    /// Make `bar` the application's menu bar. Call it once, on the main thread, before the event
    /// loop starts menus of its own. `None` when the platform refused the menus (the app keeps its
    /// in-window menus), off the main thread, when a bar is already installed, or on a platform
    /// without an application menu bar. It replaces a menu from [`install_app_menu`]. `wake` is called when menu
    /// tracking ends.
    pub fn install(bar: &Bar, options: Options, wake: Wake) -> Option<NativeBar> {
        imp::Installed::install(bar, options, wake).map(NativeBar)
    }

    /// Where chosen items go. The process has one handler, the one set last.
    pub fn set_handler(&self, handler: Handler) {
        self.0.set_handler(handler);
    }

    /// Bring the menus in step with `bar`: rebuild all, rebuild the windows menu, or update labels,
    /// enabled and checked in place, as [`Bar::structure`] and [`Bar::windows`] say. Returns
    /// whether menus were rebuilt, in which case the ids of the items may have changed.
    pub fn apply(&mut self, bar: &Bar) -> bool {
        self.0.apply(bar)
    }

    /// Whether one of the menus is open.
    pub fn tracking(&self) -> bool {
        self.0.tracking()
    }

    /// Whether a menu began tracking since the last call ([`take_menu_began`]).
    pub fn take_began(&self) -> bool {
        self.0.take_began()
    }

    /// Close the open menu, if there is one.
    pub fn cancel_tracking(&self) {
        self.0.cancel_tracking();
    }
}
