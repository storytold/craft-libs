//! The menu tree, the flat rows it can be built from, and the keys that tell a backend what
//! changed.

use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuBar {
    pub menus: Vec<Menu>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub role: MenuRole,
    pub children: Vec<Node>,
}

/// What a top-level menu is for. macOS treats the app, Window and Help menus specially; File is
/// where the system Close Window item goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuRole {
    #[default]
    Normal,
    App,
    File,
    Window,
    Help,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Item(Item),
    Separator,
    Submenu {
        label: String,
        children: Vec<Node>,
        role: Option<ItemRole>,
    },
    /// An item AppKit provides and runs itself (Services, Zoom, …), with the label to show.
    Standard {
        kind: Standard,
        label: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The app's command id (`file.open`), or one of [`HIDE`], [`HIDE_OTHERS`], [`MINIMIZE`] and
    /// [`craft_quit::QUIT_MENU_ID`]. A click reports only this id, so a command that takes a
    /// parameter (a recent file, a theme) carries it in the id: `file.recent:3`.
    pub id: String,
    pub label: String,
    /// `Cmd+Shift+Z` form; `Ctrl` is the real Control key. See [`crate::Chord`].
    pub shortcut: Option<String>,
    pub enabled: bool,
    /// `Some` for check items.
    pub checked: Option<bool>,
    pub role: Option<ItemRole>,
    /// The layout gave this item its shortcut (⌘Q on the app's Quit, ⌘, on Settings): the app's
    /// key handling doesn't know it, so a backend reports the key equivalent as a click.
    pub layout_shortcut: bool,
}

/// Items macOS expects in fixed places. The app tags its own About, Settings, Quit, Appearance,
/// Language and Document items; the layout adds Hide and Minimize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemRole {
    About,
    /// The app's settings: an item, or a submenu of settings pages.
    Settings,
    /// The app's own quit command, which asks about unsaved changes. Without one the layout adds
    /// [`craft_quit::QUIT_MENU_ID`].
    Quit,
    Hide,
    Minimize,
    /// A submenu of themes or appearance modes, and items that go with it (Next Theme).
    Appearance,
    /// The app's language chooser, a submenu.
    Language,
    /// An open document in the Window menu, where a Mac app lists its windows. Its label is the
    /// document's name.
    Document,
}

/// The items AppKit provides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Standard {
    Services,
    ShowAll,
    Minimize,
    Zoom,
    BringAllToFront,
    CloseWindow,
}

impl Standard {
    /// The English label, as passed to [`crate::LayoutOptions::label`].
    pub fn english(self) -> &'static str {
        match self {
            Standard::Services => "Services",
            Standard::ShowAll => "Show All",
            Standard::Minimize => "Minimize",
            Standard::Zoom => "Zoom",
            Standard::BringAllToFront => "Bring All to Front",
            Standard::CloseWindow => "Close Window",
        }
    }
}

/// Hide the app: a custom item, because AppKit's own always takes ⌘H, which apps often bind.
pub const HIDE: &str = "craft-menu.hide";
/// Hide the other apps: a custom item, because AppKit's own always takes ⌥⌘H.
pub const HIDE_OTHERS: &str = "craft-menu.hide-others";
/// Minimize: a custom item, used when the app binds ⌘M (AppKit's own always takes ⌘M).
/// [`crate::NativeMenu::take_clicks`] runs it.
pub const MINIMIZE: &str = "craft-menu.minimize";

impl Item {
    /// An enabled plain item with no shortcut.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Item {
        Item { id: id.into(), label: label.into(), shortcut: None, enabled: true, checked: None, role: None, layout_shortcut: false }
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Item {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Item {
        self.enabled = enabled;
        self
    }

    /// Make this a check item.
    pub fn checked(mut self, checked: bool) -> Item {
        self.checked = Some(checked);
        self
    }

    pub fn role(mut self, role: ItemRole) -> Item {
        self.role = Some(role);
        self
    }
}

impl From<Item> for Node {
    fn from(item: Item) -> Node {
        Node::Item(item)
    }
}

impl Node {
    pub fn submenu(label: impl Into<String>, children: Vec<Node>) -> Node {
        Node::Submenu { label: label.into(), children, role: None }
    }

    /// Give an item or a submenu a role: `Node::submenu("Theme", pages).role(ItemRole::Appearance)`.
    pub fn role(mut self, role: ItemRole) -> Node {
        match &mut self {
            Node::Item(it) => it.role = Some(role),
            Node::Submenu { role: r, .. } => *r = Some(role),
            Node::Separator | Node::Standard { .. } => {}
        }
        self
    }
}

impl Menu {
    pub fn new(title: impl Into<String>, children: Vec<Node>) -> Menu {
        Menu { title: title.into(), role: MenuRole::Normal, children }
    }

    pub fn role(mut self, role: MenuRole) -> Menu {
        self.role = role;
        self
    }
}

impl MenuBar {
    /// Every item, depth first.
    pub fn items(&self) -> Vec<&Item> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Item>) {
            for n in nodes {
                match n {
                    Node::Item(it) => out.push(it),
                    Node::Submenu { children, .. } => walk(children, out),
                    Node::Separator | Node::Standard { .. } => {}
                }
            }
        }
        let mut out = Vec::new();
        for m in &self.menus {
            walk(&m.children, &mut out);
        }
        out
    }

    pub fn find(&self, id: &str) -> Option<&Item> {
        self.items().into_iter().find(|it| it.id == id)
    }

    pub fn menu(&self, role: MenuRole) -> Option<&Menu> {
        self.menus.iter().find(|m| m.role == role)
    }

    /// Give every item with this id the role, at any depth.
    pub fn tag_item(&mut self, id: &str, role: ItemRole) {
        fn walk(nodes: &mut [Node], id: &str, role: ItemRole) {
            for n in nodes {
                match n {
                    Node::Item(it) if it.id == id => it.role = Some(role),
                    Node::Submenu { children, .. } => walk(children, id, role),
                    _ => {}
                }
            }
        }
        for m in &mut self.menus {
            walk(&mut m.children, id, role);
        }
    }

    /// Give the submenu `name`, directly in the menu titled `top`, the role.
    pub fn tag_submenu(&mut self, top: &str, name: &str, role: ItemRole) {
        let Some(menu) = self.menus.iter_mut().find(|m| m.title == top) else { return };
        for n in &mut menu.children {
            if let Node::Submenu { label, role: r, .. } = n
                && label == name
            {
                *r = Some(role);
            }
        }
    }

    pub fn tag_menu(&mut self, title: &str, role: MenuRole) {
        if let Some(m) = self.menus.iter_mut().find(|m| m.title == title) {
            m.role = role;
        }
    }
}

/// One row of a flat menu list: the menu path (`["File", "Open Recent"]`) and the item in it.
/// A row labelled `---` is a separator.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub path: Vec<String>,
    pub id: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
}

/// The menus as a tree, from flat rows, in the order an in-window bar draws them: one menu per
/// entry of `tops`, and at each depth leaves and separators in order, with a submenu where its
/// first row is. The result is [`tidy`].
pub fn from_items(tops: &[&str], rows: &[Row]) -> MenuBar {
    fn level(rows: &[&Row], depth: usize) -> Vec<Node> {
        let mut out = Vec::new();
        let mut shown: Vec<&str> = Vec::new();
        for it in rows {
            if it.path.len() == depth {
                if it.label == "---" {
                    out.push(Node::Separator);
                } else {
                    out.push(Node::Item(Item {
                        id: it.id.clone(),
                        label: it.label.clone(),
                        shortcut: it.shortcut.clone(),
                        enabled: it.enabled,
                        checked: it.checked,
                        role: None,
                        layout_shortcut: false,
                    }));
                }
            } else if let Some(name) = it.path.get(depth) {
                let name = name.as_str();
                if shown.contains(&name) {
                    continue;
                }
                shown.push(name);
                let child: Vec<&Row> = rows.iter().copied().filter(|c| c.path.get(depth).map(String::as_str) == Some(name)).collect();
                out.push(Node::submenu(name, level(&child, depth + 1)));
            }
        }
        tidy(out)
    }
    let menus = tops
        .iter()
        .map(|top| {
            let rows: Vec<&Row> = rows.iter().filter(|r| r.path.first().map(String::as_str) == Some(*top)).collect();
            Menu::new(*top, level(&rows, 1))
        })
        .collect();
    MenuBar { menus }
}

/// Drop separators at the start or end of a level and collapse runs of them, at every depth.
pub fn tidy(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let n = match n {
            Node::Submenu { label, children, role } => Node::Submenu { label, children: tidy(children), role },
            other => other,
        };
        if matches!(n, Node::Separator) && matches!(out.last(), None | Some(Node::Separator)) {
            continue;
        }
        out.push(n);
    }
    while matches!(out.last(), Some(Node::Separator)) {
        out.pop();
    }
    out
}

/// A hash of everything a native menu can only change by rebuilding: menus, roles, item ids,
/// kinds (plain or check), shortcuts, submenu labels and the labels of AppKit's items. Item
/// labels, enabled and checked are updated in place. Window's open documents are left out:
/// opening or closing a document rebuilds only the Window menu ([`window_key`]).
pub fn structure_key(bar: &MenuBar) -> u64 {
    let mut h = DefaultHasher::new();
    for m in &bar.menus {
        (&m.title, m.role).hash(&mut h);
        if m.role == MenuRole::Window {
            let mut rest: Vec<Node> = m.children.iter().filter(|n| !is_document(n)).cloned().collect();
            while matches!(rest.last(), Some(Node::Separator)) {
                rest.pop();
            }
            hash_nodes(&rest, &mut h);
        } else {
            hash_nodes(&m.children, &mut h);
        }
    }
    h.finish()
}

/// The Window menu's structure, its open documents included: when it changes and nothing else
/// did, a native menu rebuilds just the Window menu.
pub fn window_key(bar: &MenuBar) -> u64 {
    let mut h = DefaultHasher::new();
    if let Some(m) = bar.menu(MenuRole::Window) {
        hash_nodes(&m.children, &mut h);
    }
    h.finish()
}

pub(crate) fn is_document(n: &Node) -> bool {
    matches!(n, Node::Item(it) if it.role == Some(ItemRole::Document))
}

fn hash_nodes(nodes: &[Node], h: &mut DefaultHasher) {
    for n in nodes {
        match n {
            Node::Item(it) => ('i', &it.id, it.checked.is_some(), &it.shortcut, it.role, it.layout_shortcut).hash(h),
            Node::Separator => '-'.hash(h),
            Node::Submenu { label, children, role } => {
                ('[', label, role).hash(h);
                hash_nodes(children, h);
                ']'.hash(h);
            }
            Node::Standard { kind, label } => ('s', kind, label).hash(h),
        }
    }
}
