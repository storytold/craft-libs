//! [`FileListPicker`]: a list of chosen files (or one folder) with Browse, Remove and the other list buttons, inside a
//! dialog that loads several files.

use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Selection};
use craft_host::{PickId, PickKind, PickRequest};
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, Entry, Held, NoValue, Outcome, Props as _, Remember as _, ReorderRule, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};
use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

craft_ui::words! {
    /// The words of the file list picker.
    pub struct FileListWords {
        use_label: "fileListPicker.use" = "Use:";
        files: "fileListPicker.files" = "Files";
        folder: "fileListPicker.folder" = "Folder";
        browse: "fileListPicker.browse" = "Browse…";
        remove: "fileListPicker.remove" = "Remove";
        add_open_files: "fileListPicker.addOpenFiles" = "Add Open Files";
        sort_by_name: "fileListPicker.sortByName" = "Sort By Name";
        add_files: "fileListPicker.addFiles" = "Add the files to load";
        choose_folder: "fileListPicker.chooseFolder" = "Choose a folder to load";
    }
}

/// Files picked one by one, or one folder whose files the app lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileListMode {
    /// Individual files, listed in the user's order.
    #[default]
    Files,
    /// One folder; the app lists its files.
    Folder,
}

/// The picker's value; the dialog keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FileList {
    /// Whether files or a folder are picked.
    pub mode: FileListMode,
    /// The chosen files in Files mode, in the user's order. The paths are distinct: they are the list's row keys, so
    /// add files through [`FileList::add`].
    pub files: Vec<PathBuf>,
    /// The folder in Folder mode.
    pub folder: Option<PathBuf>,
    /// The selected file in Files mode, one of `files`.
    pub selected: Option<PathBuf>,
}

impl FileList {
    /// Adds paths not in the list yet, in order (Browse, Add Open Files, a drop).
    pub fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if !self.files.contains(&path) {
                self.files.push(path);
            }
        }
    }

    /// Whether OK may run: Files mode needs a file, Folder mode a folder.
    pub fn is_valid(&self) -> bool {
        match self.mode {
            FileListMode::Files => !self.files.is_empty(),
            FileListMode::Folder => self.folder.as_ref().is_some_and(|folder| !folder.as_os_str().is_empty()),
        }
    }

    /// Why OK can't run, for the dialog's message: "Add the files to load" or "Choose a folder to load"; `None` when
    /// [`is_valid`](Self::is_valid).
    pub fn problem<'l>(&self, words: &'l FileListWords) -> Option<&'l str> {
        if self.is_valid() {
            return None;
        }
        Some(match self.mode {
            FileListMode::Files => words.add_files.as_str(),
            FileListMode::Folder => words.choose_folder.as_str(),
        })
    }
}

/// The second line of a file's row.
type InfoLine<'a> = &'a dyn Fn(&Path) -> Option<String>;

/// The pick the picker is waiting for and what it asked for, kept in the picker's container while it is shown.
#[derive(Clone, Copy, Default)]
struct Ticket(Option<(PickId, PickKind)>);

/// What the picker reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FileListPickerOutput {
    /// The picker's response.
    pub response: Response,
    /// The list, mode or folder changed in this pass.
    pub changed: bool,
    /// Browse queued a picker; the answer fills the list on a later pass.
    pub browsing: Option<PickId>,
}

impl Outcome for FileListPickerOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Photoshop's Load Files into Stack file list, after PhotoCraft's port of it.
///
/// The user sees "Use:" with Files and Folder (when [`folder_mode`](Self::folder_mode) is on), the list (one row per
/// file showing its name, the full path as tooltip, and the app's info line under the name when given) and the buttons
/// under it: Browse… (an open-many picker in Files mode, a folder picker in Folder mode, both through `host.files()`),
/// Remove (with a row selected), Add Open Files (when the app passed open documents with a path) and Sort By Name (more
/// than one file; sorts by name ignoring case). In Folder mode the list shows the folder's files the app passes and has
/// no selection.
///
/// Keys: the list is one Tab stop; arrows move the selection; Delete and Backspace remove the selected file; Alt+Up and
/// Alt+Down move it. Dropped files are added by the dialog with [`FileList::add`].
#[must_use = "show the picker with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct FileListPicker<'a> {
    label: Label,
    list: Bind<'a, FileList>,
    /// Browse's picker in Files mode (purpose, filters); Folder mode asks for a folder with the same purpose.
    #[prop]
    request: PickRequest,
    /// Offers Folder mode.
    #[prop]
    folder_mode: bool,
    /// The open documents' paths, for Add Open Files.
    #[prop]
    open_files: &'a [PathBuf],
    /// The files the app found in the chosen folder, shown in Folder mode.
    #[prop]
    folder_files: &'a [PathBuf],
    /// A second line per file ("1200 × 800 px, 300 ppi, RGB"); none for no line.
    #[prop]
    info: Option<InfoLine<'a>>,
}

impl<'a> FileListPicker<'a> {
    /// The picker named `label` over `list`; Browse asks with `PickRequest::open_many("stack")` until
    /// [`request`](Self::request) replaces it.
    pub fn new(label: impl Into<Label>, list: Bind<'a, FileList>) -> Self {
        FileListPicker {
            label: label.into(),
            list,
            request: PickRequest::open_many("stack"),
            folder_mode: false,
            open_files: &[],
            folder_files: &[],
            info: None,
        }
    }

    /// Shows the picker. Takes the answer of its own Browse ticket from the host's file service.
    pub fn show(self, ui: &mut Ui) -> FileListPickerOutput {
        let FileListPicker { label, list, request, folder_mode, open_files, folder_files, info } = self;
        let words = Env::of(ui).words::<FileListWords>();
        let pass = RefCell::new(Pass { held: Held::new(list), seen: None, browsing: None });
        let mut stack = Stack::new().fill();
        if folder_mode {
            stack = stack.header(|ui, cx| {
                let mut pass = pass.borrow_mut();
                let now = pass.load(cx);
                let modes = [Entry::new(FileListMode::Files, words.files.clone()), Entry::new(FileListMode::Folder, words.folder.clone())];
                Field::new(words.use_label.clone()).id_salt("use").show(ui, |ui, _| {
                    if let Some(mode) = Select::new(words.use_label.clone(), Bind::given(now.mode), &modes).id_salt("mode").show(ui).value().copied()
                    {
                        now.mode = mode;
                        now.selected = None;
                    }
                });
            });
        }
        let shown = stack
            .footer(|ui, cx| {
                let mut pass = pass.borrow_mut();
                let host = cx.env.host().cloned();
                // The footer is measured in a sizing pass before it is shown: that pass only reads the list, so the
                // answer is taken and the binding written once, in the shown pass.
                let sizing = ui.is_sizing_pass();
                let mut measured;
                let now = if sizing {
                    measured = pass.peek(cx);
                    &mut measured
                } else {
                    pass.load(cx)
                };
                let folder = now.mode == FileListMode::Folder;
                let (mut browse, mut remove, mut add_open, mut sort) = (false, false, false, false);
                Row::new().show(ui, |ui, _| {
                    let button = |ui: &mut Ui, label: &Label, key: &'static str, enabled: bool| {
                        clicked(&Button::new(label.clone()).enabled(enabled).id_salt(key).show(ui))
                    };
                    browse = button(ui, &words.browse, "browse", true);
                    remove = button(ui, &words.remove, "remove", !folder && now.selected.is_some());
                    add_open = button(ui, &words.add_open_files, "add-open", !folder && !open_files.is_empty());
                    sort = button(ui, &words.sort_by_name, "sort", !folder && now.files.len() > 1);
                });
                if sizing {
                    return;
                }
                if remove && let Some(path) = now.selected.take() {
                    now.files.retain(|file| *file != path);
                }
                if add_open {
                    now.add(open_files.iter().cloned());
                }
                if sort {
                    now.files.sort_by_key(|path| file_name(path).to_lowercase());
                    now.selected = None;
                }
                if browse && let Some(host) = &host {
                    let asked = if folder { PickRequest::folder(request.purpose.clone()) } else { request.clone() };
                    let kind = asked.kind;
                    let id = host.files().pick(asked);
                    cx.memory::<Ticket>().0 = Some((id, kind));
                    pass.browsing = Some(id);
                }
                pass.finish(cx, label.as_str());
            })
            .show(ui, |ui, cx| {
                let mut pass = pass.borrow_mut();
                let now = pass.load(cx);
                let folder = now.mode == FileListMode::Folder;
                if folder && let Some(path) = &now.folder {
                    Text::new(path.to_string_lossy().into_owned()).shade(Shade::Muted).show(ui);
                }
                let paths: &[PathBuf] = if folder { folder_files } else { &now.files };
                let source = Rows::new(paths);
                let mut selection =
                    now.selected.as_ref().filter(|path| paths.contains(path)).map_or_else(Selection::default, |path| Selection::single(path.clone()));
                let list = List::new(words.files.clone()).id_salt("files");
                let list = if folder {
                    list.select(SelectMode::None)
                } else {
                    list.select(SelectMode::Single).deletable(true).reorder(ReorderRule::new()).selection(Bind::bound(&mut selection))
                };
                let events = list.show_items(ui, &source, &mut |path, _| file_row(path, info)).events;
                if folder {
                    return;
                }
                now.selected = selection.items.first().cloned();
                for event in events {
                    match event {
                        CollectionEvent::Delete(keys) => {
                            now.files.retain(|path| !keys.contains(path));
                            now.selected = None;
                        }
                        CollectionEvent::Reorder(EditEvent::Set { after, .. } | EditEvent::Commit { after, .. }) => {
                            move_files(&mut now.files, &after.keys, after.to);
                            now.selected = Some(after.key.clone());
                        }
                        _ => {}
                    }
                }
            });
        let pass = pass.into_inner();
        let changed = pass.seen.as_ref().is_some_and(|(before, now)| before != now);
        FileListPickerOutput { response: shown.output, changed, browsing: pass.browsing }
    }
}

/// What the regions of one pass share: the bound value, its working copy and the ticket this pass issued.
struct Pass<'h> {
    held: Held<'h, FileList>,
    /// The value at the start of the pass and its working copy.
    seen: Option<(FileList, FileList)>,
    browsing: Option<PickId>,
}

impl Pass<'_> {
    /// The working copy, read and given the pick answer on the region that runs first.
    /// Every load asks for the answer: a measuring run gets none, so the run that receives it puts it in the copy.
    fn load(&mut self, cx: &mut BodyCx<'_>) -> &mut FileList {
        let held = &mut self.held;
        let (_, now) = self.seen.get_or_insert_with(|| {
            let before = held.read(cx).unwrap_or_default();
            let mut now = before.clone();
            if now.selected.as_ref().is_some_and(|path| !now.files.contains(path)) {
                now.selected = None;
            }
            (before, now)
        });
        take_answer(cx, now);
        now
    }

    /// The working copy as far as it is known, without taking the pick answer.
    fn peek(&mut self, cx: &mut BodyCx<'_>) -> FileList {
        match &self.seen {
            Some((_, now)) => now.clone(),
            None => self.held.read(cx).unwrap_or_default(),
        }
    }

    /// Writes the working copy to the binding when the pass changed it.
    fn finish(&mut self, cx: &mut BodyCx<'_>, name: &str) {
        if let Some((before, now)) = &self.seen
            && before != now
        {
            let (before, now) = (before.clone(), now.clone());
            self.held.write(cx, &before, now, name);
        }
    }
}

/// Puts the answer of the ticket this picker kept into `now`.
fn take_answer(cx: &mut BodyCx<'_>, now: &mut FileList) {
    let Some((ticket, kind)) = cx.memory::<Ticket>().0 else { return };
    let Some(picked) = cx.take_pick_answer(ticket) else { return };
    cx.memory::<Ticket>().0 = None;
    if kind == PickKind::Folder {
        if let Some(folder) = picked.paths().into_iter().next() {
            now.folder = Some(folder);
        }
    } else {
        now.add(picked.paths());
    }
}

/// The files as the list's item source: a file's path is its key (the paths in a list are distinct), so the lead and the
/// selection follow a file when the list reorders.
struct Rows<'a> {
    paths: &'a [PathBuf],
    names: Vec<String>,
}

impl<'a> Rows<'a> {
    fn new(paths: &'a [PathBuf]) -> Self {
        Rows { paths, names: paths.iter().map(|path| file_name(path)).collect() }
    }
}

impl DisplayOrder<PathBuf> for Rows<'_> {
    fn len(&self) -> usize {
        self.paths.len()
    }

    fn key(&self, at: usize) -> PathBuf {
        self.paths[at].clone()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.names.get(at).map(|name| Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<PathBuf> for Rows<'_> {}

/// One file's row: its name, the path for a screen reader, the app's info line under the name.
fn file_row<'a>(path: &Path, info: Option<InfoLine<'_>>) -> ListRow<'a> {
    let name = file_name(path);
    let line = info.and_then(|info| info(path));
    let accessible = match &line {
        Some(line) => format!("{name}, {line}"),
        None => name.clone(),
    };
    let full = path.to_string_lossy().into_owned();
    let row = ListRow::new(name).accessible_name(accessible).description(full.clone()).tooltip(full);
    match line {
        Some(line) => row.detail(line),
        None => row,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy()).into_owned()
}

/// Moves the files at `keys` (display order) together, so the first of them lands at `to` in the order that remains
/// once all of them are taken out.
fn move_files(files: &mut Vec<PathBuf>, keys: &[PathBuf], to: usize) {
    let mut places: Vec<usize> = keys.iter().filter_map(|key| files.iter().position(|path| path == key)).collect();
    places.sort_unstable();
    places.dedup();
    let moved: Vec<PathBuf> = places.iter().map(|&at| files[at].clone()).collect();
    for &at in places.iter().rev() {
        files.remove(at);
    }
    let to = to.min(files.len());
    files.splice(to..to, moved);
}
