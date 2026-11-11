//! The card collection: row keys, the rows' source and [`CommentList`].

use std::borrow::Cow;

use craft_behavior::{DisplayOrder, ItemSource, Metrics, Selection, SpanLayout};
use craft_ui::contract::{
    ActivateOn, Bind, BindMut, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, ItemOutput, NoValue, Outcome,
    Props as _, Role, SelectMode, Sizing,
};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::Response;

use super::row::CommentRow;
use crate::comments::{CommentThread, ThreadId};

/// The key of a row of the card collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CardKey {
    /// The heading at this position among the headings.
    Group(usize),
    /// The card of this thread.
    Thread(ThreadId),
}

/// One row of the card collection's source.
pub(super) enum RowEntry<'a> {
    Group { title: String, count: String, page: Option<usize>, open: bool },
    Card { thread: &'a CommentThread, depth: usize },
}

/// The shown threads, under their headings, as the collection's source.
pub(super) struct Rows<'a> {
    pub(super) entries: Vec<(CardKey, RowEntry<'a>)>,
}

impl DisplayOrder<CardKey> for Rows<'_> {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn key(&self, at: usize) -> CardKey {
        self.entries[at].0
    }

    fn index_of(&self, key: &CardKey) -> Option<usize> {
        self.entries.iter().position(|(held, _)| held == key)
    }

    fn depth(&self, at: usize) -> usize {
        match &self.entries[at].1 {
            RowEntry::Group { .. } => 0,
            RowEntry::Card { depth, .. } => *depth,
        }
    }

    fn has_children(&self, at: usize) -> bool {
        matches!(self.entries[at].1, RowEntry::Group { .. })
    }

    fn open(&self, at: usize) -> bool {
        matches!(self.entries[at].1, RowEntry::Group { open: true, .. })
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.entries[at].1 {
            RowEntry::Group { title, .. } => Some(Cow::Borrowed(title.as_str())),
            RowEntry::Card { thread, .. } => thread.messages.first().map(|message| Cow::Borrowed(message.text.as_str())),
        }
    }
}

impl ItemSource<CardKey> for Rows<'_> {}

/// What a card reports; the panel applies it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CardEvent {
    /// A "..." button was pressed: the menu of message `index` hangs from `anchor`.
    /// The checkmark button was pressed.
    Mark { thread: ThreadId, marked: bool },
    /// The inline editor's text changed.
    EditText(String),
    /// The inline editor's Post.
    EditPost,
    /// The inline editor was cancelled.
    EditCancel,
    /// The reply box's text changed.
    ReplyText(String),
    /// The reply box's Post.
    ReplyPost,
}

/// What a card row returns.
#[non_exhaustive]
pub(crate) struct RowOut {
    pub(super) response: Response,
    pub(super) events: Vec<CardEvent>,
}

impl Outcome for RowOut {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<Vec<CardEvent>> for RowOut {
    fn into_event(self) -> Option<Vec<CardEvent>> {
        (!self.events.is_empty()).then_some(self.events)
    }
}

/// The card collection: a tree whose headings fold their cards.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
pub(crate) struct CommentList<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    selection: Option<Bind<'a, Selection<CardKey>>>,
    grouped: bool,
}

impl<'a> CommentList<'a> {
    /// The list named `label` over the caller's `selection` (the list keeps its own when `None`); `grouped` lists the cards under headings.
    pub(crate) fn new(label: impl Into<Label>, selection: Option<Bind<'a, Selection<CardKey>>>, grouped: bool) -> Self {
        CommentList {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Single).activate(ActivateOn::Click),
            selection,
            grouped,
        }
        .label(label)
    }
}

/// The style of the list: no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct CommentListStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for CommentList<'_> {
    const ROLE: Role = Role::TREE;
    type Style = CommentListStyle;
    type Output = CollectionResponse<CardKey, Vec<CardEvent>>;

    fn style(_: &StyleCx<'_>, _: CommentListStyleKey) -> CommentListStyle {
        CommentListStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for CommentList<'_> {
    type Key = CardKey;
    type ItemLayout = SpanLayout;
    type Selection = Selection<CardKey>;
    type Item<'r> = CommentRow<'r>;
    type ItemEvent = Vec<CardEvent>;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<CardKey>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<CardKey>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        let layout = SpanLayout::sized(source, width, cx.control().height);
        if self.grouped { layout.tree() } else { layout }
    }
}
