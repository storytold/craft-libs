//! The render (export) queue list.

#[cfg(feature = "screen-testing")]
mod stories;

use std::borrow::Cow;
use std::cell::RefCell;
use std::time::Duration;

use craft_behavior::{DisplayOrder, EditEvent, IconId, ItemSource, Selection, SelectionChange};
use craft_theme::Variant;
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, Held, NoValue, Outcome, Pattern, Props, ReorderMove, ReorderRule, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, RowEvent, TrailingFace, TrailingPart};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

/// The pressable part that cancels or retries an item.
const ACTION: TrailingPart = TrailingPart::new(0);
/// The pressable part that removes an item.
const REMOVE: TrailingPart = TrailingPart::new(1);

craft_ui::words! {
    /// The queue's words, English by default (FilmCraft's).
    #[non_exhaustive]
    pub struct RenderQueueWords {
        title: "renderQueue.title" = "Destinations";
        empty: "renderQueue.empty" = "Send exports here with Send to Queue, then start the queue.";
        ready: "renderQueue.ready" = "Ready";
        encoding: template "renderQueue.encoding" = "Encoding {n}%";
        done: "renderQueue.done" = "Done";
        failed: template "renderQueue.failed" = "Failed: {e}";
        cancelled: "renderQueue.cancelled" = "Cancelled";
        time_left: template "renderQueue.timeLeft" = "{time} left";
        seconds: count "renderQueue.seconds" = "{n} s" | "{n} s";
        cancel: "renderQueue.cancel" = "Cancel";
        retry: "renderQueue.retry" = "Retry";
        remove: "renderQueue.remove" = "Remove";
        start: "renderQueue.start" = "Start Queue";
        stop: "renderQueue.stop" = "Stop";
        clear_finished: "renderQueue.clearFinished" = "Clear Finished";
    }
}

/// An item's state.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RenderStatus {
    /// Waiting in the queue.
    Ready,
    /// Progress 0 to 1 and the time left when the encoder knows it.
    #[non_exhaustive]
    Encoding {
        /// The share done, from 0 to 1.
        progress: f32,
        /// The time left; `None` while the encoder cannot tell.
        remaining: Option<Duration>,
    },
    /// Finished.
    Done,
    /// The encoder's message, in the app's words.
    Failed(String),
    /// Cancelled by the user.
    Cancelled,
}

impl RenderStatus {
    /// An encoding item `progress` of the way done with `remaining` time left.
    pub fn encoding(progress: f32, remaining: Option<Duration>) -> Self {
        RenderStatus::Encoding { progress, remaining }
    }

    fn is_encoding(&self) -> bool {
        matches!(self, RenderStatus::Encoding { .. })
    }
}

/// A queue item's id, the app's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderItemId(u64);

impl RenderItemId {
    /// The id `value`.
    pub const fn new(value: u64) -> Self {
        RenderItemId(value)
    }
}

/// A queue row: "Sequence \u{2192} file.mp4", the preset, the status.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RenderQueueItem {
    /// The item's identity.
    pub id: RenderItemId,
    /// The source name, such as the sequence.
    pub source: String,
    /// The output file name.
    pub file: String,
    /// The name of the export preset.
    pub preset: String,
    /// The item's state.
    pub status: RenderStatus,
}

impl RenderQueueItem {
    /// An item `id` that renders `source` to `file` with `preset`, in `status`.
    pub fn new(id: RenderItemId, source: impl Into<String>, file: impl Into<String>, preset: impl Into<String>, status: RenderStatus) -> Self {
        RenderQueueItem { id, source: source.into(), file: file.into(), preset: preset.into(), status }
    }
}

/// What the queue asks of the app, in the order the user did it this pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RenderQueueEvent {
    /// A drag of rows, or the list's reorder keys on the selected rows, as one gesture.
    Reorder(EditEvent<ReorderMove<RenderItemId>>),
    /// The selection changed.
    Selection(SelectionChange<RenderItemId>),
    /// Cancel the item.
    Cancel(RenderItemId),
    /// Run a failed or cancelled item again.
    Retry(RenderItemId),
    /// The row's Remove button, or Delete or Backspace on the selection (one event per item, in order).
    Remove(RenderItemId),
    /// Start the queue.
    Start,
    /// Stop the queue.
    Stop,
    /// Remove every finished item.
    ClearFinished,
}

/// What the queue did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct RenderQueueListOutput {
    /// The list's response, for hover and focus.
    pub response: Response,
    /// What the user asked for this pass, in order.
    pub events: Vec<RenderQueueEvent>,
}

impl Outcome for RenderQueueListOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The items as a collection source, keyed by id.
struct Items<'a> {
    items: &'a [RenderQueueItem],
}

impl DisplayOrder<RenderItemId> for Items<'_> {
    fn len(&self) -> usize {
        self.items.len()
    }

    fn key(&self, at: usize) -> RenderItemId {
        self.items[at].id
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(format!("{} \u{2192} {}", self.items[at].source, self.items[at].file)))
    }
}

impl ItemSource<RenderItemId> for Items<'_> {}

/// A time left, rounded up to the second, in the user's digits: `45 s`, `2:05`, `1:02:05`.
pub fn format_eta(env: &Env, time: Duration) -> String {
    let words = env.words::<RenderQueueWords>();
    let format = env.format();
    let seconds = time.as_secs().saturating_add(u64::from(time.subsec_nanos() > 0));
    let digits = |n: u64| format.integer(n);
    let two = |n: u64| if n < 10 { format!("{}{}", digits(0), digits(n)) } else { digits(n) };
    match seconds {
        0..60 => words.seconds.text(seconds, &digits(seconds)),
        60..3600 => format!("{}:{}", digits(seconds / 60), two(seconds % 60)),
        _ => format!("{}:{}:{}", digits(seconds / 3600), two(seconds % 3600 / 60), two(seconds % 60)),
    }
}

/// The row of `item`: "source \u{2192} file", the preset and status as its detail, the time left of an encoding
/// item, and the row's action buttons.
fn queue_row(env: &Env, words: &RenderQueueWords, item: &RenderQueueItem) -> ListRow<'static> {
    let format = env.format();
    let status = match &item.status {
        RenderStatus::Ready => words.ready.as_str().to_owned(),
        RenderStatus::Encoding { progress, .. } => {
            let percent = (f64::from(progress.clamp(0.0, 1.0)) * 100.0).round() as u64;
            words.encoding.fill(&[("n", &format.integer(percent))])
        }
        RenderStatus::Done => words.done.as_str().to_owned(),
        RenderStatus::Failed(error) => words.failed.fill(&[("e", error)]),
        RenderStatus::Cancelled => words.cancelled.as_str().to_owned(),
    };
    let mut row = ListRow::new(format!("{} \u{2192} {}", item.source, item.file)).detail(format!("{} \u{b7} {status}", item.preset));
    if let RenderStatus::Encoding { remaining: Some(time), .. } = item.status {
        row = row.optional(words.time_left.fill(&[("time", &format_eta(env, time))]));
    }
    if !item.status.is_encoding() {
        row = row.trailing_part(REMOVE, TrailingFace::Icon(IconSpec::svg(IconId::new("trash"))), words.remove.clone());
    }
    match item.status {
        RenderStatus::Ready | RenderStatus::Encoding { .. } => {
            row.trailing_part(ACTION, TrailingFace::Icon(IconSpec::svg(IconId::new("close"))), words.cancel.clone())
        }
        _ => row.trailing_part(ACTION, TrailingFace::Icon(IconSpec::svg(IconId::new("refresh"))), words.retry.clone()),
    }
}

/// FilmCraft's export queue (Premiere's queue in Export mode): a `List` of the items, each row "source \u{2192}
/// file" with the preset and status as its detail line and, while encoding, the time left. Row buttons: Cancel (Ready
/// or Encoding) or Retry (Failed or Cancelled), and Remove (not while encoding). Under the list: Start Queue (when
/// something is Ready and the queue is stopped), Stop (while running) and Clear Finished (when an item is Done,
/// Failed or Cancelled). The empty queue shows the empty text.
///
/// Keys are the list's: one Tab stop, arrows move, Shift and Cmd/Ctrl select, the reorder keys move the selected
/// rows, Delete and Backspace remove them (not the encoding rows).
#[derive(Pattern, Props)]
pub struct RenderQueueList<'a> {
    label: Label,
    items: &'a [RenderQueueItem],
    selection: Bind<'a, Selection<RenderItemId>>,
    /// Whether the queue is encoding: it enables Stop and disables Start.
    #[prop]
    running: bool,
}

impl<'a> RenderQueueList<'a> {
    /// A list named `label` of `items` with the app's `selection`.
    pub fn new(label: impl Into<Label>, items: &'a [RenderQueueItem], selection: Bind<'a, Selection<RenderItemId>>) -> Self {
        RenderQueueList { label: label.into(), items, selection, running: false }
    }

    /// Shows the list and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> RenderQueueListOutput {
        let RenderQueueList { label, items, selection, running } = self;
        let env = Env::of(ui);
        let words = env.words::<RenderQueueWords>();
        let name = label.as_str().to_owned();
        let events: RefCell<Vec<RenderQueueEvent>> = RefCell::new(Vec::new());
        let ready = items.iter().any(|item| item.status == RenderStatus::Ready);
        let finished = items.iter().any(|item| matches!(item.status, RenderStatus::Done | RenderStatus::Failed(_) | RenderStatus::Cancelled));
        let mut held = Held::new(selection);

        let body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let before = held.read(cx).unwrap_or_default();
            let mut selected = before.clone();
            if items.is_empty() {
                Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
            } else {
                let source = Items { items };
                let out = List::<RenderItemId>::new(words.title.clone())
                    .select(SelectMode::Multi)
                    .deletable(true)
                    .reorder(ReorderRule::new())
                    .selection(Bind::bound(&mut selected))
                    .show_items(ui, &source, &mut |key, _| {
                        let item = items.iter().find(|item| item.id == *key).expect("the list asks for its own keys");
                        queue_row(&env, &words, item)
                    });
                let mut out_events = events.borrow_mut();
                for event in out.events {
                    match event {
                        CollectionEvent::Selection(change) => out_events.push(RenderQueueEvent::Selection(change)),
                        CollectionEvent::Reorder(edit) => out_events.push(RenderQueueEvent::Reorder(edit)),
                        CollectionEvent::Delete(keys) => {
                            let removable = |key: &RenderItemId| items.iter().any(|item| item.id == *key && !item.status.is_encoding());
                            out_events.extend(keys.into_iter().filter(removable).map(RenderQueueEvent::Remove));
                        }
                        CollectionEvent::Item(key, RowEvent::Trailing { part, .. }) => {
                            let item = items.iter().find(|item| item.id == key);
                            match (part, item.map(|item| &item.status)) {
                                (REMOVE, _) => out_events.push(RenderQueueEvent::Remove(key)),
                                (ACTION, Some(RenderStatus::Ready | RenderStatus::Encoding { .. })) => out_events.push(RenderQueueEvent::Cancel(key)),
                                (ACTION, Some(_)) => out_events.push(RenderQueueEvent::Retry(key)),
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            if selected != before {
                held.write(cx, &before, selected, &name);
            }
        };

        let footer = |ui: &mut Ui, _: &mut BodyCx<'_>| {
            Row::new().show(ui, |ui, _| {
                let mut out_events = events.borrow_mut();
                let start = Button::new(words.start.clone()).variant(Variant::PRIMARY).enabled(!running && ready).show(ui);
                if clicked(&start) {
                    out_events.push(RenderQueueEvent::Start);
                }
                if clicked(&Button::new(words.stop.clone()).enabled(running).show(ui)) {
                    out_events.push(RenderQueueEvent::Stop);
                }
                if clicked(&Button::new(words.clear_finished.clone()).enabled(finished).show(ui)) {
                    out_events.push(RenderQueueEvent::ClearFinished);
                }
            });
        };

        let shown = Stack::new().fill().label(label.clone()).footer(footer).show(ui, body);
        RenderQueueListOutput { response: shown.output, events: events.into_inner() }
    }
}
