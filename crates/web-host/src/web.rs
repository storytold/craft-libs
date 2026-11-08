//! The bridge an app's web shell calls: find the host, hear its messages, post the app's.
//!
//! ```ignore
//! if let Some(host) = craft_web_host::web::Host::connect("pdfcraft", &canvas) {
//!     host.listen(move |message| inbox.push(message));
//!     host.post_ready(env!("CARGO_PKG_VERSION"), false);
//! }
//! ```

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use web_sys::js_sys;

use crate::{Inbox, Incoming, Mode, Outgoing, Participant, VERSION, file_name, message_type};

/// A file the host passes in, not yet copied into the app's memory: check its size first.
pub enum FileData {
    /// `bytes`: an `ArrayBuffer` or `Uint8Array`, viewed in place.
    Bytes(js_sys::Uint8Array),
    /// `file`: a `File` or `Blob`, for the app to read in slices.
    Blob(web_sys::Blob),
}

impl fmt::Debug for FileData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(array) => write!(f, "Bytes({} bytes)", array.length()),
            Self::Blob(blob) => write!(f, "Blob({} bytes)", blob.size()),
        }
    }
}

impl FileData {
    /// The file's length in bytes, from the data itself (a host's `size` field is not trusted).
    pub fn size(&self) -> f64 {
        match self {
            Self::Bytes(array) => f64::from(array.length()),
            Self::Blob(blob) => blob.size(),
        }
    }
}

/// A message from the host.
pub enum Message {
    /// What the host supports; `handoff`: it routes files the app hands off.
    Host { version: String, handoff: bool },
    /// Open a document as File › Open would. `name` is already cleaned ([`crate::file_name`]).
    Open { name: String, data: FileData },
    /// The save with this `id` is stored.
    Saved { id: String },
    /// The host gave a handed-off file to the app that edits it.
    HandoffDone { name: String, app: String },
    /// Join a collaboration session: dial `ws_url` with `token` and take `role`.
    ///
    /// The token is in this message and nowhere else: the app must not put it in the socket URL,
    /// a log, an error or a DOM attribute.
    Session { session_id: String, ws_url: String, token: String, role: String },
    /// The cabinet committed the session: the stored document is now `version`, and `etag` names
    /// that stored content.
    Committed { version: u64, etag: String },
}

// The token is a credential: a debug print of a message must not put it in a log.
impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host { version, handoff } => f.debug_struct("Host").field("version", version).field("handoff", handoff).finish(),
            Self::Open { name, data } => f.debug_struct("Open").field("name", name).field("data", data).finish(),
            Self::Saved { id } => f.debug_struct("Saved").field("id", id).finish(),
            Self::HandoffDone { name, app } => f.debug_struct("HandoffDone").field("name", name).field("app", app).finish(),
            Self::Session { session_id, ws_url, role, .. } => f
                .debug_struct("Session")
                .field("session_id", session_id)
                .field("ws_url", ws_url)
                .field("token", &"<redacted>")
                .field("role", role)
                .finish(),
            Self::Committed { version, etag } => f.debug_struct("Committed").field("version", version).field("etag", etag).finish(),
        }
    }
}

enum Transport {
    /// The same-origin parent window; only it is heard and answered.
    Parent { window: web_sys::Window, parent: web_sys::Window, origin: String },
    /// The app's canvas; messages are `CustomEvent`s on it.
    Page { canvas: web_sys::EventTarget },
}

/// The connection to the page hosting the app.
pub struct Host {
    app: &'static str,
    transport: Transport,
    dirty: Cell<Option<bool>>,
}

impl Host {
    /// The host for `app` (its id, as in `<app>:ready`) running on `canvas`, or `None` when the
    /// app is not hosted: no `?host=parent` and no `data-host="page"`, no framing parent, or a
    /// parent on another origin (messages only go to this origin, so the browser would drop them).
    pub fn connect(app: &'static str, canvas: &web_sys::Element) -> Option<Self> {
        let window = web_sys::window()?;
        let query = window.location().search().unwrap_or_default();
        let transport = match Mode::detect(&query, canvas.get_attribute("data-host").as_deref())? {
            Mode::Parent => {
                let parent = window.parent().ok().flatten()?;
                if js_sys::Object::is(&parent, &window) {
                    return None;
                }
                let origin = window.location().origin().ok()?;
                // Reading another origin's location throws, which is `None` too.
                if parent.location().origin().ok()? != origin {
                    return None;
                }
                Transport::Parent { window, parent, origin }
            }
            Mode::Page => Transport::Page { canvas: canvas.clone().into() },
        };
        Some(Self { app, transport, dirty: Cell::new(None) })
    }

    pub fn mode(&self) -> Mode {
        match self.transport {
            Transport::Parent { .. } => Mode::Parent,
            Transport::Page { .. } => Mode::Page,
        }
    }

    /// Hear the host's messages for this app. The listener lives as long as the page. The
    /// listeners are `Fn`: a page host can dispatch an event while one is still running (a
    /// `saved` for the `save` the handler just posted), and wasm-bindgen refuses to re-enter an
    /// `FnMut` closure before its body runs, so the message would never reach the queue.
    pub fn listen(&self, handler: impl FnMut(Message) + 'static) {
        let app = self.app;
        let inbox = Rc::new(Inbox::new(handler));
        match &self.transport {
            Transport::Parent { window, parent, origin } => {
                let (parent, origin, inbox) = (parent.clone(), origin.clone(), inbox.clone());
                let on_message = Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                    if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                        return;
                    }
                    let data = event.data();
                    let Some(kind) = string(&data, "type").and_then(|t| Incoming::parse(app, &t)) else { return };
                    if let Some(message) = read(kind, &data) {
                        inbox.deliver(message);
                    }
                });
                if window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
                    on_message.forget();
                }
            }
            Transport::Page { canvas } => {
                for kind in Incoming::ALL {
                    let inbox = inbox.clone();
                    let on_event = Closure::<dyn Fn(web_sys::CustomEvent)>::new(move |event: web_sys::CustomEvent| {
                        if let Some(message) = read(kind, &event.detail()) {
                            inbox.deliver(message);
                        }
                    });
                    let name = message_type(app, kind.kind());
                    if canvas.add_event_listener_with_callback(&name, on_event.as_ref().unchecked_ref()).is_ok() {
                        on_event.forget();
                    }
                }
            }
        }
    }

    /// The app is listening: send a document now. `app_version` is the app's own release;
    /// `reads_files`: it takes a `File`/`Blob` and reads it in slices.
    pub fn post_ready(&self, app_version: &str, reads_files: bool) -> Result<(), String> {
        self.post(Outgoing::Ready, &[("version", VERSION.into()), ("appVersion", app_version.into()), ("file", reads_files.into())], None)
    }

    /// Unsaved changes appeared or went away. Repeats of the last state reported are not sent.
    /// The new state is recorded before the send: a page host can report another state while the
    /// event is still dispatching, and that later report wins. A failed send is forgotten.
    pub fn post_dirty(&self, dirty: bool) {
        crate::note_dirty(&self.dirty, dirty, || self.post(Outgoing::Dirty, &[("dirty", dirty.into())], None).is_ok());
    }

    /// Hand the host a file the app would otherwise download: Save, Save As, Export. With an
    /// `id`, the host answers [`Message::Saved`] once it is stored; without one the save is done
    /// once it is posted. The bytes go as a transferred `ArrayBuffer`.
    pub fn post_save(&self, name: &str, bytes: &[u8], id: Option<&str>, save_as: bool) -> Result<(), String> {
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let mut fields =
            vec![("name", JsValue::from(file_name(Some(name), "document"))), ("bytes", buffer.clone().into()), ("saveAs", save_as.into())];
        if let Some(id) = id {
            fields.push(("id", id.into()));
        }
        self.post(Outgoing::Save, &fields, Some(&buffer))
    }

    /// The app couldn't start, couldn't take a document, or a save went unconfirmed (`id`).
    pub fn post_failed(&self, error: &str, id: Option<&str>) -> Result<(), String> {
        let mut fields = vec![("error", JsValue::from(error))];
        if let Some(id) = id {
            fields.push(("id", id.into()));
        }
        self.post(Outgoing::Failed, &fields, None)
    }

    /// Pass the host a file another app edits; it answers [`Message::HandoffDone`] or, when no
    /// app it hosts takes the type, `failed`. Without a host, tell the user which app the file
    /// needs instead.
    pub fn post_handoff(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        self.post(Outgoing::Handoff, &[("name", file_name(Some(name), "document").into()), ("bytes", buffer.clone().into())], Some(&buffer))
    }

    /// The collaboration state the cabinet shows, and who is in the session. `status` is the
    /// app's own word for the sync indicator (`synced`, `pending`, `offline`, `error`), shown as
    /// it is. Send it when a session starts, when sync state changes and after a reconnect.
    pub fn post_session_state(&self, status: &str, participants: &[Participant]) -> Result<(), String> {
        self.post(Outgoing::SessionState, &session_state_fields(status, participants)?, None)
    }

    /// Who is in the session, for the cabinet's participant list. Send it at the sync server's
    /// presence pace (a few times a second at most), not on every edit.
    pub fn post_presence(&self, participants: &[Participant]) -> Result<(), String> {
        self.post(Outgoing::Presence, &[("participants", participants_array(participants)?.into())], None)
    }

    /// Ask the cabinet to commit the session. `bytes` is the document the sync server assembled,
    /// sent as a transferred `ArrayBuffer`; omit it when the host already holds that document. The
    /// host answers [`Message::Committed`] once the commit is stored.
    pub fn post_commit(&self, session_id: &str, bytes: Option<&[u8]>) -> Result<(), String> {
        let (fields, buffer) = commit_fields(session_id, bytes);
        self.post(Outgoing::Commit, &fields, buffer.as_ref())
    }

    fn post(&self, kind: Outgoing, fields: &[(&str, JsValue)], transfer: Option<&js_sys::ArrayBuffer>) -> Result<(), String> {
        let message_type = message_type(self.app, kind.kind());
        let error = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
        match &self.transport {
            Transport::Parent { parent, origin, .. } => {
                let message = object(&[("type", message_type.as_str().into())], fields)?;
                match transfer {
                    Some(buffer) => parent.post_message_with_transfer(&message, origin, &js_sys::Array::of1(buffer)),
                    None => parent.post_message(&message, origin),
                }
                .map_err(error)
            }
            Transport::Page { canvas } => {
                let init = web_sys::CustomEventInit::new();
                init.set_detail(&object(&[], fields)?.into());
                let event = web_sys::CustomEvent::new_with_event_init_dict(&message_type, &init).map_err(error)?;
                canvas.dispatch_event(&event).map(|_| ()).map_err(error)
            }
        }
    }
}

/// The fields of a `session-state`: the app's sync status and who is in the session.
fn session_state_fields(status: &str, participants: &[Participant]) -> Result<Vec<(&'static str, JsValue)>, String> {
    Ok(vec![("status", JsValue::from_str(status)), ("participants", participants_array(participants)?.into())])
}

/// `participants` as objects: `clientId`, `role`, and `color` only when the app has one. An empty
/// slice is an empty array, not a missing field.
fn participants_array(participants: &[Participant]) -> Result<js_sys::Array, String> {
    let array = js_sys::Array::new();
    for participant in participants {
        let mut fields = vec![("clientId", JsValue::from_str(&participant.client_id)), ("role", JsValue::from_str(&participant.role))];
        if let Some(color) = &participant.color {
            fields.push(("color", JsValue::from_str(color)));
        }
        let value: JsValue = object(&[], &fields)?.into();
        array.push(&value);
    }
    Ok(array)
}

/// The fields of a `commit`, and the buffer to transfer: `bytes` only when the app sends them.
fn commit_fields(session_id: &str, bytes: Option<&[u8]>) -> (Vec<(&'static str, JsValue)>, Option<js_sys::ArrayBuffer>) {
    let mut fields = vec![("sessionId", JsValue::from_str(session_id))];
    let buffer = bytes.map(|bytes| js_sys::Uint8Array::from(bytes).buffer());
    if let Some(buffer) = &buffer {
        fields.push(("bytes", buffer.clone().into()));
    }
    (fields, buffer)
}

/// The message a host's object describes, or `None` when a required field is missing.
fn read(kind: Incoming, data: &JsValue) -> Option<Message> {
    Some(match kind {
        Incoming::Host => Message::Host {
            version: string(data, "version").unwrap_or_else(|| VERSION.to_string()),
            handoff: get(data, "handoff").as_bool() == Some(true),
        },
        Incoming::Open => {
            let name = file_name(string(data, "name").as_deref(), "document");
            Message::Open { name, data: file_data(data)? }
        }
        Incoming::Saved => Message::Saved { id: string(data, "id")? },
        Incoming::HandoffDone => {
            Message::HandoffDone { name: string(data, "name").unwrap_or_default(), app: string(data, "app").unwrap_or_default() }
        }
        Incoming::Session => Message::Session {
            session_id: non_empty(data, "sessionId")?,
            ws_url: non_empty(data, "wsUrl")?,
            token: non_empty(data, "token")?,
            // A session without a role is a viewer: the server decides anyway, and the app must
            // not open an editor it was not given.
            role: non_empty(data, "role").unwrap_or_else(|| "viewer".to_string()),
        },
        Incoming::Committed => Message::Committed { version: whole_version(data)?, etag: string(data, "etag")? },
    })
}

/// `file` (a `Blob`; its `size` field is ignored for the Blob's own) or `bytes` (an `ArrayBuffer` or `Uint8Array`), not copied.
fn file_data(data: &JsValue) -> Option<FileData> {
    let file = get(data, "file");
    if let Some(blob) = file.dyn_ref::<web_sys::Blob>() {
        return Some(FileData::Blob(blob.clone()));
    }
    let bytes = get(data, "bytes");
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        Some(FileData::Bytes(array.clone()))
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(FileData::Bytes(js_sys::Uint8Array::new(&bytes)))
    } else {
        None
    }
}

fn object(head: &[(&str, JsValue)], fields: &[(&str, JsValue)]) -> Result<js_sys::Object, String> {
    let m = js_sys::Object::new();
    for (key, value) in head.iter().chain(fields) {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).map_err(|_| "couldn't build the message".to_string())?;
    }
    Ok(m)
}

fn get(data: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    get(data, key).as_string()
}

/// A string field that must carry something: an empty id, URL or token is as good as missing.
fn non_empty(data: &JsValue, key: &str) -> Option<String> {
    string(data, key).filter(|value| !value.is_empty())
}

/// A version from a host: a JavaScript number (`u64`) that is a whole count, not a fraction,
/// a negative or a value no `u64` holds. Numbers past 2^53 lose precision in JavaScript, which is
/// far beyond a session's version count.
fn whole_version(data: &JsValue) -> Option<u64> {
    let value = get(data, "version").as_f64()?;
    // `u64::MAX as f64` rounds up to 2^64, so `<` keeps every value a `u64` holds and drops that.
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value >= u64::MAX as f64 {
        return None;
    }
    Some(value as u64)
}

/// Node tests for the parsing of a host's objects: wrong shapes, missing fields, and sizes the
/// host states against the data it sends. No browser; run with `wasm-bindgen-test-runner`.
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn obj(fields: &[(&str, JsValue)]) -> JsValue {
        object(&[], fields).expect("an object").into()
    }

    fn buffer(len: u32) -> js_sys::ArrayBuffer {
        js_sys::ArrayBuffer::new(len)
    }

    #[wasm_bindgen_test]
    fn an_open_without_bytes_or_a_blob_is_dropped() {
        for data in [
            obj(&[("name", "a.pdf".into())]),
            obj(&[("name", "a.pdf".into()), ("bytes", "text".into())]),
            obj(&[("name", "a.pdf".into()), ("bytes", 42.into())]),
            obj(&[("name", "a.pdf".into()), ("file", "not a blob".into())]),
            JsValue::NULL,
            JsValue::UNDEFINED,
            JsValue::from_str("a string"),
            JsValue::from_f64(7.0),
        ] {
            assert!(read(Incoming::Open, &data).is_none());
        }
    }

    #[wasm_bindgen_test]
    fn an_open_takes_bytes_as_an_array_buffer_or_a_uint8_array() {
        let ab = buffer(3);
        let array = js_sys::Uint8Array::new_with_length(5);
        for (bytes, len) in [(JsValue::from(ab), 3.0), (JsValue::from(array), 5.0)] {
            let Some(Message::Open { data, .. }) = read(Incoming::Open, &obj(&[("name", "a.pdf".into()), ("bytes", bytes)])) else {
                panic!("an open with bytes is read");
            };
            assert_eq!(data.size(), len);
        }
    }

    #[wasm_bindgen_test]
    fn an_open_without_a_name_or_with_a_hostile_one_gets_a_clean_name() {
        let named = |name: Option<&str>| {
            let mut fields = vec![("bytes", JsValue::from(buffer(1)))];
            if let Some(n) = name {
                fields.push(("name", n.into()));
            }
            match read(Incoming::Open, &obj(&fields)) {
                Some(Message::Open { name, .. }) => name,
                _ => panic!("an open with bytes is read"),
            }
        };
        assert_eq!(named(None), "document");
        assert_eq!(named(Some("../../etc/passwd")), "passwd");
        assert_eq!(named(Some("a\u{0}b.txt")), "ab.txt");
        assert_eq!(named(Some(&"x".repeat(300))).chars().count(), crate::MAX_NAME_CHARS);
    }

    #[wasm_bindgen_test]
    fn the_size_is_the_data_s_own_not_the_hosts_claim() {
        // An ArrayBuffer of 64 MiB with a `size` field of 1: the bytes are viewed in place, not
        // copied, and the app sees their real length before it decides to read them.
        let ab = buffer(64 << 20);
        let Some(Message::Open { data, .. }) = read(Incoming::Open, &obj(&[("bytes", ab.clone().into()), ("size", 1.into())])) else {
            panic!("an open with bytes is read");
        };
        assert_eq!(data.size(), f64::from(64u32 << 20));
        let FileData::Bytes(view) = &data else { panic!("bytes stay bytes") };
        assert!(js_sys::Object::is(&view.buffer(), &ab), "a view of the host's buffer, not a copy");

        // A Blob of 5 bytes with a `size` field of a terabyte.
        let blob = web_sys::Blob::new_with_str_sequence(&js_sys::Array::of1(&"hello".into())).expect("a Blob");
        let Some(Message::Open { data, .. }) = read(Incoming::Open, &obj(&[("file", blob.into()), ("size", 1e12.into())])) else {
            panic!("an open with a Blob is read");
        };
        assert_eq!(data.size(), 5.0);
    }

    #[wasm_bindgen_test]
    fn a_saved_needs_a_string_id() {
        assert!(matches!(read(Incoming::Saved, &obj(&[("id", "s1".into())])), Some(Message::Saved { id }) if id == "s1"));
        for data in [obj(&[]), obj(&[("id", 3.into())]), JsValue::NULL, JsValue::from_str("s1")] {
            assert!(read(Incoming::Saved, &data).is_none());
        }
    }

    #[wasm_bindgen_test]
    fn a_host_message_and_a_handoff_done_read_whatever_fields_exist() {
        // Nothing required: a missing version is the contract's, `handoff` is true only for `true`.
        for data in [JsValue::NULL, obj(&[]), obj(&[("handoff", "yes".into())]), obj(&[("handoff", 1.into())])] {
            let Some(Message::Host { version, handoff }) = read(Incoming::Host, &data) else { panic!("a host message is read") };
            assert_eq!(version, crate::VERSION);
            assert!(!handoff);
        }
        assert!(matches!(read(Incoming::Host, &obj(&[("handoff", true.into())])), Some(Message::Host { handoff: true, .. })));
        assert!(matches!(
            read(Incoming::HandoffDone, &obj(&[("name", "a.pdf".into()), ("app", "pdfcraft".into())])),
            Some(Message::HandoffDone { name, app }) if name == "a.pdf" && app == "pdfcraft"
        ));
        assert!(matches!(read(Incoming::HandoffDone, &JsValue::NULL), Some(Message::HandoffDone { name, app }) if name.is_empty() && app.is_empty()));
    }

    fn session(session_id: Option<&str>, ws_url: Option<&str>, token: Option<&str>, role: Option<&str>) -> JsValue {
        let mut fields = Vec::new();
        if let Some(value) = session_id {
            fields.push(("sessionId", value.into()));
        }
        if let Some(value) = ws_url {
            fields.push(("wsUrl", value.into()));
        }
        if let Some(value) = token {
            fields.push(("token", value.into()));
        }
        if let Some(value) = role {
            fields.push(("role", value.into()));
        }
        obj(&fields)
    }

    #[wasm_bindgen_test]
    fn a_session_needs_an_id_a_url_and_a_token() {
        let Some(Message::Session { session_id, ws_url, token, role }) =
            read(Incoming::Session, &session(Some("s1"), Some("wss://collab.example/ws"), Some("jwt"), Some("editor")))
        else {
            panic!("a whole session is read");
        };
        assert_eq!(session_id, "s1");
        assert_eq!(ws_url, "wss://collab.example/ws");
        assert_eq!(token, "jwt");
        assert_eq!(role, "editor");

        // A session missing any of the three, or with an empty one, leaves the app single-user.
        for data in [
            session(None, Some("wss://x"), Some("jwt"), None),
            session(Some("s1"), None, Some("jwt"), None),
            session(Some("s1"), Some("wss://x"), None, None),
            session(Some(""), Some("wss://x"), Some("jwt"), None),
            session(Some("s1"), Some(""), Some("jwt"), None),
            session(Some("s1"), Some("wss://x"), Some(""), None),
            obj(&[("sessionId", 1.into()), ("wsUrl", "wss://x".into()), ("token", "jwt".into())]),
            JsValue::NULL,
            JsValue::from_str("pdfcraft:session"),
        ] {
            assert!(read(Incoming::Session, &data).is_none(), "{data:?}");
        }

        // No role is the least privilege, not an editor.
        let Some(Message::Session { role, .. }) = read(Incoming::Session, &session(Some("s1"), Some("wss://x"), Some("jwt"), None)) else {
            panic!("a session without a role is read");
        };
        assert_eq!(role, "viewer");
        assert_eq!(crate::message_type("pdfcraft", Incoming::Session.kind()), "pdfcraft:session");
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:session"), Some(Incoming::Session));
        assert_eq!(Incoming::parse("pdfcraft", "cadcraft:session"), None);
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:session-state"), None);
    }

    #[wasm_bindgen_test]
    fn a_committed_needs_a_whole_version_and_an_etag() {
        for version in [0u64, 7, 1 << 40] {
            let data = obj(&[("version", (version as f64).into()), ("etag", "\"abc\"".into())]);
            let Some(Message::Committed { version: read_version, etag }) = read(Incoming::Committed, &data) else {
                panic!("committed {version} is read");
            };
            assert_eq!(read_version, version);
            assert_eq!(etag, "\"abc\"");
        }
        // A version must be a whole number JavaScript can carry; an etag is required.
        for data in [
            obj(&[("version", 7.into())]),
            obj(&[("version", "7".into()), ("etag", "\"abc\"".into())]),
            obj(&[("version", 7.5.into()), ("etag", "\"abc\"".into())]),
            obj(&[("version", (-1).into()), ("etag", "\"abc\"".into())]),
            obj(&[("version", 1e300.into()), ("etag", "\"abc\"".into())]),
            obj(&[("version", f64::NAN.into()), ("etag", "\"abc\"".into())]),
            obj(&[("etag", "\"abc\"".into())]),
            obj(&[("version", 7.into()), ("etag", 7.into())]),
            JsValue::NULL,
        ] {
            assert!(read(Incoming::Committed, &data).is_none(), "{data:?}");
        }
        assert_eq!(crate::message_type("pdfcraft", Incoming::Committed.kind()), "pdfcraft:committed");
    }

    fn people() -> [Participant; 2] {
        [
            Participant { client_id: "c1".into(), role: "owner".into(), color: Some("#112233".into()) },
            Participant { client_id: "c2".into(), role: "viewer".into(), color: None },
        ]
    }

    #[wasm_bindgen_test]
    fn participants_serialize_to_camel_case_objects_and_an_empty_list_stays_empty() {
        let empty = participants_array(&[]).expect("an array");
        assert_eq!(empty.length(), 0, "an empty session still sends an empty list");

        let array = participants_array(&people()).expect("an array");
        assert_eq!(array.length(), 2);
        let first = array.get(0);
        assert_eq!(string(&first, "clientId").as_deref(), Some("c1"));
        assert_eq!(string(&first, "role").as_deref(), Some("owner"));
        assert_eq!(string(&first, "color").as_deref(), Some("#112233"));
        let second = array.get(1);
        assert_eq!(string(&second, "clientId").as_deref(), Some("c2"));
        assert_eq!(string(&second, "role").as_deref(), Some("viewer"));
        assert!(get(&second, "color").is_undefined(), "no colour when the app has none");
    }

    #[wasm_bindgen_test]
    fn session_state_and_presence_carry_the_cabinet_s_ui_state() {
        let fields = session_state_fields("pending", &people()).expect("fields");
        let message: JsValue = object(&[], &fields).expect("an object").into();
        assert_eq!(string(&message, "status").as_deref(), Some("pending"));
        let list = get(&message, "participants");
        assert!(js_sys::Array::is_array(&list));
        assert_eq!(string(&get(&list, "1"), "clientId").as_deref(), Some("c2"));

        assert_eq!(crate::message_type("pdfcraft", Outgoing::SessionState.kind()), "pdfcraft:session-state");
        assert_eq!(crate::message_type("pdfcraft", Outgoing::Presence.kind()), "pdfcraft:presence");
    }

    #[wasm_bindgen_test]
    fn a_commit_carries_bytes_only_when_the_app_has_them() {
        let (fields, transfer) = commit_fields("s1", None);
        assert!(transfer.is_none());
        let message: JsValue = object(&[], &fields).expect("an object").into();
        assert_eq!(string(&message, "sessionId").as_deref(), Some("s1"));
        assert!(get(&message, "bytes").is_undefined(), "no bytes field, not an empty one");

        let (fields, transfer) = commit_fields("s1", Some(&[1, 2, 3]));
        let buffer = transfer.expect("a buffer to transfer");
        let message: JsValue = object(&[], &fields).expect("an object").into();
        let view = js_sys::Uint8Array::new(&get(&message, "bytes"));
        assert_eq!(view.length(), 3);
        assert!(js_sys::Object::is(&view.buffer(), &buffer), "the message carries the buffer that is transferred");
        assert_eq!(crate::message_type("pdfcraft", Outgoing::Commit.kind()), "pdfcraft:commit");
    }

    #[wasm_bindgen_test]
    fn a_session_token_is_never_printed() {
        let data = session(Some("s1"), Some("wss://x"), Some("secret-jwt"), None);
        let message = read(Incoming::Session, &data).expect("a session");
        let printed = format!("{message:?}");
        assert!(!printed.contains("secret-jwt"), "{printed}");
        assert!(printed.contains("s1"));
    }
}
