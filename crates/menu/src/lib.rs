#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! A Craft app's menus as a macOS menu bar.
//!
//! The app supplies its menus as a tree ([`MenuBar`]), built directly or from the flat rows of
//! its in-window bar ([`from_items`]), and tags the items macOS puts in fixed places
//! ([`MenuBar::tag_item`], [`MenuBar::tag_submenu`], [`MenuBar::tag_menu`]). [`mac_layout`]
//! arranges them the way a Mac app's are, and [`NativeMenu`] hands the result to a platform
//! [`Backend`] and brings its events back. This part is pure and platform-free, so it is tested
//! everywhere and builds for the web; only the backend, `mac` (macOS only, built on muda), touches
//! AppKit. Its `mac::install` builds the menu bar and returns the [`NativeMenu`].
//!
//! What the layout does:
//! - an **app menu** named after the app: About (the item tagged [`ItemRole::About`]), Settings
//!   ([`ItemRole::Settings`], an item or a submenu of pages), Language ([`ItemRole::Language`]),
//!   Appearance ([`ItemRole::Appearance`]), Services, Hide ([`HIDE`]), Hide Others
//!   ([`HIDE_OTHERS`]), Show All and Quit. Quit is the app's own item tagged [`ItemRole::Quit`], so
//!   its unsaved-changes prompt runs; without one the layout adds [`craft_quit::QUIT_MENU_ID`],
//!   which asks the window to close. These items leave the menus they came from. A menu the app
//!   tagged [`MenuRole::App`] is merged into this one;
//! - **Window** ([`MenuRole::Window`], added when the app has none) gets Minimize, Zoom and Bring
//!   All to Front, with the open documents ([`ItemRole::Document`]) after them, where a Mac app
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
//! menu. A click is [`Event::Click`], and [`NativeMenu::take_clicks`] returns its id for the app
//! to run like an in-window menu click; so is a key equivalent the layout assigned (⌘Q on the
//! app's Quit item, ⌘, on Settings), which the app's key handling doesn't know, and, while a text
//! field has the keys ([`NativeMenu::set_text_editing`]), every key equivalent but the field's own
//! ⌘C, ⌘X, ⌘V, ⌘A, ⌘Z and ⇧⌘Z: an app's key handling skips shortcuts while text has the keys, and
//! a Mac app's menu commands (⌘S, ⌘N) run while typing.
//!
//! muda has one menu event handler per process, and the first one set stays:
//! `craft_quit::install_app_menu` and `mac::install` are alternatives, and an app calls one of
//! them once.
//!
//! Per frame: [`NativeMenu::raw_input`] before egui reads the input (`eframe::App::raw_input_hook`),
//! then [`NativeMenu::take_clicks`] and run the commands, then [`NativeMenu::sync_with`], which
//! lays the menus out again only when the app's state changed, and
//! [`NativeMenu::set_text_editing`] with whether a text field has the keys, for the next
//! [`NativeMenu::raw_input`]. [`NativeMenu::raw_input`] also
//! keeps a click or Escape that closes an open native menu away from egui.

mod chord;
mod driver;
mod layout;
mod model;

#[cfg(target_os = "macos")]
pub mod mac;

pub use chord::Chord;
pub use driver::{Backend, ClipboardKeys, Event, NativeMenu, had_input};
pub use layout::{Clash, Layout, LayoutOptions, mac_layout};
pub use model::{
    HIDE, HIDE_OTHERS, Item, ItemRole, MINIMIZE, Menu, MenuBar, MenuRole, Node, Row, Standard, from_items, structure_key, tidy, window_key,
};

#[cfg(test)]
mod tests;
