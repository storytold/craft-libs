//! The menu bar drawn in the window, for Windows, Linux and the web (and macOS, for testing): an
//! already laid-out [`MenuBar`] as a row of menu titles whose menus open below them.
//!
//! [`MenuBarUi`] shows the menu model through [`craft_ui::menu::MenuBar`], which owns the look and the
//! behaviour: press-drag-release, hover switching, submenus, scrolling, the keys (F10 and a lone Alt, Alt and a
//! mnemonic letter, Left and Right between menus), overflow into "…" or one "Menu", and AccessKit. Each item is a
//! command row ([`crate::context::rows`]): its enabled state, check mark and chord follow the command, and a
//! choice reaches the app as a fired command with [`craft_ui::runtime::commands::FireSource::Menu`].
//! `Builtin::Quit` and `Builtin::Minimize` act on the root window through the runtime.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_host::target::Os;
use craft_i18n::Words;
use craft_keymap::Platform;
use craft_ui::contract::{MemoProps, MemoView, Show as _, Tracked};
use craft_ui::menu::{MenuBar as Bar, MenuHelp, MenuTitle};
use craft_ui::runtime::env::{Env, Label};
use egui::{Rect, Ui};

use crate::context::Rows;
use crate::layout::{Target, node_commands};
use crate::model::{Builtin, MenuBar, MenuRole};
use crate::palette::entries_of;

/// Whether a menu of the bar is open; the app's single-key shortcuts wait while it is true.
pub use craft_ui::menu::bar::{is_focused, is_open};

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

/// A replay at another place moves the titles' rect with the bar.
impl craft_ui::runtime::Shift for MenuBarOutput {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        MenuBarOutput { titles: by.rect(self.titles), opened: self.opened }
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        MenuBarOutput { titles: craft_ui::runtime::Shift::snapped(&self.titles, &to.titles, within), opened: self.opened }
    }
}

/// What the bar did in a frame, from [`MenuBarUi::show`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MenuBarOutput {
    /// The menu titles alone, without the room the bar leaves after them
    /// ([`craft_ui::menu::MenuBarOutput::titles`]).
    pub titles: Rect,
    /// True in the one pass where a menu of the bar opened while none was open
    /// ([`craft_ui::menu::MenuBarOutput::opened`]).
    pub opened: bool,
}

/// The bar for one frame: `MenuBarUi::new(&bar).show(ui)`. Draw it at the top of the window, inside a `Stack` (a top
/// panel's body) or the title bar's row: there it folds the titles that don't fit its room into "…".
/// The commands chosen come back through the command queue, not from `show`.
///
/// The bar is a memoized view: while its model, options, place and environment are those of its last run and nothing
/// reaches it (pointer, focus, keys, an open menu), it adds that run again in place of running.
#[derive(craft_ui::contract::Pattern)]
pub struct MenuBarUi<'a> {
    bar: &'a Tracked<MenuBar>,
    /// The platform set with [`target`](Self::target); the shell's when none is.
    target: Option<Target>,
    preferred: &'a [(&'a str, char)],
    keyboard: Option<bool>,
    help_search: bool,
    icons: bool,
    compact: bool,
}

/// Every input of the bar's view.
#[derive(MemoProps)]
struct BarProps<'a> {
    bar: &'a Tracked<MenuBar>,
    target: Target,
    preferred: &'a [(&'a str, char)],
    keyboard: bool,
    help: Option<usize>,
    label: Label,
    icons: bool,
    compact: bool,
}

impl<'a> MenuBarUi<'a> {
    /// The bar for `bar`, already laid out ([`crate::layout`]), on the shell's platform ([`Target::of_shell`]).
    pub fn new(bar: &'a Tracked<MenuBar>) -> MenuBarUi<'a> {
        MenuBarUi { bar, target: None, preferred: &[], keyboard: None, help_search: false, icons: true, compact: false }
    }
}

impl<'a> MenuBarUi<'a> {
    /// The platform the bar is on, for the shortcut column ([`craft_keymap::Chord::display`]): ⌘ glyphs on
    /// [`Target::Mac`] and in a browser on a Mac, Ctrl names everywhere else. On [`Target::Mac`] the native bar
    /// has the keys, and this one takes none unless [`keyboard`](Self::keyboard) says so.
    pub fn target(mut self, target: Target) -> MenuBarUi<'a> {
        self.target = Some(target);
        self
    }

    /// Whether the bar takes F10, Alt and Alt with a letter. By default it does, except on [`Target::Mac`]. The
    /// app turns it off while a modal dialog or the shortcut recorder has the keys.
    pub fn keyboard(mut self, on: bool) -> MenuBarUi<'a> {
        self.keyboard = Some(on);
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

    /// Whether the rows show their commands' icons (on by default). Off, the menus are text only and their rows a
    /// line shorter, as a classic menu bar's.
    pub fn icons(mut self, on: bool) -> MenuBarUi<'a> {
        self.icons = on;
        self
    }

    /// Whether the rows are a line of text tall instead of a control's height (off by default): a dense classic menu.
    pub fn compact(mut self, on: bool) -> MenuBarUi<'a> {
        self.compact = on;
        self
    }

    /// Draws the bar and its open menu. Disabled rows never run. A chosen item is a fired command in the frame's
    /// output; [`Builtin::Quit`] and [`Builtin::Minimize`] act on
    /// the root window instead.
    pub fn show(self, ui: &mut Ui) -> MenuBarOutput {
        let env = Env::of(ui);
        let target = self.target.unwrap_or_else(|| Target::of_shell(env.view().shell()));
        let help = self.help_search.then(|| self.bar.menus.iter().position(|menu| menu.role == MenuRole::Help)).flatten();
        let props = BarProps {
            bar: self.bar,
            target,
            preferred: self.preferred,
            keyboard: self.keyboard.unwrap_or(target != Target::Mac),
            help,
            label: env.words::<BarWords>().label.clone(),
            icons: self.icons,
            compact: self.compact,
        };
        MemoView::new("menu-bar", props, show_bar).show(ui).unwrap_or(MenuBarOutput { titles: Rect::NOTHING, opened: false })
    }
}

/// The bar's view: the titles through [`craft_ui::menu::MenuBar`], each menu's rows as command rows.
fn show_bar(props: &mut BarProps<'_>, ui: &mut Ui) -> MenuBarOutput {
    let env = Env::of(ui);
    let cjk = crate::context::cjk(&env);
    let rows = Rows::new(props.target).cjk(cjk).preferred(props.preferred).icons(props.icons).compact(props.compact);
    let os = env.view().shell().os;
    // The text of every menu is worked out once for these inputs and kept: a pass that shows an open menu again writes none of it.
    let key = LabelKey {
        bar: props.bar,
        target: props.target,
        os,
        preferred: props.preferred,
        cjk,
        words: &env.words,
        keymap: craft_ui::runtime::commands::keymap_generation(ui.ctx()),
    };
    let labels = craft_ui::runtime::derived::derived(ui.ctx(), ui.scope_id().with("menu-labels"), &key, || {
        rows.bar_labels(&props.bar.menus, keymap_platform(props.target, os))
    });
    let (shared, labels) = (&rows, &*labels);
    let titles: Vec<MenuTitle<'_>> = props
        .bar
        .menus
        .iter()
        .zip(&labels.titles)
        .zip(&labels.menus)
        .map(|((menu, title), level)| {
            // The commands the title holds, for the palette's callout; a built-in has no command of the app.
            let held = menu.children.iter().flat_map(node_commands).filter(|command| Builtin::of(command).is_none()).collect();
            let row = MenuTitle::new(title.label.clone(), move |ui| shared.show(ui, &menu.children, level)).holds(held);
            match title.hidden {
                Some(letter) => row.mnemonic(letter),
                None => row,
            }
        })
        .collect();
    // Collecting the search's commands walks every row of every menu: the bar does it only while Help is open.
    let bar_model: &MenuBar = props.bar;
    let entries = move || entries_of(bar_model);
    let mut bar = Bar::new(props.label.clone(), titles).keyboard(props.keyboard);
    if let Some(menu) = props.help {
        bar = bar.help(MenuHelp::new(menu, &entries));
    }
    let out = bar.show(ui);
    MenuBarOutput { titles: out.titles, opened: out.opened }
}

/// What the kept text of a bar is made of: the model, the platform that names its shortcuts, the preferred mnemonics,
/// the language (its words, and whether it is CJK) and the keymap. The English names are part of the model.
#[derive(MemoProps)]
struct LabelKey<'a> {
    bar: &'a Tracked<MenuBar>,
    target: Target,
    os: Os,
    preferred: &'a [(&'a str, char)],
    cjk: bool,
    words: &'a Words,
    keymap: u64,
}
