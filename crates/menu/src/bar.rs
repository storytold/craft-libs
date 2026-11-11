//! The menu bar drawn in the window, for Windows, Linux and the web (and macOS, for testing): an
//! already laid-out [`MenuBar`] as a row of menu titles whose menus open below them.
//!
//! [`MenuBarUi`] shows the menu model through [`craft_ui::menu::MenuBar`], which owns the look and the
//! behaviour: press-drag-release, hover switching, submenus, scrolling, the keys (F10 and a lone Alt, Alt and a
//! mnemonic letter, Left and Right between menus), overflow into "…" or one "Menu", and AccessKit. Each item is a
//! command row ([`crate::context::rows`]): its enabled state, check mark and chord follow the command, and a
//! choice reaches the app as a fired command with [`craft_ui::runtime::commands::FireSource::Menu`].
//! [`Builtin::Quit`] and [`Builtin::Minimize`] act on the root window through the runtime.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_keymap::{CommandRef, Platform};
use craft_ui::contract::Show as _;
use craft_ui::menu::{MenuBar as Bar, MenuHelp, MenuTitle};
use craft_ui::runtime::env::Env;
use egui::{Rect, Ui};

use crate::context::Rows;
use crate::layout::Target;
use crate::model::{MenuBar, MenuRole};
use crate::palette::entries_of;

/// Whether a menu of the bar is open; the app's single-key shortcuts wait while it is true.
pub use craft_ui::menu::bar::{is_focused, is_open};

/// A row or title, as the bar names it to the app's mnemonic hooks ([`MenuBarUi::english`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Labeled<'s> {
    /// An item, by its command.
    Command(&'s CommandRef),
    /// A menu or submenu, by its title as shown.
    Title(&'s str),
}

/// The app's English labels ([`MenuBarUi::english`]).
pub type EnglishHook<'h> = dyn Fn(Labeled<'_>) -> Option<String> + 'h;

craft_ui::words! {
    /// The words of the in-window bar that the menu family does not hold.
    struct BarWords {
        label: "menuBar.label" = "Main menu";
    }
}

/// The platform whose key names a target's shortcut column shows; for the web, the one the browser runs on
/// (`os`, the shell's).
pub(crate) fn keymap_platform(target: Target, os: Platform) -> Platform {
    match target {
        Target::Mac => Platform::Mac,
        Target::Windows => Platform::Windows,
        Target::Linux => Platform::Linux,
        Target::Web => os,
    }
}

/// What the bar did in a frame, from [`MenuBarUi::show`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MenuBarOutput {
    /// The menu titles alone, without the room the bar leaves after them
    /// ([`craft_ui::menu::MenuBarOutput::titles`]).
    pub titles: Rect,
}

/// The bar for one frame: `MenuBarUi::new(&bar).show(ui)`. Draw it at the top of the window, in a top panel.
/// The commands chosen come back through the command queue, not from `show`.
#[derive(craft_ui::contract::Pattern)]
pub struct MenuBarUi<'a> {
    bar: &'a MenuBar,
    target: Target,
    english: Option<&'a EnglishHook<'a>>,
    preferred: &'a [(&'a str, char)],
    keyboard: Option<bool>,
    help_search: bool,
}

impl<'a> MenuBarUi<'a> {
    /// The bar for `bar`, already laid out ([`crate::layout`]), on [`Target::current`].
    pub fn new(bar: &'a MenuBar) -> MenuBarUi<'a> {
        MenuBarUi { bar, target: Target::current(), english: None, preferred: &[], keyboard: None, help_search: false }
    }

    /// The platform the bar is on, for the shortcut column ([`craft_keymap::Chord::display`]): ⌘ glyphs on
    /// [`Target::Mac`] and in a browser on a Mac, Ctrl names everywhere else. On [`Target::Mac`] the native bar
    /// has the keys, and this one takes none unless [`keyboard`](Self::keyboard) says so.
    pub fn target(mut self, target: Target) -> MenuBarUi<'a> {
        self.target = target;
        self
    }

    /// Whether the bar takes F10, Alt and Alt with a letter. By default it does, except on [`Target::Mac`]. The
    /// app turns it off while a modal dialog or the shortcut recorder has the keys.
    pub fn keyboard(mut self, on: bool) -> MenuBarUi<'a> {
        self.keyboard = Some(on);
        self
    }

    /// The English label of a row or title, for labels the keyboard can't type a letter of (Chinese, Japanese,
    /// Korean…): such a label shows its English label's mnemonic after it, "文件(F)", before a trailing ellipsis
    /// ("開く(O)…"), as Windows menus do. Without this hook (or when it returns `None`) such a label has no
    /// mnemonic. The Help menu's search also matches the English labels, on every target.
    pub fn english(mut self, english: &'a dyn Fn(Labeled<'_>) -> Option<String>) -> MenuBarUi<'a> {
        self.english = Some(english);
        self
    }

    /// Preferred mnemonics, each for a menu or submenu by its title as shown or for an item by its command id:
    /// the row takes that letter when its label has it and no row before it in its menu took it. Every other row
    /// takes the first unused letter of its label, word starts first.
    pub fn mnemonics(mut self, preferred: &'a [(&'a str, char)]) -> MenuBarUi<'a> {
        self.preferred = preferred;
        self
    }

    /// Start the [`MenuRole::Help`] menu with a search field over every command of the bar
    /// ([`MenuBar::help_entries`]). A bar with no Help menu has no field.
    pub fn help_search(mut self, on: bool) -> MenuBarUi<'a> {
        self.help_search = on;
        self
    }

    /// Draws the bar and its open menu. Disabled rows never run. A chosen item is a fired command in the frame's
    /// output; [`Builtin::Quit`](crate::Builtin::Quit) and [`Builtin::Minimize`](crate::Builtin::Minimize) act on
    /// the root window instead.
    pub fn show(self, ui: &mut Ui) -> MenuBarOutput {
        let env = Env::of(ui);
        let rows = Rows::new(self.target).english(self.english).preferred(self.preferred);
        let labels =
            rows.labels(&self.bar.menus, |menu| menu.title.as_str(), |menu| menu.title.as_str(), |menu| rows.english_of(Labeled::Title(&menu.title)));
        let shared = &rows;
        let titles: Vec<MenuTitle<'_>> =
            self.bar.menus.iter().zip(labels).map(|(menu, label)| MenuTitle::new(label, move |ui| shared.show(ui, &menu.children))).collect();
        let help = self.help_search.then(|| self.bar.menus.iter().position(|menu| menu.role == MenuRole::Help)).flatten();
        let entries = if help.is_some() { entries_of(self.bar, self.english) } else { Vec::new() };
        let mut bar = Bar::new(env.words::<BarWords>().label.clone(), titles).keyboard(self.keyboard.unwrap_or(self.target != Target::Mac));
        if let Some(menu) = help {
            bar = bar.help(MenuHelp::new(menu, &entries));
        }
        MenuBarOutput { titles: bar.show(ui).titles }
    }
}
