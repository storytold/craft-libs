//! [`FileChooser`]: the in-app file chooser a pick falls back to when no native dialog can open.
//!
//! The folder reading is craft-browse's: the chooser asks its [`FolderCache`] for a folder's entries
//! ([`FolderCache::entries`]), which lists them in a job, with the hidden rule and the natural order of the places
//! tree, and answers `Loading` until the job lands. No folder is read in the pass.

mod accept;
#[cfg(test)]
pub(crate) mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use craft_behavior::{IconId, Selection, SelectionChange, keyed};
use craft_browse::places::{Entries, Entry, FolderCache, StandardFolder, folder_within};
use craft_host::{FileFilter, PickKind, PickRequest};
use craft_ui::bars::{BreadcrumbBar, path_crumbs};
use craft_ui::contract::{Bind, CollectionEvent, Entry as Choice, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::dialog::{
    ChildDialog, Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, PromptChoice, StackDialog, StandardPrompt, dialog_id,
};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::list::{List, ListRow};
use craft_ui::pickers::Select;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::text::TextField;
use egui::{Context, Ui};
use serde_json::{Value, json};

use self::accept::{Accept, accept, needs_home, typed_path};
use super::UnavailableWords;
use crate::Answer;

/// At most this many entries of a folder are read, so a huge folder cannot stall the listing job.
// ui-egui: crates/ui-egui/src/file_browser.rs:22 (`MAX_SCANNED`).
const SCANNED: usize = 100_000;

/// The rows the list shows before it scrolls: the list owns its scroll, so the dialog's body does not scroll along it.
const LIST_ROWS: u16 = 12;

/// At most this many entries are shown; a bigger folder shows the first ones and says so.
// ui-egui: crates/ui-egui/src/file_browser.rs:20 (`MAX_LISTED`).
const SHOWN: usize = 5_000;

craft_ui::words! {
    /// The words of the [`FileChooser`].
    pub struct ChooserWords {
        open_title: "fileChooser.openTitle" = "Open";
        save_title: "fileChooser.saveTitle" = "Save As";
        folder_title: "fileChooser.folderTitle" = "Choose a Folder";
        open: "fileChooser.open" = "Open";
        save: "fileChooser.save" = "Save";
        choose: "fileChooser.choose" = "Choose";
        location: "fileChooser.location" = "Folder path";
        path: "fileChooser.path" = "Folder";
        home: "fileChooser.home" = "Home";
        entries: "fileChooser.entries" = "Files and folders";
        name: "fileChooser.name" = "File name";
        types: "fileChooser.types" = "File type";
        all_files: "fileChooser.allFiles" = "All files";
        loading: "fileChooser.loading" = "Reading the folder…";
        empty: "fileChooser.empty" = "No items to show.";
        cant_read: template "fileChooser.cantRead" = "This folder can't be read: {error}";
        not_found: template "fileChooser.notFound" = "{name} wasn't found.";
        truncated: template "fileChooser.truncated" = "Showing the first {count} items.";
    }
}

/// What the chooser's listing of the shown folder is in this pass.
#[derive(Clone, Debug, PartialEq)]
enum View {
    /// No folder yet: the home folder is still being looked up.
    Wait,
    /// The listing job runs.
    Loading,
    /// The folder can't be read; the system's message.
    Failed(String),
    /// The entries are ready; `truncated` when the folder had more than were read or shown.
    Ready { truncated: bool },
}

/// The view of a listing for a pick of `kind` under `filter`, and the entries to show: folders first, then files, each
/// group in the listing's order, at most [`SHOWN`].
fn shape(kind: PickKind, filter: Option<&FileFilter>, entries: Entries<'_>) -> (View, Vec<Entry>) {
    match entries {
        Entries::Loading => (View::Loading, Vec::new()),
        Entries::Failed(message) => (View::Failed(message.to_owned()), Vec::new()),
        Entries::Ready { entries, truncated } => {
            let mut matched: Vec<&Entry> = entries.iter().filter(|entry| shows(kind, filter, entry)).collect();
            matched.sort_by_key(|entry| !entry.folder);
            let more = matched.len() > SHOWN;
            let shown = matched.into_iter().take(SHOWN).cloned().collect();
            (View::Ready { truncated: truncated || more }, shown)
        }
    }
}

/// The Replace question as a child dialog. A host keeps a child's answer for the app, not for its parent, so the
/// prompt files its choice where the chooser reads it on its next pass.
struct ReplaceAsk {
    prompt: StandardPrompt,
    answer: Arc<Mutex<Option<PromptChoice>>>,
}

impl ReplaceAsk {
    fn file(&self, choice: PromptChoice) {
        *self.answer.lock().unwrap_or_else(PoisonError::into_inner) = Some(choice);
    }
}

impl StackDialog for ReplaceAsk {
    type Result = ();

    fn id(&self) -> DialogId {
        StackDialog::id(&self.prompt)
    }

    fn instance(&self) -> Option<DialogId> {
        StackDialog::instance(&self.prompt)
    }

    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<()>> {
        let done = StackDialog::show(&mut self.prompt, ui, stack)?;
        self.file(match done {
            DialogResult::Ok(answer) => answer.choice,
            _ => PromptChoice::Cancel,
        });
        Some(DialogResult::Ok(()))
    }

    fn confirm(&mut self, ctx: &Context) -> Option<()> {
        let answer = StackDialog::confirm(&mut self.prompt, ctx)?;
        self.file(answer.choice);
        Some(())
    }

    fn json_fields(&self) -> Option<Value> {
        StackDialog::json_fields(&self.prompt)
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        StackDialog::set_json_field(&mut self.prompt, ctx, field, value)
    }
}

/// What the body reported in its last run of a pass.
#[derive(Default)]
struct Seen {
    /// Enter in the folder field.
    location_submitted: bool,
    /// Enter in the name field.
    name_submitted: bool,
    /// The name field was edited.
    name_edited: bool,
    /// The crumb opened.
    crumb: Option<String>,
    /// The list's selection changes, in order.
    selection: Vec<SelectionChange<PathBuf>>,
    /// The row activated (Enter or a double click).
    activated: Option<PathBuf>,
}

/// The fallback file chooser: a folder field, a breadcrumb bar from the home root, a list of the folder's entries,
/// name and type rows for a Save, and the line that says why the native dialog is unavailable. It reads folders only
/// from the [`FolderCache`] its caller gives it.
///
/// - Start: in the request's `directory` (the service resolves it as it does for a native panel), else the Home root
///   of [`FolderCache::roots`].
/// - Listing: folders always show; files show when they match the type picked (a folder pick shows no files). At
///   most 5 000 show, with a note when there were more or the folder was read only in part. The folder is listed
///   again when it is entered and when the window regains focus.
/// - OK, Enter and a double click on a file accept: a folder is entered, a file is opened, a Save to a listed file
///   asks Replace once, a typed path elsewhere is listed first and accepted there. The result is
///   `Ok(Answer::Paths(..))` for Open and Folder, `Ok(Answer::Saved { .. })` with the type picked for Save; Cancel,
///   Escape and the close action give `Cancel`.
/// - Automation fields: `folder`, `name` and `selection` (names in the shown folder); `listed` (read only) says the
///   listing is ready, and `replace` (read only) names the file the open Replace question is about. The question is a
///   child prompt: it is answered as every prompt is.
/// - A second pass in a frame applies only its own input: the listing lands through the cache, and a typed name
///   waiting on it is accepted once, then dropped.
#[derive(craft_ui::contract::Pattern)]
#[pattern(dialog)]
pub struct FileChooser {
    request: PickRequest,
    cause: Option<String>,
    folders: FolderCache,
    /// The shown folder; `None` until the home folder is known.
    folder: Option<PathBuf>,
    /// The folder field's text.
    location: String,
    /// The name field's text.
    name: String,
    /// The type picked: an index into the request's filters; past them, every file.
    filter: usize,
    selection: Selection<PathBuf>,
    /// The file whose name the selection put in the name field: its exact path stays the answer while the field
    /// holds that name, even after a relisting leaves it out.
    chosen: Option<Entry>,
    /// Typed text waiting on a listing (or on the home folder) before it is accepted.
    pending: Option<String>,
    /// The name the last accept did not find.
    note: Option<String>,
    /// The path the Replace question is open for, and where its answer lands.
    replacing: Option<(PathBuf, Arc<Mutex<Option<PromptChoice>>>)>,
    /// How many Replace questions were asked: each gets a prompt of its own.
    asks: u64,
    /// The Replace question the host opens as a child after this pass.
    child: Option<ChildDialog>,
}

impl FileChooser {
    /// A chooser for `request`. `cause` is the text of the `Picked::Unavailable` that opened it;
    /// `folders` is the caller's folder source.
    pub fn new(request: PickRequest, cause: Option<String>, folders: FolderCache) -> Self {
        // PLAN(platform-w4): start folder resolved in pump_with
        let folder = request.directory.clone();
        let name = if request.kind == PickKind::Save { request.file_name.clone().unwrap_or_default() } else { String::new() };
        let location = folder.as_deref().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        Self {
            request,
            cause,
            folders,
            folder,
            location,
            name,
            filter: 0,
            selection: Selection::default(),
            chosen: None,
            pending: None,
            note: None,
            replacing: None,
            asks: 0,
            child: None,
        }
    }

    /// The home folder: `None` until the roots are listed, then the Home root if there is one.
    fn home(&mut self) -> Option<Option<PathBuf>> {
        let roots = self.folders.roots()?;
        Some(roots.standard.iter().find(|(kind, _)| *kind == StandardFolder::Home).map(|(_, path)| path.clone()))
    }

    /// Shows `folder`: listed again, with nothing selected and no note.
    fn navigate(&mut self, folder: PathBuf) {
        self.folders.refresh_folder(&folder);
        self.location = folder.to_string_lossy().into_owned();
        self.folder = Some(folder);
        self.selection = Selection::default();
        self.chosen = None;
        self.note = None;
    }

    /// Whether the type selector shows: a Save with types, or an Open with a choice of them.
    /// A Save's answer for `path`, with the file type picked when the request offers types.
    fn saved(&self, path: PathBuf) -> Answer {
        Answer::Saved { path, filter: (!self.request.filters.is_empty()).then_some(self.filter) }
    }

    fn offers_types(&self) -> bool {
        !self.request.filters.is_empty() && (self.request.kind == PickKind::Save || self.request.filters.len() > 1)
    }

    /// The view of the shown folder and its entries to show, from the cache's answer now (queues the listing).
    fn list(&mut self) -> (View, Vec<Entry>) {
        let Some(folder) = self.folder.clone() else { return (View::Wait, Vec::new()) };
        shape(self.request.kind, self.request.filters.get(self.filter), self.folders.entries(&folder, SCANNED))
    }

    /// [`list`](Self::list) from what the cache holds, for readers that cannot borrow it mutably.
    fn peek(&self) -> (View, Vec<Entry>) {
        let Some(folder) = self.folder.as_deref() else { return (View::Wait, Vec::new()) };
        let held = self.folders.held_entries(folder, SCANNED).unwrap_or(Entries::Loading);
        shape(self.request.kind, self.request.filters.get(self.filter), held)
    }

    /// The `rows` that are selected; when none is, the chosen file whose name `typed` still is.
    fn selected(&self, rows: &[Entry], typed: &str) -> Vec<Entry> {
        let shown: Vec<Entry> = rows.iter().filter(|entry| self.selection.contains(&entry.path)).cloned().collect();
        if shown.is_empty() { self.chosen.iter().filter(|file| file.name == typed).cloned().collect() } else { shown }
    }

    /// Accepts `typed`: the answer when the chooser finishes, else `None` (it moved, asks Replace, or says why not).
    /// Waits for the listing, and for the home folder when the text starts from it.
    fn accept_typed(&mut self, typed: String) -> Option<DialogResult<Answer>> {
        let home = self.home();
        let waits_for_home = home.is_none() && needs_home(typed.trim());
        let (view, rows) = self.list();
        let ready = matches!(view, View::Ready { .. });
        let Some(folder) = self.folder.clone().filter(|_| ready && !waits_for_home) else {
            self.pending = Some(typed);
            return None;
        };
        let selected = self.selected(&rows, &typed);
        let picked_folder = matches!(selected.as_slice(), [one] if one.folder);
        match accept(&self.request, &typed, &folder, &rows, &selected, home.flatten().as_deref()) {
            Accept::Open(paths) => return Some(DialogResult::Ok(Answer::Paths(paths))),
            Accept::Save(path) => return Some(DialogResult::Ok(self.saved(path))),
            Accept::Replace(path) => {
                self.asks += 1;
                let name = path.file_name().map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy()).into_owned();
                let answer = Arc::new(Mutex::new(None));
                let prompt = StandardPrompt::replace_file(name, None).instance(("craft-dialogs.chooser", self.asks));
                self.child = Some(ChildDialog::new(ReplaceAsk { prompt, answer: answer.clone() }));
                self.replacing = Some((path, answer));
            }
            Accept::Enter(path) => {
                // A typed folder name is used up by entering it; the name a selected folder leaves alone stays.
                if !picked_folder {
                    self.name.clear();
                }
                self.navigate(path);
            }
            Accept::Look { folder, name } => {
                let name = name.to_string_lossy().into_owned();
                if self.request.kind != PickKind::Folder {
                    self.name.clone_from(&name);
                }
                self.navigate(folder);
                self.pending = Some(name);
            }
            Accept::Refuse(name) => self.note = Some(name).filter(|name| !name.is_empty()),
        }
        None
    }

    /// Applies one change of the list's selection over `rows`; a selected file's name goes into the name field.
    fn select(&mut self, change: SelectionChange<PathBuf>, rows: &[Entry]) {
        match change {
            SelectionChange::Set(keys) => {
                let lead = keys.last().cloned();
                self.selection = Selection::new(keys, lead.clone(), lead);
            }
            SelectionChange::Toggle(key) => {
                if self.selection.contains(&key) {
                    self.selection.items.retain(|item| *item != key);
                } else {
                    self.selection.items.push(key.clone());
                    self.selection.lead = Some(key);
                }
            }
            SelectionChange::Clear => self.selection = Selection::default(),
            _ => return,
        }
        self.note = None;
        let lead = self.selection.lead.clone().filter(|lead| self.selection.contains(lead));
        if let Some(file) = lead.and_then(|lead| rows.iter().find(|entry| entry.path == lead && !entry.folder))
            && self.request.kind != PickKind::Folder
        {
            self.name.clone_from(&file.name);
            self.chosen = Some(file.clone());
        }
    }

    /// The crumbs of the shown folder: from the Home root when the folder lies in it, else from the top.
    fn crumbs(&mut self, words: &ChooserWords) -> Vec<Choice<String>> {
        let Some(folder) = self.folder.clone() else { return Vec::new() };
        let all = path_crumbs(&folder.to_string_lossy());
        match self.home().flatten().filter(|home| folder_within(&folder, home)) {
            Some(home) => {
                let below = all.into_iter().filter(|crumb| crumb.value().is_some_and(|path| !folder_within(&home, Path::new(path))));
                std::iter::once(Choice::new(home.to_string_lossy().into_owned(), words.home.clone())).chain(below).collect()
            }
            None => all,
        }
    }

    /// Shows one pass: the chooser, and over it the Replace question while one is open.
    fn present(&mut self, ui: &mut Ui) -> Option<DialogResult<Answer>> {
        // Folders are not watched, so the listings go stale when the window regains focus, as the places tree's do:
        // files changed in another app show on return. egui-winit sends `WindowFocused` from
        // `State::on_window_event` (`WindowEvent::Focused`), and only to a frame's first pass, so a second pass does not
        // refresh again.
        if ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(true)))) {
            self.folders.refresh();
        }
        if self.folder.is_none()
            && let Some(home) = self.home()
        {
            self.navigate(home.unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR)));
        }
        let (mut view, mut rows) = self.list();
        if matches!(view, View::Ready { .. })
            && let Some(typed) = self.pending.take()
        {
            if let Some(done) = self.accept_typed(typed) {
                return Some(done);
            }
            // The accept may have moved to another folder: show that one's listing.
            (view, rows) = self.list();
        }

        let env = Env::of(ui);
        let words = env.words::<ChooserWords>();
        let common = env.words::<CommonWords>();
        let unavailable = env.words::<UnavailableWords>();
        let crumbs = self.crumbs(&words);
        let kind = self.request.kind;
        let (title, ok) = match kind {
            PickKind::Open | PickKind::OpenMany => (words.open_title.clone(), words.open.clone()),
            PickKind::Save => (words.save_title.clone(), words.save.clone()),
            PickKind::Folder => (words.folder_title.clone(), words.choose.clone()),
        };
        let title = self.request.title.clone().map_or_else(|| title, craft_ui::runtime::env::Label::from);
        let mut types: Vec<Choice<usize>> =
            self.request.filters.iter().enumerate().map(|(at, filter)| Choice::new(at, filter.name.clone())).collect();
        types.push(Choice::new(self.request.filters.len(), words.all_files.clone()));
        let offers_types = self.offers_types();
        let names_file = kind != PickKind::Folder;
        let filter_before = self.filter;
        let buttons = [FooterButton::new(ok, DialogRole::Accept).stays_open(true), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];

        let selection_shown = self.selection.clone();
        let FileChooser { cause, location, name, filter, note, .. } = self;
        let entries = &rows;
        let shown = Dialog::new(title).resizable(true).buttons(buttons).show(ui, |ui, _| {
            let mut seen = Seen::default();
            if let Some(cause) = cause.as_deref() {
                Text::new(unavailable.sentence(cause)).show(ui);
            }
            // Not a `PathField`: its Browse button picks through `host.files()`, which from inside the fallback
            // chooser would open another pick. The folder row is a text field alone, Enter navigating.
            Field::new(words.location.clone()).id_salt("location").show(ui, |ui, _| {
                seen.location_submitted = TextField::new(words.location.clone(), Bind::bound(&mut *location)).show(ui).submitted;
            });
            // PLAN: `fair_widths` in ui-widgets `bars/breadcrumb.rs:143` floors a crumb's natural width, so a crumb with a
            // fractional width reads as cut (`width < full`) and elides with room to spare; owner: the ui-widgets set.
            seen.crumb = BreadcrumbBar::new(words.path.clone(), &crumbs).show(ui).picked;
            match &view {
                View::Wait | View::Loading => {
                    Text::new(words.loading.clone()).show(ui);
                }
                View::Failed(error) => {
                    Text::new(words.cant_read.fill(&[("error", error.as_str())])).show(ui);
                }
                View::Ready { .. } if entries.is_empty() => {
                    Text::new(words.empty.clone()).show(ui);
                }
                View::Ready { truncated } => {
                    let source = keyed(entries.as_slice(), |entry: &Entry| entry.path.clone());
                    let select = if kind == PickKind::OpenMany { SelectMode::Multi } else { SelectMode::Single };
                    let events = List::new(words.entries.clone())
                        .select(select)
                        .type_ahead(true)
                        .max_rows(LIST_ROWS)
                        .selection(Bind::given(selection_shown.clone()))
                        .show_items(ui, &source, &mut |path, _| {
                            let entry = entries.iter().find(|entry| entry.path == *path);
                            let icon = if entry.is_some_and(|entry| entry.folder) { "folder" } else { "file" };
                            ListRow::new(entry.map_or_else(String::new, |entry| entry.name.clone())).icon(IconSpec::svg(IconId::new(icon)))
                        })
                        .events;
                    for event in events {
                        match event {
                            CollectionEvent::Selection { change, .. } => seen.selection.push(change),
                            CollectionEvent::Activate(path) => seen.activated = Some(path),
                            _ => {}
                        }
                    }
                    if *truncated {
                        Text::new(words.truncated.fill(&[("count", entries.len().to_string().as_str())])).show(ui);
                    }
                }
            }
            if let Some(missing) = note.as_deref() {
                Text::new(words.not_found.fill(&[("name", missing)])).show(ui);
            }
            if names_file {
                Field::new(words.name.clone()).id_salt("name").show(ui, |ui, _| {
                    let edited = TextField::new(words.name.clone(), Bind::bound(&mut *name)).show(ui);
                    seen.name_submitted = edited.submitted;
                    seen.name_edited = !edited.events.is_empty();
                });
            }
            if offers_types {
                Field::new(words.types.clone()).id_salt("types").show(ui, |ui, _| {
                    Select::new(words.types.clone(), Bind::bound(&mut *filter), &types).show(ui);
                });
            }
            seen
        });

        if let Some((path, answer)) = self.replacing.clone() {
            let given = answer.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some(choice) = given {
                self.replacing = None;
                if choice == PromptChoice::Proceed {
                    return Some(DialogResult::Ok(self.saved(path)));
                }
            }
            return None;
        }
        if shown.output.closed.is_some() {
            return Some(DialogResult::Cancel);
        }
        let seen = shown.inner.unwrap_or_default();
        if self.filter != filter_before {
            self.selection = Selection::default();
        }
        if seen.name_edited {
            self.selection = Selection::default();
            self.chosen = None;
            self.note = None;
        }
        for change in seen.selection {
            self.select(change, &rows);
        }
        if let Some(crumb) = seen.crumb {
            self.navigate(PathBuf::from(crumb));
            return None;
        }
        if seen.location_submitted {
            let text = self.location.trim().to_owned();
            let home = self.home().flatten();
            if let Some(folder) = self.folder.clone().or_else(|| home.clone())
                && let Some(to) = typed_path(&text, &folder, home.as_deref())
            {
                self.navigate(to);
            }
            return None;
        }
        if let Some(path) = seen.activated {
            let entry = rows.iter().find(|entry| entry.path == path).cloned()?;
            if entry.folder {
                self.navigate(entry.path);
                return None;
            }
            self.selection = Selection::single(entry.path.clone());
            self.name.clone_from(&entry.name);
            let typed = entry.name.clone();
            self.chosen = Some(entry);
            return self.accept_typed(typed);
        }
        if shown.output.picked == Some(0) || seen.name_submitted {
            let typed = self.name.clone();
            return self.accept_typed(typed);
        }
        None
    }
}

/// Whether `entry` shows under `filter`: a folder always does; a file when the pick takes files and it has one of the
/// filter's extensions (any case; `*` takes any), or with no filter.
fn shows(kind: PickKind, filter: Option<&FileFilter>, entry: &Entry) -> bool {
    // ui-egui: crates/ui-egui/src/file_browser.rs:50 (`matches`).
    if entry.folder {
        return true;
    }
    if kind == PickKind::Folder {
        return false;
    }
    let Some(filter) = filter else { return true };
    let extension = Path::new(&entry.name).extension().and_then(|e| e.to_str());
    filter.extensions.iter().any(|wanted| wanted == "*" || extension.is_some_and(|ext| ext.eq_ignore_ascii_case(wanted)))
}

impl StackDialog for FileChooser {
    type Result = Answer;

    fn id(&self) -> DialogId {
        dialog_id("fileChooser")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Answer>> {
        let done = self.present(ui);
        self.folders.finish_pass();
        done
    }

    /// OK as its button gives it: the answer when the typed name or the selection is one, else `None` (the chooser
    /// moved, waits for its listing, asks Replace or shows why not).
    fn confirm(&mut self, _: &Context) -> Option<Answer> {
        if self.replacing.is_some() {
            return None;
        }
        let typed = self.name.clone();
        match self.accept_typed(typed)? {
            DialogResult::Ok(answer) => Some(answer),
            _ => None,
        }
    }

    fn take_child(&mut self) -> Option<ChildDialog> {
        self.child.take()
    }

    fn json_fields(&self) -> Option<Value> {
        let (view, rows) = self.peek();
        let selection: Vec<&str> = self.selected_names(&rows);
        let mut fields = json!({
            "folder": self.folder.as_deref().map(|f| f.to_string_lossy()),
            "name": self.name,
            "selection": selection,
            "listed": matches!(view, View::Ready { .. }),
        });
        if let Some((path, _)) = &self.replacing {
            fields["replace"] = json!(path.to_string_lossy());
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "folder" => {
                let text = value.as_str().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?.trim().to_owned();
                let home = self.home().flatten();
                let base = self.folder.clone().or_else(|| home.clone()).unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR));
                let to = typed_path(&text, &base, home.as_deref()).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.navigate(to);
            }
            "name" if self.request.kind != PickKind::Folder => {
                let text = value.as_str().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.name = text.to_owned();
                self.selection = Selection::default();
                self.note = None;
            }
            "name" => return Err(FieldRefusal::Inactive.message(field)),
            "selection" => {
                let names: Vec<String> = match &value {
                    Value::String(one) => vec![one.clone()],
                    Value::Array(many) => many
                        .iter()
                        .map(|v| v.as_str().map(str::to_owned))
                        .collect::<Option<_>>()
                        .ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?,
                    _ => return Err(FieldRefusal::Rejects(&value).message(field)),
                };
                let (_, rows) = self.list();
                let keys: Option<Vec<PathBuf>> =
                    names.iter().map(|name| rows.iter().find(|entry| entry.name == *name).map(|entry| entry.path.clone())).collect();
                let keys = keys.filter(|keys| keys.len() <= 1 || self.request.kind == PickKind::OpenMany);
                let Some(keys) = keys else { return Err(FieldRefusal::Rejects(&value).message(field)) };
                self.select(if keys.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(keys) }, &rows);
            }
            "listed" | "replace" => return Err(FieldRefusal::ReadOnly.message(field)),
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

impl FileChooser {
    /// The names of the selected `rows`, for automation.
    fn selected_names<'a>(&self, rows: &'a [Entry]) -> Vec<&'a str> {
        rows.iter().filter(|entry| self.selection.contains(&entry.path)).map(|entry| entry.name.as_str()).collect()
    }
}
