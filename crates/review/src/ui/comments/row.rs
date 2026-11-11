//! One card or heading row: [`CommentRow`] and its measure and paint sinks.

use std::sync::Arc;

use craft_behavior::{IconId, Metrics, PartKey, Proposal, Sense};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Props as _, Role, WordSetCx as _, clicked};
use craft_ui::primitives::{HAlign, IconSpec, PaintShape, TextRun, Wrap};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, Ink, StyleColor, StyleCx, TypeRole};
use craft_ui::text::TextField;
use egui::{CornerRadius, Pos2, Rect, Vec2, vec2};

use super::list::{CardEvent, RowOut};
use super::{CommentsWords, EDIT_LINES, Line, REPLY_LINES, age_text};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentThread, ReviewStatus, ThreadId};
use crate::ui::avatar::paint_face;

/// The style of a card or heading row.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct CommentRowStyle {
    #[style(metrics)]
    metrics: Metrics,
    normal: StyleColor,
    muted: StyleColor,
    good: StyleColor,
    bad: StyleColor,
    faint: StyleColor,
    highlight: StyleColor,
    bar: StyleColor,
    avatar: StyleColor,
    avatar_ink: StyleColor,
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
}

/// How a text or icon is inked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Normal,
    Muted,
    Good,
    Bad,
    Faint,
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
}

/// Where `CommentRow::lay` puts the parts of a row: the measuring pass sizes them, the paint pass shows them.
/// Rects are relative to the row's top left, laid out left to right; the sink mirrors them.
trait Sink {
    /// Text set in `at`'s width from `at`'s top left; returns its size.
    fn text(&mut self, at: Rect, spec: Spec, text: &str) -> Vec2;
    /// The width `text` takes on one line.
    fn width(&mut self, spec: Spec, text: &str) -> f32;
    /// The icon `id` in `rect`.
    fn icon(&mut self, rect: Rect, id: &'static str, tone: Tone);
    /// The avatar of `name`.
    fn avatar(&mut self, rect: Rect, name: &str);
    /// A square icon button; it reports its press.
    fn button(&mut self, slot: Slot, rect: Rect, id: &'static str, name: &Label, on: bool);
    /// A multi-line field at `at`'s top left and width; returns its height.
    fn field(&mut self, slot: Slot, at: Rect, label: &Label, text: &str, lines: usize) -> f32;
    /// The size the push button `slot` with `label` measures.
    fn push_size(&mut self, slot: Slot, label: &Label) -> Vec2;
    /// A push button.
    fn push(&mut self, slot: Slot, rect: Rect, label: &Label, enabled: bool);
}

/// The "..." button of message `index` of a card: a part of the row that opens the message's menu.
pub(super) const MORE: PartKey = PartKey::new("more");

/// A card or a heading of the card collection.
#[derive(craft_ui::contract::Component)]
pub(crate) struct CommentRow<'r> {
    #[common]
    common: Common,
    thread: Option<&'r CommentThread>,
    heading: Option<Heading>,
    selected: bool,
    editable: bool,
    now: i64,
    editing: Option<(usize, &'r str)>,
    reply: &'r str,
    words: Option<Arc<CommentsWords>>,
    /// The heights the editor field and the reply field measured, for the paint of the same pass.
    heights: [f32; 2],
    /// The sizes the push buttons measured, for the paint of the same pass.
    buttons: [Vec2; 3],
    /// The messages whose "..." button the parts step declared.
    mores: Vec<usize>,
    events: Vec<CardEvent>,
}

impl<'r> CommentRow<'r> {
    fn bare(thread: Option<&'r CommentThread>, heading: Option<Heading>, name: String) -> Self {
        CommentRow {
            common: Common::new(),
            thread,
            heading,
            selected: false,
            editable: false,
            now: 0,
            editing: None,
            reply: "",
            words: None,
            heights: [0.0; 2],
            buttons: [Vec2::ZERO; 3],
            mores: Vec::new(),
            events: Vec::new(),
        }
        .label(name)
    }

    /// A heading row.
    pub(crate) fn heading(title: String, count: String, open: bool) -> Self {
        let name = title.clone();
        Self::bare(None, Some(Heading { title, count, open }), name)
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

fn status_icon(status: ReviewStatus) -> Option<(&'static str, Tone)> {
    match status {
        ReviewStatus::None => None,
        ReviewStatus::Accepted => Some(("thumbs_up", Tone::Good)),
        ReviewStatus::Rejected => Some(("thumbs_down", Tone::Bad)),
        ReviewStatus::Cancelled => Some(("x_circle", Tone::Faint)),
        ReviewStatus::Completed => Some(("check", Tone::Good)),
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
            let chevron = if heading.open { "chevron_down" } else { "chevron_right" };
            sink.icon(square(Pos2::new(pad, pad), h), chevron, Tone::Muted);
            let count_spec = Spec::new(TypeRole::Caption, Tone::Muted, Wrap::Wrap).end();
            let count = sink.width(count_spec, &heading.count);
            let x = pad + h + gap;
            let title = Spec::new(TypeRole::BodyStrong, Tone::Muted, Wrap::Elide);
            sink.text(strip(Pos2::new(x, pad), width - pad - count - gap - x), title, &heading.title);
            sink.text(strip(Pos2::new(width - pad - count, pad), count), count_spec, &heading.count);
            return h + 2.0 * pad;
        }
        let Some(thread) = self.thread else { return h };
        let y = pad;
        let first = thread.messages.first();
        sink.icon(square(Pos2::new(pad, y), h), kind_icon(thread.kind), Tone::Normal);
        let mut end = width - pad;
        if self.selected && self.editable {
            end -= h;
            sink.button(Slot::More(0), square(Pos2::new(end, y), h), "more", &words.more, false);
        }
        if thread.locked {
            end -= h;
            sink.icon(square(Pos2::new(end, y), h), "lock", Tone::Muted);
        }
        if self.selected && self.editable {
            end -= h;
            let name = if thread.marked { &words.unmark } else { &words.mark };
            sink.button(Slot::Mark, square(Pos2::new(end, y), h), "check", name, thread.marked);
        } else if thread.marked {
            end -= h;
            sink.icon(square(Pos2::new(end, y), h), "check", Tone::Muted);
        }
        if let Some((icon, tone)) = status_icon(thread.status) {
            end -= h;
            sink.icon(square(Pos2::new(end, y), h), icon, tone);
        }
        let name_at = pad + h + gap;
        let name_room = end - gap - name_at;
        let author = first.map_or(words.unknown_author.as_str(), |message| message.author.as_str());
        let author_spec = Spec::new(TypeRole::BodyStrong, Tone::Normal, Wrap::Elide);
        let author_size = sink.text(strip(Pos2::new(name_at, y), name_room), author_spec, author);
        let mut head = author_size.y.max(h);
        if let Some(message) = first {
            let when = age(message.modified.unwrap_or(message.created));
            let caption = Spec::new(TypeRole::Caption, Tone::Muted, Wrap::Elide);
            let size = sink.text(strip(Pos2::new(name_at, y + author_size.y), name_room), caption, &when);
            head = head.max(author_size.y + size.y);
        }
        let mut y = y + head + gap;
        y = self.message(0, thread, st, Line { at: Pos2::new(pad, y), room }, &words, sink) + gap;
        for index in 1..thread.messages.len() {
            let Some(message) = thread.messages.get(index) else { break };
            sink.avatar(square(Pos2::new(pad, y), h), &message.author);
            let x = pad + h + gap;
            let name_room = width - pad - h - gap - x;
            let author_size = sink.text(strip(Pos2::new(x, y), name_room), author_spec, &message.author);
            let when = age(message.modified.unwrap_or(message.created));
            let caption = Spec::new(TypeRole::Caption, Tone::Muted, Wrap::Elide);
            let size = sink.text(strip(Pos2::new(x, y + author_size.y), name_room), caption, &when);
            sink.button(Slot::More(index), square(Pos2::new(width - pad - h, y), h), "more", &words.more, false);
            y += h.max(author_size.y + size.y) + gap;
            y = self.message(index, thread, st, Line { at: Pos2::new(x, y), room: width - pad - x }, &words, sink) + gap;
        }
        if self.selected && self.editable {
            let field = sink.field(Slot::ReplyField, strip(Pos2::new(pad, y), room), &words.reply, self.reply, REPLY_LINES);
            y += field + gap;
            let button = sink.push_size(Slot::ReplyPost, &words.post);
            let post = Rect::from_min_size(Pos2::new(width - pad - button.x, y), button);
            sink.push(Slot::ReplyPost, post, &words.post, !self.reply.trim().is_empty());
            y += button.y + gap;
        }
        y - gap + pad
    }

    /// Message `index`'s text, or its inline editor; returns the bottom.
    fn message(&self, index: usize, thread: &CommentThread, st: &CommentRowStyle, line: Line, words: &CommentsWords, sink: &mut dyn Sink) -> f32 {
        let Line { at, room } = line;
        let Some(message) = thread.messages.get(index) else { return at.y };
        match self.editing {
            Some((editing, draft)) if editing == index => {
                let field = sink.field(Slot::EditField, strip(at, room), &words.edit, draft, EDIT_LINES);
                let y = at.y + field + st.gap;
                let (post_size, cancel_size) = (sink.push_size(Slot::EditPost, &words.post), sink.push_size(Slot::EditCancel, &words.cancel));
                let right = at.x + room;
                let cancel = Rect::from_min_size(Pos2::new(right - cancel_size.x, y), cancel_size);
                let post = Rect::from_min_size(Pos2::new(cancel.left() - st.gap - post_size.x, y), post_size);
                sink.push(Slot::EditPost, post, &words.post, !draft.trim().is_empty());
                sink.push(Slot::EditCancel, cancel, &words.cancel, true);
                y + post_size.y.max(cancel_size.y)
            }
            _ => {
                let body = Spec::new(TypeRole::Body, Tone::Normal, Wrap::Wrap);
                at.y + sink.text(strip(at, room), body, &message.text).y
            }
        }
    }
}

/// Sizes the parts of a row.
struct Measure<'a, 'b, 'r> {
    cx: &'a MeasureCx<'b, CommentRow<'r>>,
    fields: [f32; 2],
    buttons: [Vec2; 3],
    /// The message index and left-to-right rect of each "..." button.
    more: Vec<(usize, Rect)>,
}

impl Sink for Measure<'_, '_, '_> {
    fn text(&mut self, at: Rect, spec: Spec, text: &str) -> Vec2 {
        self.cx.layout_at(&TextRun::new(text).role(spec.role).wrap(spec.wrap), at.width()).size()
    }

    fn width(&mut self, spec: Spec, text: &str) -> f32 {
        self.cx.layout_at(&TextRun::new(text).role(spec.role).wrap(Wrap::Line), f32::INFINITY).size().x
    }

    fn icon(&mut self, _: Rect, _: &'static str, _: Tone) {}

    fn avatar(&mut self, _: Rect, _: &str) {}

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
}

/// Where the push button `slot`'s measured size is kept.
fn push_index(slot: Slot) -> usize {
    match slot {
        Slot::EditPost => 0,
        Slot::EditCancel => 1,
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
    heights: [f32; 2],
    buttons: [Vec2; 3],
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
            Tone::Good => self.st.good,
            Tone::Bad => self.st.bad,
            Tone::Faint => self.st.faint,
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
    fn width(&mut self, spec: Spec, text: &str) -> f32 {
        self.ui.draw().layout(&TextRun::new(text).role(spec.role).wrap(Wrap::Line), f32::INFINITY).size().x
    }

    fn text(&mut self, at: Rect, spec: Spec, text: &str) -> Vec2 {
        let mut run = TextRun::new(text).role(spec.role).wrap(spec.wrap).color(self.color(spec.tone));
        if spec.end {
            run = run.align(HAlign::End);
        }
        self.ui.draw().text(self.place(at), &run).size()
    }

    fn icon(&mut self, rect: Rect, id: &'static str, tone: Tone) {
        let spec = IconSpec::svg(IconId::new(id)).color(self.color(tone));
        self.ui.draw().icon(self.place(rect), &spec);
    }

    fn avatar(&mut self, rect: Rect, name: &str) {
        let rect = self.place(rect);
        paint_face(&self.ui.draw(), rect, name, self.st.avatar, self.st.avatar_ink);
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
        let field = TextField::new(label.clone(), Bind::given(text.to_owned())).lines(lines).placeholder(label.clone());
        let out = self.ui.child(slot, rect, field);
        if let Some(value) = out.value() {
            self.events.push(if slot == Slot::EditField { CardEvent::EditText(value.clone()) } else { CardEvent::ReplyText(value.clone()) });
        }
        if out.escaped && slot == Slot::EditField {
            self.events.push(CardEvent::EditCancel);
        }
        height
    }

    fn push_size(&mut self, slot: Slot, _: &Label) -> Vec2 {
        self.buttons[push_index(slot)]
    }

    fn push(&mut self, slot: Slot, rect: Rect, label: &Label, enabled: bool) {
        let response = self.ui.child(slot, self.place(rect), Button::new(label.clone()).enabled(enabled));
        if clicked(&response) {
            self.events.push(match slot {
                Slot::EditPost => CardEvent::EditPost,
                Slot::EditCancel => CardEvent::EditCancel,
                _ => CardEvent::ReplyPost,
            });
        }
    }
}

impl Component for CommentRow<'_> {
    const ROLE: Role = Role::TREE_ITEM;
    type Style = CommentRowStyle;
    type Output = RowOut;

    fn access(&self, access: &mut craft_ui::contract::Access, cx: &Cx<'_, Self>) {
        if let Some(words) = &self.words {
            for &index in &self.mores {
                access.node(MORE.at(index as u64), Role::BUTTON).name = craft_ui::contract::Name::Text(words.more.as_str().to_owned());
            }
        }
        let Some(first) = self.thread.and_then(|thread| thread.messages.first()) else { return };
        let age = age_text(&cx.env, first.modified.unwrap_or(first.created), self.now);
        access.name(craft_ui::contract::Name::Joined(vec![cx.resolved.label.to_owned(), age]));
    }

    fn style(cx: &StyleCx<'_>, _: CommentRowStyleKey) -> CommentRowStyle {
        let color = &cx.tokens.color;
        CommentRowStyle {
            metrics: Metrics::new(),
            normal: cx.ink(Ink::OnPanel),
            muted: cx.color(color.text.muted),
            good: cx.color(color.status.good.fg),
            bad: cx.color(color.status.bad.fg),
            faint: cx.color(color.text.faint),
            highlight: cx.color(color.state.selected),
            bar: cx.color(color.accent.solid),
            avatar: cx.color(color.accent.strong),
            avatar_ink: cx.color(color.text.on_strong),
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
        let mut sink = Measure { cx, fields: [0.0; 2], buttons: [Vec2::ZERO; 3], more: Vec::new() };
        let height = self.lay(cx.style, width, &age, &mut sink);
        self.heights = sink.fields;
        self.buttons = sink.buttons;
        Vec2::new(width, height)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.words = Some(cx.words::<CommentsWords>());
        let (now, rtl) = (self.now, cx.flow.is_rtl());
        let age = |then: i64| age_text(&cx.env, then, now);
        let mut sink = Measure { cx, fields: self.heights, buttons: self.buttons, more: Vec::new() };
        self.lay(cx.style, rect.width(), &age, &mut sink);
        self.mores = sink.more.iter().map(|(index, _)| *index).collect();
        for (index, local) in sink.more {
            cx.part(MORE.at(index as u64), placed(rect.min, rect.width(), rtl, local))
                .sense(Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS)
                .focusable();
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.words = Some(cx.words::<CommentsWords>());
        let st = cx.style;
        let rect = cx.rect;
        let rtl = cx.env.direction().is_rtl();
        let now = self.now;
        let age = |then: i64| age_text(&cx.env, then, now);
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
            heights: self.heights,
            buttons: self.buttons,
            events: &mut events,
        };
        self.lay(st, rect.width(), &age, &mut sink);
        self.events = events;
    }

    fn finish(self, cx: Cx<'_, Self>) -> RowOut {
        RowOut { response: cx.response.clone(), events: self.events }
    }
}
