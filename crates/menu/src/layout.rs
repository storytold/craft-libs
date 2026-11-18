//! The platform layouts of an app's menus. See the crate docs for what they do.

use std::collections::HashMap;

use craft_keymap::{Chord, CommandRef, Key, Mods};

use crate::model::{Builtin, Item, ItemRole, Menu, MenuBar, MenuRole, Node, Platforms, Standard, tidy};

const CTRL_CMD: Mods = Mods { ctrl: true, ..Mods::CMD };
const ALT_CMD: Mods = Mods { alt: true, ..Mods::CMD };
const CMD_H: Chord = Chord::new(Mods::CMD, Key::H);
const CTRL_CMD_H: Chord = Chord::new(CTRL_CMD, Key::H);
const ALT_CMD_H: Chord = Chord::new(ALT_CMD, Key::H);
const CMD_Q: Chord = Chord::new(Mods::CMD, Key::Q);
const CMD_W: Chord = Chord::new(Mods::CMD, Key::W);
const CMD_M: Chord = Chord::new(Mods::CMD, Key::M);
const CTRL_CMD_M: Chord = Chord::new(CTRL_CMD, Key::M);
const CMD_COMMA: Chord = Chord::new(Mods::CMD, Key::Comma);

/// The platform a layout is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Mac,
    Windows,
    Linux,
    /// A browser, on any operating system.
    Web,
}

impl Target {
    /// The platform this build runs on: `wasm32` builds are [`Target::Web`], and operating
    /// systems other than macOS and Windows are [`Target::Linux`].
    pub const fn current() -> Target {
        if cfg!(target_arch = "wasm32") {
            Target::Web
        } else if cfg!(target_os = "macos") {
            Target::Mac
        } else if cfg!(target_os = "windows") {
            Target::Windows
        } else {
            Target::Linux
        }
    }

    /// The [`Platforms`] flag an item needs to be shown on this target.
    pub const fn platforms(self) -> Platforms {
        match self {
            Target::Mac => Platforms::MAC,
            Target::Windows => Platforms::WINDOWS,
            Target::Linux => Platforms::LINUX,
            Target::Web => Platforms::WEB,
        }
    }
}

/// The app's name, its translations of the labels the layout writes itself, and its shortcuts
/// that no menu item shows.
pub struct LayoutOptions<'a> {
    /// Shown in the About, Hide and Quit items.
    pub app_name: &'a str,
    /// The app's translation of an English label a layout uses: `About {app}`, `Settings…`,
    /// `Settings`, `Language`, `Appearance`, `Hide {app}`, `Hide Others`, `Quit {app}`, `Minimize`,
    /// `Preferences…`, `Preferences`, `Exit`, `Quit`, the menu titles `File`, `Edit`, `Window` and
    /// `Help`, and each [`Standard::english`]. `{app}` is replaced by [`LayoutOptions::app_name`]
    /// afterwards.
    pub label: &'a dyn Fn(&'static str) -> String,
    /// Shortcuts the app binds outside its menus (tool keys, canvas keys), with their commands:
    /// the system items give way to them as to menu shortcuts.
    pub extra_shortcuts: &'a [(Chord, CommandRef)],
}

impl<'a> LayoutOptions<'a> {
    /// English labels.
    pub fn new(app_name: &'a str) -> LayoutOptions<'a> {
        LayoutOptions { app_name, label: &english, extra_shortcuts: &[] }
    }

    pub fn extra_shortcuts(mut self, shortcuts: &'a [(Chord, CommandRef)]) -> LayoutOptions<'a> {
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

/// A system or browser shortcut the app also uses for one of its commands.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub shortcut: Chord,
    /// What macOS or the browser uses it for.
    pub system: &'static str,
    /// The app's command bound to it.
    pub command: CommandRef,
    /// What the layout did about it.
    pub resolution: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub bar: MenuBar,
    pub clashes: Vec<Clash>,
}

/// Lay an app's menus out for `target`: [`mac_layout`], [`desktop_layout`] or [`web_layout`].
pub fn layout(bar: &MenuBar, target: Target, options: &LayoutOptions) -> Layout {
    match target {
        Target::Mac => mac_layout(bar, options),
        Target::Windows | Target::Linux => desktop_layout(bar, target, options),
        Target::Web => web_layout(bar, options),
    }
}

/// Turn an app's menus, tagged with roles, into a Mac menu bar. See the crate docs.
///
/// A menu the app tagged [`MenuRole::App`] is merged into the app menu: its tagged items go where
/// their roles put them, the rest after Settings, Language and Appearance.
pub fn mac_layout(bar: &MenuBar, options: &LayoutOptions) -> Layout {
    let mut bar = shown_on(bar, Target::Mac);
    let mut clashes = Vec::new();
    let mut bound: HashMap<Chord, CommandRef> = HashMap::new();
    let menu_shortcuts = bar.items().into_iter().filter_map(|it| Some((it.shortcut?, &it.command)));
    let extra = options.extra_shortcuts.iter().map(|(sc, command)| (*sc, command));
    for (sc, command) in menu_shortcuts.chain(extra) {
        bound.entry(sc).or_insert_with(|| command.clone());
    }
    let taken = |sc: Chord, own: Option<&CommandRef>| bound.get(&sc).filter(|cmd| Some(*cmd) != own).cloned();
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
        let own = node_commands(&s);
        let comma = bound.get(&CMD_COMMA).filter(|cmd| !own.contains(*cmd)).cloned();
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
    let mut hide = Item::new(Builtin::Hide.command(), options.text("Hide {app}")).role(ItemRole::Hide);
    hide.layout_shortcut = true;
    match taken(CMD_H, None) {
        Some(command) => {
            let free = taken(CTRL_CMD_H, None).is_none();
            if free {
                hide.shortcut = Some(CTRL_CMD_H);
            }
            let resolution = if free { "Hide moves to ⌃⌘H" } else { "Hide has no shortcut" };
            clashes.push(Clash { shortcut: CMD_H, system: "Hide", command, resolution });
        }
        None => hide.shortcut = Some(CMD_H),
    }
    menu.push(Node::Item(hide));
    let mut hide_others = Item::new(Builtin::HideOthers.command(), options.text("Hide Others"));
    hide_others.layout_shortcut = true;
    match taken(ALT_CMD_H, None) {
        Some(command) => clashes.push(Clash { shortcut: ALT_CMD_H, system: "Hide Others", command, resolution: "Hide Others has no shortcut" }),
        None => hide_others.shortcut = Some(ALT_CMD_H),
    }
    menu.extend([Node::Item(hide_others), options.standard(Standard::ShowAll), Node::Separator]);
    let mut quit = match quit {
        Some(Node::Item(it)) => it,
        _ => Item::new(Builtin::Quit.command(), "").role(ItemRole::Quit),
    };
    quit.label = options.text("Quit {app}");
    match taken(CMD_Q, Some(&quit.command)) {
        Some(command) => {
            clashes.push(Clash { shortcut: CMD_Q, system: "Quit", command, resolution: "Quit has no shortcut" });
            quit.shortcut = None;
        }
        None => {
            quit.layout_shortcut = quit.shortcut != Some(CMD_Q);
            quit.shortcut = Some(CMD_Q);
        }
    }
    menu.push(Node::Item(quit));
    bar.menus.insert(0, Menu::new(options.app_name, menu).role(MenuRole::App));

    match taken(CMD_W, None) {
        Some(command) => clashes.push(Clash { shortcut: CMD_W, system: "Close Window", command, resolution: "no system Close Window item" }),
        None => {
            if let Some(file) = bar.menus.iter_mut().find(|m| m.role == MenuRole::File) {
                file.children.extend([Node::Separator, options.standard(Standard::CloseWindow)]);
            }
        }
    }

    let minimize = match taken(CMD_M, None) {
        Some(command) => {
            let mut it = Item::new(Builtin::Minimize.command(), options.text("Minimize")).role(ItemRole::Minimize);
            it.layout_shortcut = true;
            let free = taken(CTRL_CMD_M, None).is_none();
            if free {
                it.shortcut = Some(CTRL_CMD_M);
            }
            let resolution = if free { "Minimize moves to ⌃⌘M" } else { "Minimize has no shortcut" };
            clashes.push(Clash { shortcut: CMD_M, system: "Minimize", command, resolution });
            Node::Item(it)
        }
        None => options.standard(Standard::Minimize),
    };
    let documents = document_items(&bar);
    let window = ensure_menu(&mut bar, MenuRole::Window, "Window", options);
    let mut children = vec![minimize, options.standard(Standard::Zoom), Node::Separator];
    children.append(&mut window.children);
    children.extend([Node::Separator, options.standard(Standard::BringAllToFront), Node::Separator]);
    children.extend(documents);
    window.children = children;

    // The Help menu stays even when empty: it has the search field.
    finish(&mut bar, |m| m.role == MenuRole::Help);
    Layout { bar, clashes }
}

/// Lay an app's menus out the Windows or Linux way:
/// - About goes last in Help, as `About {app}`;
/// - Settings goes last in Edit, as `Preferences…` (an item) or `Preferences` (a submenu);
/// - Quit goes last in File, as `Exit` on Windows and `Quit` elsewhere: the app's item tagged
///   [`ItemRole::Quit`], or [`Builtin::Quit`];
/// - the open documents go last in Window, as radio items.
///
/// The layout creates the File, Edit, Help and Window menus these need, and adds no Services,
/// Hide or Minimize items. Language and Appearance stay where the app put them, and items keep
/// the app's shortcuts. AppKit's items ([`Node::Standard`]) are dropped.
pub fn desktop_layout(bar: &MenuBar, target: Target, options: &LayoutOptions) -> Layout {
    let quit = if target == Target::Windows { "Exit" } else { "Quit" };
    Layout { bar: desktop_bar(bar, target, Some(quit), options), clashes: Vec::new() }
}

/// Lay an app's menus out for a browser: [`desktop_layout`] without a Quit item. Shortcuts the
/// browser keeps for itself (Ctrl or ⌘ with W, N, T or Q, and with Shift+N, Shift+T or Shift+W)
/// are taken off the items and reported as clashes, since the page never sees them.
pub fn web_layout(bar: &MenuBar, options: &LayoutOptions) -> Layout {
    let mut bar = desktop_bar(bar, Target::Web, None, options);
    let mut clashes = Vec::new();
    bar.for_each_item_mut(|it| {
        if let Some(shortcut) = it.shortcut
            && let Some(system) = browser_reserved(shortcut)
        {
            clashes.push(Clash { shortcut, system, command: it.command.clone(), resolution: "not shown: the browser keeps it" });
            it.shortcut = None;
        }
    });
    Layout { bar, clashes }
}

// Browsers run these themselves and never pass them to the page. craft_keymap::reserved lists
// only the operating systems' chords.
const BROWSER: &[(Key, bool, &str)] = &[
    (Key::W, false, "Close Tab"),
    (Key::N, false, "New Window"),
    (Key::T, false, "New Tab"),
    (Key::Q, false, "Quit"),
    (Key::N, true, "New Private Window"),
    (Key::T, true, "Reopen Closed Tab"),
    (Key::W, true, "Close Window"),
];

/// What the browser uses `chord` for, if it keeps it. Either command modifier counts (Ctrl off
/// macOS, ⌘ on it), since a web build serves both.
fn browser_reserved(chord: Chord) -> Option<&'static str> {
    let m = chord.mods;
    if m.cmd == m.ctrl || m.alt {
        return None;
    }
    BROWSER.iter().find(|(key, shift, _)| *key == chord.key && *shift == m.shift).map(|(_, _, what)| *what)
}

/// [`desktop_layout`], with Quit labelled `quit` or left out.
fn desktop_bar(bar: &MenuBar, target: Target, quit: Option<&'static str>, options: &LayoutOptions) -> MenuBar {
    let mut bar = shown_on(bar, target);
    let item = |role: ItemRole| move |n: &Node| matches!(n, Node::Item(it) if it.role == Some(role));
    let about = take(&mut bar, item(ItemRole::About));
    let settings = take(&mut bar, |n| match n {
        Node::Item(it) => it.role == Some(ItemRole::Settings),
        Node::Submenu { role, .. } => *role == Some(ItemRole::Settings),
        _ => false,
    });
    let own_quit = take(&mut bar, item(ItemRole::Quit));

    if let Some(label) = quit {
        let mut it = match own_quit {
            Some(Node::Item(it)) => it,
            _ => Item::new(Builtin::Quit.command(), "").role(ItemRole::Quit),
        };
        it.label = options.text(label);
        ensure_menu(&mut bar, MenuRole::File, "File", options).children.extend([Node::Separator, Node::Item(it)]);
    }
    if let Some(s) = settings {
        let s = match s {
            Node::Item(mut it) => {
                it.label = options.text("Preferences…");
                Node::Item(it)
            }
            Node::Submenu { children, role, platforms, .. } => Node::Submenu { label: options.text("Preferences"), children, role, platforms },
            other => other,
        };
        ensure_menu(&mut bar, MenuRole::Edit, "Edit", options).children.extend([Node::Separator, s]);
    }
    let documents = document_items(&bar);
    if !documents.is_empty() {
        let window = ensure_menu(&mut bar, MenuRole::Window, "Window", options);
        window.children.push(Node::Separator);
        window.children.extend(documents);
    }
    if let Some(Node::Item(mut it)) = about {
        it.label = options.text("About {app}");
        ensure_menu(&mut bar, MenuRole::Help, "Help", options).children.extend([Node::Separator, Node::Item(it)]);
    }
    finish(&mut bar, |_| false);
    bar
}

/// The app's menus with only what `target` shows: no items or submenus whose [`Platforms`]
/// leave it out, and no AppKit items off macOS. Tidied, so a submenu left empty goes.
fn shown_on(bar: &MenuBar, target: Target) -> MenuBar {
    fn keep(nodes: &[Node], on: Platforms, appkit: bool) -> Vec<Node> {
        let shown = nodes.iter().filter_map(|n| match n {
            Node::Item(it) if !it.platforms.contains(on) => None,
            Node::Submenu { platforms, .. } if !platforms.contains(on) => None,
            Node::Submenu { label, children, role, platforms } => {
                Some(Node::Submenu { label: label.clone(), children: keep(children, on, appkit), role: *role, platforms: *platforms })
            }
            Node::Standard { .. } if !appkit => None,
            other => Some(other.clone()),
        });
        tidy(shown.collect())
    }
    let on = target.platforms();
    let menus = bar.menus.iter().map(|m| Menu { children: keep(&m.children, on, target == Target::Mac), ..m.clone() }).collect();
    MenuBar { menus, documents: bar.documents.clone() }
}

/// The listed documents as radio items, the one in front selected, after a separator.
fn document_items(bar: &MenuBar) -> Vec<Node> {
    let items: Vec<Node> =
        bar.documents.iter().filter(|d| !d.hidden).map(|d| Item::new(d.command.clone(), d.label.clone()).radio(d.active).into()).collect();
    if items.is_empty() {
        return items;
    }
    std::iter::once(Node::Separator).chain(items).collect()
}

/// The menu with `role`, added with the title `english` when the app has none: File first, Edit
/// after File, Help last, any other before Help.
fn ensure_menu<'b>(bar: &'b mut MenuBar, role: MenuRole, english: &'static str, options: &LayoutOptions) -> &'b mut Menu {
    let at = match bar.menus.iter().position(|m| m.role == role) {
        Some(i) => i,
        None => {
            let at = match role {
                MenuRole::File => 0,
                MenuRole::Edit => bar.menus.iter().position(|m| m.role == MenuRole::File).map_or(0, |i| i + 1),
                MenuRole::Help => bar.menus.len(),
                _ => bar.menus.iter().position(|m| m.role == MenuRole::Help).unwrap_or(bar.menus.len()),
            };
            bar.menus.insert(at, Menu::new(options.text(english), Vec::new()).role(role));
            at
        }
    };
    &mut bar.menus[at]
}

/// Tidy every menu and drop the ones the moves emptied, unless `keep_empty`.
fn finish(bar: &mut MenuBar, keep_empty: impl Fn(&Menu) -> bool) {
    for m in &mut bar.menus {
        m.children = tidy(std::mem::take(&mut m.children));
    }
    bar.menus.retain(|m| !m.children.is_empty() || keep_empty(m));
}

/// Settings in the app menu: an item becomes "Settings…", a submenu of settings pages
/// "Settings". ⌘, goes to the item or the first enabled page, unless the app binds ⌘, elsewhere.
fn settings_node(node: Node, comma_taken: Option<CommandRef>, options: &LayoutOptions, clashes: &mut Vec<Clash>) -> Node {
    if let Some(command) = &comma_taken {
        clashes.push(Clash { shortcut: CMD_COMMA, system: "Settings", command: command.clone(), resolution: "Settings has no shortcut" });
    }
    let give = |it: &mut Item| {
        if comma_taken.is_none() && it.shortcut.is_none() {
            it.shortcut = Some(CMD_COMMA);
            it.layout_shortcut = true;
        }
    };
    match node {
        Node::Item(mut it) => {
            it.label = options.text("Settings…");
            give(&mut it);
            Node::Item(it)
        }
        Node::Submenu { mut children, role, platforms, .. } => {
            if let Some(Node::Item(first)) = children.iter_mut().find(|n| matches!(n, Node::Item(i) if i.enabled)) {
                give(first);
            }
            Node::Submenu { label: options.text("Settings"), children, role, platforms }
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
        Some(Node::Submenu { mut children, role, platforms, .. }) => {
            if !items.is_empty() {
                children.push(Node::Separator);
                children.extend(items);
            }
            vec![Node::Submenu { label: options.text("Appearance"), children, role, platforms }]
        }
        _ => items,
    }
}

fn node_commands(node: &Node) -> Vec<CommandRef> {
    match node {
        Node::Item(it) => vec![it.command.clone()],
        Node::Submenu { children, .. } => children.iter().flat_map(node_commands).collect(),
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
