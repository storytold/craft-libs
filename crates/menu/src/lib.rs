#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! A Craft app's menus, laid out for macOS, Windows, Linux and the web.
//!
//! The app supplies its menus as a tree ([`MenuBar`]), built directly or from the flat rows of
//! its in-window bar ([`from_items`]), gives each top-level menu its role ([`MenuRole`]), and
//! tags the items the layouts put in fixed places ([`MenuBar::tag_item`], [`MenuBar::tag_submenu`]).
//! Each item runs a [`craft_keymap::CommandRef`], its shortcut is a [`craft_keymap::Chord`], and
//! its [`ItemKind`] makes it plain, a check item or a radio item. [`layout`]
//! arranges them the way the platform's apps do, and [`NativeMenu`] hands the result to a platform
//! [`Backend`] and brings its events back. This part is pure and platform-free, so it is tested
//! everywhere and builds for the web; only the backend, `mac` (macOS only, built on muda), touches
//! AppKit. Its `mac::install` builds the menu bar and returns the [`NativeMenu`].
//!
//! **Layouts.** [`layout`] takes a [`Target`] ([`Target::current`] for this build) and calls
//! [`mac_layout`], [`desktop_layout`] (Windows and Linux) or [`web_layout`]. Each first drops the
//! items and submenus whose [`Platforms`] leave the target out, and every one lists
//! [`MenuBar::documents`] (but the hidden ones) last in the Window menu, as radio items with the
//! active document selected. On Windows and Linux About goes last in Help, Settings last in Edit
//! as Preferences, and Quit last in File, as Exit on Windows; the web layout has no Quit and takes
//! the browser's own shortcuts (Ctrl+W, Ctrl+T, …) off the items, reporting each as a [`Clash`].
//! A layout adds the menus it needs when the app has none. A submenu left empty goes; a dynamic
//! list with nothing in it shows the disabled placeholder item the app supplies.
//!
//! **Keymap.** [`apply_keymap`] sets the items' shortcuts from the app's
//! [`craft_keymap::Keymap`], user overrides included, and makes toggle commands check items;
//! [`apply_state`] sets enabled and checked per frame from an [`ItemState`] the app returns for
//! each command. Both run on the app's bar, before the layout.
//!
//! What the Mac layout does:
//! - an **app menu** named after the app: About (the item tagged [`ItemRole::About`]), Settings
//!   ([`ItemRole::Settings`], an item or a submenu of pages), Language ([`ItemRole::Language`]),
//!   Appearance ([`ItemRole::Appearance`]), Services, Hide ([`Builtin::Hide`]), Hide Others
//!   ([`Builtin::HideOthers`]), Show All and Quit. Quit is the app's own item tagged
//!   [`ItemRole::Quit`], so its unsaved-changes prompt runs; without one the layout adds
//!   [`Builtin::Quit`], which asks the window to close. These items leave the menus they came from. A menu the app
//!   tagged [`MenuRole::App`] is merged into this one;
//! - **Window** ([`MenuRole::Window`], added when the app has none) gets Minimize, Zoom and Bring
//!   All to Front, with the open documents ([`MenuBar::documents`]) after them, where a Mac app
//!   lists its windows. **Help** ([`MenuRole::Help`]) gets the system search field from the
//!   backend. **File** ([`MenuRole::File`]) gets Close Window unless the app binds ⌘W;
//! - **clashes** with the system's keys (⌘H, ⌥⌘H, ⌘M, ⌘W, ⌘,, ⌘Q) are resolved in the app's
//!   favour and reported ([`Clash`]); Hide and Minimize move to ⌃⌘H and ⌃⌘M, as Photoshop does,
//!   the others go without a shortcut. Shortcuts the app binds outside its menus count too
//!   ([`LayoutOptions::extra_shortcuts`]).
//!
//! The labels the layout writes itself are English unless the app passes its translations in
//! [`LayoutOptions::label`].
//!
//! **Keys stay with egui.** AppKit runs a menu's key equivalents before the window sees the key,
//! which would skip the app's own key rules (a focused text field keeps ⌘C, ⌘V and ⌘Z, dialogs
//! are modal, a tool may have its own ⌘Z). So the backend reports a key equivalent as
//! [`Event::Key`], and [`NativeMenu::raw_input`] feeds it to egui as the input egui-winit would
//! have made of it: a key press, or for ⌘C, ⌘X and ⌘V egui's copy, cut and paste events
//! ([`ClipboardKeys`]). The shortcut runs through the app's key handling as without a native
//! menu. A click is [`Event::Click`], and [`NativeMenu::take_clicks`] returns its command for the app
//! to run like an in-window menu click; so is a key equivalent the layout assigned (⌘Q on the
//! app's Quit item, ⌘, on Settings), which the app's key handling doesn't know, and, while a text
//! field has the keys ([`Gate::text_editing`]), every key equivalent but the field's own
//! (⌘C, ⌘X, ⌘V and [`craft_keymap::text_editing_chords`]): an app's key handling skips shortcuts
//! while text has the keys, and a Mac app's menu commands (⌘S, ⌘N) run while typing. Only
//! shortcuts with ⌘ or ⌃ become key equivalents; tool keys (`B`, `⇧B`, `F5`) stay with the app.
//!
//! What may run from the menus is the app's [`Gate`], set every frame ([`NativeMenu::set_gate`]):
//! while a modal dialog is open or a shortcut is being recorded, the app's items are greyed (but
//! the text fields' own Undo, Copy, …) and their clicks dropped, also clicks that arrived before
//! the gate closed. While a shortcut is
//! recorded, key equivalents reach egui as plain key presses.
//!
//! craft-menu owns muda's one menu event handler: `mac::install` (a full menu bar) and
//! `mac::install_app_menu` (a minimal app menu, for apps without a [`MenuBar`]) both use it.
//! [`NativeMenu::on_command`] sees each allowed click as it arrives, also while egui is idle.
//!
//! Per frame: [`NativeMenu::raw_input`] before egui reads the input (`eframe::App::raw_input_hook`),
//! then [`NativeMenu::set_gate`], [`NativeMenu::take_clicks`] and run the commands, then
//! [`NativeMenu::sync_with`], which lays the menus out again only when the app's state changed
//! (or [`NativeMenu::sync`], which compares a layout built every frame).
//! [`NativeMenu::raw_input`] also keeps a click or Escape that closes an open native menu away
//! from egui.

pub mod bar;
mod driver;
mod keymap;
mod layout;
mod model;

#[cfg(target_os = "macos")]
pub mod mac;

pub use bar::{BarStyle, Labeled, MenuBarUi};
pub use craft_keymap::{Chord, CommandRef};
pub use driver::{Backend, ClipboardKeys, CommandHook, Dispatch, Event, Gate, NativeMenu, had_input};
pub use keymap::{ItemState, apply_keymap, apply_state};
pub use layout::{Clash, Layout, LayoutOptions, Target, desktop_layout, layout, mac_layout, web_layout};
pub use model::{
    Builtin, Document, Item, ItemKind, ItemRole, Menu, MenuBar, MenuRole, Node, Platforms, Row, Standard, from_items, structure_key, tidy, window_key,
};

#[cfg(test)]
mod tests;
