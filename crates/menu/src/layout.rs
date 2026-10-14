//! The Mac layout of an app's menus. See the crate docs for what it does.

use std::collections::HashMap;

use crate::chord::Chord;
use crate::model::{HIDE, HIDE_OTHERS, Item, ItemRole, MINIMIZE, Menu, MenuBar, MenuRole, Node, Standard, is_document, tidy};

/// The app's name, its translations of the labels the layout writes itself, and its shortcuts
/// that no menu item shows.
pub struct LayoutOptions<'a> {
    /// Shown in the app menu's About, Hide and Quit items.
    pub app_name: &'a str,
    /// The app's translation of an English label the layout uses: `About {app}`, `Settings…`,
    /// `Settings`, `Language`, `Appearance`, `Hide {app}`, `Hide Others`, `Quit {app}`, `Minimize`,
    /// `Window`, and each [`Standard::english`]. `{app}` is replaced by [`LayoutOptions::app_name`]
    /// afterwards.
    pub label: &'a dyn Fn(&'static str) -> String,
    /// Shortcuts the app binds outside its menus (tool keys, canvas keys), as
    /// `(shortcut, command id)`: the system items give way to them as to menu shortcuts.
    pub extra_shortcuts: &'a [(String, String)],
}

impl<'a> LayoutOptions<'a> {
    /// English labels.
    pub fn new(app_name: &'a str) -> LayoutOptions<'a> {
        LayoutOptions { app_name, label: &english, extra_shortcuts: &[] }
    }

    pub fn extra_shortcuts(mut self, shortcuts: &'a [(String, String)]) -> LayoutOptions<'a> {
        self.extra_shortcuts = shortcuts;
        self
    }

    fn text(&self, english: &'static str) -> String {
        (self.label)(english).replace("{app}", self.app_name)
    }

    fn standard(&self, kind: Standard) -> Node {
        Node::Standard { kind, label: self.text(kind.english()) }
    }
}

fn english(s: &'static str) -> String {
    s.into()
}

/// A system key equivalent the app already uses for one of its commands.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub shortcut: &'static str,
    /// What macOS uses it for.
    pub system: &'static str,
    /// The app's command bound to it.
    pub command: String,
    /// What the layout did about it.
    pub resolution: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub bar: MenuBar,
    pub clashes: Vec<Clash>,
}

/// Turn an app's menus, tagged with roles, into a Mac menu bar. See the crate docs.
///
/// A menu the app tagged [`MenuRole::App`] is merged into the app menu: its tagged items go where
/// their roles put them, the rest after Settings, Language and Appearance.
pub fn mac_layout(bar: &MenuBar, options: &LayoutOptions) -> Layout {
    let mut bar = bar.clone();
    let mut clashes = Vec::new();
    let mut bound: HashMap<String, String> = HashMap::new();
    let menu_shortcuts = bar.items().into_iter().filter_map(|it| Some((it.shortcut.as_deref()?, it.id.as_str())));
    let extra = options.extra_shortcuts.iter().map(|(sc, id)| (sc.as_str(), id.as_str()));
    for (sc, id) in menu_shortcuts.chain(extra) {
        if let Some(c) = Chord::parse(sc) {
            bound.entry(c.portable()).or_insert_with(|| id.to_string());
        }
    }
    let taken = |sc: &str, own: Option<&str>| bound.get(sc).filter(|cmd| Some(cmd.as_str()) != own).cloned();
    let tagged = |role: ItemRole| {
        move |n: &Node| match n {
            Node::Item(it) => it.role == Some(role),
            Node::Submenu { role: r, .. } => *r == Some(role),
            _ => false,
        }
    };

    let about = take(&mut bar, |n| matches!(n, Node::Item(it) if it.role == Some(ItemRole::About)));
    let settings = take(&mut bar, tagged(ItemRole::Settings));
    let language = take(&mut bar, tagged(ItemRole::Language));
    let quit = take(&mut bar, |n| matches!(n, Node::Item(it) if it.role == Some(ItemRole::Quit)));
    let appearance = appearance_node(&mut bar, options);
    let own_app: Vec<Node> = match bar.menus.iter().position(|m| m.role == MenuRole::App) {
        Some(i) => bar.menus.remove(i).children,
        None => Vec::new(),
    };

    let mut menu = Vec::new();
    if let Some(Node::Item(mut it)) = about {
        it.label = options.text("About {app}");
        menu.extend([Node::Item(it), Node::Separator]);
    }
    if let Some(s) = settings {
        let own = node_ids(&s);
        let comma = bound.get("Cmd+,").filter(|cmd| !own.contains(*cmd)).cloned();
        menu.push(settings_node(s, comma, options, &mut clashes));
    }
    if let Some(mut l) = language {
        if let Node::Submenu { label, .. } = &mut l {
            *label = options.text("Language");
        }
        menu.push(l);
    }
    menu.extend(appearance);
    if !own_app.is_empty() {
        menu.push(Node::Separator);
        menu.extend(own_app);
    }
    if !menu.is_empty() && menu.last() != Some(&Node::Separator) {
        menu.push(Node::Separator);
    }
    menu.extend([options.standard(Standard::Services), Node::Separator]);
    let mut hide = Item::new(HIDE, options.text("Hide {app}")).role(ItemRole::Hide);
    hide.layout_shortcut = true;
    match taken("Cmd+H", None) {
        Some(command) => {
            let free = taken("Ctrl+Cmd+H", None).is_none();
            if free {
                hide.shortcut = Some("Ctrl+Cmd+H".into());
            }
            let resolution = if free { "Hide moves to ⌃⌘H" } else { "Hide has no shortcut" };
            clashes.push(Clash { shortcut: "Cmd+H", system: "Hide", command, resolution });
        }
        None => hide.shortcut = Some("Cmd+H".into()),
    }
    menu.push(Node::Item(hide));
    let mut hide_others = Item::new(HIDE_OTHERS, options.text("Hide Others"));
    hide_others.layout_shortcut = true;
    match taken("Cmd+Alt+H", None) {
        Some(command) => clashes.push(Clash { shortcut: "Cmd+Alt+H", system: "Hide Others", command, resolution: "Hide Others has no shortcut" }),
        None => hide_others.shortcut = Some("Cmd+Alt+H".into()),
    }
    menu.extend([Node::Item(hide_others), options.standard(Standard::ShowAll), Node::Separator]);
    let mut quit = match quit {
        Some(Node::Item(it)) => it,
        _ => Item::new(craft_quit::QUIT_MENU_ID, "").role(ItemRole::Quit),
    };
    quit.label = options.text("Quit {app}");
    match taken("Cmd+Q", Some(&quit.id)) {
        Some(command) => {
            clashes.push(Clash { shortcut: "Cmd+Q", system: "Quit", command, resolution: "Quit has no shortcut" });
            quit.shortcut = None;
        }
        None => {
            let own = quit.shortcut.as_deref().and_then(Chord::parse).map(|c| c.portable());
            quit.layout_shortcut = own.as_deref() != Some("Cmd+Q");
            quit.shortcut = Some("Cmd+Q".into());
        }
    }
    menu.push(Node::Item(quit));
    bar.menus.insert(0, Menu::new(options.app_name, menu).role(MenuRole::App));

    match taken("Cmd+W", None) {
        Some(command) => clashes.push(Clash { shortcut: "Cmd+W", system: "Close Window", command, resolution: "no system Close Window item" }),
        None => {
            if let Some(file) = bar.menus.iter_mut().find(|m| m.role == MenuRole::File) {
                file.children.extend([Node::Separator, options.standard(Standard::CloseWindow)]);
            }
        }
    }

    if bar.menu(MenuRole::Window).is_none() {
        let at = bar.menus.iter().position(|m| m.role == MenuRole::Help).unwrap_or(bar.menus.len());
        bar.menus.insert(at, Menu::new(options.text("Window"), Vec::new()).role(MenuRole::Window));
    }
    let minimize = match taken("Cmd+M", None) {
        Some(command) => {
            let mut it = Item::new(MINIMIZE, options.text("Minimize")).role(ItemRole::Minimize);
            it.layout_shortcut = true;
            let free = taken("Ctrl+Cmd+M", None).is_none();
            if free {
                it.shortcut = Some("Ctrl+Cmd+M".into());
            }
            let resolution = if free { "Minimize moves to ⌃⌘M" } else { "Minimize has no shortcut" };
            clashes.push(Clash { shortcut: "Cmd+M", system: "Minimize", command, resolution });
            Node::Item(it)
        }
        None => options.standard(Standard::Minimize),
    };
    if let Some(window) = bar.menus.iter_mut().find(|m| m.role == MenuRole::Window) {
        let (docs, rest): (Vec<Node>, Vec<Node>) = std::mem::take(&mut window.children).into_iter().partition(is_document);
        let mut children = vec![minimize, options.standard(Standard::Zoom), Node::Separator];
        children.extend(rest);
        children.extend([Node::Separator, options.standard(Standard::BringAllToFront), Node::Separator]);
        children.extend(docs);
        window.children = children;
    }

    for m in &mut bar.menus {
        m.children = tidy(std::mem::take(&mut m.children));
    }
    // A menu emptied by the moves goes away (the Help menu stays: it has the search field).
    bar.menus.retain(|m| !m.children.is_empty() || m.role == MenuRole::Help);
    Layout { bar, clashes }
}

/// Settings in the app menu: an item becomes "Settings…", a submenu of settings pages
/// "Settings". ⌘, goes to the item or the first enabled page, unless the app binds ⌘, elsewhere.
fn settings_node(node: Node, comma_taken: Option<String>, options: &LayoutOptions, clashes: &mut Vec<Clash>) -> Node {
    if let Some(command) = &comma_taken {
        clashes.push(Clash { shortcut: "Cmd+,", system: "Settings", command: command.clone(), resolution: "Settings has no shortcut" });
    }
    let give = |it: &mut Item| {
        if comma_taken.is_none() && it.shortcut.is_none() {
            it.shortcut = Some("Cmd+,".into());
            it.layout_shortcut = true;
        }
    };
    match node {
        Node::Item(mut it) => {
            it.label = options.text("Settings…");
            give(&mut it);
            Node::Item(it)
        }
        Node::Submenu { mut children, role, .. } => {
            if let Some(Node::Item(first)) = children.iter_mut().find(|n| matches!(n, Node::Item(i) if i.enabled)) {
                give(first);
            }
            Node::Submenu { label: options.text("Settings"), children, role }
        }
        other => other,
    }
}

/// Appearance in the app menu: the submenu tagged [`ItemRole::Appearance`], renamed, with the
/// items tagged the same (Next Theme) after it. Without such a submenu the tagged items go into
/// the app menu as they are.
fn appearance_node(bar: &mut MenuBar, options: &LayoutOptions) -> Vec<Node> {
    let submenu = take(bar, |n| matches!(n, Node::Submenu { role, .. } if *role == Some(ItemRole::Appearance)));
    let mut items = Vec::new();
    while let Some(it) = take(bar, |n| matches!(n, Node::Item(it) if it.role == Some(ItemRole::Appearance))) {
        items.push(it);
    }
    match submenu {
        Some(Node::Submenu { mut children, role, .. }) => {
            if !items.is_empty() {
                children.push(Node::Separator);
                children.extend(items);
            }
            vec![Node::Submenu { label: options.text("Appearance"), children, role }]
        }
        _ => items,
    }
}

fn node_ids(node: &Node) -> Vec<String> {
    match node {
        Node::Item(it) => vec![it.id.clone()],
        Node::Submenu { children, .. } => children.iter().flat_map(node_ids).collect(),
        _ => Vec::new(),
    }
}

/// Remove and return the first node matching `pred`, at any depth.
fn take(bar: &mut MenuBar, pred: impl Fn(&Node) -> bool + Copy) -> Option<Node> {
    fn walk(nodes: &mut Vec<Node>, pred: impl Fn(&Node) -> bool + Copy) -> Option<Node> {
        if let Some(i) = nodes.iter().position(pred) {
            return Some(nodes.remove(i));
        }
        nodes.iter_mut().find_map(|n| match n {
            Node::Submenu { children, .. } => walk(children, pred),
            _ => None,
        })
    }
    bar.menus.iter_mut().find_map(|m| walk(&mut m.children, pred))
}
