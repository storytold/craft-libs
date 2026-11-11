//! The Comments panel: a header with the count, search, sort and filter menus, the add box and the cards.
//!
//! ```ignore
//! let out = CommentsPanel::new("Comments", &threads, &mut state).now(unix_seconds).show(ui);
//! for edit in out.edits {
//!     match edit {
//!         CommentEdit::Add { text } => doc.add_note(text),
//!         CommentEdit::Select(thread) => doc.select(thread),
//!         _ => {}
//!     }
//! }
//! ```

mod list;
mod menus;
mod row;
#[cfg(feature = "screen-testing")]
mod stories;

use std::collections::BTreeMap;

use craft_behavior::Selection;
use craft_i18n::{DateStyle, DateTime, TimeZone};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CollectionEvent, NoValue, Outcome, Pattern, Props as _, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::runtime::env::{AgeStyle, Env, EnvView, Label};
use craft_ui::style::TypeRole;
use craft_ui::text::TextField;
use egui::{Pos2, Response, Ui};

use super::annotate::{AnnotateWords, PALETTE};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentFilter, CommentSort, CommentThread, ReviewStatus, ThreadId, matches, sort_threads};

use list::{CardEvent, CardKey, CommentList, RowEntry, Rows};
use menus::{card_menu, filter_menu, pages_menu, sort_menu};
use row::{CommentRow, MORE};

/// Ages under this many seconds (one week) read relative; older ones read as a date.
pub(crate) const AGE_SHOWN_AS_DATE: u64 = 7 * 24 * 60 * 60;

/// The rows of the add box and of a reply box.
const REPLY_LINES: usize = 2;

/// The rows of an inline editor.
const EDIT_LINES: usize = 3;

craft_ui::words! {
    /// The panel's words, English by default.
    #[non_exhaustive]
    pub struct CommentsWords {
        title: template "comments.title" = "Comments {n}";
        replies: count "comments.replies" = "{n} reply" | "{n} replies";
        search: "comments.search" = "Search comments";
        sort_by: template "comments.sortBy" = "Sort by: {sort}";
        sort_page: "comments.sortPage" = "Page";
        sort_author: "comments.sortAuthor" = "Author";
        sort_date: "comments.sortDate" = "Date";
        sort_type: "comments.sortType" = "Type";
        sort_color: "comments.sortColor" = "Color";
        add_comment: "comments.add" = "Add a comment";
        reply: "comments.reply" = "Add a reply";
        post: "comments.post" = "Post";
        cancel: "comments.cancel" = "Cancel";
        more: "comments.more" = "More";
        edit: "comments.edit" = "Edit";
        set_status: "comments.setStatus" = "Set status";
        delete: "comments.delete" = "Delete";
        copy_text: "comments.copyText" = "Copy text";
        mark: "comments.mark" = "Mark with checkmark";
        unmark: "comments.unmark" = "Remove checkmark";
        marked: "comments.marked" = "Marked";
        locked: "comments.locked" = "Locked";
        unknown_author: "comments.unknownAuthor" = "Unknown author";
        page: template "comments.page" = "Page {label}";
        card_name: template "comments.cardName" = "{author}: {text}";
        plain_kind: "comments.plainKind" = "Comment";
        no_color: "comments.noColor" = "No color";
        no_match: "comments.noMatch" = "No comments match.";
        filter: "comments.filter" = "Filter comments";
        filter_type: "comments.filterType" = "Type";
        filter_author: "comments.filterAuthor" = "Author";
        filter_status: "comments.filterStatus" = "Status";
        filter_color: "comments.filterColor" = "Color";
        filter_check: "comments.filterCheck" = "Checkmark";
        checked: "comments.checked" = "Checked";
        unchecked: "comments.unchecked" = "Unchecked";
        show_all: "comments.showAll" = "Show all";
        pages_menu: "comments.pagesMenu" = "Pages";
        collapse_all: "comments.collapseAll" = "Collapse all pages";
        expand_all: "comments.expandAll" = "Expand all pages";
        status_none: "comments.statusNone" = "None";
        accepted: "comments.accepted" = "Accepted";
        rejected: "comments.rejected" = "Rejected";
        cancelled: "comments.cancelled" = "Cancelled";
        completed: "comments.completed" = "Completed";
        empty: "comments.empty" = "No comments yet.";
    }
}

/// What the panel keeps between passes; the app keeps one per document window.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CommentsPanelState {
    /// The search box text; threads match through [`matches`](fn@matches).
    pub search: String,
    /// The active sort.
    pub sort: CommentSort,
    /// What the filter menu hides.
    pub filter: CommentFilter,
    /// The pages whose group is folded under the Page sort.
    pub collapsed_pages: Vec<usize>,
    /// The selected thread, if any.
    pub selected: Option<ThreadId>,
    /// The message being edited inline: (thread, message index, text).
    pub editing: Option<(ThreadId, usize, String)>,
    /// The selected card's reply draft.
    pub reply: String,
    /// The "Add a comment" box's draft.
    pub new_comment: String,
}

/// Where a message's text starts and how wide a line of it may be.
#[derive(Clone, Copy)]
struct Line {
    at: Pos2,
    room: f32,
}

impl CommentsPanelState {
    /// Selects `thread` as a click on its card does, and unfolds its page group: for a comment
    /// the app selected elsewhere (on the page, or just added), whose card must be drawn.
    pub fn reveal(&mut self, thread: &CommentThread) {
        select_thread(self, thread.id);
        if let Some(page) = thread.anchor.page() {
            self.collapsed_pages.retain(|&folded| folded != page);
        }
    }
}

/// What the panel asks of the app.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CommentEdit {
    /// "Add a comment": a sticky note on the current page.
    #[non_exhaustive]
    Add {
        /// The comment text.
        text: String,
    },
    /// Reply to a thread.
    #[non_exhaustive]
    Reply {
        #[allow(missing_docs)]
        thread: ThreadId,
        /// The reply text.
        text: String,
    },
    /// Message `index` (0: the comment) edited.
    #[non_exhaustive]
    Edit {
        #[allow(missing_docs)]
        thread: ThreadId,
        /// Message index; 0 is the comment.
        index: usize,
        /// The new text.
        text: String,
    },
    /// Message `index` deleted; 0 deletes the thread.
    #[non_exhaustive]
    Delete {
        #[allow(missing_docs)]
        thread: ThreadId,
        /// Message index; 0 deletes the whole thread.
        index: usize,
    },
    /// Set a thread's review status.
    #[non_exhaustive]
    SetStatus {
        #[allow(missing_docs)]
        thread: ThreadId,
        #[allow(missing_docs)]
        status: ReviewStatus,
    },
    /// Set or clear a thread's checkmark.
    #[non_exhaustive]
    Mark {
        #[allow(missing_docs)]
        thread: ThreadId,
        #[allow(missing_docs)]
        marked: bool,
    },
    /// A card was selected: the app selects the annotation on the page and scrolls to it.
    Select(ThreadId),
    /// Copy text of message `index`; the app puts it on the clipboard.
    #[non_exhaustive]
    CopyText {
        #[allow(missing_docs)]
        thread: ThreadId,
        /// Message index.
        index: usize,
    },
    /// The viewer's reaction `emoji` on message `index` toggled (the thread popover).
    #[non_exhaustive]
    React {
        #[allow(missing_docs)]
        thread: ThreadId,
        /// Message index.
        index: usize,
        /// The reaction emoji.
        emoji: String,
    },
    /// Figma's resolve (`true`) or reopen (`false`).
    #[non_exhaustive]
    Resolve {
        #[allow(missing_docs)]
        thread: ThreadId,
        #[allow(missing_docs)]
        resolved: bool,
    },
    /// The app puts a link to the thread on the clipboard.
    #[non_exhaustive]
    CopyLink {
        #[allow(missing_docs)]
        thread: ThreadId,
    },
    /// The viewer's read state of the thread cleared.
    #[non_exhaustive]
    MarkUnread {
        #[allow(missing_docs)]
        thread: ThreadId,
    },
}

/// "5 min ago" under a week, then the date.
pub(crate) fn age_text(env: &EnvView<'_>, then: i64, now: i64) -> String {
    let age = now.saturating_sub(then).max(0).unsigned_abs();
    if age < AGE_SHOWN_AS_DATE {
        env.relative_age(age, AgeStyle::Short)
    } else {
        env.dates().format(DateTime::from_unix(then, TimeZone::Local), DateStyle::DATE)
    }
}

/// Acrobat's right-hand comment list.
///
/// - Header "Comments N" with "N replies" under it, search, the Sort by menu, the funnel menu
///   ([`CommentFilter`]: Type, Author, Status, Color, Checkmark rows that hide what they name) and,
///   under the Page sort, a menu with Collapse all pages and Expand all pages. A search or filter
///   that leaves nothing shows "No comments match."
/// - "Add a comment" box: a multi-line field and Post.
/// - Cards in one collection, grouped under headings by [`sort_threads`]; under the Page sort a
///   heading names the page, shows the count of the page's comments before filtering and folds its
///   cards. A card shows the kind's icon, author, age (relative under a week, then the date), the
///   text, a status icon, the checkmark, a lock icon and the replies, each with its own "..." menu.
/// - The selected card has the accent bar, the reply box and "..." (Edit, Set status, Delete).
///   Locked threads and `read_only` hide the editing items.
/// - Keyboard: Up and Down move between cards (selecting them), Enter selects, Escape in a control
///   of a card returns to the card.
///
/// ```ignore
/// let out = CommentsPanel::new("Comments", &threads, &mut state).now(now).show(ui);
/// ```
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct CommentsPanel<'a> {
    label: Label,
    threads: &'a [CommentThread],
    state: &'a mut CommentsPanelState,
    read_only: bool,
    now: i64,
    page_label: Option<Box<dyn Fn(usize) -> String + 'a>>,
}

impl<'a> CommentsPanel<'a> {
    /// The panel named `label` over `threads`. The edits carry only the text; the app signs new
    /// comments with its own identity.
    pub fn new(label: impl Into<Label>, threads: &'a [CommentThread], state: &'a mut CommentsPanelState) -> Self {
        CommentsPanel { label: label.into(), threads, state, read_only: false, now: 0, page_label: None }
    }

    /// No posting or editing (a document that forbids comments).
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }

    /// The current time in Unix seconds, which the ages count from.
    pub fn now(mut self, unix_seconds: i64) -> Self {
        self.now = unix_seconds;
        self
    }

    /// The text of a page heading's `{label}`, from the page stored in the anchor (page labels
    /// such as "iv" or "A-3"); that page itself by default.
    pub fn page_label(mut self, label: impl Fn(usize) -> String + 'a) -> Self {
        self.page_label = Some(Box::new(label));
        self
    }
}

/// What the Comments panel did this pass.
#[must_use]
#[non_exhaustive]
pub struct CommentsPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// This pass's edits, in order.
    pub edits: Vec<CommentEdit>,
}

impl Outcome for CommentsPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What a row of the filter menu does.
#[derive(Clone)]
enum FilterRow {
    Type(Option<AnnotationKind>),
    Author(String),
    Status(ReviewStatus),
    Color(Option<[u8; 3]>),
    Check(bool),
    ShowAll,
}

/// What a row of a card's menu does.
#[derive(Clone, Copy)]
enum CardMenu {
    Edit,
    Status(ReviewStatus),
    Copy,
    Delete,
}

/// Adds `value` to `list`, or takes it out when it is there.
fn toggle<T: PartialEq>(list: &mut Vec<T>, value: T) {
    match list.iter().position(|item| *item == value) {
        Some(at) => {
            list.remove(at);
        }
        None => list.push(value),
    }
}

/// Makes `thread` the selected one; false when it already was.
fn select_thread(state: &mut CommentsPanelState, thread: ThreadId) -> bool {
    let changed = state.selected != Some(thread);
    if changed {
        state.selected = Some(thread);
        state.reply.clear();
        state.editing = None;
    }
    changed
}

fn status_label(words: &CommentsWords, status: ReviewStatus) -> &Label {
    match status {
        ReviewStatus::None => &words.status_none,
        ReviewStatus::Accepted => &words.accepted,
        ReviewStatus::Rejected => &words.rejected,
        ReviewStatus::Cancelled => &words.cancelled,
        ReviewStatus::Completed => &words.completed,
    }
}

/// A label colour's name when it is one of the toolbar's palette colours, else its hex.
fn color_name(kinds: &AnnotateWords, color: [u8; 3]) -> String {
    match PALETTE.iter().position(|swatch| *swatch == color) {
        Some(at) => kinds.palette_color(at).to_owned(),
        None => format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]),
    }
}

/// The reader's name of a thread's kind.
fn kind_name(words: &CommentsWords, kinds: &AnnotateWords, kind: Option<AnnotationKind>) -> String {
    kind.map_or_else(|| words.plain_kind.as_str().to_owned(), |kind| kinds.kind(kind).to_owned())
}

/// The heading a thread falls under for `sort`, with its page; `None` for none.
fn group_of(
    words: &CommentsWords,
    kinds: &AnnotateWords,
    page_label: &dyn Fn(usize) -> String,
    thread: &CommentThread,
    sort: CommentSort,
) -> Option<(String, Option<usize>)> {
    match sort {
        CommentSort::Page => thread.anchor.page().map(|page| (words.page.fill(&[("label", &page_label(page))]), Some(page))),
        CommentSort::Author => {
            Some((thread.messages.first().map_or_else(|| words.unknown_author.as_str().to_owned(), |first| first.author.clone()), None))
        }
        CommentSort::Type => Some((kind_name(words, kinds, thread.kind), None)),
        CommentSort::Color => Some((thread.color.map_or_else(|| words.no_color.as_str().to_owned(), |color| color_name(kinds, color)), None)),
        CommentSort::Date => None,
    }
}

impl CommentsPanel<'_> {
    /// Shows the panel and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> CommentsPanelOutput {
        let CommentsPanel { label, threads, state, read_only, now, page_label } = self;
        let env = Env::of(ui);
        let words = env.words::<CommentsWords>();
        let kinds = env.words::<AnnotateWords>();
        let folder = env.folder();
        let format = env.format();
        let page_label = page_label.unwrap_or_else(|| Box::new(|page: usize| page.to_string()));
        let mut edits = Vec::new();
        let body = |ui: &mut Ui, _: &mut craft_ui::contract::BodyCx<'_>| -> Response {
            let digits = |n: usize| format.integer(n as u64);
            let replies: usize = threads.iter().map(CommentThread::replies).sum();
            Text::new(words.title.fill(&[("n", &digits(threads.len()))])).role(TypeRole::Title).show(ui);
            Text::new(words.replies.text(replies as u64, &digits(replies))).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            TextField::search(words.search.clone(), Bind::bound(&mut state.search)).placeholder(words.search.clone()).show(ui);
            Row::new().show(ui, |ui, _| {
                sort_menu(ui, &words, state);
                filter_menu(ui, &words, &kinds, threads, state);
                if state.sort == CommentSort::Page {
                    pages_menu(ui, &words, threads, state);
                }
            });
            if !read_only {
                TextField::new(words.add_comment.clone(), Bind::bound(&mut state.new_comment))
                    .lines(REPLY_LINES)
                    .placeholder(words.add_comment.clone())
                    .show(ui);
                let text = state.new_comment.trim().to_owned();
                let post = Button::new(words.post.clone()).enabled(!text.is_empty()).show(ui);
                if clicked(&post) {
                    edits.push(CommentEdit::Add { text });
                    state.new_comment.clear();
                }
            }
            if threads.is_empty() {
                return Notice::new(craft_theme::variant::Tone::Neutral, words.empty.clone()).layout(NoticeLayout::Centred).show(ui).response;
            }
            let query = state.search.trim().to_owned();
            let kind_of = |kind: AnnotationKind| kind_name(&words, &kinds, Some(kind));
            let shown: Vec<&CommentThread> = sort_threads(threads, state.sort, kind_of)
                .into_iter()
                .map(|i| &threads[i])
                .filter(|thread| state.filter.shows(thread))
                .filter(|thread| query.is_empty() || matches(thread, &query, &folder, kind_of))
                .collect();
            if shown.is_empty() {
                return Notice::new(craft_theme::variant::Tone::Neutral, words.no_match.clone()).layout(NoticeLayout::Centred).show(ui).response;
            }
            let by_page = state.sort == CommentSort::Page;
            let grouped = state.sort != CommentSort::Date;
            let mut page_counts: BTreeMap<usize, usize> = BTreeMap::new();
            for page in threads.iter().filter_map(|thread| thread.anchor.page()) {
                *page_counts.entry(page).or_default() += 1;
            }
            let heading_of = |thread: &CommentThread| group_of(&words, &kinds, &*page_label, thread, state.sort);
            let mut shown_counts: BTreeMap<String, usize> = BTreeMap::new();
            for thread in &shown {
                if let Some((title, _)) = heading_of(thread) {
                    *shown_counts.entry(title).or_default() += 1;
                }
            }
            let mut entries: Vec<(CardKey, RowEntry<'_>)> = Vec::new();
            let mut last: Option<String> = None;
            let mut groups = 0;
            // The source lists the rows on show: a folded group keeps its heading and drops its cards.
            let mut folded = false;
            for thread in &shown {
                let heading = if grouped { heading_of(thread) } else { None };
                let depth = usize::from(heading.is_some());
                if heading.is_none() {
                    folded = false;
                }
                if let Some((title, page)) = heading
                    && last.as_deref() != Some(title.as_str())
                {
                    let count = match page {
                        Some(number) if by_page => page_counts.get(&number).copied().unwrap_or_default(),
                        _ => shown_counts.get(&title).copied().unwrap_or_default(),
                    };
                    let open = !page.is_some_and(|number| state.collapsed_pages.contains(&number));
                    folded = !open;
                    entries.push((CardKey::Group(groups), RowEntry::Group { title: title.clone(), count: digits(count), page, open }));
                    groups += 1;
                    last = Some(title);
                }
                if !folded {
                    entries.push((CardKey::Thread(thread.id), RowEntry::Card { thread, depth }));
                }
            }
            let source = Rows { entries };
            let mut selection = state.selected.map_or_else(Selection::default, |thread| Selection::single(CardKey::Thread(thread)));
            let selected = state.selected;
            let reply: &str = &state.reply;
            let editing = state.editing.as_ref().map(|(thread, index, text)| (*thread, *index, text.as_str()));
            let mut openers = Vec::new();
            let out =
                CommentList::new(label.clone(), Some(Bind::bound(&mut selection)), grouped).show_items(ui, &source, &mut |_, item| match &source
                    .entries[item.at]
                    .1
                {
                    RowEntry::Group { title, count, open, .. } => CommentRow::heading(title.clone(), count.clone(), *open),
                    RowEntry::Card { thread, .. } => {
                        let name = thread
                            .messages
                            .first()
                            .map_or_else(String::new, |first| words.card_name.fill(&[("author", &first.author), ("text", &first.text)]));
                        let is_selected = selected == Some(thread.id);
                        let editing = editing.filter(|(id, ..)| *id == thread.id).map(|(_, index, text)| (index, text));
                        if is_selected {
                            openers.extend((0..thread.messages.len()).map(|index| (index, item.part_opener(MORE.at(index as u64)))));
                        }
                        CommentRow::card(thread, name)
                            .state(is_selected, !read_only && !thread.locked, now)
                            .drafts(editing, if is_selected { reply } else { "" })
                    }
                });
            let list = out.response.clone();
            let buttons: Vec<(usize, Response)> =
                openers.into_iter().filter_map(|(index, opener)| Some((index, out.opener(opener)?.clone()))).collect();
            let group_page = |at: usize| match source.entries.iter().find(|(key, _)| *key == CardKey::Group(at)) {
                Some((_, RowEntry::Group { page, .. })) => *page,
                _ => None,
            };
            for event in out.events {
                match event {
                    CollectionEvent::Item(_, card_events) => {
                        for event in card_events {
                            apply(event, state, &mut edits);
                        }
                    }
                    // A click or Enter on a heading folds or unfolds its page; Left and Right set it.
                    CollectionEvent::Activate(CardKey::Group(at)) => {
                        if let Some(page) = group_page(at) {
                            toggle(&mut state.collapsed_pages, page);
                        }
                    }
                    CollectionEvent::Open(change) => {
                        if let CardKey::Group(at) = change.key
                            && let Some(page) = group_page(at)
                        {
                            state.collapsed_pages.retain(|&folded| folded != page);
                            if !change.open {
                                state.collapsed_pages.push(page);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(CardKey::Thread(thread)) = selection.items.first().copied()
                && select_thread(state, thread)
            {
                edits.push(CommentEdit::Select(thread));
            }
            if let Some(thread) = state.selected.and_then(|selected| threads.iter().find(|thread| thread.id == selected)) {
                let editable = !read_only && !thread.locked;
                for (index, button) in &buttons {
                    if let Some(action) = card_menu(ui, &words, button, *index, thread, editable) {
                        apply_menu(action, thread, *index, state, &mut edits);
                    }
                }
            }
            list
        };
        let shown = Stack::new().fill().show(ui, body);
        let response = shown.inner.unwrap_or(shown.output);
        CommentsPanelOutput { response, edits }
    }
}

/// Applies what a card reported.
fn apply(event: CardEvent, state: &mut CommentsPanelState, edits: &mut Vec<CommentEdit>) {
    match event {
        CardEvent::Mark { thread, marked } => edits.push(CommentEdit::Mark { thread, marked }),
        CardEvent::EditText(text) => {
            if let Some((_, _, draft)) = state.editing.as_mut() {
                *draft = text;
            }
        }
        CardEvent::EditPost => {
            if let Some((thread, index, text)) = state.editing.take() {
                let text = text.trim().to_owned();
                if text.is_empty() {
                    state.editing = Some((thread, index, text));
                } else {
                    edits.push(CommentEdit::Edit { thread, index, text });
                }
            }
        }
        CardEvent::EditCancel => state.editing = None,
        CardEvent::ReplyText(text) => state.reply = text,
        CardEvent::ReplyPost => {
            let text = state.reply.trim().to_owned();
            if let (Some(thread), false) = (state.selected, text.is_empty()) {
                edits.push(CommentEdit::Reply { thread, text });
                state.reply.clear();
            }
        }
    }
}

/// Applies the row of a card's menu that ran.
fn apply_menu(action: CardMenu, thread: &CommentThread, index: usize, state: &mut CommentsPanelState, edits: &mut Vec<CommentEdit>) {
    let id = thread.id;
    match action {
        CardMenu::Edit => {
            let text = thread.messages.get(index).map(|message| message.text.clone()).unwrap_or_default();
            state.editing = Some((id, index, text));
        }
        CardMenu::Status(status) => edits.push(CommentEdit::SetStatus { thread: id, status }),
        CardMenu::Copy => edits.push(CommentEdit::CopyText { thread: id, index }),
        CardMenu::Delete => edits.push(CommentEdit::Delete { thread: id, index }),
    }
}
