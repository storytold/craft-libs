//! `PlacesTree`: the folder tree with lazy children, reveal, the keep, remove and reveal menu.
//!
//! ```ignore
//! let out = PlacesTree::new("Places", &places, &mut folder_cache)
//!     .current(browsing.as_path())
//!     .row_menu(|_path| vec![Entry::new(String::from("folder.rename"), "Rename Folder…")])
//!     .show(ui);
//! for event in out.events {
//!     match event {
//!         PlacesEvent::Open(path) => app.browse(path),
//!         PlacesEvent::Reveal(path) => app.reveal(&path),
//!         PlacesEvent::Menu(target, command) => app.run_folder_command(&command, target),
//!         other => settings.places.apply(&other),            // keep, remove, restore, recents
//!     }
//! }
//! ```

#[cfg(feature = "screen-testing")]
mod stories;

use std::path::{Path, PathBuf};

use craft_behavior::{DisplayOrder, IconId, ItemSource, Selection};
use craft_host::target::{Os, Runtime};
use craft_i18n::Format;
use craft_ui::contract::{ActivateOn, Bind, CollectionEvent, Entry, NoValue, Outcome, Pattern, Props, Remember as _, SelectMode, Show as _};
use craft_ui::contract::{ShowCollection as _, ShowContainer as _};
use craft_ui::display::Divider;
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow, OpenSet, RowInk};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::TypeRole;
use egui::{Response, Ui};

use crate::places::{Folder, FolderCache, Listing, PlaceGroupKind, PlaceKind, Places, PlacesSettings, StandardFolder, folder_within, same_folder};

/// What the tree reported, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PlacesEvent {
    /// A click or Enter on a row: browse `path`.
    Open(PathBuf),
    /// Keep in Places / Add to Favorites on a folder that is not a root.
    Keep(PathBuf),
    /// Remove from Places on a root (the folder stays).
    Remove(PathBuf),
    /// One hidden root restored.
    Restore(PathBuf),
    /// Restore All.
    RestoreAll,
    /// Clear Recent Folders.
    ClearRecent,
    /// Show the folder in the system's file manager: the app runs it.
    Reveal(PathBuf),
    /// A pick in the host's folder menu ([`PlacesTree::row_menu`]): its command on the folder `target`.
    /// The folder the menu was opened on, then the id of the picked command.
    Menu(PathBuf, String),
}

impl PlacesSettings {
    /// Applies `Keep`, `Remove`, `Restore`, `RestoreAll` and `ClearRecent`; ignores the rest.
    /// Returns whether the settings changed (the host saves them).
    pub fn apply(&mut self, event: &PlacesEvent) -> bool {
        match event {
            PlacesEvent::Keep(path) => self.keep(path),
            PlacesEvent::Remove(path) => self.remove(path),
            PlacesEvent::Restore(path) => self.restore(path),
            PlacesEvent::RestoreAll => self.restore_all(),
            PlacesEvent::ClearRecent => self.clear_recent(),
            PlacesEvent::Open(_) | PlacesEvent::Reveal(_) | PlacesEvent::Menu(..) => false,
        }
    }
}

/// What [`PlacesTree::show`] returns.
#[derive(Debug)]
#[must_use = "the host applies the tree's events"]
#[non_exhaustive]
pub struct PlacesOutput {
    /// The tree.
    pub response: Response,
    /// What happened this pass, in order.
    pub events: Vec<PlacesEvent>,
}

impl Outcome for PlacesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

craft_ui::words! {
    /// The tree's words, English by default.
    #[non_exhaustive]
    pub struct PlacesWords {
        favorites: "placesTree.favorites" = "Favorites";
        places: "placesTree.places" = "Places";
        volumes: "placesTree.volumes" = "Volumes";
        network: "placesTree.network" = "Network";
        recent: "placesTree.recent" = "Recent Folders";
        none: "placesTree.none" = "(none)";
        home: "placesTree.home" = "Home";
        desktop: "placesTree.desktop" = "Desktop";
        documents: "placesTree.documents" = "Documents";
        downloads: "placesTree.downloads" = "Downloads";
        pictures: "placesTree.pictures" = "Pictures";
        movies: "placesTree.movies" = "Movies";
        music: "placesTree.music" = "Music";
        keep: "placesTree.keep" = "Keep in Places";
        add_favorite: "placesTree.addFavorite" = "Add to Favorites";
        remove: "placesTree.remove" = "Remove from Places";
        /// macOS; Windows and Linux use `reveal_other`.
        reveal_mac: "placesTree.revealMac" = "Show in Finder";
        reveal_other: "placesTree.revealOther" = "Show in File Manager";
        clear_recent: "placesTree.clearRecent" = "Clear Recent Folders";
        restore_all: "placesTree.restoreAll" = "Restore All";
        loading: "placesTree.loading" = "Loading…";
        /// The hidden-places row.
        hidden: count "placesTree.hidden" = "Show {n} hidden place" | "Show {n} hidden places";
        /// The name of the row menu.
        menu: "placesTree.menu" = "Folder";
    }
}

type RowMenuFn<'a> = Box<dyn Fn(&Path) -> Vec<Entry<String>> + 'a>;

/// The host's entries for a folder row's menu.
pub struct RowMenu<'a>(RowMenuFn<'a>);

impl<'a> RowMenu<'a> {
    /// The menu that asks `menu` for the entries of a folder.
    pub fn new(menu: impl Fn(&Path) -> Vec<Entry<String>> + 'a) -> Self {
        RowMenu(Box::new(menu))
    }

    fn entries(&self, folder: &Path) -> Vec<Entry<String>> {
        (self.0)(folder)
    }
}

/// The folder tree: one `List` tree of group headings, root rows, folder rows at their depth, one loading row while a
/// listing is pending, the "(none)" row of an empty group and the hidden-places row.
///
/// - **Rows.** A group's heading is a heading row; an empty group shows `none`. Root rows show the place's name (the
///   word of a standard folder), folder rows their name; the selected row is the `current` folder's.
/// - **Children.** A row's children come from `FolderCache::subfolders` when it opens, to 8 levels. `Loading` shows
///   one `loading` row; `Failed` shows nothing and the disclosure goes. The open state is the tree's own and lives in
///   its container's slot. When the window regains focus the tree calls `FolderCache::refresh`; opening a row calls
///   `refresh_folder`. At the end of the pass `FolderCache::finish_pass`.
/// - **Click.** A click or Enter on a folder reports `Open(path)`; the tree keys of the collection contract open and
///   close rows.
/// - **Reveal.** When `current` changes, the rows on the way down to it in the owner root (`Places::owner`, folders
///   compared by [`same_folder`] and [`folder_within`]) open once, as their listings arrive, and its row scrolls
///   into view; collapsing afterwards sticks. A copy of one folder in two roots opens independently.
/// - **Menu** (secondary click, Menu key, Shift+F10): on a folder that is not a root `keep` (or `add_favorite` when
///   the host shows a Favorites group); on a root `remove`; the reveal item (desktop only), reported as
///   [`PlacesEvent::Reveal`]; `clear_recent` in the Recent group; then the host's [`row_menu`](PlacesTree::row_menu)
///   entries. On the hidden-places row the menu lists the hidden roots and `restore_all`; a click on it with one
///   hidden root restores it.
#[must_use = "show the tree with `show`"]
#[derive(Pattern, Props)]
pub struct PlacesTree<'a> {
    label: Label,
    places: &'a Places,
    cache: &'a mut FolderCache,
    /// The folder being browsed: its row is selected and revealed when it changes.
    #[prop]
    current: Option<&'a Path>,
    /// The host's entries after the library's in a folder row's menu.
    #[prop(func = "Fn(&Path) -> Vec<Entry<String>>")]
    row_menu: Option<RowMenu<'a>>,
}

impl<'a> PlacesTree<'a> {
    /// The tree of `places` named `label`, listing children through `cache`.
    pub fn new(label: impl Into<Label>, places: &'a Places, cache: &'a mut FolderCache) -> Self {
        PlacesTree { label: label.into(), places, cache, current: None, row_menu: None }
    }
}

/// Rows below this depth list no children.
const MAX_DEPTH: usize = 8;

/// A folder row's place in the tree: the group and root it hangs from, and its path. A folder shown under two roots
/// has two rows, each with its own open state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct At {
    group: usize,
    root: PathBuf,
    path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum RowKey {
    Heading(usize),
    Empty(usize),
    Folder(At),
    /// The placeholder under the folder `At` while its listing runs.
    Loading(At),
    Hidden,
}

enum Content {
    Heading(Label),
    Empty(Label),
    Loading(Label),
    Folder(Label),
    Hidden(Label),
}

struct Row {
    key: RowKey,
    depth: usize,
    has_children: bool,
    open: bool,
    content: Content,
}

/// The rows as shown, in display order.
struct Shown {
    rows: Vec<Row>,
}

impl Shown {
    fn has_loading(&self) -> bool {
        self.rows.iter().any(|row| matches!(row.key, RowKey::Loading(_)))
    }

    fn contains(&self, key: &RowKey) -> bool {
        self.rows.iter().any(|row| &row.key == key)
    }

    fn at(&self, key: &RowKey) -> Option<&Row> {
        self.rows.iter().find(|row| &row.key == key)
    }
}

impl DisplayOrder<RowKey> for Shown {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> RowKey {
        self.rows[at].key.clone()
    }

    fn depth(&self, at: usize) -> usize {
        self.rows[at].depth
    }

    fn has_children(&self, at: usize) -> bool {
        self.rows[at].has_children
    }

    fn open(&self, at: usize) -> bool {
        self.rows[at].open
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.rows[at].content, Content::Heading(_))
    }

    fn enabled(&self, at: usize) -> bool {
        !matches!(self.rows[at].content, Content::Empty(_) | Content::Loading(_))
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        match &self.rows[at].content {
            Content::Folder(name) | Content::Hidden(name) => Some(std::borrow::Cow::Borrowed(name.as_str())),
            Content::Heading(_) | Content::Empty(_) | Content::Loading(_) => None,
        }
    }
}

impl ItemSource<RowKey> for Shown {}

/// What a context menu was asked for.
#[derive(Clone, Debug)]
enum Target {
    Folder { path: PathBuf, root: Option<PlaceKind> },
    Hidden,
}

#[derive(Clone, Debug)]
enum Command {
    Keep(PathBuf),
    Remove(PathBuf),
    Reveal(PathBuf),
    ClearRecent,
    Host { path: PathBuf, command: String },
    Restore(PathBuf),
    RestoreAll,
}

/// What the tree keeps between passes, in its container's slot.
#[derive(Clone, Default)]
struct Memory {
    /// The folders shown open.
    open: OpenSet<RowKey>,
    /// The `current` of the last pass, to see it change.
    current: Option<PathBuf>,
    /// The folder whose way down is still being opened: its listings have not all arrived.
    walk: Option<PathBuf>,
    /// The row to scroll to once its parents have listed.
    pending: Option<RowKey>,
    /// The tree's selection: the lead is where the keys act. It is set from `current` when `current` changes, and
    /// the list moves it after that.
    selection: Selection<RowKey>,
    /// `current` changed and its row is not shown yet, so `selection` is not set from it yet.
    sync: bool,
    /// Whether the window was active in the last pass.
    active: Option<bool>,
    /// The target of the open context menu.
    menu: Option<Target>,
}

/// What the tree reads from the environment, once per pass.
struct Surroundings<'a> {
    window_active: bool,
    desktop: bool,
    mac: bool,
    format: &'a Format,
}

impl PlacesTree<'_> {
    /// Shows the tree and reports what happened.
    pub fn show(self, ui: &mut Ui) -> PlacesOutput {
        let PlacesTree { label, places, cache, current, row_menu } = self;
        let env = Env::of(ui);
        let words = env.words::<PlacesWords>();
        let around = {
            let view = env.view();
            let shell = view.shell();
            Surroundings {
                window_active: shell.window.active,
                desktop: shell.runtime == Runtime::Native,
                mac: shell.os == Os::Mac,
                format: view.format(),
            }
        };
        let favorites_group = places.groups.iter().any(|group| group.kind == PlaceGroupKind::Favorites);
        let mut events: Vec<PlacesEvent> = Vec::new();

        let mut shown = Stack::new().fill().show(ui, |ui, cx| {
            let mut memory = std::mem::take(cx.keep::<Memory>());

            if memory.active == Some(false) && around.window_active {
                cache.refresh();
            }
            memory.active = Some(around.window_active);

            if memory.current.as_deref() != current {
                memory.current = current.map(Path::to_path_buf);
                memory.walk = memory.current.clone();
                memory.sync = true;
            }
            if let Some(target) = memory.walk.take() {
                match reveal(places, cache, &target, &mut memory.open) {
                    Reveal::Reached(key) => memory.pending = Some(key),
                    Reveal::Waiting => memory.walk = Some(target),
                    Reveal::Unreachable => memory.pending = None,
                }
            }

            let rows = rows(places, cache, &memory.open, &words, around.format);
            if memory.pending.as_ref().is_some_and(|key| !rows.contains(key)) && !rows.has_loading() {
                memory.pending = None;
            }
            if memory.sync {
                if let Some(key) = current.and_then(|current| selected_key(places, &rows, current)) {
                    memory.selection = Selection::single(key);
                    memory.sync = false;
                } else if memory.walk.is_none() && !rows.has_loading() {
                    memory.selection = Selection::default();
                    memory.sync = false;
                }
            }
            let reveal_key = memory.pending.take_if(|key| rows.contains(key));

            let mut list = List::<RowKey>::new(label.clone())
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .tree(true)
                .selection(Bind::bound(&mut memory.selection));
            if let Some(key) = reveal_key {
                list = list.reveal(key);
            }
            let out = list.show_items(ui, &rows, &mut |key, _| {
                let Some(row) = rows.at(key) else { return ListRow::spacer() };
                match &row.content {
                    Content::Heading(text) => ListRow::new(text.clone()).heading().role(TypeRole::Caption).ink(RowInk::Dimmed),
                    Content::Empty(text) | Content::Loading(text) => ListRow::new(text.clone()).ink(RowInk::Dimmed).depth(row.depth),
                    Content::Folder(name) => {
                        let line = ListRow::new(name.clone()).depth(row.depth).icon(IconSpec::svg(IconId::new("folder"))).ink(RowInk::Muted);
                        if row.has_children { line.group(row.open) } else { line }
                    }
                    Content::Hidden(text) => ListRow::new(text.clone()),
                }
            });

            for event in out.events {
                match event {
                    CollectionEvent::Activate(RowKey::Folder(at)) => events.push(PlacesEvent::Open(at.path)),
                    CollectionEvent::Activate(RowKey::Hidden) => {
                        if let [only] = places.hidden.as_slice() {
                            events.push(PlacesEvent::Restore(only.path.clone()));
                        }
                    }
                    CollectionEvent::Open(change) => apply_open(&mut memory.open, cache, &rows, &change.key, change.open, change.all),
                    CollectionEvent::ContextMenu(request) => match request.key {
                        RowKey::Folder(at) => {
                            let root = places.groups.get(at.group).and_then(|group| group.places.iter().find(|place| place.path == at.root));
                            let root = root.filter(|_| at.path == at.root).map(|place| place.kind);
                            memory.menu = Some(Target::Folder { path: at.path, root });
                        }
                        RowKey::Hidden => memory.menu = Some(Target::Hidden),
                        _ => {}
                    },
                    _ => {}
                }
            }

            if let Some(target) = memory.menu.clone() {
                let host = match (&target, &row_menu) {
                    (Target::Folder { path, .. }, Some(menu)) => menu.entries(path),
                    _ => Vec::new(),
                };
                let mut ran: Vec<Command> = Vec::new();
                let rows = MenuRows { places, host: &host, words: &words, around: &around, favorites_group };
                let menu = Menu::context(words.menu.clone(), &out.response).show(ui, |ui, _| rows.show(ui, &target, &mut ran));
                if menu.output.closed.is_some() {
                    memory.menu = None;
                }
                events.extend(ran.into_iter().map(into_event));
            }

            *cx.keep::<Memory>() = memory;
            out.response
        });
        // A pass that did not run the body asked for no folder: dropping what was not asked would cancel every listing.
        shown.settle(|_, _| cache.finish_pass());
        let response = shown.inner.unwrap_or(shown.output);
        PlacesOutput { response, events }
    }
}

/// How far [`reveal`] got.
enum Reveal {
    /// Every folder on the way is open; the row of the folder to scroll to.
    Reached(RowKey),
    /// A listing on the way has not arrived.
    Waiting,
    /// The folder is not below the owner root, or a listing on the way fails or lacks it.
    Unreachable,
}

/// Opens the owner root and the folders on the way down to `current`, as far as their listings have arrived. Folders
/// are matched by [`same_folder`] and [`folder_within`], so a spelling of `current` that differs from the listing's
/// still finds its row.
fn reveal(places: &Places, cache: &mut FolderCache, current: &Path, open: &mut OpenSet<RowKey>) -> Reveal {
    let Some((group, index)) = places.owner else { return Reveal::Unreachable };
    let Some(root) = places.groups.get(group).and_then(|group| group.places.get(index)).map(|place| &place.path) else {
        return Reveal::Unreachable;
    };
    if !folder_within(current, root) {
        return Reveal::Unreachable;
    }
    let key = |path: &Path| RowKey::Folder(At { group, root: root.clone(), path: path.to_path_buf() });
    let mut path = root.clone();
    for _ in 0..=MAX_DEPTH {
        if same_folder(&path, current) {
            return Reveal::Reached(key(&path));
        }
        open.set(&key(&path), true);
        match cache.subfolders(&path) {
            Listing::Loading => return Reveal::Waiting,
            Listing::Failed(_) => return Reveal::Unreachable,
            Listing::Ready(folders) => match folders.iter().find(|folder| folder_within(current, &folder.path)) {
                Some(folder) => path = folder.path.clone(),
                None => return Reveal::Unreachable,
            },
        }
    }
    Reveal::Unreachable
}

/// The row of `current`: the owner root's, else the first that names it.
fn selected_key(places: &Places, shown: &Shown, current: &Path) -> Option<RowKey> {
    let named = |row: &&Row| matches!(&row.key, RowKey::Folder(at) if same_folder(&at.path, current));
    let owner = places.owner.and_then(|(group, index)| places.groups.get(group)?.places.get(index));
    let in_owner = |row: &&Row| matches!((&row.key, owner), (RowKey::Folder(at), Some(root)) if at.root == root.path);
    shown.rows.iter().filter(named).find(in_owner).or_else(|| shown.rows.iter().find(named)).map(|row| row.key.clone())
}

fn group_word(words: &PlacesWords, kind: PlaceGroupKind) -> &Label {
    match kind {
        PlaceGroupKind::Favorites => &words.favorites,
        PlaceGroupKind::Places => &words.places,
        PlaceGroupKind::Volumes => &words.volumes,
        PlaceGroupKind::Network => &words.network,
        PlaceGroupKind::Recent => &words.recent,
    }
}

fn standard_word(words: &PlacesWords, folder: StandardFolder) -> &Label {
    match folder {
        StandardFolder::Home => &words.home,
        StandardFolder::Desktop => &words.desktop,
        StandardFolder::Documents => &words.documents,
        StandardFolder::Downloads => &words.downloads,
        StandardFolder::Pictures => &words.pictures,
        StandardFolder::Movies => &words.movies,
        StandardFolder::Music => &words.music,
    }
}

/// The display order: each group's heading, its roots with their open folders, then the hidden row.
fn rows(places: &Places, cache: &mut FolderCache, open: &OpenSet<RowKey>, words: &PlacesWords, format: &Format) -> Shown {
    let mut shown = Shown { rows: Vec::new() };
    for (group, items) in places.groups.iter().enumerate() {
        shown.rows.push(Row {
            key: RowKey::Heading(group),
            depth: 0,
            has_children: false,
            open: false,
            content: Content::Heading(group_word(words, items.kind).clone()),
        });
        if items.places.is_empty() {
            shown.rows.push(Row {
                key: RowKey::Empty(group),
                depth: 0,
                has_children: false,
                open: false,
                content: Content::Empty(words.none.clone()),
            });
        }
        for place in &items.places {
            let name = match place.kind {
                PlaceKind::Standard(folder) => standard_word(words, folder).clone(),
                _ => Label::from(place.name.clone()),
            };
            let at = At { group, root: place.path.clone(), path: place.path.clone() };
            folder_rows(&mut shown, cache, open, words, at, 0, name);
        }
    }
    if !places.hidden.is_empty() {
        let count = places.hidden.len() as u64;
        let text = words.hidden.text(count, &format.integer(count));
        shown.rows.push(Row { key: RowKey::Hidden, depth: 0, has_children: false, open: false, content: Content::Hidden(Label::from(text)) });
    }
    shown
}

fn folder_rows(shown: &mut Shown, cache: &mut FolderCache, open: &OpenSet<RowKey>, words: &PlacesWords, at: At, depth: usize, name: Label) {
    let key = RowKey::Folder(at.clone());
    let is_open = open.is_open(&key);
    let mut children = None;
    let has_children = if depth >= MAX_DEPTH {
        false
    } else if !is_open {
        true
    } else {
        match cache.subfolders(&at.path) {
            Listing::Loading => {
                children = Some(None);
                true
            }
            Listing::Ready([]) | Listing::Failed(_) => false,
            Listing::Ready(folders) => {
                children = Some(Some(folders.to_vec()));
                true
            }
        }
    };
    shown.rows.push(Row { key, depth, has_children, open: is_open && has_children, content: Content::Folder(name) });
    match children {
        Some(None) => shown.rows.push(Row {
            key: RowKey::Loading(at),
            depth: depth + 1,
            has_children: false,
            open: false,
            content: Content::Loading(words.loading.clone()),
        }),
        Some(Some(folders)) => {
            for Folder { path, name } in folders {
                let child = At { group: at.group, root: at.root.clone(), path };
                folder_rows(shown, cache, open, words, child, depth + 1, Label::from(name));
            }
        }
        None => {}
    }
}

/// Applies an open or close the list asked for; an opened folder is listed again.
fn apply_open(open: &mut OpenSet<RowKey>, cache: &mut FolderCache, shown: &Shown, key: &RowKey, wanted: bool, all: bool) {
    let groups: Vec<RowKey> = shown.rows.iter().filter(|row| row.has_children).map(|row| row.key.clone()).collect();
    open.apply(key, wanted, all, groups.iter());
    if !wanted {
        return;
    }
    let opened: Vec<&RowKey> = if all { groups.iter().collect() } else { vec![key] };
    for opened in opened {
        if let RowKey::Folder(at) = opened {
            cache.refresh_folder(&at.path);
        }
    }
}

/// The rows of the context menu of `target`; the commands picked go to `ran`.
struct MenuRows<'a> {
    places: &'a Places,
    host: &'a [Entry<String>],
    words: &'a PlacesWords,
    around: &'a Surroundings<'a>,
    favorites_group: bool,
}

impl MenuRows<'_> {
    fn show(&self, ui: &mut Ui, target: &Target, ran: &mut Vec<Command>) {
        let mut item = |ui: &mut Ui, label: &Label, command: Command| {
            if MenuItem::new(label.clone()).show(ui).ran {
                ran.push(command);
            }
        };
        let words = self.words;
        match target {
            Target::Hidden => {
                for place in &self.places.hidden {
                    item(ui, &Label::from(place.name.clone()), Command::Restore(place.path.clone()));
                }
                Divider::horizontal().show(ui);
                item(ui, &words.restore_all, Command::RestoreAll);
            }
            Target::Folder { path, root } => {
                let keep = if self.favorites_group { &words.add_favorite } else { &words.keep };
                match root {
                    None | Some(PlaceKind::Session) => item(ui, keep, Command::Keep(path.clone())),
                    Some(PlaceKind::Recent) => item(ui, &words.clear_recent, Command::ClearRecent),
                    Some(_) => item(ui, &words.remove, Command::Remove(path.clone())),
                }
                if self.around.desktop {
                    let reveal = if self.around.mac { &words.reveal_mac } else { &words.reveal_other };
                    item(ui, reveal, Command::Reveal(path.clone()));
                }
                if !self.host.is_empty() {
                    Divider::horizontal().show(ui);
                }
                for entry in self.host {
                    if entry.is_separator() {
                        Divider::horizontal().show(ui);
                    } else if let Some(command) = entry.value()
                        && MenuItem::new(entry.label().clone()).enabled(entry.is_enabled()).show(ui).ran
                    {
                        ran.push(Command::Host { path: path.clone(), command: command.clone() });
                    }
                }
            }
        }
    }
}

fn into_event(command: Command) -> PlacesEvent {
    match command {
        Command::Keep(path) => PlacesEvent::Keep(path),
        Command::Remove(path) => PlacesEvent::Remove(path),
        Command::Restore(path) => PlacesEvent::Restore(path),
        Command::RestoreAll => PlacesEvent::RestoreAll,
        Command::ClearRecent => PlacesEvent::ClearRecent,
        Command::Reveal(path) => PlacesEvent::Reveal(path),
        Command::Host { path, command } => PlacesEvent::Menu(path, command),
    }
}
