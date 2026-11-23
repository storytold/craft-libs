//! One card or heading row: [`CommentRow`] and its measure and paint sinks.

use std::sync::Arc;

use craft_behavior::{IconId, Metrics, PartKey, Proposal, Sense};
use craft_theme::variant::{Emphasis, Tone as BadgeTone, Variant};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Props as _, Remember as _, Role, WordSetCx as _, clicked};
use craft_ui::controls::disclosure::DisclosureGlyph;
use craft_ui::display::Badge;
use craft_ui::primitives::{HAlign, IconSpec, PaintShape, StrokeWidth, SubmitKey, TextRun, Wrap};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::style::{ComponentStyle, DataColor, Ink, StyleColor, StyleCx, TypeRole};
use craft_ui::text::TextField;
use egui::{Color32, CornerRadius, Pos2, Rect, Vec2, vec2};

use super::list::{CardEvent, RowOut};
use super::{CommentsWords, EDIT_LINES, Line, REPLY_LINES, age_text, status_label};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentMessage, CommentThread, ReviewStatus, ReviewerStatus, ThreadId};
use crate::ui::avatar::paint_face;

/// The style of a card or heading row.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct CommentRowStyle {
    #[style(metrics)]
    metrics: Metrics,
    normal: StyleColor,
    muted: StyleColor,
    highlight: StyleColor,
    bar: StyleColor,
    /// The surface of a balloon card.
    card: StyleColor,
    /// The border of a balloon card.
    border: StyleColor,
    avatar: StyleColor,
    avatar_ink: StyleColor,
    /// The line that joins a card's avatars.
    thread_line: StyleColor,
    /// The kind icon's side.
    kind_icon: f32,
    /// The side of a plain icon: the glyph of the icon buttons beside it.
    glyph: f32,
    bar_width: f32,
    /// The width of a card when the room names none.
    column: f32,
    height: f32,
    pad: f32,
    gap: f32,
}

/// What a heading shows.
struct Heading {
    title: String,
    count: String,
    open: bool,
    /// The heading names a group that does not fold (not a page): the title alone, with no disclosure or count shown.
    plain: bool,
}

/// What a revision card shows: a tracked change's heading and its detail lines.
struct Note {
    heading: String,
    details: Vec<String>,
}

/// How a text or icon is inked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Normal,
    Muted,
}

/// How a text is set.
#[derive(Clone, Copy)]
struct Spec {
    role: TypeRole,
    tone: Tone,
    wrap: Wrap,
    end: bool,
}

impl Spec {
    const fn new(role: TypeRole, tone: Tone, wrap: Wrap) -> Self {
        Spec { role, tone, wrap, end: false }
    }

    /// Limits the text to `rows` rows, if any: the text layout cuts what is beyond (`Wrap::Clamp`, S-L173), so a long
    /// body never pushes the next card out of view.
    fn max_rows(mut self, rows: Option<usize>) -> Self {
        if let Some(rows) = rows {
            self.wrap = Wrap::Clamp(u8::try_from(rows).unwrap_or(u8::MAX));
        }
        self
    }

    const fn end(mut self) -> Self {
        self.end = true;
        self
    }
}

/// A control of a row, which names its child component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Slot {
    More(usize),
    Mark,
    EditField,
    EditPost,
    EditCancel,
    ReplyField,
    ReplyPost,
    /// The balloon button that puts the focus in the reply field.
    Reply,
    /// The balloon button that resolves or reopens the thread.
    Resolve,
    /// The balloon button that deletes the thread.
    Delete,
    /// The badge of the reviewer status at this index of the thread's statuses.
    Status(usize),
}

/// How many push buttons a row sizes: `push_index` gives each its place.
const PUSH_BUTTONS: usize = 6;

/// Where `CommentRow::lay` puts the parts of a row: the measuring pass sizes them, the paint pass shows them.
/// Rects are relative to the row's top left, laid out left to right; the sink mirrors them.
trait Sink {
    /// Text set in `at`'s width from `at`'s top left; returns its size.
    fn text(&mut self, at: Rect, spec: Spec, text: &str) -> Vec2;
    /// The size `text` takes on one line.
    fn size(&mut self, spec: Spec, text: &str) -> Vec2;
    /// The icon `id` centred in `rect`, at the glyph side of the icon buttons beside it.
    fn icon(&mut self, rect: Rect, id: &'static str, tone: Tone);
    /// A heading's disclosure, `open` or closed, centred in `rect`.
    fn disclosure(&mut self, rect: Rect, open: bool);
    /// A card's kind: the icon `id` on a sunken disc filling `rect`, in the annotation's `color` (the accent without one).
    fn kind(&mut self, rect: Rect, id: &'static str, color: Option<[u8; 3]>);
    /// The avatar of `name`.
    fn avatar(&mut self, rect: Rect, name: &str);
    /// The line that joins the avatars above and below it, from `from` down to `to`.
    fn connector(&mut self, from: Pos2, to: Pos2);
    /// A square icon button; it reports its press.
    fn button(&mut self, slot: Slot, rect: Rect, id: &'static str, name: &Label, on: bool);
    /// A multi-line field at `at`'s top left and width; returns its height.
    fn field(&mut self, slot: Slot, at: Rect, label: &Label, text: &str, lines: usize) -> f32;
    /// The size the push button `slot` with `label` measures.
    fn push_size(&mut self, slot: Slot, label: &Label) -> Vec2;
    /// A push button.
    fn push(&mut self, slot: Slot, rect: Rect, label: &Label, enabled: bool);
    /// The size the status badge `slot` with `label` measures.
    fn badge_size(&mut self, slot: Slot, label: &str) -> Vec2;
    /// A status badge showing `label` in the tone of `status`.
    fn badge(&mut self, slot: Slot, rect: Rect, label: &str, status: ReviewStatus);
    /// The extent of reply `index` (a message after the first), from its author line to the bottom of its text.
    fn reply(&mut self, index: usize, rect: Rect);
}

/// The "..." button of message `index` of a card: a part of the row that opens the message's menu.
pub(super) const MORE: PartKey = PartKey::new("more");

/// Reply `index` of a card: the access node that names it.
const REPLY: PartKey = PartKey::new("reply");

/// What a card offers for acting on its thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CardActions {
    /// The "..." menu of each message.
    #[default]
    Menu,
    /// Production's balloon buttons on the selected card: Reply, Delete, and Resolve or Reopen.
    Buttons,
}

/// The message whose inline editor a row showed at its last pass, if any.
#[derive(Clone, Copy, Default)]
struct EditShown(Option<usize>);

/// A card or a heading of the card collection.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub(crate) struct CommentRow<'r> {
    #[common]
    common: Common,
    thread: Option<&'r CommentThread>,
    heading: Option<Heading>,
    /// A revision card's text, which takes the row's own colours.
    note: Option<Note>,
    selected: bool,
    /// Whether the card is the selection's lead, which shows the reply box and the editing buttons.
    #[prop(vis = "pub(crate)")]
    lead: bool,
    editable: bool,
    now: i64,
    editing: Option<(usize, &'r str)>,
    reply: &'r str,
    /// The activation key reached the card this pass: its reply box takes the focus, when it shows one.
    #[prop]
    focus_reply: bool,
    /// The key that submits the reply box; the inline editor keeps `CommandEnter`.
    #[prop(vis = "pub(crate)")]
    submit_key: SubmitKey,
    /// The inline editor opened this pass (it did not show the pass before): its field takes the keyboard, so the
    /// first click that opens an edit is enough to type.
    focus_edit: bool,
    /// Picks the card's buttons: [`CardActions::Buttons`] shows production's balloon buttons on the selected card.
    #[prop(vis = "pub(crate)")]
    actions: CardActions,
    /// Limits each message's text to `rows` rows.
    #[prop(vis = "pub(crate)")]
    body_rows: Option<usize>,
    words: Option<Arc<CommentsWords>>,
    /// The heights the editor field and the reply field measured, for the paint of the same pass.
    heights: [f32; 2],
    /// The sizes the push buttons measured, for the paint of the same pass.
    buttons: [Vec2; PUSH_BUTTONS],
    /// The sizes the status badges measured, in the order of the thread's statuses, for the paint of the same pass.
    badges: Vec<Vec2>,
    /// The messages whose "..." button the parts step declared.
    mores: Vec<usize>,
    /// Each reply's index and its rect on the screen, from the parts step.
    replies: Vec<(usize, Rect)>,
    events: Vec<CardEvent>,
}

impl<'r> CommentRow<'r> {
    fn bare(thread: Option<&'r CommentThread>, heading: Option<Heading>, name: String) -> Self {
        CommentRow {
            common: Common::new(),
            thread,
            heading,
            note: None,
            selected: false,
            lead: false,
            editable: false,
            now: 0,
            editing: None,
            reply: "",
            focus_reply: false,
            submit_key: SubmitKey::default(),
            focus_edit: false,
            actions: CardActions::default(),
            body_rows: None,
            words: None,
            heights: [0.0; 2],
            buttons: [Vec2::ZERO; PUSH_BUTTONS],
            badges: Vec::new(),
            mores: Vec::new(),
            replies: Vec::new(),
            events: Vec::new(),
        }
        .label(name)
    }

    /// A heading row.
    pub(crate) fn heading(title: String, count: String, open: bool, plain: bool) -> Self {
        let name = title.clone();
        Self::bare(None, Some(Heading { title, count, open, plain }), name)
    }

    /// A revision card: `heading` over its `details`, in the row's colours, so balloons have one card style.
    pub(crate) fn revision(heading: String, details: Vec<String>) -> Self {
        let name = heading.clone();
        let mut row = Self::bare(None, None, name);
        row.note = Some(Note { heading, details });
        row
    }

    /// The card of `thread`, named `name`.
    pub(crate) fn card(thread: &'r CommentThread, name: String) -> Self {
        Self::bare(Some(thread), None, name)
    }

    /// Whether the card is selected, whether its editing items show and the time its ages count from.
    pub(crate) fn state(mut self, selected: bool, editable: bool, now: i64) -> Self {
        self.selected = selected;
        self.editable = editable;
        self.now = now;
        self
    }

    /// The message being edited and its draft, and the reply draft.
    pub(crate) fn drafts(mut self, editing: Option<(usize, &'r str)>, reply: &'r str) -> Self {
        self.editing = editing;
        self.reply = reply;
        self
    }
}

fn kind_icon(kind: Option<AnnotationKind>) -> &'static str {
    use AnnotationKind as K;
    match kind {
        Some(K::Highlight | K::Underline | K::StrikeOut | K::Squiggly) => "brush",
        Some(K::TextBox | K::Callout) => "text",
        Some(K::Ink) => "pencil",
        Some(K::Attach) => "link",
        _ => "note",
    }
}

/// The tone of a status badge. A status set back to None stays shown, neutral: None is a state a reviewer chose.
fn badge_tone(status: ReviewStatus) -> BadgeTone {
    match status {
        ReviewStatus::Accepted | ReviewStatus::Completed => BadgeTone::Good,
        ReviewStatus::Rejected => BadgeTone::Bad,
        ReviewStatus::Cancelled | ReviewStatus::None => BadgeTone::Neutral,
    }
}

/// A status badge's text: "{reviewer}: {status}", or the status alone for a reviewer without a name.
fn reviewer_label(words: &CommentsWords, reviewer: &ReviewerStatus) -> String {
    let status = status_label(words, reviewer.status).as_str();
    if reviewer.reviewer.is_empty() {
        status.to_owned()
    } else {
        words.reviewer_status.fill(&[("reviewer", reviewer.reviewer.as_str()), ("status", status)])
    }
}

fn square(at: Pos2, side: f32) -> Rect {
    Rect::from_min_size(at, Vec2::splat(side))
}

fn strip(at: Pos2, width: f32) -> Rect {
    Rect::from_min_size(at, vec2(width.max(0.0), 0.0))
}

impl CommentRow<'_> {
    /// Places every part of the row for `width` and returns the row's height. `age` writes an age.
    fn lay(&self, st: &CommentRowStyle, width: f32, age: &dyn Fn(i64) -> String, sink: &mut dyn Sink) -> f32 {
        let words = self.words.clone().unwrap_or_default();
        let (h, gap, pad) = (st.height, st.gap, st.pad);
        let room = (width - 2.0 * pad).max(h);
        if let Some(heading) = &self.heading {
            if heading.plain {
                let title = Spec::new(TypeRole::BodyStrong, Tone::Muted, Wrap::Elide);
                sink.text(strip(Pos2::new(pad, pad), room), title, &heading.title);
                return h + 2.0 * pad;
            }
            sink.disclosure(square(Pos2::new(pad, pad), h), heading.open);
            let count_spec = Spec::new(TypeRole::Caption, Tone::Muted, Wrap::Wrap).end();
            let count = sink.size(count_spec, &heading.count).x;
            let x = pad + h + gap;
            let title = Spec::new(TypeRole::BodyStrong, Tone::Muted, Wrap::Elide);
            sink.text(strip(Pos2::new(x, pad), width - pad - count - gap - x), title, &heading.title);
            sink.text(strip(Pos2::new(width - pad - count, pad), count), count_spec, &heading.count);
            return h + 2.0 * pad;
        }
        if let Some(note) = &self.note {
            let mut y = pad;
            let title = Spec::new(TypeRole::BodyStrong, Tone::Normal, Wrap::Wrap);
            y += sink.text(strip(Pos2::new(pad, y), room), title, &note.heading).y;
            for detail in &note.details {
                y += gap;
                y += sink.text(strip(Pos2::new(pad, y), room), Spec::new(TypeRole::Body, Tone::Muted, Wrap::Wrap), detail).y;
            }
            return y + pad;
        }
        let Some(thread) = self.thread else { return h };
        let y = pad;
        let first = thread.messages.first();
        sink.kind(square(Pos2::new(pad, y), h), kind_icon(thread.kind), thread.color);
        let mut end = width - pad;
        let menu = self.actions == CardActions::Menu;
        if self.lead && self.editable && menu {
            end -= h;
            sink.button(Slot::More(0), square(Pos2::new(end, y), h), "more", &words.more, false);
        }
        if thread.locked {
            end -= h;
            sink.icon(square(Pos2::new(end, y), h), "lock", Tone::Muted);
        }
        if self.lead && self.editable {
            end -= h;
            let name = if thread.marked { &words.unmark } else { &words.mark };
            sink.button(Slot::Mark, square(Pos2::new(end, y), h), "check", name, thread.marked);
        } else if thread.marked {
            end -= h;
            sink.icon(square(Pos2::new(end, y), h), "check", Tone::Muted);
        }
        let name_at = pad + h + gap;
        let name_room = end - gap - name_at;
        let author = first.map_or(words.unknown_author.as_str(), |message| message.author.as_str());
        let author_spec = Spec::new(TypeRole::BodyStrong, Tone::Normal, Wrap::Elide);
        let author_size = sink.text(strip(Pos2::new(name_at, y), name_room), author_spec, author);
        let mut head = author_size.y.max(h);
        // An undated message has no age line, so its author takes the head alone.
        if let Some(message) = first {
            let when = age(message.modified.unwrap_or(message.created));
            if !when.is_empty() {
                let caption = Spec::new(TypeRole::Caption, Tone::Muted, Wrap::Elide);
                let size = sink.text(strip(Pos2::new(name_at, y + author_size.y), name_room), caption, &when);
                head = head.max(author_size.y + size.y);
            }
        }
        let mut y = y + head + gap;
        // The comment's text and badges sit under the author's name, not under the kind disc.
        let under = Line { at: Pos2::new(name_at, y), room: (width - pad - name_at).max(h) };
        y = self.message(0, thread, st, under, &words, sink) + gap;
        y = self.statuses(thread, st, Line { at: Pos2::new(name_at, y), room: under.room }, &words, sink);
        // The thread line runs from the kind disc down through the replies' avatars.
        let line_x = pad + h / 2.0;
        let mut joined = pad + h;
        for index in 1..thread.messages.len() {
            let Some(message) = thread.messages.get(index) else { break };
            let top = y;
            sink.connector(Pos2::new(line_x, joined), Pos2::new(line_x, top));
            joined = top + h;
            sink.avatar(square(Pos2::new(pad, y), h), &message.author);
            let x = pad + h + gap;
            let name_room = width - pad - h - gap - x;
            // A reply's author and age share one line, each centred on it, where both fit whole; else the age goes
            // under the author, as on a card's first message. An undated reply shows its author alone.
            let when = age(message.modified.unwrap_or(message.created));
            let caption = Spec::new(TypeRole::Caption, Tone::Muted, Wrap::Elide);
            let (author_line, when_line) = (sink.size(author_spec, &message.author), sink.size(caption, &when));
            let head = if when.is_empty() {
                sink.text(strip(Pos2::new(x, y), name_room), author_spec, &message.author).y
            } else if author_line.x + gap + when_line.x <= name_room {
                let line = author_line.y.max(when_line.y);
                sink.text(strip(Pos2::new(x, y + (line - author_line.y) / 2.0), author_line.x), author_spec, &message.author);
                sink.text(strip(Pos2::new(x + author_line.x + gap, y + (line - when_line.y) / 2.0), when_line.x), caption, &when);
                line
            } else {
                let author_size = sink.text(strip(Pos2::new(x, y), name_room), author_spec, &message.author);
                author_size.y + sink.text(strip(Pos2::new(x, y + author_size.y), name_room), caption, &when).y
            };
            if menu {
                sink.button(Slot::More(index), square(Pos2::new(width - pad - h, y), h), "more", &words.more, false);
            }
            y += h.max(head) + gap;
            let bottom = self.message(index, thread, st, Line { at: Pos2::new(x, y), room: width - pad - x }, &words, sink);
            sink.reply(index, Rect::from_min_max(Pos2::new(pad, top), Pos2::new(width - pad, bottom)));
            y = bottom + gap;
        }
        if self.lead && self.editable {
            let field = sink.field(Slot::ReplyField, strip(Pos2::new(name_at, y), under.room), &words.reply, self.reply, REPLY_LINES);
            y += field + gap;
            let button = sink.push_size(Slot::ReplyPost, &words.post);
            let post = Rect::from_min_size(Pos2::new(width - pad - button.x, y), button);
            sink.push(Slot::ReplyPost, post, &words.post, !self.reply.trim().is_empty());
            y += button.y + gap;
            if !menu {
                let resolve = if thread.resolved { &words.reopen } else { &words.resolve };
                let mut x = pad;
                let mut tallest = 0.0_f32;
                for (slot, label) in [(Slot::Reply, &words.reply_button), (Slot::Resolve, resolve), (Slot::Delete, &words.delete)] {
                    let size = sink.push_size(slot, label);
                    sink.push(slot, Rect::from_min_size(Pos2::new(x, y), size), label, true);
                    x += size.x + gap;
                    tallest = tallest.max(size.y);
                }
                y += tallest + gap;
            }
        }
        y - gap + pad
    }

    /// One badge per reviewer's status, in lines from `line.at` that wrap at `line.room`; a badge wider than the room
    /// takes the room and cuts its text. Returns where the next part starts.
    fn statuses(&self, thread: &CommentThread, st: &CommentRowStyle, line: Line, words: &CommentsWords, sink: &mut dyn Sink) -> f32 {
        let Line { at, room } = line;
        if thread.statuses.is_empty() {
            return at.y;
        }
        let (mut x, mut y, mut tallest) = (at.x, at.y, 0.0_f32);
        for (index, reviewer) in thread.statuses.iter().enumerate() {
            let label = reviewer_label(words, reviewer);
            let size = sink.badge_size(Slot::Status(index), &label);
            if x > at.x && x + size.x > at.x + room {
                y += tallest + st.gap;
                x = at.x;
                tallest = 0.0;
            }
            let width = size.x.min(at.x + room - x);
            sink.badge(Slot::Status(index), Rect::from_min_size(Pos2::new(x, y), vec2(width, size.y)), &label, reviewer.status);
            x += width + st.gap;
            tallest = tallest.max(size.y);
        }
        y + tallest + st.gap
    }

    /// Message `index`'s text, or its inline editor; returns the bottom.
    fn message(&self, index: usize, thread: &CommentThread, st: &CommentRowStyle, line: Line, words: &CommentsWords, sink: &mut dyn Sink) -> f32 {
        let Line { at, room } = line;
        let Some(message) = thread.messages.get(index) else { return at.y };
        match self.editing {
            Some((editing, draft)) if editing == index => {
                let field = sink.field(Slot::EditField, strip(at, room), &words.edit, draft, EDIT_LINES);
                let y = at.y + field + st.gap;
                let (post_size, cancel_size) = (sink.push_size(Slot::EditPost, &words.save), sink.push_size(Slot::EditCancel, &words.cancel));
                let right = at.x + room;
                let cancel = Rect::from_min_size(Pos2::new(right - cancel_size.x, y), cancel_size);
                let post = Rect::from_min_size(Pos2::new(cancel.left() - st.gap - post_size.x, y), post_size);
                sink.push(Slot::EditPost, post, &words.save, !draft.trim().is_empty());
                sink.push(Slot::EditCancel, cancel, &words.cancel, true);
                y + post_size.y.max(cancel_size.y)
            }
            _ => {
                let body = Spec::new(TypeRole::Body, Tone::Normal, Wrap::Wrap);
                at.y + sink.text(strip(at, room), body.max_rows(self.body_rows), &message.text).y
            }
        }
    }
}

/// Sizes the parts of a row.
struct Measure<'a, 'b, 'r> {
    cx: &'a MeasureCx<'b, CommentRow<'r>>,
    fields: [f32; 2],
    buttons: [Vec2; PUSH_BUTTONS],
    /// The message index and left-to-right rect of each "..." button.
    more: Vec<(usize, Rect)>,
    /// The size of each status badge, in the order the row lays them.
    badges: Vec<Vec2>,
    /// The message index and left-to-right rect of each reply.
    replies: Vec<(usize, Rect)>,
}

impl Sink for Measure<'_, '_, '_> {
    fn text(&mut self, at: Rect, spec: Spec, text: &str) -> Vec2 {
        self.cx.layout_at(&TextRun::new(text).role(spec.role).wrap(spec.wrap), at.width()).size()
    }

    fn size(&mut self, spec: Spec, text: &str) -> Vec2 {
        self.cx.layout_at(&TextRun::new(text).role(spec.role).wrap(Wrap::Line), f32::INFINITY).size()
    }

    fn icon(&mut self, _: Rect, _: &'static str, _: Tone) {}

    fn disclosure(&mut self, _: Rect, _: bool) {}

    fn kind(&mut self, _: Rect, _: &'static str, _: Option<[u8; 3]>) {}

    fn avatar(&mut self, _: Rect, _: &str) {}

    fn connector(&mut self, _: Pos2, _: Pos2) {}

    fn button(&mut self, slot: Slot, rect: Rect, _: &'static str, _: &Label, _: bool) {
        if let Slot::More(index) = slot {
            self.more.push((index, rect));
        }
    }

    fn field(&mut self, slot: Slot, at: Rect, label: &Label, text: &str, lines: usize) -> f32 {
        let mut field = TextField::new(label.clone(), Bind::given(text.to_owned())).lines(lines);
        let height = self.cx.child(slot, &mut field, Proposal::Width(at.width())).y;
        self.fields[usize::from(slot == Slot::ReplyField)] = height;
        height
    }

    fn push_size(&mut self, slot: Slot, label: &Label) -> Vec2 {
        let size = self.cx.child(slot, &mut Button::new(label.clone()), Proposal::Ideal);
        self.buttons[push_index(slot)] = size;
        size
    }

    fn push(&mut self, _: Slot, _: Rect, _: &Label, _: bool) {}

    fn badge_size(&mut self, slot: Slot, label: &str) -> Vec2 {
        let size = self.cx.child(slot, &mut Badge::new(label.to_owned()), Proposal::Ideal);
        self.badges.push(size);
        size
    }

    fn badge(&mut self, _: Slot, _: Rect, _: &str, _: ReviewStatus) {}

    fn reply(&mut self, index: usize, rect: Rect) {
        self.replies.push((index, rect));
    }
}

/// Where the push button `slot`'s measured size is kept.
fn push_index(slot: Slot) -> usize {
    match slot {
        Slot::EditPost => 0,
        Slot::EditCancel => 1,
        Slot::Reply => 3,
        Slot::Resolve => 4,
        Slot::Delete => 5,
        _ => 2,
    }
}

/// Shows the parts of a row.
struct Paint<'a, 'b> {
    ui: &'a mut PaintUi<'b>,
    st: &'a CommentRowStyle,
    origin: Pos2,
    width: f32,
    rtl: bool,
    thread: Option<ThreadId>,
    resolved: bool,
    heights: [f32; 2],
    buttons: [Vec2; PUSH_BUTTONS],
    badges: &'a [Vec2],
    reply_submit: SubmitKey,
    /// The context focus requests go to.
    ctx: &'a egui::Context,
    /// Set when the reply box takes the focus as it shows.
    focus_reply: bool,
    /// Set when the inline editor's field takes the focus as it shows.
    focus_edit: bool,
    /// The reply field's id once it is shown, for the Reply button to focus.
    reply_id: Option<egui::Id>,
    events: &'a mut Vec<CardEvent>,
}

impl Paint<'_, '_> {
    /// `rect` from the row's left-to-right layout, on the screen.
    fn place(&self, rect: Rect) -> Rect {
        placed(self.origin, self.width, self.rtl, rect)
    }

    fn color(&self, tone: Tone) -> StyleColor {
        match tone {
            Tone::Normal => self.st.normal,
            Tone::Muted => self.st.muted,
        }
    }
}

/// `rect` from a row's left-to-right layout, on the screen: moved to the row's `origin` and mirrored across its
/// `width` under right-to-left.
fn placed(origin: Pos2, width: f32, rtl: bool, rect: Rect) -> Rect {
    let moved = rect.translate(origin.to_vec2());
    if rtl {
        let right = origin.x + width;
        let left = origin.x;
        let (min_x, max_x) = (left + right - moved.max.x, left + right - moved.min.x);
        Rect::from_min_max(Pos2::new(min_x, moved.min.y), Pos2::new(max_x, moved.max.y))
    } else {
        moved
    }
}

impl Sink for Paint<'_, '_> {
    fn size(&mut self, spec: Spec, text: &str) -> Vec2 {
        self.ui.draw().layout(&TextRun::new(text).role(spec.role).wrap(Wrap::Line), f32::INFINITY).size()
    }

    fn text(&mut self, at: Rect, spec: Spec, text: &str) -> Vec2 {
        let mut run = TextRun::new(text).role(spec.role).wrap(spec.wrap).color(self.color(spec.tone));
        if spec.end {
            run = run.align(HAlign::End);
        }
        let rect = self.place(at);
        self.ui.draw().text(rect, &run).size()
    }

    fn icon(&mut self, rect: Rect, id: &'static str, tone: Tone) {
        let spec = IconSpec::svg(IconId::new(id)).color(self.color(tone));
        self.ui.draw().icon(Rect::from_center_size(self.place(rect).center(), Vec2::splat(self.st.glyph)), &spec);
    }

    fn disclosure(&mut self, rect: Rect, open: bool) {
        let glyph = DisclosureGlyph::new(if open { 1.0 } else { 0.0 });
        self.ui.draw().icon(self.place(rect), &IconSpec::glyph(&glyph).color(self.st.muted));
    }

    fn kind(&mut self, rect: Rect, id: &'static str, color: Option<[u8; 3]>) {
        let rect = self.place(rect);
        let draw = self.ui.draw();
        draw.fill(&PaintShape::Circle(rect.center(), rect.width().min(rect.height()) / 2.0), self.st.avatar);
        let spec = IconSpec::svg(IconId::new(id));
        let spec = match color {
            Some([r, g, b]) => spec.data_color(DataColor::new(Color32::from_rgb(r, g, b))),
            None => spec.color(self.st.bar),
        };
        draw.icon(Rect::from_center_size(rect.center(), Vec2::splat(self.st.kind_icon)), &spec);
    }

    fn avatar(&mut self, rect: Rect, name: &str) {
        let rect = self.place(rect);
        paint_face(&self.ui.draw(), rect, name, self.st.avatar, self.st.avatar_ink);
    }

    fn connector(&mut self, from: Pos2, to: Pos2) {
        let rect = self.place(Rect::from_two_pos(from, to));
        self.ui.draw().stroke(&PaintShape::Path(vec![rect.center_top(), rect.center_bottom()]), StrokeWidth::Thin, self.st.thread_line);
    }

    fn button(&mut self, slot: Slot, rect: Rect, id: &'static str, name: &Label, on: bool) {
        let button = IconButton::new(IconSpec::svg(IconId::new(id))).label(name.clone()).selected(on);
        if let Slot::More(index) = slot {
            self.ui.face(MORE.at(index as u64), button);
            return;
        }
        let response = self.ui.child(slot, self.place(rect), button);
        let (Some(thread), true) = (self.thread, clicked(&response)) else { return };
        if slot == Slot::Mark {
            self.events.push(CardEvent::Mark { thread, marked: !on })
        }
    }

    fn field(&mut self, slot: Slot, at: Rect, label: &Label, text: &str, lines: usize) -> f32 {
        let height = self.heights[usize::from(slot == Slot::ReplyField)];
        let rect = self.place(Rect::from_min_size(at.min, vec2(at.width(), height)));
        let field = TextField::new(label.clone(), Bind::given(text.to_owned())).lines(lines).placeholder(label.clone()).submits();
        let field = if slot == Slot::ReplyField { field.submit_key(self.reply_submit) } else { field };
        let out = self.ui.child(slot, rect, field);
        if let Some(value) = out.value() {
            self.events.push(if slot == Slot::EditField { CardEvent::EditText(value.clone()) } else { CardEvent::ReplyText(value.clone()) });
        }
        if out.submitted {
            self.events.push(if slot == Slot::EditField { CardEvent::EditPost } else { CardEvent::ReplyPost });
        }
        if out.escaped && slot == Slot::EditField {
            self.events.push(CardEvent::EditCancel);
        }
        if slot == Slot::ReplyField {
            self.reply_id = Some(out.response.id);
            if self.focus_reply {
                focus::request(self.ctx, FocusTarget::Id(out.response.id));
            }
        } else if self.focus_edit {
            focus::request(self.ctx, FocusTarget::Id(out.response.id));
        }
        height
    }

    fn push_size(&mut self, slot: Slot, _: &Label) -> Vec2 {
        self.buttons[push_index(slot)]
    }

    fn push(&mut self, slot: Slot, rect: Rect, label: &Label, enabled: bool) {
        let button = Button::new(label.clone()).enabled(enabled);
        let button = if matches!(slot, Slot::EditPost | Slot::ReplyPost) { button.variant(Variant::PRIMARY) } else { button };
        let response = self.ui.child(slot, self.place(rect), button);
        if !clicked(&response) {
            return;
        }
        match (slot, self.thread) {
            (Slot::EditPost, _) => self.events.push(CardEvent::EditPost),
            (Slot::EditCancel, _) => self.events.push(CardEvent::EditCancel),
            (Slot::Reply, _) => {
                if let Some(id) = self.reply_id {
                    focus::request(self.ctx, FocusTarget::Id(id));
                }
            }
            (Slot::Resolve, Some(thread)) => self.events.push(CardEvent::Resolve { thread, resolved: !self.resolved }),
            (Slot::Delete, Some(thread)) => self.events.push(CardEvent::Delete { thread }),
            (Slot::ReplyPost, _) => self.events.push(CardEvent::ReplyPost),
            _ => {}
        }
    }

    fn badge_size(&mut self, slot: Slot, _: &str) -> Vec2 {
        match slot {
            Slot::Status(index) => self.badges.get(index).copied().unwrap_or(Vec2::ZERO),
            _ => Vec2::ZERO,
        }
    }

    fn badge(&mut self, slot: Slot, rect: Rect, label: &str, status: ReviewStatus) {
        let badge = Badge::new(label.to_owned()).variant(Variant::look(Emphasis::Soft, badge_tone(status)));
        let _ = self.ui.child(slot, self.place(rect), badge);
    }

    fn reply(&mut self, _: usize, _: Rect) {}
}

impl Component for CommentRow<'_> {
    const ROLE: Role = Role::TREE_ITEM;
    type Style = CommentRowStyle;
    type Output = RowOut;

    /// A balloon card is selected by a click on it: nothing around it senses the press as a collection does.
    fn sense(&self) -> Sense {
        if self.actions == CardActions::Buttons { Sense::HOVER | Sense::CLICK } else { Sense::HOVER }
    }

    fn access(&self, access: &mut craft_ui::contract::Access, cx: &Cx<'_, Self>) {
        if let Some(words) = &self.words {
            for &index in &self.mores {
                access.node(MORE.at(index as u64), Role::BUTTON).name = craft_ui::contract::Name::Text(words.more.as_str().to_owned());
            }
        }
        if let Some(heading) = &self.heading {
            let parts = if heading.count.is_empty() { vec![heading.title.clone()] } else { vec![heading.title.clone(), heading.count.clone()] };
            access.name(craft_ui::contract::Name::Joined(parts));
            return;
        }
        let Some(thread) = self.thread else { return };
        let Some(first) = thread.messages.first() else { return };
        let named = |name: String, message: &CommentMessage| {
            let age = age_text(&cx.env, message.modified.unwrap_or(message.created), self.now);
            craft_ui::contract::Name::Joined(if age.is_empty() { vec![name] } else { vec![name, age] })
        };
        access.name(named(cx.resolved.label.to_owned(), first));
        // Each reply is a node of its own inside the card, named as the card is by its comment.
        let Some(words) = &self.words else { return };
        for &(index, rect) in &self.replies {
            let Some(message) = thread.messages.get(index) else { continue };
            let name = words.card_name.fill(&[("author", &message.author), ("text", &message.text)]);
            let node = access.node(REPLY.at(index as u64), Role::GROUP);
            node.name = named(name, message);
            node.bounds(rect);
        }
    }

    fn style(cx: &StyleCx<'_>, _: CommentRowStyleKey) -> CommentRowStyle {
        let color = &cx.tokens.color;
        CommentRowStyle {
            metrics: Metrics::new(),
            normal: cx.ink(Ink::OnPanel),
            muted: cx.color(color.text.muted),
            highlight: cx.color(color.surface.raised),
            bar: cx.color(color.accent.solid),
            card: cx.color(color.surface.raised),
            border: cx.color(color.line.border),
            avatar: cx.color(color.line.border),
            avatar_ink: cx.color(color.text.normal),
            thread_line: cx.color(color.line.divider),
            kind_icon: cx.tokens.size.icon.sm,
            glyph: cx.glyph(),
            bar_width: cx.tokens.stroke.thick,
            column: cx.tokens.size.dock.column,
            height: cx.control().height,
            pad: cx.tokens.space.pad.control,
            gap: cx.tokens.space.gap.row,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let width = match cx.proposal {
            Proposal::Width(width) => width,
            _ => cx.room.unwrap_or(cx.style.column),
        };
        self.words = Some(cx.words::<CommentsWords>());
        let now = self.now;
        let age = |then: i64| age_text(&cx.env, then, now);
        let mut sink =
            Measure { cx, fields: [0.0; 2], buttons: [Vec2::ZERO; PUSH_BUTTONS], more: Vec::new(), badges: Vec::new(), replies: Vec::new() };
        let height = self.lay(cx.style, width, &age, &mut sink);
        self.heights = sink.fields;
        self.buttons = sink.buttons;
        self.badges = sink.badges;
        Vec2::new(width, height)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.words = Some(cx.words::<CommentsWords>());
        let (now, rtl) = (self.now, cx.flow.is_rtl());
        let age = |then: i64| age_text(&cx.env, then, now);
        let mut sink = Measure { cx, fields: self.heights, buttons: self.buttons, more: Vec::new(), badges: Vec::new(), replies: Vec::new() };
        self.lay(cx.style, rect.width(), &age, &mut sink);
        self.mores = sink.more.iter().map(|(index, _)| *index).collect();
        self.replies = sink.replies.iter().map(|&(index, local)| (index, placed(rect.min, rect.width(), rtl, local))).collect();
        for (index, local) in sink.more {
            cx.part(MORE.at(index as u64), placed(rect.min, rect.width(), rtl, local))
                .sense(Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS)
                .focusable();
        }
    }

    /// Notes whether the inline editor opened this pass: the row remembers which message's editor it showed, and an
    /// editor it did not show the pass before takes the keyboard.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let editing = self.editing.map(|(index, _)| index);
        let shown = cx.memory::<EditShown>();
        self.focus_edit = editing.is_some() && shown.0 != editing;
        shown.0 = editing;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.words = Some(cx.words::<CommentsWords>());
        let st = cx.style;
        let rect = cx.rect;
        let rtl = cx.env.direction().is_rtl();
        let now = self.now;
        let age = |then: i64| age_text(&cx.env, then, now);
        // A balloon card sits over the page, so it carries its own surface and border.
        if self.actions == CardActions::Buttons {
            let shape = PaintShape::Rect(rect, CornerRadius::ZERO);
            ui.draw().fill(&shape, st.card);
            ui.draw().stroke(&shape, StrokeWidth::Thin, st.border);
        }
        if self.selected {
            ui.draw().fill(&PaintShape::Rect(rect, CornerRadius::ZERO), st.highlight);
            let bar = if rtl {
                Rect::from_min_max(Pos2::new(rect.max.x - st.bar_width, rect.min.y), rect.max)
            } else {
                Rect::from_min_max(rect.min, Pos2::new(rect.min.x + st.bar_width, rect.max.y))
            };
            ui.draw().fill(&PaintShape::Rect(bar, CornerRadius::ZERO), st.bar);
        }
        let mut events = std::mem::take(&mut self.events);
        let mut sink = Paint {
            ui,
            st,
            origin: rect.min,
            width: rect.width(),
            rtl,
            thread: self.thread.map(|t| t.id),
            resolved: self.thread.is_some_and(|t| t.resolved),
            heights: self.heights,
            buttons: self.buttons,
            badges: &self.badges,
            reply_submit: self.submit_key,
            ctx: &cx.ctx,
            focus_reply: self.focus_reply,
            focus_edit: self.focus_edit,
            reply_id: None,
            events: &mut events,
        };
        self.lay(st, rect.width(), &age, &mut sink);
        self.events = events;
    }

    fn finish(self, cx: Cx<'_, Self>) -> RowOut {
        RowOut { response: cx.response.clone(), events: self.events }
    }
}
