//! The host messages between a Craft app's web build and the page that hosts it: one contract for
//! the whole family (storytold/craft-libs#358), so each app's bridge is a few calls instead of its
//! own copy.
//!
//! Messages are plain objects with a `type` of `<app>:<kind>` (`cadcraft:open`):
//!
//! | Kind | Direction | Fields |
//! |---|---|---|
//! | `ready` | app → host | `version`, `appVersion?`, `file?` (`true`: it reads a `File`/`Blob` in slices) |
//! | `host` | host → app | `version`, `handoff?` (`true`: the host routes hand-offs) |
//! | `open` | host → app | `name`, and `bytes` or `file` + `size` |
//! | `save` | app → host | `name`, `bytes`, `id?`, `saveAs?` |
//! | `saved` | host → app | `id` |
//! | `dirty` | app → host | `dirty` |
//! | `failed` | app → host | `error`, `id?` |
//! | `handoff` | app → host | `name`, and `bytes` or `file` + `size` |
//! | `handoff-done` | host → app | `name`, `app` |
//!
//! A host answers `saved` only for a `save` that carries an `id`; an app that sends none treats a
//! save as done once it is posted. Fields marked `?` are optional, so the bridges shipped before
//! this crate already speak version 1. Those bridges put the app's own release in `ready`'s
//! `version` (`"0.14.0"`): a host treats any `version` it doesn't know as `"1"`.
//!
//! The app is hosted when its URL has `?host=parent` (framed by a same-origin page: messages go
//! through `postMessage`) or its canvas has `data-host="page"` (the page runs it directly:
//! messages are `CustomEvent`s on the canvas, the fields in `detail`). This module is the contract
//! itself and builds everywhere; `web` (wasm32 only) is the bridge an app's web shell calls.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

#[cfg(target_arch = "wasm32")]
pub mod web;

/// The contract version an app reports in `ready` and a host in `host`. It changes only when a
/// field changes meaning; new optional fields keep it.
pub const VERSION: &str = "1";

/// Longest file name taken from a host or put in a message, in characters.
pub const MAX_NAME_CHARS: usize = 255;

/// How a page hosts the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `?host=parent`: a same-origin page frames the app; messages go through `postMessage`.
    Parent,
    /// `data-host="page"` on the app's canvas: the page runs the app itself; messages are
    /// `CustomEvent`s on the canvas.
    Page,
}

impl Mode {
    /// The mode for a page URL's query (with or without its leading `?`) and the canvas's
    /// `data-host` attribute. `?host=parent` wins; anything else is not hosted.
    pub fn detect(query: &str, data_host: Option<&str>) -> Option<Self> {
        let query = query.strip_prefix('?').unwrap_or(query);
        if query.split('&').any(|pair| pair == "host=parent") {
            Some(Self::Parent)
        } else if data_host == Some("page") {
            Some(Self::Page)
        } else {
            None
        }
    }
}

/// What an app posts to its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outgoing {
    /// The app is ready to receive messages.
    Ready,
    /// The user asked to save the document.
    Save,
    /// The document's unsaved-changes state changed.
    Dirty,
    /// An operation failed.
    Failed,
    /// The app hands a file off to another app through the host.
    Handoff,
}

/// What a host sends to an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Incoming {
    /// The host's reply to `ready`: its contract version and options.
    Host,
    /// The host sends a file for the app to open.
    Open,
    /// The host confirms a save that carried an `id`.
    Saved,
    /// The host confirms a hand-off is complete.
    HandoffDone,
}

impl Outgoing {
    /// The message kind, the part of the `type` field after `<app>:`.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Save => "save",
            Self::Dirty => "dirty",
            Self::Failed => "failed",
            Self::Handoff => "handoff",
        }
    }
}

impl Incoming {
    /// Every incoming kind.
    pub const ALL: [Self; 4] = [Self::Host, Self::Open, Self::Saved, Self::HandoffDone];

    /// The message kind, the part of the `type` field after `<app>:`.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Open => "open",
            Self::Saved => "saved",
            Self::HandoffDone => "handoff-done",
        }
    }

    /// The incoming kind a message `type` names for `app`, or `None` for another app's message or
    /// one this contract doesn't send to apps.
    pub fn parse(app: &str, message_type: &str) -> Option<Self> {
        let kind = message_type.strip_prefix(app)?.strip_prefix(':')?;
        Self::ALL.into_iter().find(|k| k.kind() == kind)
    }
}

/// The `type` of a message: `<app>:<kind>`.
pub fn message_type(app: &str, kind: &str) -> String {
    format!("{app}:{kind}")
}

/// A file name from a host, made safe to show and save under: path separators and control
/// characters dropped, surrounding space trimmed, at most [`MAX_NAME_CHARS`] characters, and
/// `fallback` when nothing is left.
pub fn file_name(raw: Option<&str>, fallback: &str) -> String {
    let base = raw.unwrap_or_default().rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return fallback.to_string();
    }
    trimmed.chars().take(MAX_NAME_CHARS).collect()
}

/// Remember `next` before the send. A page host can, while that send is still
/// running, report a different state; that later report must win. A failed send
/// restores the previous state only when nothing else recorded one.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn note_dirty(dirty: &Cell<Option<bool>>, next: bool, post: impl FnOnce() -> bool) {
    if dirty.get() == Some(next) {
        return;
    }
    let previous = dirty.replace(Some(next));
    if !post() && dirty.get() == Some(next) {
        dirty.set(previous);
    }
}

/// Hands the host's messages to the app's handler one at a time. A page host dispatches its events
/// synchronously, so it can answer while the handler runs (a `saved` for the `save` the handler
/// just posted): that message waits here and is handled as soon as the running call returns, never
/// dropped and never given to a second, overlapping call.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct Inbox<M, F> {
    handler: RefCell<F>,
    queue: RefCell<VecDeque<M>>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl<M, F: FnMut(M)> Inbox<M, F> {
    pub(crate) fn new(handler: F) -> Self {
        Self { handler: RefCell::new(handler), queue: RefCell::new(VecDeque::new()) }
    }

    pub(crate) fn deliver(&self, message: M) {
        self.queue.borrow_mut().push_back(message);
        // Already running: the outer call drains the queue.
        let Ok(mut handler) = self.handler.try_borrow_mut() else { return };
        loop {
            let next = self.queue.borrow_mut().pop_front();
            let Some(message) = next else { break };
            handler(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dirty_report_during_the_send_wins_and_a_failed_send_is_forgotten() {
        let dirty = Cell::new(Some(false));
        note_dirty(&dirty, true, || {
            note_dirty(&dirty, false, || true);
            true
        });
        assert_eq!(dirty.get(), Some(false));

        let dirty = Cell::new(None);
        note_dirty(&dirty, true, || false);
        assert_eq!(dirty.get(), None);

        let dirty = Cell::new(Some(true));
        let mut sent = false;
        note_dirty(&dirty, true, || {
            sent = true;
            true
        });
        assert!(!sent);
        assert_eq!(dirty.get(), Some(true));
    }

    #[test]
    fn host_file_names_with_hostile_parts_are_cleaned() {
        // Only control characters, or only separators: nothing is left, so the fallback.
        assert_eq!(file_name(Some("\u{0}\u{7}\n\t"), "Drawing.dxf"), "Drawing.dxf");
        assert_eq!(file_name(Some("///"), "Drawing.dxf"), "Drawing.dxf");
        assert_eq!(file_name(Some("   "), "Drawing.dxf"), "Drawing.dxf");
        // Mixed separators and a control character inside the last part.
        assert_eq!(file_name(Some("a/b\\c\u{1b}[31m.txt"), "x"), "c[31m.txt");
        // Exactly at the limit stays whole; one over is cut; the limit counts characters, not bytes.
        assert_eq!(file_name(Some(&"a".repeat(MAX_NAME_CHARS)), "x").chars().count(), MAX_NAME_CHARS);
        assert_eq!(file_name(Some(&"a".repeat(MAX_NAME_CHARS + 1)), "x").chars().count(), MAX_NAME_CHARS);
        assert_eq!(file_name(Some(&"\u{1F4C4}".repeat(300)), "x"), "\u{1F4C4}".repeat(MAX_NAME_CHARS));
    }

    #[test]
    fn messages_that_arrive_while_the_handler_runs_wait_their_turn() {
        use std::rc::Rc;
        type Shared = Rc<Inbox<&'static str, Box<dyn FnMut(&'static str)>>>;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let inbox: Rc<RefCell<Option<Shared>>> = Rc::new(RefCell::new(None));
        let (log, me) = (seen.clone(), inbox.clone());
        let handler: Box<dyn FnMut(&'static str)> = Box::new(move |m: &'static str| {
            log.borrow_mut().push(format!("start {m}"));
            // The page answers inside the handler, as a synchronous dispatchEvent does.
            if m == "open"
                && let Some(inbox) = me.borrow().clone()
            {
                inbox.deliver("saved");
                inbox.deliver("handoff-done");
            }
            log.borrow_mut().push(format!("end {m}"));
        });
        let shared = Rc::new(Inbox::new(handler));
        *inbox.borrow_mut() = Some(shared.clone());
        shared.deliver("open");
        assert_eq!(*seen.borrow(), ["start open", "end open", "start saved", "end saved", "start handoff-done", "end handoff-done"]);
        shared.deliver("host");
        assert_eq!(seen.borrow().len(), 8, "the inbox is usable again after draining");
    }
}
