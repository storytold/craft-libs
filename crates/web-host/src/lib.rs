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
//! | `fonts` | host → app | `indexes`: 1 to [`MAX_FONT_INDEXES`] entries, each `{bytes}` or `{url, sha256}` (see [`FontIndex`]) |
//! | `save` | app → host | `name`, `bytes`, `id?`, `saveAs?` |
//! | `saved` | host → app | `id` |
//! | `dirty` | app → host | `dirty` |
//! | `failed` | app → host | `error`, `id?` |
//! | `handoff` | app → host | `name`, and `bytes` or `file` + `size` |
//! | `handoff-done` | host → app | `name`, `app` |
//!
//! `fonts` hands the app font indexes in craft-font-index format (RFC-0034, storytold/craft-libs#2331).
//! The host is trusted (RFC-0034 section 7). Its message is plain data, as structured clone gives in
//! parent mode: arrays, objects, strings, `ArrayBuffer`, `Uint8Array`. A page host that passes an
//! object with getters, a `Proxy`, a subclass or a view of the app's own memory is outside the
//! contract, and the bridge promises nothing for it. The host's sending `fonts` is its consent to
//! fetch from those indexes; a script able to run in the host page (page mode) or the app's frame
//! (parent mode) can dispatch one too, and a document and the app's command channel give no way to.
//! The bridge only catches honest mistakes, as it does for `open`, and does not parse an index or a
//! URL: the app checks each index itself (`craft-font-store`'s `Index::from_user_bytes` rules) and
//! takes the whole list or none of it. A list the bridge refuses is refused whole:
//! `web::Message::Fonts` carries the [`FontsProblem`] in place of the indexes, and the app reports it
//! with `failed` (no `id`; the error starts `fonts:`, and an app starts its own fonts refusals with
//! `fonts:` too) and keeps the set it had. An app that does not handle `fonts` ignores it, so silence
//! tells the host nothing. A host sends `fonts` after `ready`, and again after every `ready`: a
//! reloaded app starts with no host set, and a `fonts` sent before the app listens is lost.
//! Each `fonts` message replaces the host's set: a host adds an index by sending the whole set
//! again, and an index it leaves out is gone. An app applies sets in the order they arrive: a later
//! accepted set replaces an earlier one, and a refused one changes nothing. A host that transfers
//! `bytes` gives the buffer up, so it keeps its own copy if it may send the set again. In page mode
//! `bytes` come from the app page's own realm (an `ArrayBuffer` from another frame is not one).
//! An app's own errors about a document never start `fonts:`, so a host can tell the two apart.
//!
//! A host answers `saved` only for a `save` that carries an `id`; an app that sends none treats a
//! save as done once it is posted. Fields marked `?` are optional, so the bridges shipped before
//! this crate already speak version 1. Those bridges put the app's own release in `ready`'s
//! `version` (`"0.14.0"`): a host treats any `version` it doesn't know as `"1"`.
//!
//! The app is hosted when its URL has `?host=parent` (framed by a same-origin page: messages go
//! through `postMessage`) or its canvas has `data-host="page"` (the page runs it directly:
//! messages are `CustomEvent`s on the canvas, the fields in `detail`). This module is the contract
//! itself and builds everywhere; [`web`] (wasm32 only) is the bridge an app's web shell calls.

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

/// The most indexes one `fonts` message may carry.
pub const MAX_FONT_INDEXES: usize = 16;

/// The most bytes one index's `bytes` may hold (the size `craft-font-store` reads an index up to).
pub const MAX_FONT_INDEX_BYTES: usize = 32 << 20;

/// The longest index URL taken, in bytes: `craft-font-store`'s bound for a string (`MAX_STR`).
pub const MAX_FONT_INDEX_URL_BYTES: usize = 1024;

/// A font index a host gives: the constructors check the shape, and the app parses it and refuses a
/// bad one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontIndex {
    /// An index in craft-font-index format (JSON or gzipped JSON), at most [`MAX_FONT_INDEX_BYTES`].
    Bytes(Vec<u8>),
    /// An index to fetch: an `https` URL and the lowercase hex sha256 of the file there.
    Url { url: String, sha256: String },
}

/// Why a `fonts` message is refused. The whole message is: nothing is taken from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontsProblem {
    /// `indexes` is missing or not an array.
    NotAList,
    /// `indexes` is empty: a host gives one or more indexes.
    NoIndexes,
    /// More than [`MAX_FONT_INDEXES`] entries.
    TooMany,
    /// An entry that is not an object with exactly one of `bytes` and `url`: neither, `{bytes, url}`,
    /// `{bytes, sha256}`, or a `bytes` that is neither an `ArrayBuffer` nor a `Uint8Array` (in page
    /// mode one from the app page's own realm). A field
    /// that is `undefined` or `null` counts as absent.
    BadEntry,
    /// `bytes` that are empty or were transferred away (a detached buffer reads as empty).
    NoBytes,
    /// `bytes` over [`MAX_FONT_INDEX_BYTES`].
    TooLarge,
    /// A `url` that is not a string, is not lowercase `https://` with something after it, is over
    /// [`MAX_FONT_INDEX_URL_BYTES`], or holds a control character, whitespace or `\`. The bridge does
    /// not parse the URL: one the browser refuses fails when the app fetches it, which the app
    /// reports as `failed`.
    BadUrl,
    /// A `sha256` that is missing (`{url}` alone: it is required with `url`), not a string, or not 64
    /// lowercase hex digits.
    BadSha256,
}

impl std::fmt::Display for FontsProblem {
    /// The reason, as an app puts it in `failed`: it starts `fonts:`, so a host can tell a refused
    /// list from a failed document.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "fonts: {}",
            match self {
                Self::NotAList => "`indexes` is missing or is not a list of font indexes".to_string(),
                Self::NoIndexes => "`indexes` is empty: give one or more font indexes".to_string(),
                Self::TooMany => format!("`indexes` has more than {MAX_FONT_INDEXES} entries"),
                Self::BadEntry => "an index is `{bytes}` or `{url, sha256}`, one of them, where `bytes` is an ArrayBuffer or Uint8Array".to_string(),
                Self::NoBytes => "an index's `bytes` are empty or were transferred away".to_string(),
                Self::TooLarge => format!("an index's `bytes` is over {} MiB", MAX_FONT_INDEX_BYTES >> 20),
                Self::BadUrl => format!(
                    "an index's `url` is a lowercase https:// URL string with something after https://, of at most {MAX_FONT_INDEX_URL_BYTES} bytes, without whitespace, control characters or backslashes"
                ),
                Self::BadSha256 => "an index's `sha256` is required with `url`: 64 lowercase hexadecimal digits".to_string(),
            }
        )
    }
}

impl FontIndex {
    /// An index by its bytes; refuses empty bytes and bytes over [`MAX_FONT_INDEX_BYTES`]. A caller
    /// checks the length before it copies them.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, FontsProblem> {
        check_font_index_len(bytes.len())?;
        Ok(Self::Bytes(bytes))
    }

    /// An index by its `https` URL and the sha256 of the file there; the URL is judged first.
    pub fn from_url(url: &str, sha256: &str) -> Result<Self, FontsProblem> {
        check_font_index_url(url, sha256)?;
        Ok(Self::Url { url: url.to_string(), sha256: sha256.to_string() })
    }
}

/// Refuse a URL and sha256 that [`FontIndex::from_url`] would refuse, without building the value.
pub(crate) fn check_font_index_url(url: &str, sha256: &str) -> Result<(), FontsProblem> {
    let lowercase_https = url.strip_prefix("https://").is_some_and(|rest| !rest.is_empty());
    if !lowercase_https || url.len() > MAX_FONT_INDEX_URL_BYTES || url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return Err(FontsProblem::BadUrl);
    }
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(FontsProblem::BadSha256);
    }
    Ok(())
}

/// Refuse an index of `len` bytes that is empty or over [`MAX_FONT_INDEX_BYTES`], before it is copied.
pub(crate) fn check_font_index_len(len: usize) -> Result<(), FontsProblem> {
    if len == 0 {
        Err(FontsProblem::NoBytes)
    } else if len > MAX_FONT_INDEX_BYTES {
        Err(FontsProblem::TooLarge)
    } else {
        Ok(())
    }
}

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
    Fonts,
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
    pub const ALL: [Self; 6] = [Self::Host, Self::Open, Self::AttachPdf, Self::Fonts, Self::Saved, Self::HandoffDone];

    pub const fn kind(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Open => "open",
            Self::AttachPdf => "attach-pdf",
            Self::Fonts => "fonts",
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
    fn the_query_switch_wins_over_the_canvas_attribute() {
        assert_eq!(Mode::detect("?host=parent", None), Some(Mode::Parent));
        assert_eq!(Mode::detect("file=a.pdf&host=parent", Some("page")), Some(Mode::Parent));
        assert_eq!(Mode::detect("", Some("page")), Some(Mode::Page));
        assert_eq!(Mode::detect("?host=parents", None), None);
        assert_eq!(Mode::detect("?xhost=parent", None), None);
        assert_eq!(Mode::detect("", Some("frame")), None);
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
    fn an_index_url_is_lowercase_https_within_its_bound_and_without_spaces() {
        let sha = "0123456789abcdef".repeat(4);
        let url = |len: usize| format!("https://a.example/{}", "x".repeat(len - "https://a.example/".len()));
        assert!(FontIndex::from_url("https://fonts.example:8443/index.json?v=2", &sha).is_ok());
        assert!(FontIndex::from_url(&url(MAX_FONT_INDEX_URL_BYTES), &sha).is_ok(), "the bound itself");
        for refused in [
            "http://fonts.example/i.json".to_string(),
            "HTTPS://fonts.example/i.json".to_string(),
            "https://".to_string(),
            url(MAX_FONT_INDEX_URL_BYTES + 1),
            "https://fonts.example/a b.json".to_string(),
            "https://fonts.example/a\u{0}b.json".to_string(),
            "https://fonts.example/a\nb.json".to_string(),
            "https://fonts.example\\i.json".to_string(),
        ] {
            assert_eq!(FontIndex::from_url(&refused, &sha), Err(FontsProblem::BadUrl), "{refused:?}");
        }
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
