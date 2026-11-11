//! `EventLog`, the warnings and errors of commands and background work as an item source, and `EventList`: the log's
//! rows, newest first, each a `ListRow` with a level icon, the message and its age.

use super::view_words::EventWords;
use super::{ListRow, RowEvent};
use crate::contract::{
    Bind, BindMut, BodyCx, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, Props as _, Role, SelectMode,
    Show as _, Sizing, WordSetCx as _,
};
use crate::notices::{Notice, NoticeLayout};
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{DisplayOrder, IconId, ItemSource, Metrics, Selection, SpanLayout};
use craft_i18n::Format;
use craft_theme::variant::Tone;
use egui::Ui;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// How bad an event is; ordered, so a filter keeps the levels at or above its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EventLevel {
    /// For information.
    #[default]
    Info,
    /// Worth a look.
    Warning,
    /// Something failed.
    Error,
}

impl EventLevel {
    fn icon(self) -> IconId {
        match self {
            EventLevel::Info => IconId::new("info"),
            EventLevel::Warning => IconId::new("warning"),
            EventLevel::Error => IconId::new("error"),
        }
    }

    fn tone(self) -> Tone {
        match self {
            EventLevel::Info => Tone::Info,
            EventLevel::Warning => Tone::Warn,
            EventLevel::Error => Tone::Bad,
        }
    }

    fn word(self, words: &EventWords) -> &str {
        match self {
            EventLevel::Info => words.info.as_str(),
            EventLevel::Warning => words.warning.as_str(),
            EventLevel::Error => words.error.as_str(),
        }
    }
}

/// One event, in the app's words.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LogEvent {
    /// The app's id; it keeps the selection while new events arrive.
    pub id: u64,
    /// How serious the event is.
    pub level: EventLevel,
    /// What reported it: "Auto-save", "Render", a command's name.
    pub source: String,
    /// What happened, already translated.
    pub message: String,
    /// How many times in a row it happened (1 for once).
    pub count: u32,
    /// When, in milliseconds since the Unix epoch, from the app's clock.
    pub time_ms: u64,
}

impl LogEvent {
    /// An event `id` of `level` from `source` that says `message`, once, at `time_ms`.
    pub fn new(id: u64, level: EventLevel, source: impl Into<String>, message: impl Into<String>, time_ms: u64) -> Self {
        LogEvent { id, level, source: source.into(), message: message.into(), count: 1, time_ms }
    }

    /// The event happened `count` times in a row.
    #[must_use]
    pub const fn count(mut self, count: u32) -> Self {
        self.count = count;
        self
    }

    /// The message with its repeat count: "Disk almost full  (\u{d7}3)".
    pub fn message_with_count(&self, words: &EventWords, format: &Format) -> String {
        match self.count {
            0 | 1 => self.message.clone(),
            n => words.repeated.fill(&[("message", self.message.as_str()), ("count", format.integer(n).as_str())]),
        }
    }

    /// The age of the event at `now_ms`, in whole seconds; an event after `now_ms` is 0 seconds old.
    pub fn age_seconds(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.time_ms) / 1000
    }

    /// The event as a list row: its level icon, its message with the repeat count and, at the end, `age` in the
    /// caption role. A screen reader reads the message, the level and the age. `striped` is `ItemCx::row.striped`.
    pub fn row(&self, age: &str, words: &EventWords, format: &Format, striped: bool) -> ListRow<'static> {
        let message = self.message_with_count(words, format);
        let name = words.row_name.fill(&[("level", self.level.word(words)), ("message", message.as_str()), ("age", age)]);
        let age = age.to_owned();
        ListRow::new(message).accessible_name(name).icon(IconSpec::svg(self.level.icon())).tone(self.level.tone()).striped(striped).trailing(age)
    }
}

/// What the log keeps between passes; the app stores it with its panel state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EventLogState {
    /// The least level shown.
    pub min_level: EventLevel,
}

/// How many events of each level a log holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LevelCounts {
    /// Events of `Info`.
    pub info: usize,
    /// Events of `Warning`.
    pub warning: usize,
    /// Events of `Error`.
    pub error: usize,
}

/// The events at or above a level, newest first, as an item source keyed by event id.
#[derive(Clone, Debug)]
pub struct EventLog<'a> {
    events: &'a [LogEvent],
    shown: Vec<usize>,
}

impl<'a> EventLog<'a> {
    /// The events of `events`, given in the order they happened (oldest first), that are at or above the level of
    /// `state`.
    pub fn new(events: &'a [LogEvent], state: &EventLogState) -> Self {
        let shown = (0..events.len()).rev().filter(|&at| events[at].level >= state.min_level).collect();
        EventLog { events, shown }
    }

    /// The event shown at position `at`.
    pub fn event(&self, at: usize) -> Option<&'a LogEvent> {
        self.shown.get(at).map(|&index| &self.events[index])
    }

    /// How many events of each level the whole log holds, shown or not.
    pub fn counts(&self) -> LevelCounts {
        self.events.iter().fold(LevelCounts::default(), |mut counts, event| {
            match event.level {
                EventLevel::Info => counts.info += 1,
                EventLevel::Warning => counts.warning += 1,
                EventLevel::Error => counts.error += 1,
            }
            counts
        })
    }
}

impl DisplayOrder<u64> for EventLog<'_> {
    fn len(&self) -> usize {
        self.shown.len()
    }

    fn key(&self, at: usize) -> u64 {
        self.events[self.shown[at]].id
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.event(at).map(|event| Cow::Borrowed(event.message.as_str()))
    }
}

impl ItemSource<u64> for EventLog<'_> {}

/// The style of the event list: it has no look of its own beyond its container's surface.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct EventListStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The list of events: one selected event; an empty log says so.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection)]
pub struct EventList<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    /// The selection the caller owns.
    #[prop]
    selection: Option<Bind<'a, Selection<u64>>>,
}

impl<'a> EventList<'a> {
    /// An event list named `label`.
    pub fn new(label: impl Into<Label>) -> Self {
        EventList {
            common: Common::new(),
            props: ContainerProps::new().surface(SurfaceKind::Sunken).sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Single).striped(true).allow_empty(true),
            selection: None,
        }
        .label(label)
    }
}

impl Component for EventList<'_> {
    const ROLE: Role = Role::LIST;
    const PARTS: bool = false;
    type Style = EventListStyle;
    type Output = CollectionResponse<u64, RowEvent>;

    fn style(_: &StyleCx<'_>, _: EventListStyleKey) -> EventListStyle {
        EventListStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for EventList<'_> {
    type Key = u64;
    type ItemLayout = SpanLayout;
    type Selection = Selection<u64>;
    type Item<'r> = ListRow<'r>;
    type ItemEvent = RowEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<u64>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<u64>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::sized(source, width, cx.control().height)
    }

    fn empty_height(&self, _width: f32, _ui: &Ui, cx: &StyleCx<'_>) -> f32 {
        cx.control().height
    }

    fn empty(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let words = cx.words::<EventWords>();
        Notice::new(Tone::Neutral, words.none.clone()).layout(NoticeLayout::Centred).show_at(ui, cx.rect);
    }
}
