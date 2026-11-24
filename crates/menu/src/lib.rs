#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! A Craft app's menus, laid out for macOS, Windows, Linux and the web.
//!
//! The app supplies its menus as a tree ([`MenuBar`]), built directly or from the flat rows of
//! its in-window bar ([`from_items`]), gives each top-level menu its role ([`MenuRole`]), and
//! tags the items the layouts put in fixed places ([`MenuBar::tag_item`], [`MenuBar::tag_submenu`]).
//! Each item runs a [`craft_keymap::CommandRef`], its shortcut is a [`craft_keymap::Chord`], and
//! its [`ItemKind`] makes it plain, a check item or a radio item; [`Item::sample()`] gives its row a
//! painted sample in the in-window bar. A [`Node::Heading`] is a caption over the rows after it,
//! never a command: the native menu shows it as a disabled item, and [`tidy`] drops one with no
//! item after it. [`layout`]
//! arranges them the way the platform's apps do, and [`NativeMenu`] hands the result to a platform
//! [`Backend`] and brings its events back. This part is pure and platform-free, so it is tested
//! everywhere and builds for the web; only the backend touches AppKit, in `craft_window::native_menu`.
//! `mac::install` builds the menu bar through it and returns the [`NativeMenu`] (`None` off macOS).
//!
//! **Layouts.** [`layout`] takes a [`Target`] ([`Target::current`] for this build) and calls
//! [`mac_layout`], [`desktop_layout`] (Windows and Linux) or [`web_layout`]. Each first drops the
//! items and submenus whose [`Platforms`] leave the target out, and every one lists
//! [`MenuBar::documents`] (but the hidden ones) last in the Window menu, as check items with the
//! active document checked. On Windows and Linux About goes last in Help, Settings last in Edit
//! as Preferences, and Quit last in File, as Exit on Windows; the web layout has no Quit and takes
//! the browser's own shortcuts (Ctrl+W, Ctrl+T, …) off the items, reporting each as a [`Clash`].
//! A layout adds the menus it needs when the app has none. A submenu left empty goes; a dynamic
//! list with nothing in it shows the disabled placeholder item the app supplies.
//!
//! **In-window bar.** [`MenuBarUi`] shows a laid-out [`MenuBar`] in the window, and [`context::rows`] shows model
//! rows in any menu body (a context menu's included). Each item is a command row: its enabled state, check mark
//! and chord follow the command in the runtime's command state and keymap, and a choice is a fired command with
//! `FireSource::Menu`. [`MenuBar::help_entries`] lists the bar's commands for the Help menu's search and the
//! command palette.
//!
//! **Keymap.** For the native menu, [`apply_keymap`] sets the items' shortcuts from the app's
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
//!   the others go without a shortcut. A system item gives way to the command the app's keymap
//!   gives the key to, in a menu or not ([`LayoutOptions::keys`]), and to an item that shows it.
//!
//! **Alternates.** An [`Item`] or [`Row`] can carry an [`Alternate`] (craft-ui's, which panel menus
//! share): while its modifiers are held as the row is clicked, the row runs the alternate's
//! command, in the in-window bar and the native menu. Key equivalents are never swapped.
//!
//! **Customization.** [`Customization`] drops the rows the user hid and tints the ones they
//! coloured, before [`from_items`].
//!
//! **Appearance.** [`ThemeMenu`] builds the Appearance submenu and the quick favourites rows, as
//! [`LanguageMenu`] does for languages.
//!
//! The labels the layout writes itself are English unless the app passes its translations in
//! [`LayoutOptions::label`].
//!
//! **Keys stay with egui.** AppKit runs a menu's key equivalents before the window sees the key,
//! which would skip the app's own key rules (a focused text field keeps ⌘C, ⌘V and ⌘Z, dialogs
//! are modal, a tool may have its own ⌘Z). So the backend reports a key equivalent as
//! [`Event::Key`], and [`NativeMenu::poll`] hands it to the window host's input filter, which feeds it to egui as the input egui-winit would
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
//! What may run from the native menu is the app's [`Gate`], set every frame ([`NativeMenu::set_gate`]):
//! while a modal dialog is open or a shortcut is being recorded, the app's items are greyed (but
//! the text fields' own Undo, Copy, …) and their clicks dropped, also clicks that arrived before
//! the gate closed. While a shortcut is
//! recorded, key equivalents reach egui as plain key presses.
//!
//! craft-menu owns the process's one menu event handler: `mac::install` (a full menu bar) and
//! `mac::install_app_menu` (a minimal app menu, for apps without a [`MenuBar`]) both use it.
//! [`NativeMenu::on_command`] sees each allowed click as it arrives, also while egui is idle.
//!
//! A native menu opening is [`Event::Opened`]; [`NativeMenu::take_opened`] reports it once, for an
//! app that refreshes what a menu lists (a scripts folder) when it opens. It changes nothing the menus
//! show, so it does not re-sync them.
//!
//! Per frame: [`NativeMenu::poll`], then [`NativeMenu::set_gate`], [`NativeMenu::take_clicks`] and run the commands, then
//! [`NativeMenu::sync_with`], which lays the menus out again only when its key changed (the menu model's
//! `Tracked` revision and the other inputs of the layout).
//! The input filter also keeps a click or Escape that closes an open native menu away
//! from egui.
#![warn(missing_docs)]

pub mod bar;
pub mod context;
mod customize;
mod documents;
mod driver;
mod help;
mod keymap;
pub(crate) mod keytips;
mod language;
mod layout;
mod modal;
mod model;
mod palette;
mod theme;

pub mod mac;

pub mod ribbon;
pub mod shortcuts;

pub use bar::{MenuBarOutput, MenuBarUi};
pub use craft_keymap::{Chord, CommandRef};
pub use craft_ui::menu::Alternate;
pub use customize::Customization;
pub use documents::DOCUMENT_COMMAND_PREFIX;
pub use driver::{Backend, ClipboardKeys, CommandHook, Dispatch, Event, Gate, NativeMenu};
pub use help::{HelpCommand, HelpLabels, HelpLink, HelpMenu};
pub use keymap::{ItemState, apply_keymap, apply_state};
pub use keytips::{KeytipControl, Keytips, KeytipsOutput};
pub use language::{LANGUAGE_COMMAND, LanguageChoice, LanguageLabels, LanguageMenu, LanguageRow};
pub use layout::{Clash, Layout, LayoutOptions, Target, desktop_layout, layout, mac_layout, web_layout};
pub use modal::{ModalRules, NAVIGATION_COMMANDS, PendingRule};
pub use model::{
    Builtin, Document, Item, ItemKind, ItemRole, Menu, MenuBar, MenuRole, Node, Platforms, Row, Standard, from_items, structure_key, tidy, window_key,
};
pub use ribbon::{
    Gallery, GalleryFace, GalleryItem, GalleryMore, GalleryOutput, GroupLevel, GroupWidths, Ribbon, RibbonGroup, RibbonMenuRequest, RibbonOutput,
    RibbonRow, RibbonTab, plan,
};
pub use theme::{THEME_COMMAND, ThemeChoice, ThemeLabels, ThemeMenu, ThemeRow};

#[cfg(feature = "screen-testing")]
/// The stories and words this crate registers, for the story tests.
pub mod testing {

    #[cfg(feature = "suite")]
    craft_ui::registry!();
}
