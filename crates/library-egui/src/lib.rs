//! The Library pane the Craft apps share (storytold/deckcraft#958), in egui: a strip of tabs, then
//! the items of the documents in the user's Library folders (slides, pages, sheets) as thumbnails
//! grouped by file, with a folder drop-down (Add Folder…, Remove, Refresh) and a search box.
//! Clicking an item inserts it; its context menu can also put its shapes, or one named element,
//! where the person is working, and holds the app's own inserts ([`Host::menu_inserts`]). An app
//! that takes dropped items ([`Host::drag_hint`]) lets the person drag one out of the pane: the
//! drag carries a [`Dragged`], which the app inserts where it is let go.
//!
//! The pane holds no document and reads no file: it speaks only the `library.*` commands, through
//! the app's [`Host`]. The app also gives its words ([`Labels`], whose nouns are the commands'
//! keys) and colours ([`Palette`]), and may add tabs of its own beside the shared one ([`ITEMS`]),
//! which it draws in [`Host::tab`]. Every word the pane shows, the app's and the pane's own
//! ([`words`]), goes through [`Host::text`], so the app can show it in the person's language.
//!
//! Files are read, and searched, a few per frame: a frame that has spent its few milliseconds
//! starts no other file, so a large Library fills in while the window keeps drawing. Each read is
//! a call to the app that the frame waits for, though, so one slow file, or listing the folders,
//! holds that frame until it is done. At most [`MAX_SHOWN_FILES`] files and about
//! [`MAX_SHOWN_ITEMS`] items are read and laid out at once, and only the rows of the grid in view
//! are drawn. The pane scrolls its own grid within the space it can be seen in, so its folder and
//! search controls stay in view even when the app puts it in a scroll area. A Library whose files each hold one item (pictures,
//! icons) shows them in one grid ([`Labels::one_per_file`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use craft_library::Nouns;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, Sizing};
use craft_ui::pickers::Select;
use egui::{Color32, CornerRadius, Rect, RichText, Role, Sense, Stroke, StrokeKind, TextureOptions, Ui, WidgetInfo, vec2};
use serde_json::{Value, json};

/// The shared tab's id: the items of the Library's documents.
pub const ITEMS: &str = "items";

/// Time spent reading or searching files per frame, in seconds: no file is started after it, but
/// one started runs to its end.
const READ_BUDGET: f64 = 0.012;
/// Thumbnails drawn per frame.
const THUMBS_PER_FRAME: usize = 2;
/// Thumbnail textures kept.
const MAX_TEXTURES: usize = 256;
/// Search runs this long after the last key, in seconds.
const SEARCH_DELAY: f64 = 0.25;
/// Files laid out at once; past this the pane asks to choose a folder or search.
pub const MAX_SHOWN_FILES: usize = 300;
/// Items laid out at once: files are read until they hold this many (the last may add up to
/// [`craft_library::MAX_ITEMS`] more), and the pane asks to choose a folder or search past it.
pub const MAX_SHOWN_ITEMS: usize = 10_000;
/// Items a search finds and shows, as `library.search` answers at most.
const MAX_HITS: usize = 500;
/// The height of the name under a thumbnail.
const LABEL_H: f32 = 16.0;
/// Elements listed in an item's Insert Element menu, and items whose lists are kept.
const MAX_MENU_ELEMENTS: usize = 40;
const MAX_ELEMENT_LISTS: usize = 64;
/// The largest thumbnail decoded, in pixels.
const MAX_THUMB_PIXELS: u64 = 512 * 2048;
/// Items whose height over width is kept once drawn; past this they are measured again.
const MAX_ASPECTS: usize = 20_000;
/// The grid's least height, when the pane is barely in view.
const MIN_GRID_HEIGHT: f32 = 120.0;
/// The widest thumbnail that follows the pointer while an item is dragged, in points.
const GHOST_WIDTH: f32 = 72.0;
/// The narrowest cell of a one-item-per-file grid, which fits as many columns as this allows.
const FLAT_CELL: f32 = 96.0;
const MAX_FLAT_COLUMNS: usize = 6;

/// The pane's own words, in English. Each goes through [`Host::text`] before it is shown; a
/// `{name}` placeholder stays in the translation and is filled in after.
pub mod words {
    pub const ALL_FOLDERS: &str = "All folders";
    /// The folder drop-down's accessible name.
    pub const FOLDER: &str = "Library folder";
    pub const ADD_FOLDER: &str = "Add Folder…";
    pub const REMOVE: &str = "Remove";
    pub const REMOVE_TIP: &str = "Stop showing this folder in the Library (its files stay where they are)";
    pub const REFRESH: &str = "Refresh";
    pub const REFRESH_TIP: &str = "Read the folders again";
    /// The hint of the path field, when the app has no folder picker.
    pub const FOLDER_PATH: &str = "Folder path";
    pub const ADD: &str = "Add";
    pub const READING: &str = "Reading {read} of {total} files…";
    pub const NO_MATCH: &str = "Nothing matches \"{query}\".";
    pub const NOTHING_HERE: &str = "Nothing in this folder yet.";
    pub const SHOWING_FIRST: &str = "Showing the first {shown} of {total} files: choose a folder or search to see the others.";
    pub const EMPTY_FILE: &str = "Nothing in this file.";
    pub const HIDDEN: &str = "Hidden";
    pub const CLICK_TO_INSERT: &str = "Click to insert";
    /// An item's accessible name: `{item}` is [`super::Labels::item`], `{number}` counts from 1.
    pub const ITEM_TITLED: &str = "{item} {number}: {title}";
    pub const ITEM_UNTITLED: &str = "{item} {number}";
    /// An item's accessible name in a one-item-per-file grid.
    pub const ITEM_NAMED: &str = "{item}: {title}";
    pub const ELEMENT_TIP: &str = "Put this element in the same place";
    /// An element name several shapes carry.
    pub const ELEMENT_COUNT: &str = "{name} (×{count})";
    pub const MORE: &str = "and {count} more";

    /// Every word above, for an app's translation catalog and its checks.
    pub const ALL: &[&str] = &[
        ALL_FOLDERS,
        FOLDER,
        ADD_FOLDER,
        REMOVE,
        REMOVE_TIP,
        REFRESH,
        REFRESH_TIP,
        FOLDER_PATH,
        ADD,
        READING,
        NO_MATCH,
        NOTHING_HERE,
        SHOWING_FIRST,
        EMPTY_FILE,
        HIDDEN,
        CLICK_TO_INSERT,
        ITEM_TITLED,
        ITEM_UNTITLED,
        ITEM_NAMED,
        ELEMENT_TIP,
        ELEMENT_COUNT,
        MORE,
    ];
}

/// `template` with each `{name}` in `values` replaced by its value, in one pass, so a value that
/// holds braces is shown as it is. Other braces stay.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from) = rest.split_at(open);
        out.push_str(before);
        let value = from.find('}').and_then(|close| {
            let name = from.get(1..close)?;
            values.iter().find(|(n, _)| *n == name).map(|(_, v)| (*v, close))
        });
        match value {
            Some((v, close)) => {
                out.push_str(v);
                rest = from.get(close + 1..).unwrap_or_default();
            }
            None => {
                out.push('{');
                rest = from.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// `english` in the app's language.
fn tr(host: &dyn Host, english: &str) -> String {
    host.text(english).into_owned()
}

/// The template `english` in the app's language, filled in.
fn trf(host: &dyn Host, english: &str, values: &[(&str, &str)]) -> String {
    fill(&host.text(english), values)
}

/// What the pane needs from the app.
pub trait Host {
    /// Run a `library.*` command the person asked for (an insert, adding or removing a folder);
    /// the host shows a failure itself.
    fn act(&mut self, id: &str, params: Value) -> Result<Value, String>;
    /// Run a reading `library.*` command (list, search, thumbnail, shapes) quietly.
    fn ask(&mut self, id: &str, params: Value) -> Result<Value, String>;
    /// The Library folders, as the preferences keep them.
    fn folders(&self) -> Vec<String>;
    /// The host's folder picker: `None` when it has none (the pane then takes a typed path),
    /// `Some(None)` when the person cancelled it.
    fn pick_folder(&mut self) -> Option<Option<String>>;
    /// Is there somewhere to put shapes (a current slide, page or sheet)?
    fn can_place(&self) -> bool;
    /// Tell the person something (the status bar).
    fn report(&mut self, message: String);
    /// Draw the app's own tab `id` (one of [`Labels::tabs`] other than [`ITEMS`]).
    fn tab(&mut self, _ui: &mut Ui, _id: &str) {}
    /// `english` in the person's language: one of [`words::ALL`], a word of the app's [`Labels`],
    /// or a word of the app's own [`Host::menu_inserts`] and [`Host::drag_hint`]. A `{name}`
    /// placeholder stays in the answer; the pane fills it in. The default shows English.
    fn text<'a>(&self, english: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(english)
    }
    /// Inserts of the app's own, after the pane's in an item's context menu ("Place in Cell"),
    /// in order. Each puts the item where the person is working, so it is offered when
    /// [`Host::can_place`]. The default: none.
    fn menu_inserts(&self) -> Vec<MenuInsert> {
        Vec::new()
    }
    /// The app takes an item dropped on it ([`Dragged`]): what an item's tooltip says about
    /// dragging, in English ("Drag onto a cell to put it there"). With a mouse or pen, an item's
    /// cell is then dragged rather than the grid scrolled; a finger still scrolls the grid. The
    /// default, `None`: items aren't dragged out of the pane.
    fn drag_hint(&self) -> Option<&'static str> {
        None
    }
}

/// An insert of the app's own in an item's context menu ([`Host::menu_inserts`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuInsert {
    /// The menu item, in English (through [`Host::text`]).
    pub label: &'static str,
    /// What it adds to `library.insert`'s params, such as `{"placement": "cell"}`.
    pub params: Value,
}

/// The item being dragged out of the pane: egui's drag-and-drop payload
/// ([`egui::DragAndDrop`]), set when a drag starts on an item's cell in an app that takes drops
/// ([`Host::drag_hint`]). The app takes it where the pointer is let go
/// ([`egui::Response::dnd_release_payload`]) and inserts it there with [`Dragged::params`]. Escape,
/// or letting go anywhere else, drops nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dragged {
    pub file: String,
    /// The item's index in its file, as `library.list` numbers it.
    pub item: usize,
}

impl Dragged {
    /// `library.insert`'s params for the item, keyed with the app's noun: `{"file", "slide": 2}`.
    pub fn params(&self, nouns: Nouns) -> Value {
        let mut p = json!({"file": self.file});
        p[nouns.one] = json!(self.item);
        p
    }
}

/// The app's words, in English (each goes through [`Host::text`]), and how its items are laid out.
/// `nouns` also name the commands' keys (`slide`, `slides`) and are never shown.
#[derive(Clone, Debug)]
pub struct Labels {
    pub nouns: Nouns,
    /// The tab strip, in order: `(id, label)`; [`ITEMS`] is the shared tab.
    pub tabs: &'static [(&'static str, &'static str)],
    /// An item's accessible name before its number: `Slide` gives "Slide 2: Agenda".
    pub item: &'static str,
    /// "Insert slide".
    pub insert_item: &'static str,
    /// "Insert shapes on current slide"; `None` when the app has no such insert.
    pub insert_shapes: Option<&'static str>,
    /// "Insert element"; `None` when the app has no named elements.
    pub insert_element: Option<&'static str>,
    pub search_hint: &'static str,
    /// The tooltip of Add Folder….
    pub add_folder_tip: &'static str,
    /// What the pane says with no folders yet.
    pub empty: &'static str,
    /// What the pane says in a browser build.
    pub not_in_browser: &'static str,
    /// An item's height over its width, before its thumbnail arrives.
    pub aspect: f32,
    /// Every file is one item (a picture, an icon): the items share one grid, each named under
    /// its thumbnail, rather than each file having its own heading.
    pub one_per_file: bool,
}

/// The app's colours.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub separator: Color32,
    /// An item's background before its thumbnail arrives.
    pub control: Color32,
    pub border: Color32,
    pub accent: Color32,
}

impl Palette {
    /// Colours from egui's own visuals, for an app without theme tokens.
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        Palette {
            text_dim: v.weak_text_color(),
            text_faint: v.weak_text_color().gamma_multiply(0.7),
            separator: v.widgets.noninteractive.bg_stroke.color,
            control: v.faint_bg_color,
            border: v.widgets.noninteractive.bg_stroke.color,
            accent: v.selection.stroke.color,
        }
    }
}

/// One item as the pane lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRow {
    pub index: usize,
    pub title: String,
    pub hidden: bool,
}

/// One document; `items` is `None` until it has been read.
#[derive(Clone, Debug)]
pub struct FileRow {
    pub file: String,
    pub title: String,
    pub items: Option<Arc<Vec<ItemRow>>>,
    pub error: Option<String>,
}

/// Items under one file's heading.
#[derive(Clone, Debug)]
struct Group {
    file: String,
    title: String,
    error: Option<String>,
    items: Arc<Vec<ItemRow>>,
}

enum Thumb {
    Ready(egui::TextureHandle),
    Failed,
}

/// A thumbnail: its file, the item and its width in pixels.
type ThumbKey = (String, usize, u32);

/// A search the pane runs a few files at a time.
struct Search {
    query: String,
    /// The next of the listed files to search.
    next: usize,
    /// What was found, by how it matched (title, text, file name), each at most [`MAX_HITS`]: the
    /// file's place in the listing and the item.
    found: [Vec<(usize, ItemRow)>; 3],
    /// `found` grouped by file, best first, as drawn.
    groups: Vec<Group>,
    /// An error was shown (once a search: they are likely the same for every file).
    reported: bool,
}

impl Search {
    /// Every file has been searched, or enough title matches found that no other could be shown.
    fn done(&self, files: usize) -> bool {
        self.next >= files || self.found.first().is_some_and(|t| t.len() >= MAX_HITS)
    }
}

/// Pane-only state: nothing here is part of a document or a preference.
#[derive(Default)]
pub struct State {
    /// The tab shown (empty: the first).
    pub tab: String,
    /// The folder shown; `None`: every folder.
    pub folder: Option<String>,
    pub query: String,
    /// A folder path typed in (when the host has no folder picker).
    pub new_folder: String,
    /// The path field is shown.
    pub adding: bool,
    /// The Library folders and the folder shown that `files` was listed for.
    listed_for: Option<(Vec<String>, Option<String>)>,
    /// The files listed; only those laid out keep their items.
    pub files: Vec<FileRow>,
    /// How many of `files` have been read, in order, and the items they hold.
    read: usize,
    read_items: usize,
    /// The next listing reads again the files that couldn't be read (after Refresh).
    reread: bool,
    /// The search being run, or run last.
    search: Option<Search>,
    typed_at: f64,
    /// Thumbnails by file, then by item and width, and the order they were drawn in.
    thumbs: HashMap<String, HashMap<(usize, u32), Thumb>>,
    thumb_order: VecDeque<ThumbKey>,
    /// Each item's height over its width, by file, once its thumbnail is drawn: kept when the
    /// texture goes, so the grid's rows keep their heights. How many are kept.
    aspects: HashMap<String, HashMap<usize, f32>>,
    aspect_count: usize,
    /// An item's top-level shape names and how many shapes carry each, for Insert Element.
    elements: HashMap<(String, usize), Vec<(String, usize)>>,
}

impl State {
    /// List and read the folders again, files that couldn't be read too, and draw the
    /// thumbnails again (after files changed).
    pub fn refresh(&mut self) {
        self.listed_for = None;
        self.reread = true;
        self.search = None;
        self.forget_drawn();
    }

    /// Forget what was asked for each item (its thumbnail, its height over width and its element
    /// names), which is asked for again when it is next drawn.
    fn forget_drawn(&mut self) {
        self.thumbs.clear();
        self.thumb_order.clear();
        self.aspects.clear();
        self.aspect_count = 0;
        self.elements.clear();
    }

    /// Every file to be laid out has been read.
    pub fn ready(&self) -> bool {
        self.listed_for.is_some() && !self.reading()
    }

    /// Files to be laid out are still being read.
    fn reading(&self) -> bool {
        self.read < self.files.len().min(MAX_SHOWN_FILES) && self.read_items < MAX_SHOWN_ITEMS
    }

    /// How many items the last search found, once every file has been searched.
    pub fn hits(&self) -> Option<usize> {
        self.search.as_ref().filter(|s| s.done(self.files.len())).map(|s| s.groups.iter().map(|g| g.items.len()).sum())
    }

    fn thumb(&self, file: &str, item: usize, px: u32) -> Option<&Thumb> {
        self.thumbs.get(file)?.get(&(item, px))
    }
}

/// Draw the pane.
pub fn show(ui: &mut Ui, st: &mut State, host: &mut dyn Host, labels: &Labels, pal: &Palette) {
    if !labels.tabs.iter().any(|(id, _)| *id == st.tab) {
        st.tab = labels.tabs.first().map(|(id, _)| (*id).to_string()).unwrap_or_else(|| ITEMS.to_string());
    }
    let tabs: Vec<(&str, String)> = labels.tabs.iter().map(|(id, label)| (*id, tr(host, label))).collect();
    ui.horizontal(|ui| {
        for (id, label) in tabs {
            if ui.selectable_label(st.tab == id, label).clicked() {
                st.tab = id.to_string();
            }
        }
    });
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, pal.separator);
    ui.add_space(6.0);
    if st.tab == ITEMS {
        items(ui, st, host, labels, pal);
    } else {
        let tab = st.tab.clone();
        host.tab(ui, &tab);
    }
}

/// The last part of a folder's path, which the drop-down shows.
fn folder_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or(path)
}

fn dim(pal: &Palette, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(pal.text_dim)
}

/// Add Folder…: the host's folder picker, else a field to type the path in.
fn add_folder(st: &mut State, host: &mut dyn Host) {
    match host.pick_folder() {
        Some(Some(path)) => {
            if let Ok(v) = host.act("library.addFolder", json!({"path": path})) {
                st.folder = v["path"].as_str().map(String::from);
            }
        }
        Some(None) => {}
        None => st.adding = true,
    }
}

fn folder_controls(ui: &mut Ui, st: &mut State, host: &mut dyn Host, labels: &Labels) {
    let folders = host.folders();
    if st.folder.as_ref().is_some_and(|f| !folders.contains(f)) {
        st.folder = None;
    }
    // Entry 0 is every folder; entry i + 1 is folders[i], shown by its name, with its path as its description.
    let entries: Vec<Entry<usize>> = std::iter::once(Entry::new(0, tr(host, words::ALL_FOLDERS)))
        .chain(folders.iter().enumerate().map(|(i, f)| Entry::new(i + 1, folder_name(f).to_string()).description(f.clone())))
        .collect();
    let held = st.folder.as_ref().and_then(|f| folders.iter().position(|g| g == f)).map_or(0, |i| i + 1);
    let picked = Select::new(tr(host, words::FOLDER), Bind::given(held), &entries).id_salt("library_folder").sizing(Sizing::FillWidth).show(ui);
    if let Some(&at) = picked.value() {
        st.folder = at.checked_sub(1).and_then(|i| folders.get(i)).cloned();
    }
    let [add_label, add_tip, remove_label, remove_tip, refresh_label, refresh_tip, path_hint, add_path] = [
        words::ADD_FOLDER,
        labels.add_folder_tip,
        words::REMOVE,
        words::REMOVE_TIP,
        words::REFRESH,
        words::REFRESH_TIP,
        words::FOLDER_PATH,
        words::ADD,
    ]
    .map(|w| tr(host, w));
    ui.horizontal(|ui| {
        let full = folders.len() >= craft_library::MAX_FOLDERS;
        let add = ui.add_enabled(!full, egui::Button::new(add_label));
        if add.on_hover_text(add_tip).clicked() {
            add_folder(st, host);
        }
        let remove = ui.add_enabled(st.folder.is_some(), egui::Button::new(remove_label));
        if remove.on_hover_text(remove_tip).clicked()
            && let Some(f) = st.folder.take()
        {
            let _ = host.act("library.removeFolder", json!({"path": f}));
        }
        if ui.button(refresh_label).on_hover_text(refresh_tip).clicked() {
            st.refresh();
        }
    });
    if st.adding {
        ui.horizontal(|ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut st.new_folder).hint_text(path_hint).desired_width(ui.available_width() - 60.0));
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button(add_path).clicked() || enter) && !st.new_folder.trim().is_empty() {
                let path = st.new_folder.trim().to_string();
                if let Ok(v) = host.act("library.addFolder", json!({"path": path})) {
                    st.folder = v["path"].as_str().map(String::from);
                    st.new_folder.clear();
                    st.adding = false;
                }
            }
        });
    }
}

/// The rows `library.list` answered; only the first `keep` keep their items, each at most
/// [`craft_library::MAX_ITEMS`] of them.
fn rows(v: &Value, nouns: Nouns, keep: usize) -> Vec<FileRow> {
    let item = |x: &Value| ItemRow {
        index: x["index"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0),
        title: x["title"].as_str().unwrap_or_default().to_string(),
        hidden: x["hidden"].as_bool().unwrap_or(false),
    };
    let Some(a) = v.as_array() else { return vec![] };
    a.iter()
        .enumerate()
        .map(|(i, f)| FileRow {
            file: f["file"].as_str().unwrap_or_default().to_string(),
            title: f["title"].as_str().unwrap_or_default().to_string(),
            items: f[nouns.many].as_array().filter(|_| i < keep).map(|s| Arc::new(s.iter().take(craft_library::MAX_ITEMS).map(item).collect())),
            error: f["error"].as_str().map(String::from),
        })
        .collect()
}

/// List the files of the folder shown (without opening them); they are read a few per frame.
fn list_files(st: &mut State, host: &mut dyn Host, nouns: Nouns, key: (Vec<String>, Option<String>)) {
    let mut p = json!({});
    p[nouns.many] = json!(false);
    if let Some(f) = &key.1 {
        p["folder"] = json!(f);
    }
    if std::mem::take(&mut st.reread) {
        p["refresh"] = json!(true);
    }
    let mut files = match host.ask("library.list", p) {
        // Files listed with their items (pictures) keep them only when they can be laid out.
        Ok(v) => rows(&v, nouns, MAX_SHOWN_FILES),
        Err(e) => {
            host.report(e);
            vec![]
        }
    };
    // A file listed with an error (too large) is not opened.
    for f in &mut files {
        if f.error.is_some() {
            f.items = Some(Arc::default());
        }
    }
    st.files = files;
    st.read = 0;
    st.read_items = 0;
    st.listed_for = Some(key);
    st.search = None;
}

/// Read the files to be laid out, in order, until this frame's budget is spent.
fn read_some(st: &mut State, host: &mut dyn Host, nouns: Nouns, start: web_time::Instant) {
    while st.reading() && start.elapsed().as_secs_f64() < READ_BUDGET {
        let i = st.read;
        st.read += 1;
        let Some(file) = st.files.get(i).map(|f| f.file.clone()) else { break };
        if st.files.get(i).is_some_and(|f| f.items.is_none()) {
            let answer = host.ask("library.list", json!({"file": file}));
            let Some(row) = st.files.get_mut(i) else { break };
            match answer.map(|v| rows(&v, nouns, 1)) {
                Ok(read) => match read.into_iter().next() {
                    Some(r) => {
                        row.title = r.title;
                        row.items = Some(r.items.unwrap_or_default());
                        row.error = r.error;
                    }
                    None => row.items = Some(Arc::default()),
                },
                Err(e) => {
                    row.items = Some(Arc::default());
                    row.error = Some(e);
                }
            }
        }
        st.read_items += st.files.get(i).and_then(|f| f.items.as_ref()).map_or(0, |s| s.len());
    }
    if !st.reading() {
        // Only the files read are laid out: the others let go of the items they were listed with.
        for f in st.files.iter_mut().take(MAX_SHOWN_FILES).skip(st.read) {
            f.items = None;
        }
    }
}

/// Search the next listed files with `library.search`, one at a time, until this frame's budget
/// is spent.
fn search_some(s: &mut Search, files: &[FileRow], host: &mut dyn Host, nouns: Nouns, start: web_time::Instant) {
    let mut found = false;
    while !s.done(files.len()) && start.elapsed().as_secs_f64() < READ_BUDGET {
        let i = s.next;
        s.next += 1;
        // A file that couldn't be read is searched too: the Library reads it again if it changed.
        let Some(f) = files.get(i) else { continue };
        let answer = match host.ask("library.search", json!({"query": s.query, "file": f.file, "limit": MAX_HITS})) {
            Ok(a) => a,
            Err(e) => {
                if !std::mem::replace(&mut s.reported, true) {
                    host.report(e);
                }
                continue;
            }
        };
        for h in answer.as_array().into_iter().flatten() {
            let Some(item) = h[nouns.one].as_u64().and_then(|i| usize::try_from(i).ok()) else { continue };
            let how = match h["match"].as_str() {
                Some("title") => 0,
                Some("text") => 1,
                _ => 2,
            };
            let hidden =
                h["hidden"].as_bool().unwrap_or_else(|| f.items.as_ref().and_then(|s| s.iter().find(|x| x.index == item)).is_some_and(|x| x.hidden));
            let title = h["title"].as_str().unwrap_or_default().to_string();
            if let Some(kind) = s.found.get_mut(how).filter(|k| k.len() < MAX_HITS) {
                kind.push((i, ItemRow { index: item, title, hidden }));
                found = true;
            }
        }
    }
    if found {
        s.groups = hit_groups(&s.found, files);
    }
}

/// The best [`MAX_HITS`] hits (title matches, then text, then file names), grouped by file in
/// the order the files were first found.
fn hit_groups(found: &[Vec<(usize, ItemRow)>; 3], files: &[FileRow]) -> Vec<Group> {
    let mut groups: Vec<(usize, Vec<ItemRow>)> = vec![];
    let mut at: HashMap<usize, usize> = HashMap::new();
    for (file, row) in found.iter().flatten().take(MAX_HITS) {
        match at.get(file).and_then(|g| groups.get_mut(*g)) {
            Some((_, items)) => items.push(row.clone()),
            None => {
                at.insert(*file, groups.len());
                groups.push((*file, vec![row.clone()]));
            }
        }
    }
    groups
        .into_iter()
        .filter_map(|(i, items)| files.get(i).map(|f| Group { file: f.file.clone(), title: f.title.clone(), error: None, items: Arc::new(items) }))
        .collect()
}

fn items(ui: &mut Ui, st: &mut State, host: &mut dyn Host, labels: &Labels, pal: &Palette) {
    if cfg!(target_arch = "wasm32") {
        ui.label(dim(pal, tr(host, labels.not_in_browser)));
        return;
    }
    folder_controls(ui, st, host, labels);
    let folders = host.folders();
    if folders.is_empty() {
        // Listed and drawn again when folders come back, even the same ones: what they hold may
        // differ.
        st.listed_for = None;
        st.files.clear();
        st.search = None;
        st.forget_drawn();
        ui.add_space(6.0);
        ui.label(dim(pal, tr(host, labels.empty)));
        return;
    }
    ui.add_space(4.0);
    // The Library searches for at most MAX_QUERY characters: no more can be typed, and a query
    // the app set longer is cut to them.
    if st.query.chars().count() > craft_library::MAX_QUERY {
        st.query = st.query.chars().take(craft_library::MAX_QUERY).collect();
    }
    let hint = tr(host, labels.search_hint);
    let field = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text(hint).char_limit(craft_library::MAX_QUERY).desired_width(f32::INFINITY));
    if field.changed() {
        st.typed_at = ui.input(|i| i.time);
    }
    let key = (folders, st.folder.clone());
    if st.listed_for.as_ref() != Some(&key) {
        // Folders added or removed (not another one chosen): what is drawn from their files is
        // asked for again, as a folder that comes back may hold other files at the same paths.
        if st.listed_for.as_ref().is_none_or(|(listed, _)| *listed != key.0) {
            st.forget_drawn();
        }
        list_files(st, host, labels.nouns, key);
    }
    let start = web_time::Instant::now();
    let query = st.query.trim().to_string();
    let groups: Vec<Group> = if query.is_empty() {
        read_some(st, host, labels.nouns, start);
        if st.reading() {
            let (read, total) = (st.read.to_string(), st.files.len().min(MAX_SHOWN_FILES).to_string());
            ui.label(dim(pal, trf(host, words::READING, &[("read", &read), ("total", &total)])));
            ui.ctx().request_repaint();
        }
        // The files read, in order: one past them listed with its items (from its name) waits.
        let groups: Vec<Group> = st
            .files
            .iter()
            .take(st.read)
            .filter_map(|f| Some(Group { file: f.file.clone(), title: f.title.clone(), error: f.error.clone(), items: f.items.clone()? }))
            .collect();
        if !st.reading() {
            if groups.is_empty() {
                ui.label(dim(pal, tr(host, words::NOTHING_HERE)));
            } else if groups.len() < st.files.len() {
                let (shown, total) = (groups.len().to_string(), st.files.len().to_string());
                ui.label(dim(pal, trf(host, words::SHOWING_FIRST, &[("shown", &shown), ("total", &total)])));
            }
        }
        groups
    } else {
        if st.search.as_ref().is_none_or(|s| s.query != query) {
            let waited = ui.input(|i| i.time) - st.typed_at;
            if waited >= SEARCH_DELAY {
                st.search = Some(Search { query: query.clone(), next: 0, found: Default::default(), groups: vec![], reported: false });
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(SEARCH_DELAY - waited));
            }
        }
        match st.search.as_mut().filter(|s| s.query == query) {
            Some(s) => {
                search_some(s, &st.files, host, labels.nouns, start);
                if s.done(st.files.len()) {
                    if s.groups.is_empty() {
                        ui.label(dim(pal, trf(host, words::NO_MATCH, &[("query", &query)])));
                    }
                } else {
                    let (read, total) = (s.next.to_string(), st.files.len().to_string());
                    ui.label(dim(pal, trf(host, words::READING, &[("read", &read), ("total", &total)])));
                    ui.ctx().request_repaint();
                }
                s.groups.clone()
            }
            None => vec![],
        }
    };
    // The grid scrolls within the part of the pane that can be seen, so the controls above stay
    // in view even inside the app's own scroll area (where the height left is unbounded).
    let visible = ui.clip_rect().bottom() - ui.cursor().top();
    let room = ui.available_height().min(visible).max(MIN_GRID_HEIGHT);
    egui::ScrollArea::vertical().id_salt("library_items").max_height(room).auto_shrink([false, false]).show(ui, |ui| {
        let mut budget = THUMBS_PER_FRAME;
        if labels.one_per_file {
            flat(ui, st, host, labels, pal, &groups, &mut budget);
        } else {
            for g in &groups {
                group(ui, st, host, labels, pal, g, &mut budget);
            }
        }
        if budget == 0 {
            ui.ctx().request_repaint();
        }
    });
}

/// Pixels to draw a cell `w` points wide, in steps of 32 so nearby pane widths share thumbnails.
fn thumb_px(ui: &Ui, w: f32) -> u32 {
    let px = ((w * ui.ctx().pixels_per_point()) / 32.0).ceil() as u32 * 32;
    px.clamp(*craft_library::THUMB_WIDTHS.start(), *craft_library::THUMB_WIDTHS.end())
}

/// An item's height over its width: its thumbnail's, once drawn, even after its texture goes.
fn aspect(st: &State, labels: &Labels, file: &str, item: usize) -> f32 {
    st.aspects.get(file).and_then(|a| a.get(&item)).copied().unwrap_or(labels.aspect)
}

/// One row of cells `w` points wide: drawn when it is in view, else only its space taken, so a
/// long grid costs only the rows that can be seen. `cells` gives each cell's file, its item and,
/// in a one-item-per-file grid, the file's title, which names an untitled item.
#[allow(clippy::too_many_arguments)]
fn grid_row(
    ui: &mut Ui,
    st: &mut State,
    host: &mut dyn Host,
    labels: &Labels,
    pal: &Palette,
    cells: &[(&str, &ItemRow, &str)],
    size: (f32, u32, bool),
    budget: &mut usize,
) {
    let (w, _, named) = size;
    let tallest = cells.iter().map(|(file, it, _)| w * aspect(st, labels, file, it.index)).fold(0.0, f32::max);
    let h = (tallest + LABEL_H).max(ui.spacing().interact_size.y);
    let row = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), h));
    if !ui.is_rect_visible(row) {
        ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (file, it, file_title) in cells {
            if named && it.title.trim().is_empty() {
                let it = ItemRow { title: (*file_title).to_string(), ..(*it).clone() };
                cell(ui, st, host, labels, pal, file, &it, size, budget);
            } else {
                cell(ui, st, host, labels, pal, file, it, size, budget);
            }
        }
    });
}

/// Every file's one item in one grid, named under its thumbnail; files that couldn't be read
/// after it.
fn flat(ui: &mut Ui, st: &mut State, host: &mut dyn Host, labels: &Labels, pal: &Palette, groups: &[Group], budget: &mut usize) {
    let gap = 6.0;
    let width = ui.available_width();
    let columns = (((width + gap) / (FLAT_CELL + gap)).floor() as usize).clamp(2, MAX_FLAT_COLUMNS);
    let w = ((width - gap * (columns - 1) as f32) / columns as f32).floor().max(32.0);
    let px = thumb_px(ui, w);
    let cells: Vec<(&str, &ItemRow, &str)> =
        groups.iter().flat_map(|g| g.items.iter().map(move |it| (g.file.as_str(), it, g.title.as_str()))).collect();
    ui.add_space(6.0);
    for row in cells.chunks(columns) {
        grid_row(ui, st, host, labels, pal, row, (w, px, true), budget);
    }
    for g in groups {
        if let Some(e) = &g.error {
            ui.label(RichText::new(format!("{}: {e}", g.title)).color(pal.text_dim).small()).on_hover_text(&g.file);
        }
    }
}

fn group(ui: &mut Ui, st: &mut State, host: &mut dyn Host, labels: &Labels, pal: &Palette, g: &Group, budget: &mut usize) {
    ui.add_space(6.0);
    ui.label(RichText::new(&g.title).strong()).on_hover_text(&g.file);
    if let Some(e) = &g.error {
        ui.label(RichText::new(e).color(pal.text_dim).small());
        return;
    }
    if g.items.is_empty() {
        ui.label(dim(pal, tr(host, words::EMPTY_FILE)));
        return;
    }
    let gap = 6.0;
    let w = ((ui.available_width() - gap) / 2.0).floor().max(48.0);
    let px = thumb_px(ui, w);
    for pair in g.items.chunks(2) {
        let row: Vec<(&str, &ItemRow, &str)> = pair.iter().map(|it| (g.file.as_str(), it, g.title.as_str())).collect();
        grid_row(ui, st, host, labels, pal, &row, (w, px, false), budget);
    }
}

/// The accessible name of an item's cell, in the app's language: "Slide 2: Agenda", or in a
/// one-item-per-file grid "Picture: Harbour".
pub fn cell_label(host: &dyn Host, labels: &Labels, row: &ItemRow) -> String {
    let item = tr(host, labels.item);
    let number = (row.index + 1).to_string();
    let values = [("item", item.as_str()), ("number", number.as_str()), ("title", row.title.as_str())];
    let template = match (labels.one_per_file, row.title.is_empty()) {
        (true, false) => words::ITEM_NAMED,
        (_, true) => words::ITEM_UNTITLED,
        (false, false) => words::ITEM_TITLED,
    };
    trf(host, template, &values)
}

/// One item's cell, `size.0` points wide, its thumbnail drawn `size.1` pixels wide; `size.2`: in
/// a one-item-per-file grid, which names the item without its number.
#[allow(clippy::too_many_arguments)]
fn cell(
    ui: &mut Ui,
    st: &mut State,
    host: &mut dyn Host,
    labels: &Labels,
    pal: &Palette,
    file: &str,
    row: &ItemRow,
    size: (f32, u32, bool),
    budget: &mut usize,
) {
    let (w, px, named) = size;
    let key: ThumbKey = (file.to_string(), row.index, px);
    let h = w * aspect(st, labels, file, row.index);
    let drag_hint = host.drag_hint();
    let sense = if drag_hint.is_some() { Sense::click_and_drag() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h + LABEL_H), sense);
    let label = cell_label(host, labels, row);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
    let img = Rect::from_min_size(rect.min, vec2(w, h));
    if ui.is_rect_visible(rect) {
        if st.thumb(file, row.index, px).is_none() && *budget > 0 {
            *budget -= 1;
            load_thumb(st, host, ui.ctx(), labels.nouns, &key);
        }
        let p = ui.painter();
        let tint = if row.hidden { Color32::from_white_alpha(110) } else { Color32::WHITE };
        match st.thumb(file, row.index, px) {
            Some(Thumb::Ready(tex)) => {
                p.image(tex.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
            }
            Some(Thumb::Failed) => {
                p.rect_filled(img, CornerRadius::same(2), pal.control);
                p.text(img.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(16.0), pal.text_dim);
            }
            None => {
                p.rect_filled(img, CornerRadius::same(2), pal.control);
            }
        }
        let edge = if resp.hovered() { Stroke::new(2.0, pal.accent) } else { Stroke::new(1.0, pal.border) };
        p.rect_stroke(img, CornerRadius::same(2), edge, StrokeKind::Inside);
        let text = match (named, row.title.is_empty()) {
            (true, false) => row.title.clone(),
            (_, true) => format!("{}", row.index + 1),
            (false, false) => format!("{}  {}", row.index + 1, row.title),
        };
        let galley = p.layout_no_wrap(text, egui::FontId::proportional(11.0), if row.hidden { pal.text_faint } else { pal.text_dim });
        let at = egui::pos2(rect.left(), img.bottom() + 2.0);
        p.with_clip_rect(Rect::from_min_max(at, rect.max)).galley(at, galley, pal.text_dim);
    }
    let mut hint = label.clone();
    if row.hidden {
        hint = format!("{hint}\n{}", tr(host, words::HIDDEN));
    }
    hint = format!("{hint}\n{}", tr(host, words::CLICK_TO_INSERT));
    if let Some(drag) = drag_hint {
        hint = format!("{hint}\n{}", tr(host, drag));
    }
    let resp = resp.on_hover_text(hint);
    let dragged = Dragged { file: file.to_string(), item: row.index };
    if drag_hint.is_some() {
        drag(ui, st, &resp, &dragged, &key, pal);
    }
    let target = |extra: Value| {
        let mut p = dragged.params(labels.nouns);
        if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(extra.clone());
        }
        p
    };
    if resp.clicked() {
        let _ = host.act("library.insert", target(json!({})));
    }
    let place = host.can_place();
    let insert_item = tr(host, labels.insert_item);
    let insert_shapes = labels.insert_shapes.map(|w| tr(host, w));
    let insert_element = labels.insert_element.map(|w| tr(host, w));
    resp.context_menu(|ui| {
        if ui.button(insert_item.as_str()).clicked() {
            let _ = host.act("library.insert", target(json!({})));
            ui.close();
        }
        if let Some(shapes) = &insert_shapes
            && ui.add_enabled(place, egui::Button::new(shapes.as_str())).clicked()
        {
            let _ = host.act("library.insert", target(json!({"as": "shapes"})));
            ui.close();
        }
        if let Some(element) = &insert_element {
            let names = element_names(st, host, labels.nouns, file, row.index);
            let tip = tr(host, words::ELEMENT_TIP);
            let shown: Vec<String> = names
                .iter()
                .take(MAX_MENU_ELEMENTS)
                .map(
                    |(name, count)| {
                        if *count > 1 { trf(host, words::ELEMENT_COUNT, &[("name", name), ("count", &count.to_string())]) } else { name.clone() }
                    },
                )
                .collect();
            let more = names.len().checked_sub(MAX_MENU_ELEMENTS).filter(|n| *n > 0).map(|n| trf(host, words::MORE, &[("count", &n.to_string())]));
            ui.add_enabled_ui(place && !names.is_empty(), |ui| {
                ui.menu_button(element.as_str(), |ui| {
                    for ((name, _), shown) in names.iter().zip(shown) {
                        if ui.button(shown).on_hover_text(tip.as_str()).clicked() {
                            let _ = host.act("library.insert", target(json!({"names": [name]})));
                            ui.close();
                        }
                    }
                    if let Some(more) = &more {
                        ui.label(dim(pal, more.as_str()));
                    }
                });
            });
        }
        for extra in host.menu_inserts() {
            let label = tr(host, extra.label);
            if ui.add_enabled(place, egui::Button::new(label)).clicked() {
                let _ = host.act("library.insert", target(extra.params));
                ui.close();
            }
        }
    });
}

/// An item's cell, in an app that takes drops: a drag with a mouse or pen carries the item
/// ([`Dragged`]) and shows its thumbnail beside the pointer; a finger's drag scrolls the grid, as
/// it does over the gaps between cells.
fn drag(ui: &Ui, st: &State, resp: &egui::Response, dragged: &Dragged, key: &ThumbKey, pal: &Palette) {
    let ctx = ui.ctx();
    let touch = ui.input(|i| i.any_touches());
    if touch {
        if resp.dragged() {
            ui.scroll_with_delta(resp.drag_delta());
        }
        return;
    }
    // Only the primary button carries the item: a right-click that wobbles still opens its menu.
    let primary = egui::PointerButton::Primary;
    if resp.drag_started_by(primary) {
        egui::DragAndDrop::set_payload(ctx, dragged.clone());
    }
    let carried = egui::DragAndDrop::payload::<Dragged>(ctx).is_some_and(|d| *d == *dragged);
    let Some(at) = ctx.pointer_latest_pos().filter(|_| resp.dragged_by(primary) && carried) else { return };
    let (size, tex) = match st.thumb(&key.0, key.1, key.2) {
        Some(Thumb::Ready(tex)) => {
            let s = tex.size_vec2();
            let w = s.x.clamp(1.0, GHOST_WIDTH);
            (vec2(w, (w * s.y / s.x.max(1.0)).clamp(8.0, GHOST_WIDTH * 2.0)), Some(tex.id()))
        }
        _ => (vec2(GHOST_WIDTH, GHOST_WIDTH * 0.75), None),
    };
    let rect = Rect::from_min_size(at + vec2(12.0, 8.0), size);
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, resp.id.with("library_drag")));
    match tex {
        Some(tex) => p.image(tex, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(220)),
        None => p.rect_filled(rect, CornerRadius::same(2), pal.control),
    };
    p.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, pal.accent), StrokeKind::Inside);
}

/// The names of an item's top-level shapes that Insert Element offers (not empty placeholders),
/// each with how many shapes carry it; asked for once per item.
fn element_names(st: &mut State, host: &mut dyn Host, nouns: Nouns, file: &str, index: usize) -> Vec<(String, usize)> {
    let key = (file.to_string(), index);
    if let Some(names) = st.elements.get(&key) {
        return names.clone();
    }
    let mut p = json!({"file": file});
    p[nouns.one] = json!(index);
    let mut names: Vec<(String, usize)> = vec![];
    if let Ok(v) = host.ask("library.shapes", p) {
        for sh in v["shapes"].as_array().into_iter().flatten() {
            let name = sh["name"].as_str().unwrap_or_default().trim();
            if name.is_empty() || sh.get("parent").is_some() || sh["empty"].as_bool() == Some(true) {
                continue;
            }
            match names.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                Some((_, c)) => *c += 1,
                None => names.push((name.to_string(), 1)),
            }
        }
    }
    if st.elements.len() >= MAX_ELEMENT_LISTS {
        st.elements.clear();
    }
    st.elements.insert(key, names.clone());
    names
}

/// The PNG of a thumbnail as an image egui can draw, when it is a sane size.
fn decode(png: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_THUMB_PIXELS {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()))
}

/// The base64 in a `library.thumbnail` answer, decoded.
fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// Draw a thumbnail through `library.thumbnail` into a texture; one that can't be drawn is
/// remembered as failed, so it isn't asked for every frame.
fn load_thumb(st: &mut State, host: &mut dyn Host, ctx: &egui::Context, nouns: Nouns, key: &ThumbKey) {
    let mut p = json!({"file": key.0, "width": key.2});
    p[nouns.one] = json!(key.1);
    let thumb = host
        .ask("library.thumbnail", p)
        .ok()
        .and_then(|v| v["png"].as_str().and_then(unbase64))
        .and_then(|png| decode(&png))
        .map(|image| Thumb::Ready(ctx.load_texture(format!("library {} {} {}", key.0, key.1, key.2), image, TextureOptions::LINEAR)))
        .unwrap_or(Thumb::Failed);
    if let Thumb::Ready(tex) = &thumb {
        let size = tex.size_vec2();
        if size.x > 0.0 {
            if st.aspect_count >= MAX_ASPECTS {
                st.aspects.clear();
                st.aspect_count = 0;
            }
            if st.aspects.entry(key.0.clone()).or_default().insert(key.1, (size.y / size.x).clamp(0.1, 4.0)).is_none() {
                st.aspect_count += 1;
            }
        }
    }
    st.thumbs.entry(key.0.clone()).or_default().insert((key.1, key.2), thumb);
    st.thumb_order.push_back(key.clone());
    while st.thumb_order.len() > MAX_TEXTURES {
        let Some((file, item, px)) = st.thumb_order.pop_front() else { break };
        if let Some(of_file) = st.thumbs.get_mut(&file) {
            of_file.remove(&(item, px));
            if of_file.is_empty() {
                st.thumbs.remove(&file);
            }
        }
    }
}

#[cfg(test)]
mod tests;
