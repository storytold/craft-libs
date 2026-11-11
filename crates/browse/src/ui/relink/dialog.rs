//! `RelinkDialog`: the Link Media dialog for a document's missing links.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, IconId, ItemSource, Selection};
use craft_docpath::links::{
    Candidate, CandidateProblem, Link, LinkFiles, LinkSearch, MatchCriteria, SearchEnd, SearchHit, SearchUpdate, check_candidate, derive_remap, rank,
};
use craft_host::queue::{JobQueue, Priority, QueueOptions};
use craft_host::{Host, Jobs, PickId, PickRequest, Wake};
use craft_i18n::Format;
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, Politeness, Props as _, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, Sizing, announce, clicked,
};
use craft_ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::{Divider, Shade, Text};
use craft_ui::layout_components::Row;
use craft_ui::list::{List, ListRow};
use craft_ui::patterns::form_dialogs::{PreviewPane, PreviewState, PreviewView};
use craft_ui::primitives::IconSpec;
use craft_ui::primitives::textures::{TextureState, TextureWords, ThumbRequest, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::table::{Table, sort_rows};
use craft_ui::text::TextField;
use egui::load::SizedTexture;
use egui::{Context, Ui};

use crate::links::SearchRun;
use crate::thumbs::{FileThumbKey, FileThumbSource};

/// The id the dialog's widgets hang under, and its `DialogId` ([`dialog_id`] of it).
const KEY: &str = "relink";

/// The folder memory of the dialog's picks.
const PICK_PURPOSE: &str = "relink";

/// The longest side the preview asks its picture at, in points.
const PREVIEW_SIDE: f32 = 256.0;

const OWNER: ColumnKey = ColumnKey::new("owner");
const FILE_NAME: ColumnKey = ColumnKey::new("fileName");
const FILE_PATH: ColumnKey = ColumnKey::new("filePath");
const STATUS: ColumnKey = ColumnKey::new("status");

/// One missing link as the dialog's table shows it: Clip Name, File Name, File Path, Status.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelinkRow<K> {
    /// The link.
    pub link: Link<K>,
    /// The first column: the app's name for what uses the file (a clip, a graphic).
    pub owner: String,
    /// The Status column ("Media missing"), translated.
    pub status: String,
}

impl<K> RelinkRow<K> {
    /// A row of `link` with the app's name for its user and the translated status.
    pub fn new(link: Link<K>, owner: impl Into<String>, status: impl Into<String>) -> Self {
        Self { link, owner: owner.into(), status: status.into() }
    }
}

/// An app's own match property (a duration, a frame size and rate): a check row under the
/// library's name, extension and same-file checks.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelinkCriterion {
    /// The id the app's checker reads.
    pub id: &'static str,
    /// The check's text, translated.
    pub label: String,
    /// Whether it is on.
    pub on: bool,
}

impl RelinkCriterion {
    /// The check `id` shown as `label`, `on` at the start.
    pub fn new(id: &'static str, label: impl Into<String>, on: bool) -> Self {
        Self { id, label: label.into(), on }
    }
}

/// The app's extra checks of a candidate, after `craft_docpath::links::check_candidate`: it reads
/// the file (in a job) and adds `CandidateProblem::Other` entries and score. `Send + Sync`, run
/// in the search's and Locate's jobs.
pub trait CandidateChecker<K>: Send + Sync {
    /// Checks `candidate` (already through `check_candidate`) as the new file of `link` with the
    /// app's `criteria` on; returns it with its problems, `ok` and `score` updated.
    fn check(&self, link: &Link<K>, candidate: Candidate, criteria: &[RelinkCriterion]) -> Candidate;
}

/// The dialog's start values.
#[derive(Clone)]
#[non_exhaustive]
pub struct RelinkOptions {
    /// The library's match checks (default all on).
    pub criteria: MatchCriteria,
    /// The app's checks, in order.
    pub app_criteria: Vec<RelinkCriterion>,
    /// "Relink others automatically" (default on).
    pub relink_others: bool,
    /// "Exact name matches only" (default off).
    pub exact_names: bool,
    /// The search folder to start with (the document's folder, the last one used).
    pub folder: Option<PathBuf>,
    /// The document's path, for `resolve_absolute`.
    pub document: Option<PathBuf>,
    /// Jobs for the search and checks; `None` uses the host's jobs (stories pass a `ManualJobs`).
    ///
    /// Without jobs and without a host the dialog can't run a search: Search stays disabled.
    pub jobs: Option<Arc<dyn Jobs>>,
}

impl RelinkOptions {
    /// Starts the search in `folder`.
    #[must_use]
    pub fn folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.folder = Some(folder.into());
        self
    }

    /// Whether Relink others automatically starts on.
    #[must_use]
    pub fn relink_others(mut self, on: bool) -> Self {
        self.relink_others = on;
        self
    }

    /// Runs the search and checks on `jobs`.
    #[must_use]
    pub fn jobs(mut self, jobs: Arc<dyn Jobs>) -> Self {
        self.jobs = Some(jobs);
        self
    }
}

impl Default for RelinkOptions {
    fn default() -> Self {
        Self {
            criteria: MatchCriteria::default(),
            app_criteria: Vec::new(),
            relink_others: true,
            exact_names: false,
            folder: None,
            document: None,
            jobs: None,
        }
    }
}

/// What Done returns: every link the dialog made and every row left offline, applied by the
/// host as one undo step.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelinkOutcome<K> {
    /// The links relinked, each with its checked candidate (path and identity to store).
    pub linked: Vec<(K, Candidate)>,
    /// The links the user left offline.
    pub offline: Vec<K>,
}

craft_ui::words! {
    /// The dialog's words, English by default.
    #[non_exhaustive]
    pub struct RelinkWords {
        title: "relinkDialog.title" = "Link Media";
        message: "relinkDialog.message" = "Find the missing files, or leave them offline.";
        owner: "relinkDialog.owner" = "Clip Name";
        file_name: "relinkDialog.fileName" = "File Name";
        file_path: "relinkDialog.filePath" = "File Path";
        status: "relinkDialog.status" = "Status";
        match_heading: "relinkDialog.match" = "Match file properties";
        match_name: "relinkDialog.matchName" = "File Name";
        match_extension: "relinkDialog.matchExtension" = "File Extension";
        match_same_file: "relinkDialog.matchSameFile" = "Same File";
        relink_others: "relinkDialog.others" = "Relink others automatically";
        search_in: "relinkDialog.searchIn" = "Search in:";
        browse: "relinkDialog.browse" = "Browse…";
        exact_names: "relinkDialog.exactNames" = "Exact name matches only";
        search: "relinkDialog.search" = "Search";
        cancel_search: "relinkDialog.cancelSearch" = "Stop";
        search_started: "relinkDialog.searchStarted" = "Searching…";
        search_done: "relinkDialog.searchDone" = "Search finished";
        /// `{n}` the files indexed so far.
        searching: count "relinkDialog.searching" = "Searching… {n} file" | "Searching… {n} files";
        same_file: "relinkDialog.sameFile" = "same file";
        matches: "relinkDialog.matches" = "matches";
        different_file: "relinkDialog.differentFile" = "different file";
        not_found: "relinkDialog.notFound" = "No candidates found";
        name_differs: template "relinkDialog.nameDiffers" = "File name differs ({found} vs {wanted})";
        extension_differs: template "relinkDialog.extensionDiffers" = "Extension differs (.{found} vs .{wanted})";
        size_differs: template "relinkDialog.sizeDiffers" = "Not the same file: {found} bytes, the original had {wanted}";
        fingerprint_differs: "relinkDialog.fingerprintDiffers" = "Not the same file: the contents differ";
        unreadable: template "relinkDialog.unreadable" = "Can't read the file: {reason}";
        candidates: "relinkDialog.candidates" = "Candidates";
        preview: "relinkDialog.preview" = "Preview";
        no_preview: "relinkDialog.noPreview" = "No preview";
        locate: "relinkDialog.locate" = "Locate…";
        link: "relinkDialog.link" = "Link";
        link_anyway: "relinkDialog.linkAnyway" = "Link Anyway";
        offline: "relinkDialog.offline" = "Leave Offline";
        offline_all: "relinkDialog.offlineAll" = "Leave All Offline";
        done: "relinkDialog.done" = "Done";
    }
}

/// The Link Media dialog for a document's missing links; a `StackDialog` returning
/// [`RelinkOutcome`] (Done) or `Cancel` (Cancel, Esc).
///
/// - **Layout.** A `Dialog`: the message; the missing `Table` (owner, file name, path, status);
///   the match checks (library, then the app's [`RelinkCriterion`]s) and Relink others; "Search
///   in:" a `TextField` with Browse… (a folder pick through the host's files), Exact name matches
///   only, Search; the candidate `List` beside a preview of the selected candidate (a
///   `PreviewPane` over a `Thumbnails` cache of the `FileThumbSource` the host gives with
///   `.preview(..)`); the buttons Leave All Offline, Leave Offline, Locate…, Done, Cancel and Link.
/// - **Flow.** The table selects the first row; picking a row clears the candidates. Search
///   starts a [`SearchRun`] over the `LinkFiles`, on `RelinkOptions::jobs` or the host's (with
///   neither, Search is disabled); the progress line shows the count and a polite announcement
///   tells the start and the end only; Stop cancels; hits for the selected row fill the list as
///   they arrive, already through the app's checker; ranked (`rank`), a passing same file first.
///   Each candidate shows its mark (check or cross, in words) with its problems as the tooltip.
///   Locate… picks one file and adds it, checked in a job, as the selected candidate. Link (the
///   default button, which keeps the dialog open) links the selected candidate and moves to the
///   next missing row; when the candidate fails a check the button reads Link Anyway. With Relink
///   others, a link derives a `Remap` (`derive_remap`) and every other missing row it fits is
///   checked in a job and linked; rows that fail stay missing. Leave Offline and Leave All
///   Offline mark rows. Done returns the outcome; Cancel and Esc return nothing; Esc while a pick
///   is open closes the pick only. A network or device link is refused before any read.
/// - **Never blocks.** The search and every check are jobs; results arrive through `Wake`.
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(RelinkDialog<u32>))]
pub struct RelinkDialog<K> {
    id: DialogId,
    /// The dialog's open state; Esc with a pick open writes it false and the dialog sets it back.
    open: bool,
    model: Model<K>,
}

/// The row a missing link is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    Missing,
    Linked,
    Offline,
}

/// What a check job was asked for, so its answer finds its place.
#[derive(Clone, Copy, Debug)]
enum Purpose {
    /// A file the user located for `row`.
    Locate { row: usize, generation: u64 },
    /// The file a remap offers for `row` when another row was linked.
    Other { row: usize },
}

struct Checked {
    purpose: Purpose,
    candidate: Candidate,
}

/// A folder or file pick the user has open.
#[derive(Clone, Copy, Debug)]
enum Pick {
    Folder(PickId),
    Locate { id: PickId, row: usize },
}

impl Pick {
    fn id(self) -> PickId {
        match self {
            Pick::Folder(id) | Pick::Locate { id, .. } => id,
        }
    }
}

/// What a footer button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    OfflineAll,
    Offline,
    Locate,
    Link,
    Done,
    Cancel,
}

/// The queues and wake the dialog starts jobs with; made on the first pass, when the host or the
/// options give jobs.
struct Runtime {
    jobs: Arc<dyn Jobs>,
    wake: Wake,
    /// Checks of located files and of the files a remap offers; a search checks its own hits.
    checks: JobQueue<u64, Checked>,
}

/// The progress line's last announcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Announced {
    Nothing,
    Started,
    Ended,
}

struct Model<K> {
    rows: Vec<RelinkRow<K>>,
    fates: Vec<Fate>,
    files: Arc<dyn LinkFiles>,
    options: RelinkOptions,
    checker: Option<Arc<dyn CandidateChecker<K>>>,
    preview: Option<FileThumbSource>,
    runtime: Option<Runtime>,
    /// The host of the last pass, for the picks a cancel forgets.
    host: Option<Arc<Host>>,
    sort: Option<ColumnSort>,
    row_sel: Selection<usize>,
    cand_sel: Selection<PathBuf>,
    /// The candidate selection was made by the dialog (the best one so far), not by the user.
    auto_selected: bool,
    folder: String,
    search: Option<SearchRun<K>>,
    /// Files indexed by the running search.
    progress: usize,
    /// The last search ran to its end.
    searched: bool,
    announced: Announced,
    candidates: Vec<Candidate>,
    /// Bumped whenever the candidate list is emptied, so answers for an older list are dropped.
    generation: u64,
    ticket: u64,
    pick: Option<Pick>,
    linked: Vec<(K, Candidate)>,
    offline: Vec<K>,
}

impl<K: Clone + PartialEq + Send + Sync + 'static> RelinkDialog<K> {
    /// The dialog over the missing `rows`, reading files through `files`.
    pub fn new(rows: Vec<RelinkRow<K>>, files: Arc<dyn LinkFiles>, options: RelinkOptions) -> Self {
        let folder = options.folder.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
        let row_sel = if rows.is_empty() { Selection::default() } else { Selection::single(0) };
        let model = Model {
            fates: vec![Fate::Missing; rows.len()],
            rows,
            files,
            options,
            checker: None,
            preview: None,
            runtime: None,
            host: None,
            sort: None,
            row_sel,
            cand_sel: Selection::default(),
            auto_selected: true,
            folder,
            search: None,
            progress: 0,
            searched: false,
            announced: Announced::Nothing,
            candidates: Vec::new(),
            generation: 0,
            ticket: 0,
            pick: None,
            linked: Vec::new(),
            offline: Vec::new(),
        };
        Self { id: dialog_id(KEY), open: true, model }
    }

    /// The app's extra checks.
    #[must_use]
    pub fn checker(mut self, checker: Arc<dyn CandidateChecker<K>>) -> Self {
        self.model.checker = Some(checker);
        self
    }

    /// The source of the preview's picture of the selected candidate. Without one the preview
    /// stays empty.
    #[must_use]
    pub fn preview(mut self, source: FileThumbSource) -> Self {
        self.model.preview = Some(source);
        self
    }
}

#[cfg(feature = "screen-testing")]
impl<K: Clone + PartialEq + Send + Sync + 'static> RelinkDialog<K> {
    /// Starts the search as the Search button does, for a story that shows a search in progress.
    pub(super) fn begin_search(&mut self) {
        self.model.start_search();
    }
}

impl<K: Clone + PartialEq + Send + Sync + 'static> StackDialog for RelinkDialog<K> {
    type Result = RelinkOutcome<K>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<RelinkOutcome<K>>> {
        let env = Env::of(ui);
        let words = env.words::<RelinkWords>();
        let common = env.words::<CommonWords>();
        let model = &mut self.model;
        model.host = env.view().host().cloned();
        model.poll(ui.ctx());

        let open = model.fates.contains(&Fate::Missing);
        let candidate = model.selected_candidate();
        let link_label = match candidate {
            _ if !open => &words.done,
            Some(c) if !c.ok => &words.link_anyway,
            _ => &words.link,
        };
        let link_enabled = !open || candidate.is_some();
        let mut actions = vec![
            (Action::OfflineAll, FooterButton::new(words.offline_all.clone(), DialogRole::Other).stays_open(true).enabled(open)),
            (Action::Offline, FooterButton::new(words.offline.clone(), DialogRole::Other).stays_open(true).enabled(open)),
            (Action::Locate, FooterButton::new(words.locate.clone(), DialogRole::Other).stays_open(true).enabled(open && model.pick.is_none())),
        ];
        if open {
            actions.push((Action::Done, FooterButton::new(words.done.clone(), DialogRole::Other)));
        }
        actions.push((Action::Cancel, FooterButton::new(common.cancel.clone(), DialogRole::Cancel)));
        actions.push((Action::Link, FooterButton::new(link_label.clone(), DialogRole::Accept).stays_open(open).enabled(link_enabled)));
        let buttons: Vec<FooterButton> = actions.iter().map(|(_, button)| button.clone()).collect();

        let shown = Dialog::new(words.title.clone())
            .collection_body()
            .opened(Bind::bound(&mut self.open))
            .buttons(buttons)
            .show(ui, |ui, cx| model.body(ui, cx, &env, &words));
        let output = shown.output;
        if output.closed == Some(CloseReason::Escape) && model.pick.is_some() {
            // Esc while a pick is open closes the pick only.
            model.forget_pick();
            self.open = true;
            return None;
        }
        let picked = output.picked.and_then(|at| actions.get(at)).map(|(action, _)| *action);
        match picked {
            Some(Action::OfflineAll) => model.leave_all_offline(),
            Some(Action::Offline) => model.leave_selected_offline(),
            Some(Action::Locate) => model.locate(&words),
            Some(Action::Link) if open => model.link_selected(ui.ctx()),
            Some(Action::Link | Action::Done) => return Some(DialogResult::Ok(model.outcome())),
            Some(Action::Cancel) | None => {}
        }
        if output.closed.is_some() {
            model.search = None;
            model.forget_pick();
            return Some(DialogResult::Cancel);
        }
        None
    }

    fn cancelled(&mut self, _ctx: &Context) {
        self.model.search = None;
        self.model.forget_pick();
    }
}

/// The rows of the missing table in the order shown, keyed by the row's position in the dialog.
struct MissingRows(Vec<usize>);

impl DisplayOrder<usize> for MissingRows {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        self.0[at]
    }
}

impl ItemSource<usize> for MissingRows {}

/// The candidates as the list's item source: a candidate's path is its key.
struct CandidateRows<'a>(&'a [Candidate]);

impl DisplayOrder<PathBuf> for CandidateRows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> PathBuf {
        self.0[at].path.clone()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.0[at].path.display().to_string()))
    }
}

impl ItemSource<PathBuf> for CandidateRows<'_> {}

impl<K: Clone + PartialEq + Send + Sync + 'static> Model<K> {
    // --- state ---

    fn unresolved(&self) -> Vec<usize> {
        (0..self.rows.len()).filter(|&i| self.fates[i] == Fate::Missing).collect()
    }

    /// The row shown selected, while it is still missing.
    fn selected_row(&self) -> Option<usize> {
        self.row_sel.lead.filter(|&i| self.fates.get(i) == Some(&Fate::Missing))
    }

    fn selected_candidate(&self) -> Option<&Candidate> {
        let path = self.cand_sel.lead.as_ref()?;
        self.candidates.iter().find(|c| &c.path == path)
    }

    fn outcome(&mut self) -> RelinkOutcome<K> {
        self.search = None;
        RelinkOutcome { linked: std::mem::take(&mut self.linked), offline: std::mem::take(&mut self.offline) }
    }

    /// Empties the candidate list and stops the search: the selected row changed or was resolved.
    fn clear_candidates(&mut self) {
        self.candidates.clear();
        self.cand_sel = Selection::default();
        self.auto_selected = true;
        self.generation += 1;
        self.search = None;
        self.progress = 0;
        self.searched = false;
    }

    /// Selects the next missing row after `row` (wrapping), or none.
    fn advance(&mut self, row: usize) {
        self.clear_candidates();
        let missing = self.unresolved();
        let next = missing.iter().copied().find(|&i| i > row).or_else(|| missing.first().copied());
        self.row_sel = next.map_or_else(Selection::default, Selection::single);
    }

    fn add_candidate(&mut self, candidate: Candidate, chosen: bool) {
        let path = candidate.path.clone();
        self.candidates.retain(|c| c.path != path);
        self.candidates.push(candidate);
        rank(&mut self.candidates);
        if chosen {
            self.cand_sel = Selection::single(path);
            self.auto_selected = false;
        } else if self.cand_sel.items.is_empty() || self.auto_selected {
            self.cand_sel = Selection::single(self.candidates[0].path.clone());
            self.auto_selected = true;
        }
    }

    // --- jobs ---

    /// Makes the queues once jobs are known, then takes in whatever the jobs and picks finished.
    fn poll(&mut self, ctx: &Context) {
        if self.runtime.is_none()
            && let Some(jobs) = self.options.jobs.clone().or_else(|| self.host.as_ref().map(|host| Arc::clone(host.jobs_shared())))
        {
            let wake = {
                let ctx = ctx.clone();
                Wake::new(move || ctx.request_repaint())
            };
            let checks = JobQueue::new(Arc::clone(&jobs), wake.clone(), QueueOptions { lanes: 1, name: "relink-check" });
            self.runtime = Some(Runtime { jobs, wake, checks });
        }
        self.poll_pick();
        self.poll_search();
        self.poll_checks();
    }

    fn poll_pick(&mut self) {
        let (Some(pick), Some(host)) = (self.pick, self.host.clone()) else { return };
        let Some(answer) = host.files().take_answer(pick.id()) else { return };
        self.pick = None;
        let Some(path) = answer.first_path() else { return };
        match pick {
            Pick::Folder(_) => self.folder = path.display().to_string(),
            Pick::Locate { row, .. } => {
                let generation = self.generation;
                self.submit_check(Purpose::Locate { row, generation }, row, path);
            }
        }
    }

    fn poll_search(&mut self) {
        let Some(run) = self.search.as_mut() else { return };
        let end = run.end();
        let updates = run.take_updates();
        for update in updates {
            match update {
                SearchUpdate::Progress { files } => self.progress = files,
                SearchUpdate::Hit(hit) => self.on_hit(hit),
            }
        }
        if let Some(end) = end {
            self.search = None;
            self.searched = end != SearchEnd::Cancelled;
        }
    }

    /// A hit arrives checked: the search's checker hook ran in its job.
    fn on_hit(&mut self, hit: SearchHit<K>) {
        if self.selected_row().is_some() {
            self.add_candidate(hit.candidate, false);
        }
    }

    fn poll_checks(&mut self) {
        let Some(runtime) = &self.runtime else { return };
        let mut done = Vec::new();
        while let Some(finished) = runtime.checks.try_recv() {
            done.push(finished.result);
        }
        for Checked { purpose, candidate } in done {
            match purpose {
                Purpose::Locate { row, generation } if generation == self.generation && self.selected_row() == Some(row) => {
                    self.add_candidate(candidate, true);
                }
                Purpose::Other { row } if self.fates[row] == Fate::Missing && candidate.ok => {
                    self.record_link(row, candidate);
                    if self.selected_row().is_none() {
                        self.advance(row);
                    }
                }
                _ => {}
            }
        }
    }

    /// Starts the checks of `path` as the new file of `row` in a job.
    fn submit_check(&mut self, purpose: Purpose, row: usize, path: PathBuf) {
        let Some(runtime) = &self.runtime else { return };
        let link = self.rows[row].link.clone();
        let files = Arc::clone(&self.files);
        let checker = self.checker.clone();
        let criteria = self.options.criteria;
        let app = self.options.app_criteria.clone();
        self.ticket += 1;
        runtime.checks.submit(self.ticket, Priority::VISIBLE, move |_| {
            let candidate = check_candidate(&link, &path, &criteria, &*files);
            let candidate = match &checker {
                Some(checker) => checker.check(&link, candidate, &app),
                None => candidate,
            };
            Checked { purpose, candidate }
        });
    }

    // --- commands ---

    fn start_search(&mut self) {
        let (Some(row), Some(runtime)) = (self.selected_row(), &self.runtime) else { return };
        let folder = self.folder.trim();
        if folder.is_empty() {
            return;
        }
        let mut search = LinkSearch::new(folder);
        search.exact_names = self.options.exact_names;
        search.criteria = self.options.criteria;
        let checker = self.checker.clone();
        let app = self.options.app_criteria.clone();
        let checker = Arc::new(move |link: &Link<K>, candidate: Candidate| match &checker {
            Some(checker) => checker.check(link, candidate, &app),
            None => candidate,
        });
        let run = SearchRun::start(
            search,
            vec![self.rows[row].link.clone()],
            Arc::clone(&self.files),
            checker,
            Arc::clone(&runtime.jobs),
            runtime.wake.clone(),
        );
        self.clear_candidates();
        self.search = Some(run);
    }

    fn stop_search(&mut self) {
        if let Some(run) = self.search.as_mut() {
            run.cancel();
        }
        self.search = None;
        self.searched = false;
    }

    fn browse(&mut self, words: &RelinkWords) {
        let Some(host) = self.host.clone() else { return };
        self.forget_pick();
        let mut request = PickRequest::folder(PICK_PURPOSE).title(words.browse.as_str());
        if let Some(folder) = self.options.folder.as_ref().filter(|_| self.folder.trim().is_empty()) {
            request = request.directory(folder.clone());
        }
        self.pick = Some(Pick::Folder(host.files().pick(request)));
    }

    fn locate(&mut self, words: &RelinkWords) {
        let (Some(host), Some(row)) = (self.host.clone(), self.selected_row()) else { return };
        self.forget_pick();
        let id = host.files().pick(PickRequest::open(PICK_PURPOSE).title(words.locate.as_str()));
        self.pick = Some(Pick::Locate { id, row });
    }

    fn forget_pick(&mut self) {
        if let (Some(pick), Some(host)) = (self.pick.take(), &self.host) {
            host.files().forget(pick.id());
        }
    }

    fn record_link(&mut self, row: usize, candidate: Candidate) {
        self.fates[row] = Fate::Linked;
        self.linked.push((self.rows[row].link.key.clone(), candidate));
    }

    /// Links the selected row to the selected candidate and goes on to the next row; with Relink
    /// others, every other missing row the same folder change fits is checked in a job.
    fn link_selected(&mut self, ctx: &Context) {
        let (Some(row), Some(candidate)) = (self.selected_row(), self.selected_candidate().cloned()) else { return };
        let remap = self.options.relink_others.then(|| derive_remap(&self.rows[row].link.stored, &candidate.path.to_string_lossy())).flatten();
        self.record_link(row, candidate);
        if let Some(remap) = remap {
            for other in self.unresolved() {
                let moved = remap.apply(&self.rows[other].link.stored);
                // A network or device path is refused here, before any file is read.
                let Some(path) = moved.and_then(|moved| craft_docpath::resolve_absolute(&moved, self.options.document.as_deref()).ok()) else {
                    continue;
                };
                self.submit_check(Purpose::Other { row: other }, other, path);
            }
        }
        self.advance(row);
        ctx.request_repaint();
    }

    fn leave_selected_offline(&mut self) {
        let Some(row) = self.selected_row() else { return };
        self.fates[row] = Fate::Offline;
        self.offline.push(self.rows[row].link.key.clone());
        self.advance(row);
    }

    fn leave_all_offline(&mut self) {
        for row in self.unresolved() {
            self.fates[row] = Fate::Offline;
            self.offline.push(self.rows[row].link.key.clone());
        }
        self.clear_candidates();
        self.row_sel = Selection::default();
    }

    // --- drawing ---

    fn body(&mut self, ui: &mut Ui, _cx: &mut BodyCx<'_>, env: &Env, words: &RelinkWords) {
        Text::new(words.message.clone()).show(ui);
        self.table(ui, env, words);
        Divider::horizontal().show(ui);
        self.match_checks(ui, words);
        Divider::horizontal().show(ui);
        self.search_rows(ui, words);
        self.progress_line(ui, env, words);
        self.candidates_and_preview(ui, env, words);
    }

    fn table(&mut self, ui: &mut Ui, env: &Env, words: &RelinkWords) {
        let columns = vec![
            ColumnSpec::new(OWNER, words.owner.clone(), ColumnWidth::fill(1.0)).sortable(),
            ColumnSpec::new(FILE_NAME, words.file_name.clone(), ColumnWidth::fill(1.0)).sortable(),
            ColumnSpec::new(FILE_PATH, words.file_path.clone(), ColumnWidth::fill(2.0)).sortable(),
            ColumnSpec::new(STATUS, words.status.clone(), ColumnWidth::fill(1.0)).sortable(),
        ];
        let collator = env.collator();
        let mut order = self.unresolved();
        let rows = &self.rows;
        let text = |row: usize, column: &ColumnKey| -> &str {
            let row = &rows[row];
            match column {
                c if *c == OWNER => row.owner.as_str(),
                c if *c == FILE_NAME => file_name(&row.link.stored),
                c if *c == FILE_PATH => row.link.stored.as_str(),
                _ => row.status.as_str(),
            }
        };
        sort_rows(&mut order, self.sort.as_ref(), |&a, &b, column| collator.compare(text(a, column), text(b, column)));
        let source = MissingRows(order);
        let before = self.row_sel.lead;
        let mut selection = self.row_sel.clone();
        let out = Table::<usize>::new(words.title.clone(), columns.clone())
            .id_salt((KEY, "table"))
            .select(SelectMode::Single)
            .selection(Bind::bound(&mut selection))
            .sort(self.sort.clone())
            .sizing(Sizing::Content)
            .show_items(ui, &source, &mut |&row, icx| {
                let data = &rows[row];
                ListRow::new(data.owner.clone())
                    .columns(&columns)
                    .striped(icx.at % 2 == 1)
                    .cell(FILE_NAME, file_name(&data.link.stored).to_owned())
                    .cell(FILE_PATH, data.link.stored.clone())
                    .cell(STATUS, data.status.clone())
            });
        for event in out.events {
            if let CollectionEvent::Column(ColumnEvent::Sort { key, descending, .. }) = event {
                self.sort = Some(ColumnSort::new(key, descending));
            }
        }
        self.row_sel = selection;
        if self.row_sel.lead.is_none() {
            // A row stays selected while any is missing.
            self.row_sel = before.map_or_else(Selection::default, Selection::single);
        }
        if self.row_sel.lead != before {
            self.clear_candidates();
        }
    }

    fn match_checks(&mut self, ui: &mut Ui, words: &RelinkWords) {
        let criteria = &mut self.options.criteria;
        let app = &mut self.options.app_criteria;
        Row::new().wrap(true).show(ui, |ui, _| {
            Text::new(words.match_heading.clone()).show(ui);
            for (label, value) in [
                (&words.match_name, &mut criteria.file_name),
                (&words.match_extension, &mut criteria.extension),
                (&words.match_same_file, &mut criteria.same_file),
            ] {
                Checkbox::new(Bind::bound(value)).label(label.clone()).show(ui);
            }
            for criterion in app.iter_mut() {
                Checkbox::new(Bind::bound(&mut criterion.on)).label(criterion.label.clone()).show(ui);
            }
        });
        Checkbox::new(Bind::bound(&mut self.options.relink_others)).label(words.relink_others.clone()).show(ui);
    }

    fn search_rows(&mut self, ui: &mut Ui, words: &RelinkWords) {
        let mut browse = false;
        let mut submitted = false;
        Row::new().show(ui, |ui, _| {
            Text::new(words.search_in.clone()).show(ui);
            let field = TextField::new(words.search_in.clone(), Bind::bound(&mut self.folder)).show(ui);
            submitted = field.submitted;
            browse = clicked(&Button::new(words.browse.clone()).show(ui));
        });
        if submitted && self.search.is_none() {
            self.start_search();
        }
        if browse {
            self.browse(words);
        }
        let searching = self.search.is_some();
        let (label, enabled) = if searching {
            (&words.cancel_search, true)
        } else {
            (&words.search, self.runtime.is_some() && self.selected_row().is_some() && !self.folder.trim().is_empty())
        };
        let mut toggle = false;
        Row::new().show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut self.options.exact_names)).label(words.exact_names.clone()).show(ui);
            toggle = clicked(&Button::new(label.clone()).enabled(enabled).show(ui));
        });
        if toggle {
            if searching {
                self.stop_search();
            } else {
                self.start_search();
            }
        }
    }

    /// The progress line. Its count changes every pass, so it is drawn silent; a polite
    /// announcement tells the start and the end of a search only.
    fn progress_line(&mut self, ui: &mut Ui, env: &Env, words: &RelinkWords) {
        let state = if self.search.is_some() {
            Announced::Started
        } else if self.searched {
            Announced::Ended
        } else {
            Announced::Nothing
        };
        if self.search.is_some() {
            let n = self.progress as u64;
            let text = words.searching.text(n, &env.format().integer(n));
            Text::new(text).shade(Shade::Muted).show(ui);
        } else if self.searched && self.candidates.is_empty() {
            Text::new(words.not_found.clone()).shade(Shade::Muted).show(ui);
        }
        if state != self.announced {
            let message = match (state, self.candidates.is_empty()) {
                (Announced::Started, _) => Some(words.search_started.as_str()),
                (Announced::Ended, true) => Some(words.not_found.as_str()),
                (Announced::Ended, false) => Some(words.search_done.as_str()),
                (Announced::Nothing, _) => None,
            };
            if let Some(message) = message {
                announce(ui.ctx(), message, Politeness::Polite);
            }
            self.announced = state;
        }
    }

    fn candidates_and_preview(&mut self, ui: &mut Ui, env: &Env, words: &RelinkWords) {
        let selected_path = self.selected_candidate().map(|c| c.path.clone());
        let format = env.format();
        Row::new().show(ui, |ui, _| {
            if !self.candidates.is_empty() {
                let before = self.cand_sel.lead.clone();
                let mut selection = self.cand_sel.clone();
                let candidates = &self.candidates;
                let source = CandidateRows(candidates);
                let _ = List::<PathBuf>::new(words.candidates.clone())
                    .id_salt((KEY, "candidates"))
                    .select(SelectMode::Single)
                    .selection(Bind::bound(&mut selection))
                    .sizing(Sizing::Content)
                    .show_items(ui, &source, &mut |_, icx| candidate_row(words, format, &candidates[icx.at], icx.at));
                self.cand_sel = selection;
                if self.cand_sel.lead != before {
                    self.auto_selected = false;
                }
            }
            self.preview_box(ui, env, words, selected_path.clone());
        });
    }

    fn preview_box(&self, ui: &mut Ui, env: &Env, words: &RelinkWords, path: Option<PathBuf>) {
        let state = match (&self.preview, path, &self.runtime) {
            (Some(source), Some(path), Some(runtime)) => {
                let ctx = ui.ctx().clone();
                let options = ThumbnailOptions::new().jobs(Arc::clone(&runtime.jobs));
                let thumbnails = Thumbnails::<FileThumbKey>::in_context(&ctx, (KEY, "preview"), options);
                let request = ThumbRequest::for_cell(PREVIEW_SIDE, ctx.pixels_per_point());
                match thumbnails.state(&ctx, source, &FileThumbKey::file(path), &request, Priority::VISIBLE) {
                    TextureState::Ready(texture) | TextureState::Loading { stand_in: Some(texture), .. } => {
                        PreviewState::Ready(SizedTexture::new(texture.id, texture.size_vec2()))
                    }
                    TextureState::Failed { failure, .. } => PreviewState::Failed(Label::from(failure.text(&env.words::<TextureWords>()).to_owned())),
                    TextureState::Empty => PreviewState::Failed(words.no_preview.clone()),
                    _ => PreviewState::Pending(None),
                }
            }
            _ => PreviewState::Failed(words.no_preview.clone()),
        };
        PreviewPane::state(words.preview.clone(), state, &PreviewView::fitting()).show(ui);
    }
}

/// The identity-free file tail of a stored path, for the File Name column.
fn file_name(stored: &str) -> &str {
    stored.rsplit(['/', '\\']).next().unwrap_or(stored)
}

/// The mark of a candidate: its icon, tone and words ("same file", "matches", "different file";
/// none for a candidate that fails on a name or extension check).
fn mark<'w>(candidate: &Candidate, words: &'w RelinkWords) -> (&'static str, Tone, Option<&'w Label>) {
    match (candidate.ok, candidate.same_file) {
        (true, Some(true)) => ("check", Tone::Good, Some(&words.same_file)),
        (true, _) => ("check", Tone::Good, Some(&words.matches)),
        (false, Some(false)) => ("close", Tone::Bad, Some(&words.different_file)),
        (false, _) => ("close", Tone::Bad, None),
    }
}

/// Why a candidate fails, in the user's words, in check order.
fn problem_words(candidate: &Candidate, words: &RelinkWords, format: &Format) -> Vec<String> {
    let number = |n: u64| format.integer(n);
    candidate
        .problems
        .iter()
        .map(|problem| match problem {
            CandidateProblem::NameDiffers { found, wanted } => words.name_differs.fill(&[("found", found), ("wanted", wanted)]),
            CandidateProblem::ExtensionDiffers { found, wanted } => words.extension_differs.fill(&[("found", found), ("wanted", wanted)]),
            CandidateProblem::DifferentFile { found, wanted } if found == wanted => words.fingerprint_differs.as_str().to_owned(),
            CandidateProblem::DifferentFile { found, wanted } => words.size_differs.fill(&[("found", &number(*found)), ("wanted", &number(*wanted))]),
            CandidateProblem::Unreadable(reason) => words.unreadable.fill(&[("reason", reason)]),
            CandidateProblem::Other(text) => text.clone(),
            _ => String::new(),
        })
        .filter(|words| !words.is_empty())
        .collect()
}

/// The list row of one candidate: its path, striped, with its mark before the path and in words
/// at the end, and its problems as the tooltip.
fn candidate_row<'r>(words: &RelinkWords, format: &Format, candidate: &Candidate, at: usize) -> ListRow<'r> {
    let (icon, tone, mark_words) = mark(candidate, words);
    let path = candidate.path.display().to_string();
    let problems = problem_words(candidate, words, format).join("; ");
    let name = match mark_words {
        Some(mark_words) => format!("{path}, {}", mark_words.as_str()),
        None => path.clone(),
    };
    let mut row = ListRow::new(path).striped(at % 2 == 1).icon(IconSpec::svg(IconId::new(icon))).tone(tone).accessible_name(name);
    if let Some(mark_words) = mark_words {
        row = row.trailing(mark_words.clone());
    }
    if !problems.is_empty() {
        row = row.tooltip(problems);
    }
    row
}
