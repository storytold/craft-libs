//! The bridge an app's web shell calls: find the host, hear its messages, post the app's.
//!
//! ```ignore
//! if let Some(host) = craft_web_host::web::Host::connect("pdfcraft", &canvas) {
//!     host.listen(move |message| inbox.push(message));
//!     host.post_ready(env!("CARGO_PKG_VERSION"), false);
//! }
//! ```

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use web_sys::js_sys;

use crate::{Inbox, Incoming, Mode, Outgoing, VERSION, file_name, message_type};

/// A file the host passes in, not yet copied into the app's memory: check its size first.
pub enum FileData {
    /// `bytes`: an `ArrayBuffer` or `Uint8Array`, viewed in place.
    Bytes(js_sys::Uint8Array),
    /// `file`: a `File` or `Blob`, for the app to read in slices.
    Blob(web_sys::Blob),
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
}

#[derive(Clone)]
enum Transport {
    /// The same-origin parent window; only it is heard and answered.
    Parent { window: web_sys::Window, parent: web_sys::Window, origin: String },
    /// The app's canvas; messages are `CustomEvent`s on it.
    Page { canvas: web_sys::EventTarget },
}

/// The connection to the page hosting the app.
#[derive(Clone)]
pub struct Host {
    app: &'static str,
    transport: Transport,
    dirty: Cell<Option<bool>>,
    commands: Rc<RefCell<Vec<(String, String, Option<String>)>>>,
    queued_bytes: Cell<usize>,
    queued_id_bytes: Cell<usize>,
    repaint: Rc<Cell<Option<Rc<dyn Fn()>>>>,
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
        Some(Self { app, transport, dirty: Cell::new(None), commands: Rc::default(), queued_bytes: Cell::new(0), queued_id_bytes: Cell::new(0), repaint: Rc::default() })
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
        let host = self.clone();
        let commands = self.commands.clone();
        let queued_bytes = self.queued_bytes.clone();
        let queued_id_bytes = self.queued_id_bytes.clone();
        let repaint = self.repaint.clone();
        match &self.transport {
            Transport::Parent { window, parent, origin } => {
                let (parent, origin, inbox, host, commands, queued_bytes, queued_id_bytes, repaint) =
                    (parent.clone(), origin.clone(), inbox.clone(), host.clone(), commands.clone(), queued_bytes.clone(), queued_id_bytes.clone(), repaint.clone());
                let on_message = Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                    if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                        return;
                    }
                    let data = event.data();
                    let Some(kind) = string(&data, "type").and_then(|t| Incoming::parse(app, &t)) else { return };
                    if kind == Incoming::Cmd {
                        let id_raw = string(&data, "id");
                        let request_id = string(&data, "requestId").filter(|r| !r.is_empty());
                        if id_raw.as_ref().map(|s| s.len()).unwrap_or(0) > crate::MAX_ID_BYTES {
                            host.post_response("", request_id.as_deref(), false, Some(&format!("id is larger than {} bytes", crate::MAX_ID_BYTES)));
                            if let Some(f) = repaint.borrow().clone() { f(); }
                            return;
                        }
                        let Some(id) = id_raw.filter(|i| !i.trim().is_empty()) else {
                            host.post_response("", request_id.as_deref(), false, Some("a command needs an id"));
                            if let Some(f) = repaint.borrow().clone() { f(); }
                            return;
                        };
                        if request_id.as_ref().map(str::len).unwrap_or(0) > crate::MAX_ID_BYTES {
                            host.post_response(&id, request_id.as_deref(), false, Some(&format!("requestId is larger than {} bytes", crate::MAX_ID_BYTES)));
                            if let Some(f) = repaint.borrow().clone() { f(); }
                            return;
                        }
                        let params_value = js_sys::Reflect::get(&data, &JsValue::from_str("params")).ok().filter(|v| !v.is_undefined() && !v.is_null());
                        let params = match params_value {
                            None => "{}".to_string(),
                            Some(v) => {
                                let js_len = v.dyn_ref::<js_sys::JsString>().map(|s| s.length() as usize);
                                if js_len.unwrap_or(usize::MAX) > crate::MAX_PARAMS_BYTES {
                                    host.post_response(&id, request_id.as_deref(), false, Some(&format!("params are larger than {} bytes", crate::MAX_PARAMS_BYTES)));
                                    if let Some(f) = repaint.borrow().clone() { f(); }
                                    return;
                                }
                                match v.as_string() {
                                    Some(text) if text.len() <= crate::MAX_PARAMS_BYTES => text,
                                    Some(_) => {
                                        host.post_response(&id, request_id.as_deref(), false, Some(&format!("params are larger than {} bytes", crate::MAX_PARAMS_BYTES)));
                                        if let Some(f) = repaint.borrow().clone() { f(); }
                                        return;
                                    }
                                    None => {
                                        host.post_response(&id, request_id.as_deref(), false, Some("params must be a string of valid JSON"));
                                        if let Some(f) = repaint.borrow().clone() { f(); }
                                        return;
                                    }
                                }
                            }
                        };
                        let id_bytes = id.len() + request_id.as_deref().map(str::len).unwrap_or(0);
                        if commands.borrow().len() >= crate::MAX_PENDING_COMMANDS
                            || queued_bytes.get().saturating_add(params.len()) > crate::MAX_PENDING_PARAMS_BYTES
                            || queued_id_bytes.get().saturating_add(id_bytes) > crate::MAX_PENDING_PARAMS_BYTES
                        {
                            host.post_response(&id, request_id.as_deref(), false, Some("too many commands are waiting; try again"));
                            if let Some(f) = repaint.borrow().clone() { f(); }
                            return;
                        }
                        queued_bytes.set(queued_bytes.get().saturating_add(params.len()));
                        queued_id_bytes.set(queued_id_bytes.get().saturating_add(id_bytes));
                        commands.borrow_mut().push((id, params, request_id));
                        if let Some(f) = repaint.borrow().clone() { f(); }
                        return;
                    }
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
                    let host = host.clone();
                    let commands = commands.clone();
                    let queued_bytes = queued_bytes.clone();
                    let queued_id_bytes = queued_id_bytes.clone();
                    let repaint = repaint.clone();
                    let on_event = Closure::<dyn Fn(web_sys::CustomEvent)>::new(move |event: web_sys::CustomEvent| {
                        if kind == Incoming::Cmd {
                            let data = event.detail();
                            let id_raw = string(&data, "id");
                            let request_id = string(&data, "requestId").filter(|r| !r.is_empty());
                            if id_raw.as_ref().map(|s| s.len()).unwrap_or(0) > crate::MAX_ID_BYTES {
                                host.post_response("", request_id.as_deref(), false, Some(&format!("id is larger than {} bytes", crate::MAX_ID_BYTES)));
                                if let Some(f) = repaint.borrow().clone() { f(); }
                                return;
                            }
                            let Some(id) = id_raw.filter(|i| !i.trim().is_empty()) else {
                                host.post_response("", request_id.as_deref(), false, Some("a command needs an id"));
                                if let Some(f) = repaint.borrow().clone() { f(); }
                                return;
                            };
                            if request_id.as_ref().map(str::len).unwrap_or(0) > crate::MAX_ID_BYTES {
                                host.post_response(&id, request_id.as_deref(), false, Some(&format!("requestId is larger than {} bytes", crate::MAX_ID_BYTES)));
                                if let Some(f) = repaint.borrow().clone() { f(); }
                                return;
                            }
                            let params_value = js_sys::Reflect::get(&data, &JsValue::from_str("params")).ok().filter(|v| !v.is_undefined() && !v.is_null());
                            let params = match params_value {
                                None => "{}".to_string(),
                                Some(v) => {
                                    let js_len = v.dyn_ref::<js_sys::JsString>().map(|s| s.length() as usize);
                                    if js_len.unwrap_or(usize::MAX) > crate::MAX_PARAMS_BYTES {
                                        host.post_response(&id, request_id.as_deref(), false, Some(&format!("params are larger than {} bytes", crate::MAX_PARAMS_BYTES)));
                                        if let Some(f) = repaint.borrow().clone() { f(); }
                                        return;
                                    }
                                    match v.as_string() {
                                        Some(text) if text.len() <= crate::MAX_PARAMS_BYTES => text,
                                        Some(_) => {
                                            host.post_response(&id, request_id.as_deref(), false, Some(&format!("params are larger than {} bytes", crate::MAX_PARAMS_BYTES)));
                                            if let Some(f) = repaint.borrow().clone() { f(); }
                                            return;
                                        }
                                        None => {
                                            host.post_response(&id, request_id.as_deref(), false, Some("params must be a string of valid JSON"));
                                            if let Some(f) = repaint.borrow().clone() { f(); }
                                            return;
                                        }
                                    }
                                }
                            };
                            let id_bytes = id.len() + request_id.as_deref().map(str::len).unwrap_or(0);
                            if commands.borrow().len() >= crate::MAX_PENDING_COMMANDS
                                || queued_bytes.get().saturating_add(params.len()) > crate::MAX_PENDING_PARAMS_BYTES
                                || queued_id_bytes.get().saturating_add(id_bytes) > crate::MAX_PENDING_PARAMS_BYTES
                            {
                                host.post_response(&id, request_id.as_deref(), false, Some("too many commands are waiting; try again"));
                                if let Some(f) = repaint.borrow().clone() { f(); }
                                return;
                            }
                            queued_bytes.set(queued_bytes.get().saturating_add(params.len()));
                            queued_id_bytes.set(queued_id_bytes.get().saturating_add(id_bytes));
                            commands.borrow_mut().push((id, params, request_id));
                            if let Some(f) = repaint.borrow().clone() { f(); }
                            return;
                        }
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

    pub fn take_commands(&self) -> Vec<(String, String, Option<String>)> {
        self.queued_bytes.set(0);
        self.queued_id_bytes.set(0);
        std::mem::take(&mut *self.commands.borrow_mut())
    }

    pub fn post_response(&self, id: &str, request_id: Option<&str>, ok: bool, text: Option<&str>) {
        let mut fields: Vec<(&str, JsValue)> = vec![("id", JsValue::from(id))];
        if let Some(request_id) = request_id {
            fields.push(("requestId", JsValue::from(request_id)));
        }
        fields.push(("ok", JsValue::from(ok)));
        if ok {
            if let Some(value) = text {
                fields.push(("value", JsValue::from(value)));
            }
        } else {
            fields.push(("error", JsValue::from(text.unwrap_or("unknown failure"))));
        }
        let _ = self.post(Outgoing::Response, &fields, None);
    }

    pub fn post_doc_closed(&self) {
        let _ = self.post(Outgoing::DocClosed, &[], None);
    }

    pub fn set_repaint(&self, f: Rc<dyn Fn()>) {
        self.repaint.set(Some(f));
    }

    /// Commands the host sent since the last frame, oldest first: (id, params JSON, request id).
    pub fn take_commands(&self) -> Vec<(String, String, Option<String>)> {
        self.queued_bytes.set(0);
        self.queued_id_bytes.set(0);
        std::mem::take(&mut *self.commands.borrow_mut())
    }

    /// Tell the host how a command (`<app>:cmd`) ended.
    pub fn post_response(&self, id: &str, request_id: Option<&str>, ok: bool, text: Option<&str>) {
        let mut fields: Vec<(&str, JsValue)> = vec![("id", JsValue::from(id))];
        if let Some(request_id) = request_id {
            fields.push(("requestId", JsValue::from(request_id)));
        }
        fields.push(("ok", JsValue::from(ok)));
        if ok {
            if let Some(value) = text {
                fields.push(("value", JsValue::from(value)));
            }
        } else {
            fields.push(("error", JsValue::from(text.unwrap_or("unknown failure"))));
        }
        let _ = self.post(Outgoing::Response, &fields, None);
    }

    /// Tell the host the document it opened has closed.
    pub fn post_doc_closed(&self) {
        let _ = self.post(Outgoing::DocClosed, &[], None);
    }

    /// Let the host wake the frame loop when a command is queued.
    pub fn set_repaint(&self, f: Rc<dyn Fn()>) {
        self.repaint.set(Some(f));
    }

    /// Pass the host a file another app edits; it answers [`Message::HandoffDone`] or, when no
    /// app it hosts takes the type, `failed`. Without a host, tell the user which app the file
    /// needs instead.
    pub fn post_handoff(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        self.post(Outgoing::Handoff, &[("name", file_name(Some(name), "document").into()), ("bytes", buffer.clone().into())], Some(&buffer))
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
}
