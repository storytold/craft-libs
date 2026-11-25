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
//! | `attach-pdf` | host → app | `name`, `bytes`, `page` (from 1), `insertion` ([x,y,z]), `width`, `height` in drawing units |
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
//! The app is hosted when its URL has `?host=parent` (framed by a same-origin page, or by a page on
//! an origin listed in the canvas's `data-host-origins`: messages go through `postMessage`) or its
//! canvas has `data-host="page"` (the page runs it directly: messages are `CustomEvent`s on the
//! canvas, the fields in `detail`). This module is the contract itself and builds everywhere;
//! [`web`] (wasm32 only) is the bridge an app's web shell calls.

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
    /// `?host=parent`: a same-origin page, or one on an origin the canvas's `data-host-origins`
    /// lists, frames the app; messages go through `postMessage`.
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
    Ready,
    Save,
    Dirty,
    Failed,
    Handoff,
}

/// What a host sends to an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Incoming {
    Host,
    Open,
    AttachPdf,
    Saved,
    HandoffDone,
}

impl Outgoing {
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
    pub const ALL: [Self; 5] = [Self::Host, Self::Open, Self::AttachPdf, Self::Saved, Self::HandoffDone];

    pub const fn kind(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Open => "open",
            Self::AttachPdf => "attach-pdf",
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

/// The canvas attribute that names the other origins allowed to frame the app with `?host=parent`.
pub const HOST_ORIGINS_ATTRIBUTE: &str = "data-host-origins";

/// Whether a framing page on `origin` may host the app, by the list in the app's own page: the
/// canvas's `data-host-origins`, origins separated by spaces or commas
/// (`https://*.code.example.com vscode-webview://*`). A `*` may only start the host, and stands
/// for one or more whole host labels: never part of a label, a `:`, a `/` or an empty name. An
/// `http` or `https` pattern needs a fixed domain after its `*`. The list is in the page the
/// app's server sends, not in the URL, so a framing page can't add itself. An opaque origin
/// (`null`) is never allowed.
pub fn origin_allowed(origin: &str, allowed: &str) -> bool {
    if origin.is_empty() || origin == "null" {
        return false;
    }
    let origin = origin.to_ascii_lowercase();
    allowed
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|pattern| !pattern.is_empty())
        .any(|pattern| origin_matches(&origin, &pattern.to_ascii_lowercase()))
}

fn origin_matches(origin: &str, pattern: &str) -> bool {
    let Some((head, tail)) = pattern.split_once('*') else { return origin == pattern };
    let web = head == "http://" || head == "https://";
    if !head.ends_with("://") || head.len() == 3 || tail.contains('*') || (!tail.is_empty() && !tail.starts_with('.')) || (web && tail.is_empty()) {
        return false;
    }
    let Some(labels) = origin.strip_prefix(head).and_then(|rest| rest.strip_suffix(tail)) else { return false };
    !labels.is_empty()
        && !labels.starts_with('.')
        && !labels.ends_with('.')
        && !labels.contains("..")
        && labels.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// The origin (`scheme://host:port`, lowercase) of an absolute URL such as `document.referrer`,
/// or `None` for an empty, relative or opaque one. Credentials before an `@` are not part of it.
/// Only for URLs the browser has already serialized: raw input (`\` for `/`, odd escapes) is not
/// cleaned up.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let valid_scheme = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    (valid_scheme && !host.is_empty()).then(|| format!("{scheme}://{host}").to_ascii_lowercase())
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
    fn the_query_switch_wins_over_the_canvas_attribute() {
        assert_eq!(Mode::detect("?host=parent", None), Some(Mode::Parent));
        assert_eq!(Mode::detect("file=a.pdf&host=parent", Some("page")), Some(Mode::Parent));
        assert_eq!(Mode::detect("", Some("page")), Some(Mode::Page));
        assert_eq!(Mode::detect("?host=parents", None), None);
        assert_eq!(Mode::detect("?xhost=parent", None), None);
        assert_eq!(Mode::detect("", Some("frame")), None);
    }

    #[test]
    fn only_listed_other_origins_may_frame_the_app() {
        let list = "https://*.code.example.com, vscode-webview://*  https://editor.example.org:8443";
        for origin in [
            "https://8080--main--ws--ana.code.example.com",
            "https://a.b.code.example.com",
            "HTTPS://Ws.Code.Example.com",
            "vscode-webview://1a2b3c4d",
            "https://editor.example.org:8443",
        ] {
            assert!(origin_allowed(origin, list), "{origin} is listed");
        }
        for origin in [
            "https://code.example.com",            // a * needs at least one label
            "https://evil.com/.code.example.com",  // not an origin a browser reports
            "https://evil.com:1.code.example.com", // no ports or paths inside a *
            "https://x..code.example.com",
            "https://.code.example.com",
            "https://ws.code.example.com.evil.com",
            "http://ws.code.example.com", // another scheme
            "https://editor.example.org", // another port
            "https://editor.example.org:84430",
            "null",
            "",
        ] {
            assert!(!origin_allowed(origin, list), "{origin:?} is not listed");
        }
        assert!(!origin_allowed("https://a.example.com", ""), "no list allows nothing");
        assert!(!origin_allowed("https://a.example.com", "*"), "a bare * is no origin");
        assert!(!origin_allowed("https://a.b.example.com", "https://*.*.example.com"), "one * per origin");
        // A * covers whole labels at the start of the host, nothing else.
        assert!(!origin_allowed("https://evilexample.com", "https://*example.com"));
        assert!(!origin_allowed("https://editor.attacker.com", "https://editor.*"));
        assert!(!origin_allowed("https://anything.com", "https://*"), "any https site is no list");
        assert!(!origin_allowed("https://a.example.com", "*://a.example.com"));
        assert!(origin_allowed("vscode-webview://1a2b", "vscode-webview://*"));
        // Patterns: any case, commas alone, a port after the domain.
        assert!(origin_allowed("https://ws.example.com", "HTTPS://*.EXAMPLE.COM"));
        assert!(origin_allowed("https://b.example.com", "https://a.example.com,https://b.example.com"));
        assert!(origin_allowed("https://ws.x.com:8443", "https://*.x.com:8443"));
        assert!(!origin_allowed("https://ws.x.com", "https://*.x.com:8443"));
    }

    #[test]
    fn a_referrer_gives_its_origin() {
        assert_eq!(origin_of("https://Ws.Example.com:8443/a/b?c#d").as_deref(), Some("https://ws.example.com:8443"));
        assert_eq!(origin_of("https://ws.example.com").as_deref(), Some("https://ws.example.com"));
        assert_eq!(origin_of("https://user:pw@ws.example.com/x").as_deref(), Some("https://ws.example.com"));
        assert_eq!(origin_of("vscode-webview://1a2b/index.html").as_deref(), Some("vscode-webview://1a2b"));
        assert_eq!(origin_of("https://[::1]:8080/a").as_deref(), Some("https://[::1]:8080"));
        for url in ["", "/relative/page", "about:blank", "https:///path", "://host", "1https://host"] {
            assert_eq!(origin_of(url), None, "{url:?}");
        }
    }

    #[test]
    fn only_this_apps_incoming_kinds_parse() {
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:open"), Some(Incoming::Open));
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:handoff-done"), Some(Incoming::HandoffDone));
        assert_eq!(Incoming::parse("pdfcraft", "cadcraft:open"), None, "another app's message");
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:save"), None, "app-to-host kinds are not incoming");
        assert_eq!(Incoming::parse("pdf", "pdfcraft:open"), None, "an app id that is only a prefix");
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraftopen"), None);
    }

    #[test]
    fn every_kind_round_trips_through_its_type() {
        for kind in Incoming::ALL {
            assert_eq!(Incoming::parse("gridcraft", &message_type("gridcraft", kind.kind())), Some(kind));
        }
        assert_eq!(message_type("cadcraft", Outgoing::Handoff.kind()), "cadcraft:handoff");
    }

    #[test]
    fn host_file_names_are_cleaned() {
        assert_eq!(file_name(Some("plan.dxf"), "Drawing.dxf"), "plan.dxf");
        assert_eq!(file_name(Some("  ../../etc/passwd "), "x"), "passwd");
        assert_eq!(file_name(Some("C:\\Users\\sam\\notes.pdf"), "x"), "notes.pdf");
        assert_eq!(file_name(Some("a\u{0}b\nc.txt"), "x"), "abc.txt");
        assert_eq!(file_name(Some("dir/"), "Drawing.dxf"), "Drawing.dxf");
        assert_eq!(file_name(Some(".."), "x"), "x");
        assert_eq!(file_name(None, "Document.pdf"), "Document.pdf");
        assert_eq!(file_name(Some(&"é".repeat(300)), "x").chars().count(), MAX_NAME_CHARS);
    }

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
    fn malformed_message_types_name_no_kind() {
        for bad in
            ["", ":", "pdfcraft", "pdfcraft:", ":open", "pdfcraft:open:extra", "pdfcraft:OPEN", "pdfcraft: open", "pdfcraftx:open", "pdfcraft/open"]
        {
            assert_eq!(Incoming::parse("pdfcraft", bad), None, "{bad:?}");
        }
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:open"), Some(Incoming::Open));
        // The app's own outgoing kinds are not messages to it.
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:save"), None);
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
