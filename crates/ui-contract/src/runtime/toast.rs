//! The toast queue: one per viewport, filled from any code with a `Context`.
//!
//! The shell's `ToastHost` shows the queue and tells it which cards the pointer or focus holds. A card's timer runs
//! only while nothing holds it. Each new toast is announced once, when it is first shown. At most `MAX_CARDS` cards
//! are queued; a toast's end (an action taken, a dismissal, the timer) is read once with `end`.

use crate::runtime::access::{self, Politeness};
use crate::runtime::egui::{pass, repaint};
use crate::runtime::env::Label;
use crate::runtime::frame::{self, PerViewport};
use craft_theme::variant::Tone;
use egui::{Context, ViewportId};

/// How long a toast stays when it names no timeout, in seconds: the source apps' notification duration.
pub const TIMEOUT: f32 = 6.0;

/// Most cards queued at once; posting past it drops the oldest card that has no action.
pub const MAX_CARDS: usize = 3;

/// Ended toasts whose end nobody has read yet; the oldest report goes first.
const ENDS_KEPT: usize = 32;

/// An action of a toast card: a button-like link that ends the toast and is reported by `end`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToastAction {
    /// The action's text.
    pub label: Label,
}

impl ToastAction {
    /// An action named `label`.
    pub fn new(label: impl Into<Label>) -> Self {
        ToastAction { label: label.into() }
    }
}

/// How a toast left the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastEnd {
    /// The action at this index was taken.
    Action(usize),
    /// The user closed the card.
    Dismissed,
    /// The timer ran out.
    Expired,
}

/// A message the user sees for a while.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Toast {
    /// The text of the card.
    pub message: Label,
    /// Seconds the card stays; `None` keeps it until it is dismissed.
    pub timeout: Option<f32>,
    /// How the announcement is made.
    pub politeness: Politeness,
    /// The tone the card reads in.
    pub tone: Tone,
    /// A title above the message.
    pub title: Option<Label>,
    /// The actions after the message, in reading order.
    pub actions: Vec<ToastAction>,
}

impl Toast {
    /// A neutral toast of `message` that stays [`TIMEOUT`] seconds and is announced politely.
    pub fn new(message: impl Into<Label>) -> Self {
        Toast {
            message: message.into(),
            timeout: Some(TIMEOUT),
            politeness: Politeness::Polite,
            tone: Tone::Neutral,
            title: None,
            actions: Vec::new(),
        }
    }

    /// Sets the tone the card reads in.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Sets the title above the message.
    pub fn title(mut self, title: impl Into<Label>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Adds an action after the message.
    pub fn action(mut self, action: ToastAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Sets how long the card stays.
    pub fn timeout(mut self, timeout: Option<f32>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the announcement's politeness.
    pub fn politeness(mut self, politeness: Politeness) -> Self {
        self.politeness = politeness;
        self
    }
}

/// A posted toast's handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastId(u64);

/// A toast in the queue, as the host reads it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Card {
    /// The handle.
    pub id: ToastId,
    /// The data.
    pub toast: Toast,
}

struct Live {
    card: Card,
    shown: bool,
    remaining: Option<f32>,
    held: bool,
}

#[derive(Default)]
struct Queue {
    next: u64,
    live: Vec<Live>,
    /// The pass time of the last end hook, to measure the timers' elapsed time.
    last_tick: Option<f64>,
    /// How toasts ended, until `end` reads them.
    ended: Vec<(ToastId, ToastEnd)>,
}

fn record(ended: &mut Vec<(ToastId, ToastEnd)>, id: ToastId, end: ToastEnd) {
    ended.push((id, end));
    if ended.len() > ENDS_KEPT {
        ended.remove(0);
    }
}

/// Posts `toast` to the running viewport.
pub fn post(ctx: &Context, toast: Toast) -> ToastId {
    post_to(ctx, pass::current_viewport(ctx), toast)
}

/// Posts `toast` to `viewport`.
pub fn post_to(ctx: &Context, viewport: ViewportId, toast: Toast) -> ToastId {
    if !crate::runtime::measure::effect(ctx, "toast::post") {
        // No toast is numbered 0, so a later dismiss of this id removes nothing.
        return ToastId(0);
    }
    let id = PerViewport::<Queue>::with(ctx, viewport, |queue| {
        queue.next += 1;
        let id = ToastId(queue.next);
        let remaining = toast.timeout;
        queue.live.push(Live { card: Card { id, toast }, shown: false, remaining, held: false });
        while queue.live.len() > MAX_CARDS {
            let at = queue.live.iter().position(|live| live.card.toast.actions.is_empty()).unwrap_or(0);
            queue.live.remove(at);
        }
        id
    });
    repaint::request_of(ctx, viewport);
    id
}

/// Removes a toast before its timer ends, from whichever viewport holds it. The app asked for it, so no end is
/// reported.
pub fn dismiss(ctx: &Context, id: ToastId) {
    remove(ctx, id, None);
}

/// The host closes the card of `id` for `end`; `end` reports it.
pub(crate) fn close(ctx: &Context, id: ToastId, end: ToastEnd) {
    remove(ctx, id, Some(end));
}

fn remove(ctx: &Context, id: ToastId, end: Option<ToastEnd>) {
    for viewport in pass::live_viewports(ctx) {
        let removed = PerViewport::<Queue>::with(ctx, viewport, |queue| {
            let before = queue.live.len();
            queue.live.retain(|live| live.card.id != id);
            let removed = queue.live.len() != before;
            if let Some(end) = end.filter(|_| removed) {
                record(&mut queue.ended, id, end);
            }
            removed
        });
        if removed {
            repaint::request_of(ctx, viewport);
        }
    }
}

/// How the toast `id` ended, once: `None` while it is queued, after the report was read, and for a toast the cap
/// dropped or the app removed.
pub fn end(ctx: &Context, id: ToastId) -> Option<ToastEnd> {
    pass::live_viewports(ctx).into_iter().find_map(|viewport| {
        PerViewport::<Queue>::with(ctx, viewport, |queue| {
            let at = queue.ended.iter().position(|(ended, _)| *ended == id)?;
            Some(queue.ended.remove(at).1)
        })
    })
}

/// Keeps the toast `id` until it is closed, whatever its timeout (an error the user must read).
pub fn keep(ctx: &Context, id: ToastId) {
    for viewport in pass::live_viewports(ctx) {
        PerViewport::<Queue>::with(ctx, viewport, |queue| {
            if let Some(live) = queue.live.iter_mut().find(|live| live.card.id == id) {
                live.remaining = None;
            }
        });
    }
}

/// The cards of the running viewport, oldest first.
pub fn cards(ctx: &Context) -> Vec<Card> {
    PerViewport::<Queue>::with(ctx, pass::current_viewport(ctx), |queue| queue.live.iter().map(|live| live.card.clone()).collect())
}

/// The host shows `id` in this pass; `held` says the pointer or focus is on it. The first show announces it.
pub fn shown(ctx: &Context, id: ToastId, text: &str, held: bool) {
    let announce = PerViewport::<Queue>::with(ctx, pass::current_viewport(ctx), |queue| {
        let live = queue.live.iter_mut().find(|live| live.card.id == id)?;
        live.held = held;
        let first = !live.shown;
        live.shown = true;
        first.then_some(live.card.toast.politeness)
    });
    if let Some(politeness) = announce {
        access::announce(ctx, text, politeness);
    }
}

/// The end hook: runs the timers of the shown, unheld cards, drops the expired ones (reporting `Expired`) and asks for a pass when the
/// next one expires.
pub(crate) fn end_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let now = frame::clock(ctx);
    let wake = PerViewport::<Queue>::with(ctx, viewport, |queue| {
        let elapsed = queue.last_tick.map_or(0.0, |last| (now - last).max(0.0)) as f32;
        queue.last_tick = Some(now);
        for live in &mut queue.live {
            if live.shown
                && !live.held
                && let Some(remaining) = &mut live.remaining
            {
                *remaining -= elapsed;
            }
        }
        for live in std::mem::take(&mut queue.live) {
            if live.remaining.is_none_or(|remaining| remaining > 0.0) {
                queue.live.push(live);
            } else {
                record(&mut queue.ended, live.card.id, ToastEnd::Expired);
            }
        }
        queue.live.iter().filter(|live| live.shown && !live.held).filter_map(|live| live.remaining).reduce(f32::min)
    });
    if let Some(seconds) = wake {
        repaint::request_after(ctx, viewport, f64::from(seconds));
    }
}
