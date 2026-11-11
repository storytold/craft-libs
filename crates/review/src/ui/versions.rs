//! The version history panel and the preview banner.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashSet;

use craft_behavior::{DisplayOrder, IconId, ItemSource, Selection};
use craft_theme::variant::Variant;
use craft_ui::buttons::{Button, ButtonOutput, IconButton, Link};
use craft_ui::contract::overlay::Anchor;
use craft_ui::contract::{
    ActivateOn, Bind, CollectionEvent, Entry, NoValue, OpenChange, Outcome, Pattern, Props, Remember as _, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, Trigger, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, RowEvent, TrailingFace, TrailingPart};
use craft_ui::menu::Menu;
use craft_ui::popover::Popover;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::TypeRole;
use craft_ui::text::TextField;
use egui::{Pos2, Rect, Response, Ui};

use super::comments::age_text;
use crate::versions::{Branch, BranchId, BranchState, HistoryRow, Version, VersionId, history_rows};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The panel's words, English by default.
    #[non_exhaustive]
    pub struct VersionsWords {
        title: "versions.title" = "Version history";
        current: "versions.current" = "Current version";
        save: "versions.save" = "Save version";
        title_field: "versions.titleField" = "Title";
        description: "versions.description" = "Description";
        autosaves: count "versions.autosaves" = "{n} autosave version" | "{n} autosave versions";
        /// The date and the author under a version's title.
        detail: template "versions.detail" = "{date} \u{b7} {author}";
        no_versions: "versions.noVersions" = "No saved versions yet. Save a version to mark a point in your work you can come back to.";
        restore: "versions.restore" = "Restore this version";
        rename: "versions.rename" = "Rename";
        edit_description: "versions.editDescription" = "Edit description";
        duplicate: "versions.duplicate" = "Duplicate";
        delete: "versions.delete" = "Delete";
        delete_confirm: template "versions.deleteConfirm" = "Delete \u{201c}{title}\u{201d}? This can't be undone.";
        row_more: "versions.rowMore" = "More";
        branches: "versions.branches" = "Branches";
        branch_name: "versions.branchName" = "Branch name";
        no_branches: "versions.noBranches" = "No branches. A branch is a copy of the file to try changes in, then review and merge.";
        branch_active: "versions.branchActive" = "Active";
        branch_merged: "versions.branchMerged" = "Merged";
        branch_archived: "versions.branchArchived" = "Archived";
        /// The state and the date under a branch's name.
        branch_detail: template "versions.branchDetail" = "{state} \u{b7} {date}";
        /// The name of a branch's "\u{2026}" button.
        branch_more: template "versions.branchMore" = "More ({branch})";
        new_branch: "versions.newBranch" = "Create branch";
        open: "versions.open" = "Open";
        review_merge: "versions.reviewMerge" = "Review and merge";
        archive: "versions.archive" = "Archive";
        previewing: template "versions.previewing" = "You're viewing {title}";
        go_back: "versions.goBack" = "Back to current version";
        cancel: "versions.cancel" = "Cancel";
    }
}

/// What the panel asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VersionAction {
    /// Save the current state as a named version.
    #[non_exhaustive]
    Save {
        /// The title.
        title: String,
        /// The description.
        description: String,
    },
    /// Show a version on the canvas, read-only.
    Preview(VersionId),
    /// Back to the current version.
    Current,
    /// Restore a version as the current one.
    Restore(VersionId),
    /// A new title and description (a title edited in the row keeps the description).
    #[non_exhaustive]
    Rename {
        /// The version to rename.
        id: VersionId,
        /// The title.
        title: String,
        /// The description.
        description: String,
    },
    /// Duplicate a version.
    Duplicate(VersionId),
    /// Delete a version.
    Delete(VersionId),
    /// Create a branch.
    #[non_exhaustive]
    CreateBranch {
        /// The branch's name.
        name: String,
    },
    /// Open a branch.
    OpenBranch(BranchId),
    /// Merge a branch back into the main file.
    MergeBranch(BranchId),
    /// Archive a branch.
    ArchiveBranch(BranchId),
}

/// A row of the history list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum RowKey {
    Current,
    Version(VersionId),
    /// A run of autosaves, by its newest version.
    Group(VersionId),
}

/// What the panel keeps between passes.
#[derive(Clone, Default)]
struct PanelState {
    /// Autosave groups the user opened, by their newest version.
    open: HashSet<VersionId>,
    /// The Save version form: title and description.
    form: Option<(String, String)>,
    /// The Create branch form: the name.
    branch: Option<String>,
    /// The version whose title and description are being edited, with the draft of each.
    edit: Option<(VersionId, String, String)>,
    /// The version whose Delete awaits its confirmation.
    confirm: Option<VersionId>,
    /// The row menu: whose it is, and where it opens.
    menu: Option<(VersionId, Pos2)>,
}

/// One displayed row of the history: its key, depth and, for a group, whether it is open.
struct Shown {
    key: RowKey,
    depth: usize,
    group: bool,
    open: bool,
    text: String,
}

struct Order(Vec<Shown>);

impl DisplayOrder<RowKey> for Order {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> RowKey {
        self.0[at].key.clone()
    }

    fn index_of(&self, key: &RowKey) -> Option<usize> {
        self.0.iter().position(|row| row.key == *key)
    }

    fn depth(&self, at: usize) -> usize {
        self.0[at].depth
    }

    fn has_children(&self, at: usize) -> bool {
        self.0[at].group
    }

    fn open(&self, at: usize) -> bool {
        self.0[at].open
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0[at].text.as_str()))
    }
}

impl ItemSource<RowKey> for Order {}

/// The title of `version`, or its age for an autosave.
fn version_name(env: &craft_ui::runtime::env::EnvView<'_>, version: &Version, now: i64) -> String {
    if version.title.trim().is_empty() { age_text(env, version.created, now) } else { version.title.clone() }
}

/// The Title and Description fields with a submit button and Cancel: `Some(true)` to submit (the button or Enter),
/// `Some(false)` to cancel (the button or Escape).
fn details_form(ui: &mut Ui, words: &VersionsWords, salt: &str, title: &mut String, description: &mut String, submit: &Label) -> Option<bool> {
    let field =
        TextField::new(words.title_field.clone(), Bind::bound(&mut *title)).id_salt((salt, "title")).placeholder(words.title_field.clone()).show(ui);
    let (mut save, mut cancel) = (field.submitted, field.escaped);
    let field = TextField::new(words.description.clone(), Bind::bound(&mut *description))
        .id_salt((salt, "description"))
        .placeholder(words.description.clone())
        .show(ui);
    (save, cancel) = (save | field.submitted, cancel | field.escaped);
    Row::new().show(ui, |ui, _| {
        save |= clicked(&Button::new(submit.clone()).variant(Variant::PRIMARY).id_salt((salt, "submit")).show(ui));
        cancel |= clicked(&Button::new(words.cancel.clone()).id_salt((salt, "cancel")).show(ui));
    });
    if save { Some(true) } else { cancel.then_some(false) }
}

/// Figma's Version history panel: the branches above (create, open, review and merge, archive), then Current
/// version, then the history newest first on a list with a dot per row, each named version with its title, date and
/// author, its description wrapped under it, runs of two or more autosaves folded into a group that opens (a lone
/// autosave is a row). A row's "…" opens its menu (Restore, Rename, Duplicate, Delete; Delete turns the row into the
/// confirmation in place); F2 or Rename renames the title in place. Enter or a click previews. Save version opens an
/// inline form (Title, Description; Enter saves, Escape cancels).
#[derive(Pattern, Props)]
pub struct VersionHistoryPanel<'a> {
    label: Label,
    versions: &'a [Version],
    branches: &'a [Branch],
    /// The version being previewed, if any; its row is selected.
    #[prop]
    previewing: Option<VersionId>,
    now: i64,
}

impl<'a> VersionHistoryPanel<'a> {
    /// A panel listing `versions` and `branches`; `now` is the time, in Unix seconds, the ages count from.
    pub fn new(versions: &'a [Version], branches: &'a [Branch], now: i64) -> Self {
        VersionHistoryPanel { label: Label::Empty, versions, branches, previewing: None, now }
    }

    /// Shows the panel and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> VersionHistoryPanelOutput {
        let VersionHistoryPanel { label, versions, branches, previewing, now } = self;
        let env = Env::of(ui);
        let words = env.words::<VersionsWords>();
        let view = env.view();
        let mut actions = Vec::new();
        let shown = Stack::new().label(label).show(ui, |ui, cx| {
            let mut state = std::mem::take(cx.keep::<PanelState>());
            Row::new().show(ui, |ui, _| {
                Text::new(words.title.clone()).role(TypeRole::Title).show(ui);
                if state.form.is_none() && clicked(&Button::new(words.save.clone()).show(ui)) {
                    state.form = Some((String::new(), String::new()));
                }
            });
            if let Some((title, description)) = state.form.as_mut() {
                match details_form(ui, &words, "form", title, description, &words.save) {
                    Some(true) => {
                        let (title, description) = state.form.take().unwrap_or_default();
                        actions.push(VersionAction::Save { title, description });
                    }
                    Some(false) => state.form = None,
                    None => {}
                }
            }
            branch_section(ui, &words, &view, branches, now, &mut state, &mut actions);
            if let Some((id, title, description)) = state.edit.as_mut() {
                let id = *id;
                match details_form(ui, &words, "edit", title, description, &words.save) {
                    Some(true) => {
                        if let Some((_, title, description)) = state.edit.take() {
                            let title = title.trim().to_owned();
                            if !title.is_empty() {
                                actions.push(VersionAction::Rename { id, title, description: description.trim().to_owned() });
                            }
                        }
                    }
                    Some(false) => state.edit = None,
                    None => {}
                }
            }
            history(ui, &env, &words, (versions, previewing, now), &mut state, &mut actions);
            *cx.keep::<PanelState>() = state;
        });
        VersionHistoryPanelOutput { response: shown.output, actions }
    }
}

fn branch_section(
    ui: &mut Ui,
    words: &VersionsWords,
    view: &craft_ui::runtime::env::EnvView<'_>,
    branches: &[Branch],
    now: i64,
    state: &mut PanelState,
    actions: &mut Vec<VersionAction>,
) {
    Text::new(words.branches.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    if branches.is_empty() {
        Text::new(words.no_branches.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    for branch in branches {
        Row::new().show(ui, |ui, _| {
            if clicked(&Link::new(branch.name.clone()).id_salt(("branch", branch.id)).show(ui)) {
                actions.push(VersionAction::OpenBranch(branch.id));
            }
            let state_word = match branch.state {
                BranchState::Active => &words.branch_active,
                BranchState::Merged => &words.branch_merged,
                BranchState::Archived => &words.branch_archived,
            };
            let date = age_text(view, branch.created, now);
            Text::new(words.branch_detail.fill(&[("state", state_word.as_str()), ("date", date.as_str())]))
                .role(TypeRole::Caption)
                .shade(Shade::Muted)
                .show(ui);
            let name = words.branch_more.fill(&[("branch", branch.name.as_str())]);
            let more = IconButton::new(IconId::new("more")).label(name.clone()).id_salt(("more", branch.id)).show(ui);
            let mut entries = vec![Entry::new(0u8, words.open.clone()), Entry::new(1, words.review_merge.clone())];
            if branch.state == BranchState::Active {
                entries.push(Entry::new(2, words.archive.clone()));
            }
            match Menu::new(name).opened_by(&more, Trigger::Press).pick(ui, &entries).picked {
                Some(0) => actions.push(VersionAction::OpenBranch(branch.id)),
                Some(1) => actions.push(VersionAction::MergeBranch(branch.id)),
                Some(2) => actions.push(VersionAction::ArchiveBranch(branch.id)),
                _ => {}
            }
        });
    }
    let Some(name) = state.branch.as_mut() else {
        if clicked(&Button::new(words.new_branch.clone()).show(ui)) {
            state.branch = Some(String::new());
        }
        return;
    };
    let field = TextField::new(words.branch_name.clone(), Bind::bound(&mut *name)).placeholder(words.branch_name.clone()).show(ui);
    let (mut create, mut cancel) = (field.submitted, field.escaped);
    let named = !name.trim().is_empty();
    Row::new().show(ui, |ui, _| {
        create |= clicked(&Button::new(words.new_branch.clone()).variant(Variant::PRIMARY).enabled(named).id_salt("branch-create").show(ui));
        cancel |= clicked(&Button::new(words.cancel.clone()).id_salt("branch-cancel").show(ui));
    });
    if create && named {
        let name = state.branch.take().unwrap_or_default();
        actions.push(VersionAction::CreateBranch { name: name.trim().to_owned() });
    } else if cancel {
        state.branch = None;
    }
}

/// What the delete confirmation of a version row reports.
#[derive(Clone, Copy)]
enum Confirmed {
    Delete,
    Cancel,
}

/// The rows displayed: Current version, then the history newest first, an open autosave group followed by its
/// versions.
fn display_order(env: &Env, words: &VersionsWords, versions: &[Version], previewing: Option<VersionId>, now: i64, state: &PanelState) -> Order {
    let view = env.view();
    let name = |version: &Version| version_name(&view, version, now);
    let mut rows = vec![Shown { key: RowKey::Current, depth: 0, group: false, open: false, text: words.current.as_str().to_owned() }];
    for row in history_rows(versions) {
        match row {
            HistoryRow::Version(at) => {
                rows.push(Shown { key: RowKey::Version(versions[at].id), depth: 0, group: false, open: false, text: name(&versions[at]) });
            }
            HistoryRow::Autosaves(members) => {
                let head = versions[members[0]].id;
                let open = state.open.contains(&head) || members.iter().any(|&at| Some(versions[at].id) == previewing);
                let count = u64::try_from(members.len()).unwrap_or(u64::MAX);
                let text = words.autosaves.text(count, &env.format().integer(count));
                rows.push(Shown { key: RowKey::Group(head), depth: 0, group: true, open, text });
                if open {
                    rows.extend(members.iter().map(|&at| Shown {
                        key: RowKey::Version(versions[at].id),
                        depth: 1,
                        group: false,
                        open: false,
                        text: name(&versions[at]),
                    }));
                }
            }
        }
    }
    Order(rows)
}

/// Opens or closes the autosave group headed by `head`: to `open` when given, else the other way.
fn fold(state: &mut PanelState, head: VersionId, open: Option<bool>) {
    let open = open.unwrap_or(!state.open.contains(&head));
    if open {
        state.open.insert(head);
    } else {
        state.open.remove(&head);
    }
}

fn history(
    ui: &mut Ui,
    env: &Env,
    words: &VersionsWords,
    (versions, previewing, now): (&[Version], Option<VersionId>, i64),
    state: &mut PanelState,
    actions: &mut Vec<VersionAction>,
) {
    let view = env.view();
    let order = display_order(env, words, versions, previewing, now, state);
    let current = previewing.map_or(RowKey::Current, RowKey::Version);
    let pending = state.confirm;
    let outcome = Cell::new(None);
    let find = |id: VersionId| versions.iter().find(|version| version.id == id);
    let list_id = ui.scope_id().with("versions-history");
    let list = List::new(words.title.clone())
        .id(list_id)
        .tree(true)
        .select(SelectMode::Single)
        .rename(true)
        .activate(ActivateOn::Click)
        .selection(Bind::given(Selection::single(current)))
        .can_rename(|key| matches!(key, RowKey::Version(id) if find(*id).is_some_and(|version| !version.autosave)));
    let out = list.show_items(ui, &order, &mut |key, icx| {
        let dot = IconSpec::from(IconId::new("circle"));
        let text = order.0[icx.at].text.clone();
        match key {
            RowKey::Current => ListRow::new(text).icon(dot),
            RowKey::Group(_) => ListRow::new(text).depth(icx.depth).group(icx.row.open).icon(dot),
            RowKey::Version(id) => {
                let Some(version) = find(*id) else { return ListRow::new(Label::Empty) };
                if pending == Some(*id) {
                    let question = words.delete_confirm.fill(&[("title", text.as_str())]);
                    let (outcome_delete, outcome_cancel) = (&outcome, &outcome);
                    return ListRow::new(question)
                        .depth(icx.depth)
                        .icon(dot)
                        .trailing_field(Button::new(words.delete.clone()).variant(Variant::PRIMARY), move |button: ButtonOutput| {
                            if clicked(&button) {
                                outcome_delete.set(Some(Confirmed::Delete));
                            }
                        })
                        .trailing_field(Button::new(words.cancel.clone()), move |button: ButtonOutput| {
                            if clicked(&button) {
                                outcome_cancel.set(Some(Confirmed::Cancel));
                            }
                        });
                }
                let date = age_text(&view, version.created, now);
                let detail =
                    if version.author.is_empty() { date } else { words.detail.fill(&[("date", date.as_str()), ("author", version.author.as_str())]) };
                let mut row = ListRow::new(text).depth(icx.depth).icon(dot).detail(detail);
                if !version.description.is_empty() {
                    row = row.lines(version.description.clone());
                }
                row.trailing_part(TrailingPart::new(0), TrailingFace::Icon(IconSpec::from(IconId::new("more"))), words.row_more.clone())
            }
        }
    });
    match outcome.get() {
        Some(Confirmed::Delete) => {
            state.confirm = None;
            actions.extend(pending.map(VersionAction::Delete));
        }
        Some(Confirmed::Cancel) => state.confirm = None,
        None => {}
    }
    if order.0.len() == 1 && versions.is_empty() {
        Text::new(words.no_versions.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    for event in out.events {
        match event {
            CollectionEvent::Activate(RowKey::Version(id)) => actions.push(VersionAction::Preview(id)),
            CollectionEvent::Activate(RowKey::Current) if previewing.is_some() => actions.push(VersionAction::Current),
            CollectionEvent::Activate(RowKey::Group(head)) => fold(state, head, None),
            CollectionEvent::Open(OpenChange { key: RowKey::Group(head), open, .. }) => fold(state, head, Some(open)),
            CollectionEvent::Rename(edit) => {
                if let Some(rename) = edit.value()
                    && let RowKey::Version(id) = rename.key
                {
                    let title = rename.name.trim();
                    if let Some(version) = find(id).filter(|version| version.title != title).filter(|_| !title.is_empty()) {
                        actions.push(VersionAction::Rename { id, title: title.to_owned(), description: version.description.clone() });
                    }
                }
            }
            CollectionEvent::ContextMenu(request) => {
                if let RowKey::Version(id) = request.key {
                    state.menu = Some((id, request.at));
                }
            }
            CollectionEvent::Item(RowKey::Version(id), RowEvent::Trailing { .. }) => state.menu = Some((id, out.response.rect.right_top())),
            _ => {}
        }
    }
    row_menu(ui, words, &out.response, list_id, find, state, actions);
}

/// The menu of the version row a context request or its "…" part named.
fn row_menu<'v>(
    ui: &mut Ui,
    words: &VersionsWords,
    list: &Response,
    list_id: egui::Id,
    find: impl Fn(VersionId) -> Option<&'v Version>,
    state: &mut PanelState,
    actions: &mut Vec<VersionAction>,
) {
    let Some((id, at)) = state.menu else { return };
    let Some(version) = find(id) else {
        state.menu = None;
        return;
    };
    let mut open = true;
    let mut entries = vec![Entry::new(0u8, words.restore.clone())];
    if !version.autosave {
        entries.push(Entry::new(1, words.rename.clone()));
        entries.push(Entry::new(2, words.edit_description.clone()));
    }
    entries.push(Entry::new(3, words.duplicate.clone()));
    entries.push(Entry::new(4, words.delete.clone()));
    let picked = Menu::new(words.title.clone())
        .opened_by(list, Trigger::Manual)
        .anchor(Anchor::Point(at))
        .open(Bind::bound(&mut open))
        .pick(ui, &entries)
        .picked;
    match picked {
        Some(0) => actions.push(VersionAction::Restore(id)),
        Some(1) => craft_ui::contract::request_rename(ui.ctx(), list_id, &RowKey::Version(id)),
        Some(2) => {
            state.edit = Some((id, version.title.clone(), version.description.clone()));
            state.confirm = None;
        }
        Some(3) => actions.push(VersionAction::Duplicate(id)),
        Some(4) => state.confirm = Some(id),
        _ => {}
    }
    if !open || picked.is_some() {
        state.menu = None;
    }
}

/// What the Version history panel did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct VersionHistoryPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// In the order they happened this pass.
    pub actions: Vec<VersionAction>,
}

impl Outcome for VersionHistoryPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The banner over the canvas while a version is previewed: "You're viewing {title}" with its date, Restore this
/// version and Back to current version (two Tab stops). Escape goes back while the banner holds the focus.
#[derive(Pattern, Props)]
pub struct VersionPreviewBanner<'a> {
    label: Label,
    version: &'a Version,
    now: i64,
}

impl<'a> VersionPreviewBanner<'a> {
    /// A banner for the previewed `version`; `now` is the time, in Unix seconds, its age counts from.
    pub fn new(version: &'a Version, now: i64) -> Self {
        VersionPreviewBanner { label: Label::Empty, version, now }
    }

    /// Shows the banner at the top of `canvas`.
    pub fn show(self, ui: &mut Ui, canvas: Rect) -> VersionPreviewBannerOutput {
        let VersionPreviewBanner { label, version, now } = self;
        let env = Env::of(ui);
        let words = env.words::<VersionsWords>();
        let name = version_name(&env.view(), version, now);
        let title = words.previewing.fill(&[("title", name.as_str())]);
        let mut open = true;
        let mut action = None;
        let shown = Popover::new(title.clone(), Bind::bound(&mut open)).at_point(Pos2::new(canvas.center().x, canvas.top())).show(ui, |ui, _| {
            Row::new().label(label.clone()).show(ui, |ui, _| {
                Text::new(title.clone()).role(TypeRole::Label).show(ui);
                Text::new(age_text(&env.view(), version.created, now)).shade(Shade::Muted).show(ui);
                if clicked(&Button::new(words.restore.clone()).variant(Variant::PRIMARY).show(ui)) {
                    action = Some(VersionAction::Restore(version.id));
                }
                if clicked(&Button::new(words.go_back.clone()).show(ui)) {
                    action = Some(VersionAction::Current);
                }
            });
        });
        if action.is_none() && shown.output.closed == Some(craft_ui::runtime::overlay::CloseReason::Escape) {
            action = Some(VersionAction::Current);
        }
        VersionPreviewBannerOutput { response: shown.output.response, action }
    }
}

/// What the preview banner did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct VersionPreviewBannerOutput {
    /// The banner's response.
    pub response: Response,
    /// Restore or Back to current, if chosen this pass.
    pub action: Option<VersionAction>,
}

impl Outcome for VersionPreviewBannerOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}
