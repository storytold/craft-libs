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
/// A [`Node::Item`] is a command row, a [`Node::Separator`] a divider, a [`Node::Heading`] a caption that takes no
/// mnemonic letter and a [`Node::Submenu`] a submenu row;
/// AppKit's own items ([`Node::Standard`]) are left out. The chord of a row without a binding is the item's own
/// shortcut, written for the shell's operating system.
pub fn rows(ui: &mut Ui, nodes: &[Node]) {
    let env = Env::of(ui);
    let rows = Rows::new(Target::of_shell(env.view().shell())).cjk(cjk(&env));
    let labels = rows.level_labels(nodes, rows.platform(ui));
    rows.show(ui, nodes, &labels);
}

/// The text a menu level shows for its nodes, parallel to them: what [`LevelLabels::build`] works out once for a
/// model, so a pass that shows the menu again writes none of it.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LevelLabels {
    rows: Vec<RowLabels>,
}

/// One node's text: its label as the menu shows it (markers escaped, mnemonic marked), the letter it takes without
/// showing it, the item's shortcut text, and the text of a submenu's own level. Separators and AppKit's items hold
/// empty text.
#[derive(Debug, Default, PartialEq)]
struct RowLabels {
    label: Shown,
    shortcut: Option<String>,
    children: LevelLabels,
}

/// The text of a bar: its titles as the bar shows them and the text of each menu's rows.
#[derive(Debug, PartialEq)]
pub(crate) struct BarLabels {
    pub(crate) titles: Vec<Shown>,
    pub(crate) menus: Vec<LevelLabels>,
}

/// A label as its menu shows it, and the English letter it takes without showing it: a translated label with no
/// letter of its own in a script that writes none after it (Cyrillic, Greek, Arabic…).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shown {
    pub(crate) label: String,
    pub(crate) hidden: Option<char>,
}

/// Whether the interface language of `env`'s scope is written in a CJK script.
pub(crate) fn cjk(env: &Env) -> bool {
    let code = env.view().host().map_or("en", |host| host.language_code());
    craft_i18n::scripts(code).first().is_some_and(|script| script.cjk())
}

/// How the rows of a menu are written: the target that names the shortcuts, the app's preferred mnemonic letters and
/// whether the interface language is CJK.
pub(crate) struct Rows<'a> {
    target: Target,
    /// Whether a label with no letter of its own shows its English label's letter after it.
    cjk: bool,
    preferred: &'a [(&'a str, char)],
    /// Whether rows show their command's icon.
    icons: bool,
    /// Whether rows are a text line tall.
    compact: bool,
}

impl<'a> Rows<'a> {
    pub(crate) fn new(target: Target) -> Self {
        Rows { target, cjk: false, preferred: &[], icons: true, compact: false }
    }

    pub(crate) fn cjk(mut self, on: bool) -> Self {
        self.cjk = on;
        self
    }

    pub(crate) fn icons(mut self, on: bool) -> Self {
        self.icons = on;
        self
    }

    pub(crate) fn compact(mut self, on: bool) -> Self {
        self.compact = on;
        self
    }

    pub(crate) fn preferred(mut self, preferred: &'a [(&'a str, char)]) -> Self {
        self.preferred = preferred;
        self
    }

    /// The labels of `siblings` as their menu shows them: markers escaped and the app's preferred letter marked. A
    /// label with no letter or digit of its own takes its English label's letter: in a CJK language shown after it,
    /// "文件(&F)", the convention there; in any other script hidden, so Alt+F still opens "Файл". `key` names each
    /// sibling for the preferred letters, `english` asks for its English label.
    pub(crate) fn labels<T>(
        &self,
        siblings: &[T],
        label: impl Fn(&T) -> &str,
        key: impl Fn(&T) -> &str,
        english: impl Fn(&T) -> Option<String>,
    ) -> Vec<Shown> {
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
                match letter.filter(|_| !typed) {
                    Some((letter, _)) if self.cjk => Shown { label: with_key(&escaped, letter), hidden: None },
                    Some((letter, _)) => Shown { label: escaped, hidden: Some(letter) },
                    None => {
                        let label = match self.preferred_letter(key(sibling)) {
                            Some(letter) => marked(&escaped, letter),
                            None => escaped,
                        };
                        Shown { label, hidden: None }
                    }
                }
            })
            .collect()
    }

    fn preferred_letter(&self, key: &str) -> Option<char> {
        self.preferred.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
    }

    /// The label a node shows as the menu's sibling, for the preferred letters and the English labels.
    pub(crate) fn node_labels(&self, nodes: &[Node]) -> Vec<Shown> {
        let shown: Vec<&Node> = nodes.iter().filter(|node| !takes_no_letter(node)).collect();
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
        nodes.iter().map(|node| if takes_no_letter(node) { Shown::default() } else { labels.next().unwrap_or_default() }).collect()
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
                Node::Separator | Node::Heading(_) | Node::Standard { .. } => RowLabels::default(),
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
                Node::Submenu { children, enabled, .. } => {
                    if !enabled || is_dead(children) {
                        let mut row = MenuItem::submenu(label.label).enabled(false);
                        if let Some(letter) = label.hidden {
                            row = row.mnemonic(letter);
                        }
                        row.show(ui);
                    } else {
                        Menu::submenu_row_lettered(ui, label.label, true, label.hidden, |ui, _| self.show(ui, children, &text.children));
                    }
                }
                Node::Heading(text) => {
                    MenuItem::heading(text.clone()).show(ui);
                }
                Node::Standard { .. } => {}
            }
        }
    }

    fn item(&self, ui: &mut Ui, item: &Item, label: Shown, shortcut: Option<String>) {
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
                command_row(ui, item, label, shortcut).no_icon(!self.icons).compact(self.compact).show(ui);
            }
        }
    }

    /// The platform whose key names the shortcut column shows.
    pub(crate) fn platform(&self, ui: &Ui) -> Platform {
        keymap_platform(self.target, Env::of(ui).view().shell().os)
    }
}

/// A local row for a built-in command: the bar runs it itself.
fn local(ui: &mut Ui, item: &Item, label: Shown, shortcut: Option<String>) -> craft_ui::menu::MenuItemOutput {
    let mut row = MenuItem::new(label.label).enabled(item.enabled);
    if let Some(letter) = label.hidden {
        row = row.mnemonic(letter);
    }
    if let Some(shortcut) = shortcut {
        row = row.shortcut(shortcut);
    }
    row.show(ui)
}

/// The command row of `item`: the label and mnemonic come from the model, the state from the command.
fn command_row<'a>(ui: &Ui, item: &Item, label: Shown, shortcut: Option<String>) -> MenuItem<'a> {
    // The alternate swaps the command only; the label, kind, shortcut, tint and tooltip stay the
    // item's. The salt is the item's own command, so hover and focus stay on the row through the swap.
    let held = pointer::frame(ui.ctx()).mods;
    let command = match &item.alternate {
        Some(alternate) if alternate.selected_by(held) => alternate.command.clone(),
        _ => item.command.clone(),
    };
    let mut row = MenuItem::command(command).label(label.label).enabled(item.enabled).id_salt(item.command.key());
    if let Some(letter) = label.hidden {
        row = row.mnemonic(letter);
    }
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
    if let Some((aspect, face)) = &item.sample {
        row = row.sample(*aspect, face.clone());
    }
    row
}

/// Whether `node` shows no row of its own to choose, so it takes no mnemonic letter and leaves its siblings'
/// letters as they are.
fn takes_no_letter(node: &Node) -> bool {
    matches!(node, Node::Separator | Node::Heading(_) | Node::Standard { .. })
}

/// The checked state the command itself reports.
fn checked(ui: &Ui, command: &CommandRef) -> Option<bool> {
    commands::resolve(ui.ctx(), command.clone()).checked
}

/// Whether no row of `nodes` can be chosen.
fn is_dead(nodes: &[Node]) -> bool {
    !nodes.iter().any(|node| match node {
        Node::Item(item) => item.enabled,
        Node::Submenu { children, enabled, .. } => *enabled && !is_dead(children),
        Node::Separator | Node::Heading(_) | Node::Standard { .. } => false,
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
