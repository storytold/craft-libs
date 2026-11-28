//! The Comments panel: a header with the count, search, sort and filter menus, the add box and the cards.
//!
//! ```ignore
//! let out = CommentsPanel::new("Comments", &threads, &mut state).now(unix_seconds).show(ui);
//! for edit in out.edits {
//!     match edit {
//!         CommentEdit::Add { text } => doc.add_note(text),
//!         CommentEdit::Select(threads) => doc.select(threads),
//!         _ => {}
//!     }
//! }
//! ```

pub(crate) mod list;
mod menus;
pub(crate) mod row;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::collections::BTreeMap;

use craft_behavior::{IconId, Selection, Value};
use craft_i18n::{DateStyle, DateTime, TimeZone};
use craft_ui::contract::{Bind, CollectionEvent, Entry, NoValue, Outcome, Pattern, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::{Notice, NoticeIcon, NoticeLayout};
use craft_ui::primitives::{IconSpec, SubmitKey};
use craft_ui::runtime::env::{AgeStyle, Env, EnvView, Label};
use craft_ui::style::TypeRole;
use craft_ui::text::TextField;
use egui::{Pos2, Response, Ui};

use super::annotate::{AnnotateWords, PALETTE};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentFilter, CommentSort, CommentThread, PageOrder, ReviewStatus, ThreadId, matches, sort_threads};

use list::{CardKey, CommentList, RowEntry, Rows};
use menus::{card_menu, filter_menu, pages_menu, sort_menu};
pub use menus::{comments_filter_menu, comments_more_menu};
use row::MORE;

pub(super) use list::CardEvent;
pub(super) use row::{CardActions, CommentRow};

/// Ages under this many seconds (one week) read relative; older ones read as a date.
pub(crate) const AGE_SHOWN_AS_DATE: u64 = 7 * 24 * 60 * 60;

/// The rows of a reply box.
const REPLY_LINES: usize = 1;

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
        save: "comments.save" = "Save";
        cancel: "comments.cancel" = "Cancel";
        more: "comments.more" = "More";
        edit: "comments.edit" = "Edit";
        set_status: "comments.setStatus" = "Set status";
        delete: "comments.delete" = "Delete";
        reply_button: "comments.replyButton" = "Reply";
        resolve: "comments.resolve" = "Resolve";
        reopen: "comments.reopen" = "Reopen";
        copy_text: "comments.copyText" = "Copy text";
        mark: "comments.mark" = "Mark with checkmark";
        unmark: "comments.unmark" = "Remove checkmark";
        marked: "comments.marked" = "Marked";
        locked: "comments.locked" = "Locked";
        unknown_author: "comments.unknownAuthor" = "Unknown author";
        page: template "comments.page" = "Page {label}";
        card_name: template "comments.cardName" = "{author}: {text}";
        reviewer_status: template "comments.reviewerStatus" = "{reviewer}: {status}";
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
        empty_hint: "comments.emptyHint" = "Pick an annotation tool, or type above.";
        sort_heading: "comments.sortHeading" = "Sort by";
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
    /// The selected threads; the first is the lead, whose card shows the reply box.
    pub selected: Selection<ThreadId>,
    /// The search box is open.
    pub search_open: bool,
    /// The thread the next `show` reveals, once.
    revealing: Option<ThreadId>,
    /// The message being edited inline: (thread, message index, text). [`edit`](Self::edit) opens it.
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
    /// Selects only `thread`, and has the next `show` unfold its page group and scroll its card into view: for a
    /// comment the app selected elsewhere (on the page, or just added), whose card must be drawn.
    pub fn reveal(&mut self, thread: ThreadId) {
        set_selection(self, Selection::single(thread));
        self.revealing = Some(thread);
    }

    /// Opens the inline editor on message `index` of `thread` (0: the comment) with `text`: for an editor the app
    /// opens (a new comment's note), as a double click on a card or the message menu's Edit opens one. Like every
    /// inline editor, it takes the keyboard in the pass its card first shows it, once.
    pub fn edit(&mut self, thread: ThreadId, index: usize, text: impl Into<String>) {
        self.editing = Some((thread, index, text.into()));
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
    /// Set a thread's review status; the app records it for its current user.
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
    /// The selected cards changed, the lead first: the app selects the annotations on the page and scrolls to the
    /// lead's.
    Select(Vec<ThreadId>),
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

/// "5 min ago" under a week, then the date; nothing for `then` 0, a message or version that carries no date.
pub(crate) fn age_text(env: &EnvView<'_>, then: i64, now: i64) -> String {
    if then == 0 {
        return String::new();
    }
    let age = now.saturating_sub(then).max(0).unsigned_abs();
    if age < AGE_SHOWN_AS_DATE {
        env.relative_age(age, AgeStyle::Short)
    } else {
        env.dates().format(DateTime::from_unix(then, TimeZone::Local), DateStyle::DATE)
    }
}

/// The host's own menu rows for a thread's message at an index.
type HostRows<'a, H> = Box<dyn Fn(&CommentThread, usize) -> Vec<Entry<H>> + 'a>;

/// Acrobat's right-hand comment list.
///
/// - Header ([`CommentsHeader::Own`]) "Comments N" with "N replies" under it, search, the Sort by menu, the funnel
///   menu ([`CommentFilter`]: Type, Author, Status, Color, Checkmark rows that hide what they name) and, under the
///   Page sort, a menu with Collapse all pages and Expand all pages. Under [`CommentsHeader::Hosted`] the host draws
///   the header and places [`comments_more_menu`] and [`comments_filter_menu`]; the panel shows the search box while
///   [`CommentsPanelState::search_open`]. A search or filter that leaves nothing shows "No comments match."
/// - "Add a comment" box: a field whose Enter posts a sticky note.
/// - Cards in one collection, grouped under headings by [`sort_threads`]; under the Page sort a
///   heading names the page, shows the count of the page's comments before filtering and folds its
///   cards. A card shows the kind's icon, author, age (relative under a week, then the date), the
///   text, a badge per reviewer's status, the checkmark, a lock icon and the replies, each with its own "..." menu.
/// - Selected cards have the highlight; the lead (the first selected) has the accent bar, the reply box and "..."
///   (Edit, Set status, Delete, then the host's [`card_rows`](CommentsPanel::card_rows)). Locked threads and
///   `read_only` hide the editing items. A double click on a card edits its comment in place; an inline editor
///   takes the keyboard in the pass it opens. A new lead drops the reply draft and the edit of another thread; a
///   card made the lead again keeps its own edit draft.
/// - Keyboard: Up and Down move between cards (selecting them), Enter puts the focus in the lead card's reply box,
///   the [`reply_submit`](CommentsPanel::reply_submit) key in the reply box and Cmd+Enter (Ctrl+Enter off macOS) in
///   an inline editor post, Escape in a control of a card returns to the card.
///
/// ```ignore
/// let out = CommentsPanel::new("Comments", &threads, &mut state).now(now).show(ui);
/// ```
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
#[pattern(probe(CommentsPanel<'static, NoHostRow>))]
pub struct CommentsPanel<'a, H: Value = NoHostRow> {
    label: Label,
    threads: &'a [CommentThread],
    state: &'a mut CommentsPanelState,
    read_only: bool,
    now: i64,
    page_label: Option<Box<dyn Fn(usize) -> String + 'a>>,
    header: CommentsHeader,
    page_order: PageOrder,
    reply_submit: SubmitKey,
    select_mode: SelectMode,
    host_rows: Option<HostRows<'a, H>>,
}

/// The host row type of a panel whose cards' menus take no host rows: it has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NoHostRow {}

/// Who draws the panel's header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommentsHeader {
    /// The panel draws its own header.
    #[default]
    Own,
    /// The host draws the header, placing [`comments_more_menu`] and [`comments_filter_menu`].
    Hosted,
}

impl<'a> CommentsPanel<'a> {
    /// The panel named `label` over `threads`. The edits carry only the text; the app signs new
    /// comments with its own identity.
    pub fn new(label: impl Into<Label>, threads: &'a [CommentThread], state: &'a mut CommentsPanelState) -> Self {
        CommentsPanel {
            label: label.into(),
            threads,
            state,
            read_only: false,
            now: 0,
            page_label: None,
            header: CommentsHeader::Own,
            page_order: PageOrder::default(),
            reply_submit: SubmitKey::default(),
            select_mode: SelectMode::Single,
            host_rows: None,
        }
    }

    /// The rows a card's menu appends behind a separator, from the thread and the card's message index; a row the
    /// user picks is reported as `CommentsPanelOutput::host`.
    pub fn card_rows<H: Value>(self, rows: impl Fn(&CommentThread, usize) -> Vec<Entry<H>> + 'a) -> CommentsPanel<'a, H> {
        let CommentsPanel { label, threads, state, read_only, now, page_label, header, page_order, reply_submit, select_mode, host_rows: _ } = self;
        CommentsPanel {
            label,
            threads,
            state,
            read_only,
            now,
            page_label,
            header,
            page_order,
            reply_submit,
            select_mode,
            host_rows: Some(Box::new(rows)),
        }
    }
}

impl<'a, H: Value> CommentsPanel<'a, H> {
    /// Who draws the header.
    pub fn header(mut self, header: CommentsHeader) -> Self {
        self.header = header;
        self
    }

    /// How threads on one page are ordered under the Page sort.
    pub fn page_order(mut self, order: PageOrder) -> Self {
        self.page_order = order;
        self
    }

    /// The key that submits the reply box; an inline editor keeps `CommandEnter`.
    pub fn reply_submit(mut self, key: SubmitKey) -> Self {
        self.reply_submit = key;
        self
    }

    /// How a click selects cards.
    pub fn select_mode(mut self, mode: SelectMode) -> Self {
        self.select_mode = mode;
        self
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
pub struct CommentsPanelOutput<H = NoHostRow> {
    /// The panel's response.
    pub response: Response,
    /// This pass's edits, in order.
    pub edits: Vec<CommentEdit>,
    /// The host row picked in a card's menu: the thread, the card's message index and the row.
    pub host: Option<(ThreadId, usize, H)>,
}

impl<H: Value> Outcome for CommentsPanelOutput<H> {
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
    /// The host row at this index of the card's host rows.
    Host(usize),
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

/// Makes `next` the selection; true when its threads changed. A new lead drops the reply draft, which belongs to the
/// old lead's card, and an edit of another thread; an edit of the new lead itself stays, so a draft the app could not
/// write back (a document that refused it) is there again when its card leads again.
fn set_selection(state: &mut CommentsPanelState, next: Selection<ThreadId>) -> bool {
    let changed = next.items != state.selected.items;
    if next.lead != state.selected.lead {
        state.reply.clear();
        let kept = state.editing.as_ref().is_some_and(|(editing, ..)| Some(*editing) == next.lead);
        if !kept {
            state.editing = None;
        }
    }
    state.selected = next;
    changed
}

/// The card collection's selection of the threads in `selected`, in its order.
fn card_selection(selected: &Selection<ThreadId>) -> Selection<CardKey> {
    Selection::new(
        selected.items.iter().copied().map(CardKey::Thread).collect(),
        selected.lead.map(CardKey::Thread),
        selected.anchor.map(CardKey::Thread),
    )
}

/// The threads the collection's `selection` holds, headings left out.
fn thread_selection(selection: &Selection<CardKey>) -> Selection<ThreadId> {
    let thread = |key: &CardKey| match key {
        CardKey::Thread(id) => Some(*id),
        CardKey::Group(_) => None,
    };
    Selection::new(
        selection.items.iter().filter_map(thread).collect(),
        selection.lead.as_ref().and_then(thread),
        selection.anchor.as_ref().and_then(thread),
    )
}

/// The selected threads in the order [`CommentEdit::Select`] reports them: the lead first.
fn lead_first(selected: &Selection<ThreadId>) -> Vec<ThreadId> {
    let mut items = selected.items.clone();
    if let Some(lead) = selected.lead
        && let Some(at) = items.iter().position(|id| *id == lead)
    {
        let lead = items.remove(at);
        items.insert(0, lead);
    }
    items
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

/// A group heading's title, with its page under the Page sort.
type Heading = (String, Option<usize>);

/// The list's rows: each shown thread's card under its group heading (by `sort`; none by date), with the group's
/// count, and a folded page group's heading without its cards.
fn row_entries<'t>(
    threads: &[CommentThread],
    shown: &[&'t CommentThread],
    sort: CommentSort,
    collapsed: &[usize],
    heading_of: &dyn Fn(&CommentThread) -> Option<Heading>,
    digits: &dyn Fn(usize) -> String,
) -> Vec<(CardKey, RowEntry<'t>)> {
    let by_page = sort == CommentSort::Page;
    let grouped = sort != CommentSort::Date;
    let mut page_counts: BTreeMap<usize, usize> = BTreeMap::new();
    for page in threads.iter().filter_map(|thread| thread.anchor.page()) {
        *page_counts.entry(page).or_default() += 1;
    }
    let mut shown_counts: BTreeMap<String, usize> = BTreeMap::new();
    for thread in shown {
        if let Some((title, _)) = heading_of(thread) {
            *shown_counts.entry(title).or_default() += 1;
        }
    }
    let mut entries: Vec<(CardKey, RowEntry<'_>)> = Vec::new();
    let mut last: Option<String> = None;
    let mut groups = 0;
    // The source lists the rows on show: a folded group keeps its heading and drops its cards.
    let mut folded = false;
    for &thread in shown {
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
            let open = !page.is_some_and(|number| collapsed.contains(&number));
            folded = !open;
            entries.push((CardKey::Group(groups), RowEntry::Group { title: title.clone(), count: digits(count), page, open }));
            groups += 1;
            last = Some(title);
        }
        if !folded {
            entries.push((CardKey::Thread(thread.id), RowEntry::Card { thread, depth }));
        }
    }
    entries
}

/// The heading a thread falls under for `sort`, with its page; `None` for none.
fn group_of(
    words: &CommentsWords,
    kinds: &AnnotateWords,
    page_label: &dyn Fn(usize) -> String,
    thread: &CommentThread,
    sort: CommentSort,
) -> Option<Heading> {
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

impl<H: Value> CommentsPanel<'_, H> {
    /// Shows the panel and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> CommentsPanelOutput<H> {
        let CommentsPanel { label, threads, state, read_only, now, page_label, header, page_order, reply_submit, select_mode, host_rows } = self;
        let env = Env::of(ui);
        let words = env.words::<CommentsWords>();
        let kinds = env.words::<AnnotateWords>();
        let folder = env.folder();
        let format = env.format();
        let page_label = page_label.unwrap_or_else(|| Box::new(|page: usize| page.to_string()));
        // A reveal is a request for one pass: this one unfolds the thread's page and scrolls its card in, so later
        // passes leave the list where the user puts it.
        let revealing = state.revealing.take();
        if let Some(page) = revealing.and_then(|id| threads.iter().find(|thread| thread.id == id)).and_then(|thread| thread.anchor.page()) {
            state.collapsed_pages.retain(|&folded| folded != page);
        }
        let mut edits = Vec::new();
        let mut host = None;
        let body = |ui: &mut Ui, _: &mut craft_ui::contract::BodyCx<'_>| -> Response {
            let digits = |n: usize| format.integer(n as u64);
            match header {
                CommentsHeader::Own => {
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
                }
                CommentsHeader::Hosted => {
                    if state.search_open {
                        TextField::search(words.search.clone(), Bind::bound(&mut state.search)).placeholder(words.search.clone()).show(ui);
                    }
                }
            }
            if !read_only {
                let field = TextField::new(words.add_comment.clone(), Bind::bound(&mut state.new_comment))
                    .placeholder(words.add_comment.clone())
                    .submits()
                    .show(ui);
                let text = state.new_comment.trim().to_owned();
                if field.submitted && !text.is_empty() {
                    edits.push(CommentEdit::Add { text });
                    state.new_comment.clear();
                }
            }
            if threads.is_empty() {
                // A comment icon over what is missing and what to do about it.
                return Notice::new(craft_theme::variant::Tone::Neutral, words.empty_hint.clone())
                    .title(words.empty.clone())
                    .icon(NoticeIcon::Custom(IconSpec::svg(IconId::new("note"))))
                    .layout(NoticeLayout::Centred)
                    .show(ui)
                    .response;
            }
            let query = state.search.trim().to_owned();
            let kind_of = |kind: AnnotationKind| kind_name(&words, &kinds, Some(kind));
            let shown: Vec<&CommentThread> = sort_threads(threads, state.sort, page_order, kind_of)
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
            let heading_of = |thread: &CommentThread| group_of(&words, &kinds, &*page_label, thread, state.sort);
            let source = Rows { entries: row_entries(threads, &shown, state.sort, &state.collapsed_pages, &heading_of, &digits) };
            let lead = state.selected.lead;
            let chosen = state.selected.items.clone();
            let mut selection = card_selection(&state.selected);
            let reply: &str = &state.reply;
            let editing = state.editing.as_ref().map(|(thread, index, text)| (*thread, *index, text.as_str()));
            let mut openers = Vec::new();
            let list = CommentList::new(label.clone(), Some(Bind::bound(&mut selection)), grouped, select_mode);
            let list = match revealing {
                Some(thread) => list.revealing(CardKey::Thread(thread)),
                None => list,
            };
            let out = list.show_items(ui, &source, &mut |_, item| match &source.entries[item.at].1 {
                RowEntry::Group { title, count, open, page } => CommentRow::heading(title.clone(), count.clone(), *open, !by_page || page.is_none()),
                RowEntry::Card { thread, .. } => {
                    let name = thread
                        .messages
                        .first()
                        .map_or_else(String::new, |first| words.card_name.fill(&[("author", &first.author), ("text", &first.text)]));
                    let is_lead = lead == Some(thread.id);
                    let editing = editing.filter(|(id, ..)| *id == thread.id).map(|(_, index, text)| (index, text));
                    if is_lead {
                        openers.extend((0..thread.messages.len()).map(|index| (index, item.part_opener(MORE.at(index as u64)))));
                    }
                    CommentRow::card(thread, name)
                        .state(chosen.contains(&thread.id), !read_only && !thread.locked, now)
                        .lead(is_lead)
                        .focus_reply(is_lead && item.row.activated_by_key)
                        .submit_key(reply_submit)
                        .drafts(editing, if is_lead { reply } else { "" })
                }
            });
            let list = out.response.clone();
            let clicks = out.activation_clicks;
            let mut buttons: Vec<(usize, Response)> = Vec::new();
            // The collection may build the lead's card more than once in a pass: each message's menu opens once, from
            // the last build's "..." button, so no two menus share a base id.
            for (index, opener) in openers.into_iter().rev() {
                if !buttons.iter().any(|(seen, _)| *seen == index)
                    && let Some(button) = out.opener(opener)
                {
                    buttons.push((index, button.clone()));
                }
            }
            buttons.reverse();
            let group_page = |at: usize| match source.entries.iter().find(|(key, _)| *key == CardKey::Group(at)) {
                Some((_, RowEntry::Group { page, .. })) => *page,
                _ => None,
            };
            let picked = thread_selection(&selection);
            // A click on a heading picks the heading alone: the threads stay selected.
            let headings_only = picked.items.is_empty() && !selection.items.is_empty();
            if !headings_only && set_selection(state, picked) {
                edits.push(CommentEdit::Select(lead_first(&state.selected)));
            }
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
                    // A double click on a card edits its comment in place.
                    CollectionEvent::Activate(CardKey::Thread(id)) if clicks == 2 && !read_only => {
                        if let Some(thread) = threads.iter().find(|thread| thread.id == id && !thread.locked) {
                            let text = thread.messages.first().map(|message| message.text.clone()).unwrap_or_default();
                            state.edit(id, 0, text);
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
            // The "..." buttons are the card drawn as the lead this pass.
            if let Some(thread) = lead.and_then(|selected| threads.iter().find(|thread| thread.id == selected)) {
                let editable = !read_only;
                for (index, button) in &buttons {
                    let rows = host_rows.as_ref().map_or_else(Vec::new, |rows| rows(thread, *index));
                    if let Some(action) = card_menu(ui, &words, button, *index, thread, editable, &rows) {
                        apply_menu(action, thread, *index, &rows, state, &mut edits, &mut host);
                    }
                }
            }
            list
        };
        let shown = Stack::new().fill().show(ui, body);
        let response = shown.inner.unwrap_or(shown.output);
        CommentsPanelOutput { response, edits, host }
    }
}

/// Applies what a card reported.
pub(super) fn apply(event: CardEvent, state: &mut CommentsPanelState, edits: &mut Vec<CommentEdit>) {
    match event {
        CardEvent::Mark { thread, marked } => edits.push(CommentEdit::Mark { thread, marked }),
        CardEvent::Resolve { thread, resolved } => edits.push(CommentEdit::Resolve { thread, resolved }),
        // Index 0 deletes the whole thread.
        CardEvent::Delete { thread } => edits.push(CommentEdit::Delete { thread, index: 0 }),
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
            if let (Some(thread), false) = (state.selected.lead, text.is_empty()) {
                edits.push(CommentEdit::Reply { thread, text });
                state.reply.clear();
            }
        }
    }
}

/// Applies the row of a card's menu that ran; a host row goes to `host` with the value of `rows` at its index.
fn apply_menu<H: Value>(
    action: CardMenu,
    thread: &CommentThread,
    index: usize,
    rows: &[Entry<H>],
    state: &mut CommentsPanelState,
    edits: &mut Vec<CommentEdit>,
    host: &mut Option<(ThreadId, usize, H)>,
) {
    let id = thread.id;
    match action {
        CardMenu::Edit => {
            let text = thread.messages.get(index).map(|message| message.text.clone()).unwrap_or_default();
            state.edit(id, index, text);
        }
        CardMenu::Status(status) => edits.push(CommentEdit::SetStatus { thread: id, status }),
        CardMenu::Copy => edits.push(CommentEdit::CopyText { thread: id, index }),
        CardMenu::Delete => edits.push(CommentEdit::Delete { thread: id, index }),
        CardMenu::Host(at) => {
            if let Some(value) = rows.get(at).and_then(Entry::value) {
                *host = Some((id, index, value.clone()));
            }
        }
    }
}
