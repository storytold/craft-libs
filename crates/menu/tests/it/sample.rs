//! A small app's menus and short constructors shared by the layout and native-menu trials.

use craft_keymap::Mods;
use craft_menu::{Chord, CommandRef, Document, Gate, ItemRole, Menu, MenuBar, MenuRole, Node, Row, from_items};

pub fn chord(s: &str) -> Chord {
    Chord::parse(s).unwrap()
}

/// The modifiers held for a key-down: ⌘, ⌃, ⌥, ⇧.
pub fn held([cmd, ctrl, alt, shift]: [bool; 4]) -> Option<Mods> {
    Some(Mods { cmd, ctrl, alt, shift })
}

pub fn typing(text_editing: bool) -> Gate {
    Gate::default().text_editing(text_editing)
}

pub fn cmd(id: &str) -> CommandRef {
    CommandRef::new(id)
}

pub fn row(path: &[&str], id: &str, label: &str, shortcut: Option<&str>) -> Row {
    let row = Row::new(path, id, label);
    match shortcut {
        Some(s) => row.shortcut(chord(s)),
        None => row,
    }
}

pub fn sep(path: &[&str]) -> Row {
    Row::separator(path)
}

/// A small app's menus, tagged. With `clashing`, app commands take ⌘H, ⌘M, ⌘W and ⌘,.
pub fn sample(clashing: bool, tag_quit: bool) -> MenuBar {
    let k = |own: &'static str| clashing.then_some(own);
    let rows = vec![
        row(&["File"], "file.new", "New", Some("Cmd+N")),
        sep(&["File"]),
        row(&["File"], "file.close", "Close", k("Cmd+W")),
        sep(&["File"]),
        row(&["File"], "file.exit", "Exit", None),
        row(&["Edit"], "edit.undo", "Undo", Some("Cmd+Z")),
        row(&["Edit"], "edit.redo", "Redo", Some("Cmd+Shift+Z")),
        sep(&["Edit"]),
        row(&["Edit", "Preferences"], "prefs.general", "General", None),
        row(&["Edit", "Preferences"], "prefs.keys", "Keys", None),
        row(&["Edit", "Language"], "lang.en", "English", None),
        row(&["View"], "view.extras", "Extras", k("Cmd+H")),
        row(&["View"], "view.hideLayers", "Hide Layers", k("Cmd+,")),
        row(&["View"], "view.curves", "Curves", k("Cmd+M")),
        row(&["View"], "view.zoomIn", "Zoom In", Some("Cmd++")),
        row(&["Window", "Theme"], "theme.dark", "Dark", None),
        row(&["Window"], "theme.next", "Next Theme", None),
        row(&["Window"], "panel.layers", "Layers", Some("F7")),
        row(&["Help"], "help.about", "About", None),
        row(&["Help"], "help.docs", "Docs", None),
    ];
    let mut bar = from_items(&["File", "Edit", "View", "Window", "Help"], &rows);
    bar.tag_menu("File", MenuRole::File);
    bar.tag_menu("Edit", MenuRole::Edit);
    bar.tag_menu("View", MenuRole::View);
    bar.tag_menu("Window", MenuRole::Window);
    bar.tag_menu("Help", MenuRole::Help);
    bar.tag_item(&cmd("help.about"), ItemRole::About);
    if tag_quit {
        bar.tag_item(&cmd("file.exit"), ItemRole::Quit);
    }
    bar.documents = vec![Document::new("doc.0", "Untitled-1").active(true)];
    bar.tag_item(&cmd("theme.next"), ItemRole::Appearance);
    bar.tag_submenu(MenuRole::Edit, "Preferences", ItemRole::Settings);
    bar.tag_submenu(MenuRole::Edit, "Language", ItemRole::Language);
    bar.tag_submenu(MenuRole::Window, "Theme", ItemRole::Appearance);
    bar
}

pub fn ids(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| match n {
            Node::Item(it) => it.command.key(),
            Node::Separator => "---".into(),
            Node::Submenu { label, .. } => format!("[{label}]"),
            Node::Standard { kind, .. } => format!("<{kind:?}>"),
            _ => "?".into(),
        })
        .collect()
}

pub fn titled<'a>(bar: &'a MenuBar, title: &str) -> &'a Menu {
    bar.menus.iter().find(|m| m.title == title).unwrap()
}
