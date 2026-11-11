//! The menu tree, the flat rows it can be built from, and the keys that tell a backend what
//! changed.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::BitOr;

use craft_keymap::{Chord, CommandRef};
use craft_theme::Srgba;

/// The app's menus, in order, and its open documents.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct MenuBar {
    /// The top-level menus, left to right.
    pub menus: Vec<Menu>,
    /// The app's open documents, for the Window menu.
    pub documents: Vec<Document>,
}

/// An open document, listed in the Window menu.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Document {
    /// What choosing it runs (bring the document to the front).
    pub command: CommandRef,
    /// The document's name.
    pub label: String,
    /// The document in front.
    pub active: bool,
    /// Open but not listed: a document the app keeps open in the background.
    pub hidden: bool,
}

impl Document {
    /// A listed document, not the one in front.
    pub fn new(command: impl Into<CommandRef>, label: impl Into<String>) -> Document {
        Document { command: command.into(), label: label.into(), active: false, hidden: false }
    }

    /// Keep the document open but not listed.
    pub fn hidden(mut self, hidden: bool) -> Document {
        self.hidden = hidden;
        self
    }

    /// Mark the document as the one in front.
    pub fn active(mut self, active: bool) -> Document {
        self.active = active;
        self
    }
}

/// A top-level menu.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Menu {
    /// The title in the bar.
    pub title: String,
    /// What the menu is for.
    pub role: MenuRole,
    /// The menu's entries, top to bottom.
    pub children: Vec<Node>,
}

/// What a top-level menu is for, set by the app. macOS treats the app, Window and Help menus
/// specially; File is where the system Close Window item goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MenuRole {
    /// Any menu without a fixed place.
    #[default]
    Other,
    #[allow(missing_docs)]
    App,
    #[allow(missing_docs)]
    File,
    #[allow(missing_docs)]
    Edit,
    #[allow(missing_docs)]
    View,
    #[allow(missing_docs)]
    Window,
    #[allow(missing_docs)]
    Help,
}

/// The platforms an item or submenu is shown on. All of them by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Platforms(u8);

impl Platforms {
    /// No platform.
    pub const NONE: Platforms = Platforms(0);
    /// macOS.
    pub const MAC: Platforms = Platforms(1);
    /// Windows.
    pub const WINDOWS: Platforms = Platforms(1 << 1);
    /// Linux.
    pub const LINUX: Platforms = Platforms(1 << 2);
    /// A browser, on any operating system.
    pub const WEB: Platforms = Platforms(1 << 3);
    /// Every platform.
    pub const ALL: Platforms = Platforms(0b1111);

    /// Whether every platform in `other` is in `self`.
    pub const fn contains(self, other: Platforms) -> bool {
        self.0 & other.0 == other.0
    }

    /// `self` without the platforms in `other`: `Platforms::ALL.without(Platforms::WEB)`.
    pub const fn without(self, other: Platforms) -> Platforms {
        Platforms(self.0 & !other.0)
    }

    /// Whether no platform is in the set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl Default for Platforms {
    fn default() -> Platforms {
        Platforms::ALL
    }
}

impl BitOr for Platforms {
    type Output = Platforms;
    fn bitor(self, other: Platforms) -> Platforms {
        Platforms(self.0 | other.0)
    }
}

/// One entry of a menu.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Node {
    /// An item the user can choose.
    Item(Item),
    /// A dividing line.
    Separator,
    /// A nested menu.
    #[non_exhaustive]
    Submenu {
        /// The submenu's title.
        label: String,
        /// The submenu's entries.
        children: Vec<Node>,
        /// The place a layout gives it, if any.
        role: Option<ItemRole>,
        /// The platforms it is shown on.
        platforms: Platforms,
    },
    /// An item AppKit provides and runs itself (Services, Zoom, …), with the label to show.
    #[non_exhaustive]
    Standard {
        /// Which AppKit item.
        kind: Standard,
        /// The label to show.
        label: String,
    },
}

/// One menu item: a command with its label, shortcut and state.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Item {
    /// What the item runs: the app's command (`file.open`), with its parameters
    /// (`CommandRef::with_params("file.open_recent", json!({"index": 3}))`), or a [`Builtin`]. A
    /// click reports it.
    pub command: CommandRef,
    /// The text shown.
    pub label: String,
    /// `cmd` is ⌘ and `ctrl` the Control key: ⌃⌘M and ⌘M are different shortcuts.
    pub shortcut: Option<Chord>,
    /// Whether the item can be chosen.
    pub enabled: bool,
    /// Plain, check or radio.
    pub kind: ItemKind,
    /// The fixed place a layout gives the item, if any.
    pub role: Option<ItemRole>,
    /// The platforms the item is shown on.
    pub platforms: Platforms,
    /// The layout gave this item its shortcut (⌘Q on the app's Quit, ⌘, on Settings): the app's
    /// key handling doesn't know it, so a backend reports the key equivalent as a click.
    pub layout_shortcut: bool,
    /// A label colour for the row (document data, such as a user's menu colours). The in-window
    /// bar shades the row's background with it; the native macOS menu has no row colour and
    /// ignores it.
    pub tint: Option<Srgba>,
    /// A hover tooltip. The in-window bar shows it after a delay and reports it as the row's
    /// accessible description; the native macOS menu ignores it (muda has no handle on the
    /// `NSMenuItem`'s tool tip).
    pub tooltip: Option<String>,
}

/// A plain item, or one with a checked state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ItemKind {
    /// An ordinary item.
    #[default]
    Plain,
    /// A check item, and whether it is checked.
    Check(bool),
    /// One choice of a group: the radio items next to each other between separators. Whether it
    /// is the selected one. macOS shows it as a check item.
    Radio(bool),
}

impl ItemKind {
    /// The checked or selected state, `None` for a plain item.
    pub fn checked(self) -> Option<bool> {
        match self {
            ItemKind::Plain => None,
            ItemKind::Check(on) | ItemKind::Radio(on) => Some(on),
        }
    }
}

/// Items the layouts put in fixed places. The app tags its own About, Settings, Quit, Appearance
/// and Language items; the Mac layout adds Hide and Minimize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ItemRole {
    /// The About item.
    About,
    /// The app's settings: an item, or a submenu of settings pages.
    Settings,
    /// The app's own quit command, which asks about unsaved changes. Without one the layout adds
    /// [`Builtin::Quit`].
    Quit,
    /// Hides the app (macOS).
    Hide,
    /// Minimizes the window (macOS).
    Minimize,
    /// A submenu of themes or appearance modes, and items that go with it (Next Theme).
    Appearance,
    /// The app's language chooser, a submenu.
    Language,
}

/// The items AppKit provides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Standard {
    #[allow(missing_docs)]
    Services,
    #[allow(missing_docs)]
    ShowAll,
    #[allow(missing_docs)]
    Minimize,
    #[allow(missing_docs)]
    Zoom,
    #[allow(missing_docs)]
    BringAllToFront,
    #[allow(missing_docs)]
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

/// `label` unless the app supplied no translation (the label is still the English name), then the
/// system's title for `english`.
pub(crate) fn label_or_system(label: &str, english: &str, system: impl FnOnce(&str) -> String) -> String {
    if label == english { system(english) } else { label.to_owned() }
}

/// The commands of the items the crate adds itself. The crate runs them; an app sees them only
/// in a layout's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Builtin {
    /// Hide the app: a custom item, because AppKit's own always takes ⌘H, which apps often bind.
    Hide,
    /// Hide the other apps: a custom item, because AppKit's own always takes ⌥⌘H.
    HideOthers,
    /// Minimize: a custom item, used when the app binds ⌘M (AppKit's own always takes ⌘M).
    /// [`crate::NativeMenu::take_clicks`] runs it.
    Minimize,
    /// Quit for an app without its own quit item: asks the window to close, through
    /// [`craft_quit::on_menu_event`], so the app's unsaved-changes check runs.
    Quit,
}

impl Builtin {
    /// The command id.
    pub const fn id(self) -> &'static str {
        match self {
            Builtin::Hide => "craft-menu.hide",
            Builtin::HideOthers => "craft-menu.hide-others",
            Builtin::Minimize => "craft-menu.minimize",
            Builtin::Quit => craft_quit::QUIT_MENU_ID,
        }
    }

    /// The command an item with this built-in runs.
    pub fn command(self) -> CommandRef {
        CommandRef::new(self.id())
    }

    /// The built-in command `command` is, if any.
    pub fn of(command: &CommandRef) -> Option<Builtin> {
        if !command.params.is_null() {
            return None;
        }
        [Builtin::Hide, Builtin::HideOthers, Builtin::Minimize, Builtin::Quit].into_iter().find(|b| b.id() == command.id)
    }
}

impl Item {
    /// An enabled plain item with no shortcut, shown on every platform.
    pub fn new(command: impl Into<CommandRef>, label: impl Into<String>) -> Item {
        Item {
            command: command.into(),
            label: label.into(),
            shortcut: None,
            enabled: true,
            kind: ItemKind::Plain,
            role: None,
            platforms: Platforms::ALL,
            layout_shortcut: false,
            tint: None,
            tooltip: None,
        }
    }

    /// Set the shortcut.
    pub fn shortcut(mut self, shortcut: Chord) -> Item {
        self.shortcut = Some(shortcut);
        self
    }

    /// Set whether the item can be chosen.
    pub fn enabled(mut self, enabled: bool) -> Item {
        self.enabled = enabled;
        self
    }

    /// Make this a check item.
    pub fn check(mut self, checked: bool) -> Item {
        self.kind = ItemKind::Check(checked);
        self
    }

    /// Make this a radio item, `selected` or not.
    pub fn radio(mut self, selected: bool) -> Item {
        self.kind = ItemKind::Radio(selected);
        self
    }

    /// Give the item a role, which fixes its place in a layout.
    pub fn role(mut self, role: ItemRole) -> Item {
        self.role = Some(role);
        self
    }

    /// Show the item only on `platforms`.
    pub fn platforms(mut self, platforms: Platforms) -> Item {
        self.platforms = platforms;
        self
    }

    /// Colour the row's background in the in-window bar.
    pub fn tint(mut self, color: impl Into<Srgba>) -> Item {
        self.tint = Some(color.into());
        self
    }

    /// Show `text` when the pointer rests on the row in the in-window bar.
    pub fn tooltip(mut self, text: impl Into<String>) -> Item {
        self.tooltip = Some(text.into());
        self
    }
}

impl From<Item> for Node {
    fn from(item: Item) -> Node {
        Node::Item(item)
    }
}

impl Node {
    /// An item AppKit provides and runs itself, with the label to show.
    pub fn standard(kind: Standard, label: impl Into<String>) -> Node {
        Node::Standard { kind, label: label.into() }
    }

    /// A submenu shown on every platform.
    pub fn submenu(label: impl Into<String>, children: Vec<Node>) -> Node {
        Node::Submenu { label: label.into(), children, role: None, platforms: Platforms::ALL }
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

    /// Show an item or a submenu only on `platforms`.
    pub fn platforms(mut self, platforms: Platforms) -> Node {
        match &mut self {
            Node::Item(it) => it.platforms = platforms,
            Node::Submenu { platforms: p, .. } => *p = platforms,
            Node::Separator | Node::Standard { .. } => {}
        }
        self
    }
}

impl Menu {
    /// A menu with no special role.
    pub fn new(title: impl Into<String>, children: Vec<Node>) -> Menu {
        Menu { title: title.into(), role: MenuRole::Other, children }
    }

    /// Set the menu's role.
    pub fn role(mut self, role: MenuRole) -> Menu {
        self.role = role;
        self
    }
}

impl MenuBar {
    /// A bar of `menus` with no open documents.
    pub fn new(menus: Vec<Menu>) -> MenuBar {
        MenuBar { menus, documents: Vec::new() }
    }

    /// Set the open documents, for the Window menu.
    pub fn documents(mut self, documents: Vec<Document>) -> MenuBar {
        self.documents = documents;
        self
    }

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

    /// Every item, depth first, changed in place.
    pub(crate) fn for_each_item_mut(&mut self, mut f: impl FnMut(&mut Item)) {
        fn walk(nodes: &mut [Node], f: &mut dyn FnMut(&mut Item)) {
            for n in nodes {
                match n {
                    Node::Item(it) => f(it),
                    Node::Submenu { children, .. } => walk(children, f),
                    Node::Separator | Node::Standard { .. } => {}
                }
            }
        }
        for m in &mut self.menus {
            walk(&mut m.children, &mut f);
        }
    }

    /// Whether `command` opens one of [`MenuBar::documents`].
    pub(crate) fn is_document(&self, command: &CommandRef) -> bool {
        self.documents.iter().any(|d| d.command == *command)
    }

    /// The first item whose command is `command`, at any depth.
    pub fn find(&self, command: &CommandRef) -> Option<&Item> {
        self.items().into_iter().find(|it| it.command == *command)
    }

    /// The first menu with `role`.
    pub fn menu(&self, role: MenuRole) -> Option<&Menu> {
        self.menus.iter().find(|m| m.role == role)
    }

    /// Give every item with this command the role, at any depth.
    pub fn tag_item(&mut self, command: &CommandRef, role: ItemRole) {
        fn walk(nodes: &mut [Node], command: &CommandRef, role: ItemRole) {
            for n in nodes {
                match n {
                    Node::Item(it) if it.command == *command => it.role = Some(role),
                    Node::Submenu { children, .. } => walk(children, command, role),
                    _ => {}
                }
            }
        }
        for m in &mut self.menus {
            walk(&mut m.children, command, role);
        }
    }

    /// Give the submenu `name`, directly in the menu with the role `menu`, the role.
    pub fn tag_submenu(&mut self, menu: MenuRole, name: &str, role: ItemRole) {
        let Some(menu) = self.menus.iter_mut().find(|m| m.role == menu) else { return };
        for n in &mut menu.children {
            if let Node::Submenu { label, role: r, .. } = n
                && label == name
            {
                *r = Some(role);
            }
        }
    }

    /// Give the menu titled `title` the role: a shortcut for apps that build the bar from rows.
    pub fn tag_menu(&mut self, title: &str, role: MenuRole) {
        if let Some(m) = self.menus.iter_mut().find(|m| m.title == title) {
            m.role = role;
        }
    }
}

/// One row of a flat menu list: the menu path (`["File", "Open Recent"]`) and the item in it.
/// A row labelled `---` is a separator.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Row {
    /// The menu path to the item's menu; `["File", "Open Recent"]`.
    pub path: Vec<String>,
    /// What the row runs.
    pub command: CommandRef,
    /// The text shown.
    pub label: String,
    /// The row's shortcut.
    pub shortcut: Option<Chord>,
    /// Whether the row can be chosen.
    pub enabled: bool,
    /// Plain, check or radio.
    pub kind: ItemKind,
}

impl Row {
    /// An enabled plain row for `command` labelled `label` in the menu at `path`.
    pub fn new(path: &[&str], command: impl Into<CommandRef>, label: impl Into<String>) -> Row {
        Row {
            path: path.iter().map(|s| (*s).to_owned()).collect(),
            command: command.into(),
            label: label.into(),
            shortcut: None,
            enabled: true,
            kind: ItemKind::Plain,
        }
    }

    /// A separator in the menu at `path`.
    pub fn separator(path: &[&str]) -> Row {
        Row::new(path, "", "---")
    }

    /// The row's shortcut.
    pub fn shortcut(mut self, shortcut: Chord) -> Row {
        self.shortcut = Some(shortcut);
        self
    }
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
                        shortcut: it.shortcut,
                        enabled: it.enabled,
                        kind: it.kind,
                        ..Item::new(it.command.clone(), it.label.clone())
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
    MenuBar { menus, documents: Vec::new() }
}

/// Drop separators at the start or end of a level and collapse runs of them, and drop empty
/// submenus, at every depth. A submenu holding only a disabled placeholder (`No Recent Files`)
/// stays.
pub fn tidy(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let n = match n {
            Node::Submenu { label, children, role, platforms } => {
                let children = tidy(children);
                if children.is_empty() {
                    continue;
                }
                Node::Submenu { label, children, role, platforms }
            }
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

/// A hash of everything a native menu can only change by rebuilding: menus, roles, item
/// commands, kinds (plain, check or radio), shortcuts, submenu labels and the labels of AppKit's items. Item
/// labels, enabled and checked are updated in place. Window's open documents (the items running a
/// [`MenuBar::documents`] command) are left out: opening or closing a document rebuilds only the
/// Window menu ([`window_key`]).
pub fn structure_key(bar: &MenuBar) -> u64 {
    let mut h = DefaultHasher::new();
    for m in &bar.menus {
        (&m.title, m.role).hash(&mut h);
        if m.role == MenuRole::Window {
            let document = |n: &Node| matches!(n, Node::Item(it) if bar.is_document(&it.command));
            let mut rest: Vec<Node> = m.children.iter().filter(|n| !document(n)).cloned().collect();
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

fn hash_nodes(nodes: &[Node], h: &mut DefaultHasher) {
    for n in nodes {
        match n {
            Node::Item(it) => ('i', it.command.key(), std::mem::discriminant(&it.kind), &it.shortcut, it.role, it.layout_shortcut).hash(h),
            Node::Separator => '-'.hash(h),
            Node::Submenu { label, children, role, .. } => {
                ('[', label, role).hash(h);
                hash_nodes(children, h);
                ']'.hash(h);
            }
            Node::Standard { kind, label } => ('s', kind, label).hash(h),
        }
    }
}

/// The native id of an item with `command`: its [`CommandRef::key`], or `key#2`, `key#3`, …
/// while `taken`, since native ids must be unique and a command can be shown twice (Keyboard
/// Shortcuts in Edit and Window).
pub(crate) fn native_id(command: &CommandRef, taken: impl Fn(&str) -> bool) -> String {
    let key = command.key();
    let mut unique = key.clone();
    let mut n = 2;
    while taken(&unique) {
        unique = format!("{key}#{n}");
        n += 1;
    }
    unique
}
