//! The menu commands as help-search entries.

use craft_ui::menu::HelpEntry;

use crate::bar::{EnglishHook, Labeled};
use crate::model::{Builtin, MenuBar, Node};
use crate::{LayoutOptions, Target, layout};

impl MenuBar {
    /// The commands of the menus as the help search and the command palette list them, in menu order.
    ///
    /// The menus are laid out for `target` first, so the paths are the titles the user sees in the bar
    /// ("File › Export › PNG…"; on a Mac the app menu carries the app's name). Separators, submenu rows,
    /// AppKit's own items and the [`Builtin`] commands (which the crate runs itself) are not listed; the open
    /// documents of the Window menu are. Chords and enabled state are the commands': the search shows each entry
    /// as a command row.
    ///
    /// Building the list lays out the whole bar, so a host caches the result and rebuilds it when the bar
    /// changes, not every frame.
    ///
    /// `english` gives a command's or menu title's English name (as [`crate::MenuBarUi::english`]), asked on
    /// every target, so the search finds a translated command by its English name.
    pub fn help_entries(&self, target: Target, options: &LayoutOptions, english: Option<&EnglishHook<'_>>) -> Vec<HelpEntry> {
        entries_of(&layout(self, target, options).bar, english)
    }
}

/// The entries of an already laid-out bar.
pub(crate) fn entries_of(bar: &MenuBar, english: Option<&EnglishHook<'_>>) -> Vec<HelpEntry> {
    let english_of = |labeled: Labeled<'_>| english.and_then(|hook| hook(labeled));
    let mut entries = Vec::new();
    let mut path: Vec<String> = Vec::new();
    for menu in &bar.menus {
        path.push(menu.title.clone());
        collect(&menu.children, &mut path, &english_of, &mut entries);
        path.pop();
    }
    entries
}

fn collect(nodes: &[Node], path: &mut Vec<String>, english_of: &dyn Fn(Labeled<'_>) -> Option<String>, entries: &mut Vec<HelpEntry>) {
    for node in nodes {
        match node {
            Node::Item(item) if Builtin::of(&item.command).is_none() => {
                let mut entry = HelpEntry::new(item.command.clone(), item.label.clone()).with_path(path.iter().cloned());
                if let Some(english) = english_of(Labeled::Command(&item.command)) {
                    entry = entry.with_english(english);
                }
                entries.push(entry);
            }
            Node::Submenu { label, children, .. } => {
                path.push(label.clone());
                collect(children, path, english_of, entries);
                path.pop();
            }
            Node::Item(_) | Node::Separator | Node::Standard { .. } => {}
        }
    }
}
