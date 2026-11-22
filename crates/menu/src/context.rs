//! The rows of the app's menu model as command rows, for the bar's menus and for context menus.
//!
//! A row is a [`MenuItem`] bound to the item's command: its enabled state, check mark and chord follow the
//! command, the model supplies the label, mnemonic, tint and tooltip, and choosing it fires the command with
//! [`craft_ui::runtime::commands::FireSource::Menu`]. [`Builtin::Quit`] and [`Builtin::Minimize`] are local rows
//! that act on the root window through the runtime.

use craft_keymap::{CommandRef, Platform, mnemonic};
use craft_ui::contract::{Props as _, Show as _};
use craft_ui::display::Divider;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::commands;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::pointer;
use craft_ui::runtime::windows::{WindowAction, WindowHost};
use egui::{Ui, ViewportId};

use crate::bar::keymap_platform;
use crate::layout::Target;
use crate::model::{Builtin, Item, ItemKind, Node};

/// Shows `nodes` as rows of the menu body `ui` is in: a context menu's
/// (`Menu::context(label, &response).show(ui, |ui, _| craft_menu::context::rows(ui, &nodes))`) or any other menu's.
///
/// A [`Node::Item`] is a command row, a [`Node::Separator`] a divider and a [`Node::Submenu`] a submenu row;
/// AppKit's own items ([`Node::Standard`]) are left out. The chord of a row without a binding is the item's own
/// shortcut, written for the shell's operating system.
pub fn rows(ui: &mut Ui, nodes: &[Node]) {
    let rows = Rows::new(Target::of_shell(Env::of(ui).view().shell()));
    let labels = rows.level_labels(nodes, rows.platform(ui));
    rows.show(ui, nodes, &labels);
}

/// The text a menu level shows for its nodes, parallel to them: what [`LevelLabels::build`] works out once for a
/// model, so a pass that shows the menu again writes none of it.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LevelLabels {
    rows: Vec<RowLabels>,
}

/// One node's text: its label as the menu shows it (markers escaped, mnemonic marked), the item's shortcut text, and the
/// text of a submenu's own level. Separators and AppKit's items hold empty text.
#[derive(Debug, Default, PartialEq)]
struct RowLabels {
    label: String,
    shortcut: Option<String>,
    children: LevelLabels,
}

/// The text of a bar: its titles as the bar shows them and the text of each menu's rows.
#[derive(Debug, PartialEq)]
pub(crate) struct BarLabels {
    pub(crate) titles: Vec<String>,
    pub(crate) menus: Vec<LevelLabels>,
}

/// How the rows of a menu are written: the target that names the shortcuts and the app's preferred mnemonic
/// letters.
pub(crate) struct Rows<'a> {
    target: Target,
    preferred: &'a [(&'a str, char)],
}

impl<'a> Rows<'a> {
    pub(crate) fn new(target: Target) -> Self {
        Rows { target, preferred: &[] }
    }

    pub(crate) fn preferred(mut self, preferred: &'a [(&'a str, char)]) -> Self {
        self.preferred = preferred;
        self
    }

    /// The labels of `siblings` as their menu shows them: markers escaped, the app's preferred letter marked, and
    /// a label with no letter or digit of its own followed by its English label's letter, "文件(&F)". `key` names
    /// each sibling for the preferred letters, `english` asks for its English label.
    pub(crate) fn labels<T>(
        &self,
        siblings: &[T],
        label: impl Fn(&T) -> &str,
        key: impl Fn(&T) -> &str,
        english: impl Fn(&T) -> Option<String>,
    ) -> Vec<String> {
        let english: Vec<String> = siblings.iter().map(|sibling| english(sibling).unwrap_or_default()).collect();
        let english_refs: Vec<&str> = english.iter().map(String::as_str).collect();
        let letters = mnemonic::assign(&english_refs, &craft_i18n::Folder::new("en"));
        siblings
            .iter()
            .zip(letters)
            .map(|(sibling, letter)| {
                let text = label(sibling);
                let escaped = text.replace('&', "&&");
                let typed = text.chars().any(|c| c.is_ascii_alphanumeric());
                if let Some((letter, _)) = letter.filter(|_| !typed) {
                    return with_key(&escaped, letter);
                }
                match self.preferred_letter(key(sibling)) {
                    Some(letter) => marked(&escaped, letter),
                    None => escaped,
                }
            })
            .collect()
    }

    fn preferred_letter(&self, key: &str) -> Option<char> {
        self.preferred.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
    }

    /// The label a node shows as the menu's sibling, for the preferred letters and the English labels.
    pub(crate) fn node_labels(&self, nodes: &[Node]) -> Vec<String> {
        let shown: Vec<&Node> = nodes.iter().filter(|node| !matches!(node, Node::Separator | Node::Standard { .. })).collect();
        let mut labels = self
            .labels(
                &shown,
                |node| match node {
                    Node::Item(item) => item.label.as_str(),
                    Node::Submenu { label, .. } => label.as_str(),
                    _ => "",
                },
                |node| match node {
                    Node::Item(item) => item.command.id.as_str(),
                    Node::Submenu { label, .. } => label.as_str(),
                    _ => "",
                },
                |node| match node {
                    Node::Item(item) => item.english.clone(),
                    Node::Submenu { english, .. } => english.clone(),
                    _ => None,
                },
            )
            .into_iter();
        nodes
            .iter()
            .map(|node| if matches!(node, Node::Separator | Node::Standard { .. }) { String::new() } else { labels.next().unwrap_or_default() })
            .collect()
    }

    /// The text of `nodes` and of every submenu below them, with shortcuts written for `platform`.
    pub(crate) fn level_labels(&self, nodes: &[Node], platform: Platform) -> LevelLabels {
        let labels = self.node_labels(nodes);
        let rows = nodes
            .iter()
            .zip(labels)
            .map(|(node, label)| match node {
                Node::Item(item) => {
                    RowLabels { label, shortcut: item.shortcut.as_ref().map(|chord| chord.display(platform)), children: LevelLabels::default() }
                }
                Node::Submenu { children, .. } => RowLabels { label, shortcut: None, children: self.level_labels(children, platform) },
                Node::Separator | Node::Standard { .. } => RowLabels::default(),
            })
            .collect();
        LevelLabels { rows }
    }

    /// The text of a whole bar: the titles and each menu's rows.
    pub(crate) fn bar_labels(&self, menus: &[crate::model::Menu], platform: Platform) -> BarLabels {
        let titles = self.labels(menus, |menu| menu.title.as_str(), |menu| menu.title.as_str(), |menu| menu.english.clone());
        BarLabels { titles, menus: menus.iter().map(|menu| self.level_labels(&menu.children, platform)).collect() }
    }

    /// Shows `nodes` as the rows of the menu body `ui` is in, with their `labels`.
    pub(crate) fn show(&self, ui: &mut Ui, nodes: &[Node], labels: &LevelLabels) {
        for (node, text) in nodes.iter().zip(&labels.rows) {
            let label = text.label.clone();
            match node {
                Node::Item(item) => self.item(ui, item, label, text.shortcut.clone()),
                Node::Separator => {
                    Divider::horizontal().show(ui);
                }
                Node::Submenu { children, .. } => {
                    if is_dead(children) {
                        MenuItem::submenu(label).enabled(false).show(ui);
                    } else {
                        Menu::submenu_row(ui, label, |ui, _| self.show(ui, children, &text.children));
                    }
                }
                Node::Standard { .. } => {}
            }
        }
    }

    fn item(&self, ui: &mut Ui, item: &Item, label: String, shortcut: Option<String>) {
        match Builtin::of(&item.command) {
            Some(Builtin::Quit) => {
                if local(ui, item, label, shortcut).ran {
                    WindowHost::of(ui.ctx()).act(ViewportId::ROOT, WindowAction::Close);
                }
            }
            Some(Builtin::Minimize) => {
                if local(ui, item, label, shortcut).ran {
                    WindowHost::of(ui.ctx()).act(ViewportId::ROOT, WindowAction::Minimize);
                }
            }
            _ => {
                command_row(ui, item, label, shortcut).show(ui);
            }
        }
    }

    /// The platform whose key names the shortcut column shows.
    pub(crate) fn platform(&self, ui: &Ui) -> Platform {
        keymap_platform(self.target, Env::of(ui).view().shell().os)
    }
}

/// A local row for a built-in command: the bar runs it itself.
fn local(ui: &mut Ui, item: &Item, label: String, shortcut: Option<String>) -> craft_ui::menu::MenuItemOutput {
    let mut row = MenuItem::new(label).enabled(item.enabled);
    if let Some(shortcut) = shortcut {
        row = row.shortcut(shortcut);
    }
    row.show(ui)
}

/// The command row of `item`: the label and mnemonic come from the model, the state from the command.
fn command_row<'a>(ui: &Ui, item: &Item, label: String, shortcut: Option<String>) -> MenuItem<'a> {
    // The alternate swaps the command only; the label, kind, shortcut, tint and tooltip stay the
    // item's. The salt is the item's own command, so hover and focus stay on the row through the swap.
    let held = pointer::frame(ui.ctx()).mods;
    let command = match &item.alternate {
        Some(alternate) if alternate.selected_by(held) => alternate.command.clone(),
        _ => item.command.clone(),
    };
    let mut row = MenuItem::command(command).label(label).enabled(item.enabled).id_salt(item.command.key());
    row = match item.kind {
        ItemKind::Plain => row,
        ItemKind::Check(on) => row.check(checked(ui, &item.command).unwrap_or(on)),
        ItemKind::Radio(on) => row.radio_mark(on),
    };
    if let Some(shortcut) = shortcut {
        row = row.shortcut(shortcut);
    }
    if let Some(tint) = item.tint {
        row = row.tint(tint);
    }
    if let Some(tooltip) = &item.tooltip {
        row = row.tooltip(tooltip.clone());
    }
    row
}

/// The checked state the command itself reports.
fn checked(ui: &Ui, command: &CommandRef) -> Option<bool> {
    commands::resolve(ui.ctx(), command.clone()).checked
}

/// Whether no row of `nodes` can be chosen.
fn is_dead(nodes: &[Node]) -> bool {
    !nodes.iter().any(|node| match node {
        Node::Item(item) => item.enabled,
        Node::Submenu { children, .. } => !is_dead(children),
        Node::Separator | Node::Standard { .. } => false,
    })
}

/// `label` with a marker before the first `letter` it has.
fn marked(label: &str, letter: char) -> String {
    match label.char_indices().find(|(_, c)| c.eq_ignore_ascii_case(&letter)) {
        Some((at, _)) => format!("{}&{}", &label[..at], &label[at..]),
        None => label.to_owned(),
    }
}

/// `label` followed by its English label's `letter` as the mnemonic, before a trailing ellipsis: "新建(&N)…".
fn with_key(label: &str, letter: char) -> String {
    match label.strip_suffix('…').map(|base| (base, "…")).or_else(|| label.strip_suffix("...").map(|base| (base, "..."))) {
        Some((base, tail)) => format!("{base}(&{letter}){tail}"),
        None => format!("{label}(&{letter})"),
    }
}
