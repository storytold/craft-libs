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

use crate::{
    FontIndex, FontsProblem, Inbox, Incoming, MAX_FONT_INDEX_URL_BYTES, MAX_FONT_INDEXES, Mode, Outgoing, VERSION, check_font_index_len,
    check_font_index_url, file_name, message_type,
};

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

/// A font index a host gives, checked for shape only, its `bytes` still a view: the bridge copies
/// no index's bytes into wasm memory until the app takes the set ([`FontIndexView::copy`]), so an app
/// that ignores `fonts` holds only the `url` and `sha256` strings (at most 1024 and 64 bytes). In
/// parent mode a queued message still keeps the browser's structured clone of each whole buffer
/// alive. In page mode the buffers must come from the app page's own realm. Mirrors [`FontIndex`].
#[derive(Debug, Clone)]
pub enum FontIndexView {
    /// `bytes`, 1 to [`crate::MAX_FONT_INDEX_BYTES`] of them, viewed in place.
    Bytes(js_sys::Uint8Array),
    /// An `https` URL and the sha256 of the index file there: copied, at most
    /// [`MAX_FONT_INDEX_URL_BYTES`] and 64 bytes once read. (Before the URL is judged it is copied
    /// from a string of at most that many UTF-16 units, so at most 3 times as many bytes.)
    Url { url: String, sha256: String },
}

impl FontIndexView {
    /// The contract's [`FontIndex`]. The bytes' length is checked again, as a view's buffer may have
    /// been transferred since the message was read, and only then are they copied through a view of
    /// that fixed length (a growing buffer cannot change it): at most [`crate::MAX_FONT_INDEX_BYTES`]
    /// per index and 16 times that for a set.
    pub fn copy(&self) -> Result<FontIndex, FontsProblem> {
        match self {
            Self::Bytes(view) => {
                let len = byte_length(view)?;
                let fixed = view.subarray(0, len as u32);
                if fixed.length() as usize != len {
                    return Err(FontsProblem::NoBytes);
                }
                let mut bytes = vec![0; len];
                fixed.copy_to(&mut bytes);
                FontIndex::from_bytes(bytes)
            }
            Self::Url { url, sha256 } => FontIndex::from_url(url, sha256),
        }
    }
}

/// A message from the host.
pub enum Message {
    /// What the host supports; `handoff`: it routes files the app hands off.
    Host { version: String, handoff: bool },
    /// Open a document as File › Open would. `name` is already cleaned ([`crate::file_name`]).
    Open { name: String, data: FileData },
    /// Attach one PDF page to the current drawing without opening another document.
    /// Bytes stay a view until the app checks their length. Page numbers start at one.
    AttachPdf { name: String, bytes: js_sys::Uint8Array, page: u32, insertion: [f64; 3], width: f64, height: f64 },
    /// Font indexes the host supplies (RFC-0034), each checked for shape only and its bytes still a
    /// view ([`FontIndexView::copy`] copies them): the app checks the indexes' contents itself and
    /// takes the list whole or not at all. A list the bridge refuses arrives as its problem, for the
    /// app to report with `failed` (no `id`, the text starting `fonts:`). The host's sending it is
    /// its consent; a script able to run in the host page or the app's frame can send it too.
    Fonts { indexes: Result<Vec<FontIndexView>, FontsProblem> },
    /// The save with this `id` is stored.
    Saved { id: String },
    /// The host gave a handed-off file to the app that edits it.
    HandoffDone { name: String, app: String },
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

    /// The app couldn't start, couldn't take a document, a save went unconfirmed (`id`), or the
    /// bridge refused a `fonts` list ([`Message::Fonts`]: no `id`, and the error starts `fonts:`).
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

/// The message a host's object describes, or `None` when a required field is missing. `fonts` is
/// the exception: it always reaches the app, with its [`FontsProblem`] in place of the indexes, even
/// when `indexes` is missing.
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
        Incoming::AttachPdf => {
            let FileData::Bytes(bytes) = file_data(data)? else { return None };
            let page = get(data, "page").as_f64()?;
            if !(page.is_finite() && page >= 1.0 && page <= f64::from(u32::MAX) && page.fract() == 0.0) {
                return None;
            }
            let coords = get(data, "insertion").dyn_into::<js_sys::Array>().ok()?;
            if coords.length() != 3 {
                return None;
            }
            let mut insertion = [0.0; 3];
            for (i, out) in insertion.iter_mut().enumerate() {
                *out = coords.get(u32::try_from(i).ok()?).as_f64().filter(|v| v.is_finite())?;
            }
            let width = get(data, "width").as_f64().filter(|v| v.is_finite() && *v > 0.0)?;
            let height = get(data, "height").as_f64().filter(|v| v.is_finite() && *v > 0.0)?;
            Message::AttachPdf {
                name: file_name(string(data, "name").as_deref(), "document.pdf"),
                bytes,
                page: page as u32,
                insertion,
                width,
                height,
            }
        }
        Incoming::Fonts => Message::Fonts { indexes: font_indexes(&get(data, "indexes")) },
        Incoming::Saved => Message::Saved { id: string(data, "id")? },
        Incoming::HandoffDone => {
            Message::HandoffDone { name: string(data, "name").unwrap_or_default(), app: string(data, "app").unwrap_or_default() }
        }
    })
}

/// The `indexes` of a `fonts` message: all of them (one or more), or the problem with the first one
/// refused. The host is trusted and its message is plain data (see the crate's `fonts` notes), so
/// this reads it as `open` does.
fn font_indexes(list: &JsValue) -> Result<Vec<FontIndexView>, FontsProblem> {
    let list = list.dyn_ref::<js_sys::Array>().ok_or(FontsProblem::NotAList)?;
    let count = list.length();
    if count == 0 {
        return Err(FontsProblem::NoIndexes);
    }
    if count as usize > MAX_FONT_INDEXES {
        return Err(FontsProblem::TooMany);
    }
    (0..count).map(|i| font_index(&list.get(i))).collect()
}

/// One entry: exactly `{bytes}` or `{url, sha256}`, where `undefined` and `null` count as absent.
fn font_index(entry: &JsValue) -> Result<FontIndexView, FontsProblem> {
    if !entry.is_object() {
        return Err(FontsProblem::BadEntry);
    }
    let (bytes, url, sha256) = (get(entry, "bytes"), get(entry, "url"), get(entry, "sha256"));
    let absent = |v: &JsValue| v.is_undefined() || v.is_null();
    match (absent(&bytes), absent(&url)) {
        (false, true) if absent(&sha256) => font_bytes(&bytes).map(FontIndexView::Bytes),
        (true, false) => {
            // `length()` counts UTF-16 units, so the URL is copied from at most 1024 units: at most
            // 3072 bytes, and `check_font_index_url` refuses one over 1024 bytes. A sha256 is copied only at 64.
            let Some(url) = url.dyn_ref::<js_sys::JsString>().filter(|u| u.length() as usize <= MAX_FONT_INDEX_URL_BYTES) else {
                return Err(FontsProblem::BadUrl);
            };
            let sha256 = sha256.dyn_ref::<js_sys::JsString>().filter(|s| s.length() == 64).map(String::from).unwrap_or_default();
            let url = String::from(url);
            // The URL is judged before the sha256, as `FontIndex::from_url` judges them.
            check_font_index_url(&url, &sha256)?;
            Ok(FontIndexView::Url { url, sha256 })
        }
        _ => Err(FontsProblem::BadEntry),
    }
}

/// `bytes` as a view, not copied: a `Uint8Array` as it is, an `ArrayBuffer` after its length is read
/// (a detached one reads 0 and never reaches the constructor).
fn font_bytes(bytes: &JsValue) -> Result<js_sys::Uint8Array, FontsProblem> {
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        byte_length(array)?;
        Ok(array.clone())
    } else if let Some(buffer) = bytes.dyn_ref::<js_sys::ArrayBuffer>() {
        byte_length(buffer)?;
        Ok(js_sys::Uint8Array::new(buffer))
    } else {
        Err(FontsProblem::BadEntry)
    }
}

/// The length of an `ArrayBuffer` or `Uint8Array`, checked: `byteLength` is read as a float, since
/// the `u32` getter wraps a buffer of 4 GiB or more to a small length.
fn byte_length(bytes: &JsValue) -> Result<usize, FontsProblem> {
    let length = get(bytes, "byteLength").as_f64().unwrap_or(0.0);
    if length > crate::MAX_FONT_INDEX_BYTES as f64 {
        return Err(FontsProblem::TooLarge);
    }
    let length = length as usize;
    check_font_index_len(length)?;
    Ok(length)
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

/// Browser tests for parsing and delivery of a host's objects, including origin/source checks.
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_attach_pdf_rejects_foreign_origin() {
        let window = web_sys::window().unwrap();
        let origin = window.location().origin().unwrap();
        let host = Host {
            app: "pdf-attach-test",
            transport: Transport::Parent { window: window.clone(), parent: window.clone(), origin: origin.clone() },
            dirty: Cell::new(None),
        };
        let received = Rc::new(Cell::new(0));
        let counter = received.clone();
        host.listen(move |_| counter.set(counter.get() + 1));
        let data = obj(&[
            ("type", "pdf-attach-test:attach-pdf".into()),
            ("name", "page.pdf".into()),
            ("bytes", buffer(3).into()),
            ("page", 1.into()),
            ("insertion", js_sys::Array::of3(&10.into(), &20.into(), &3.into()).into()),
            ("width", 100.into()),
            ("height", 80.into()),
        ]);
        let send = |origin: &str, source: Option<&web_sys::Window>| {
            let init = web_sys::MessageEventInit::new();
            init.set_origin(origin);
            init.set_source(source.map(|s| s.as_ref()));
            init.set_data(&data);
            let event = web_sys::MessageEvent::new_with_event_init_dict("message", &init).unwrap();
            window.dispatch_event(&event).unwrap();
        };
        send("https://foreign.example", Some(&window));
        assert_eq!(received.get(), 0, "a foreign origin changes nothing");
        send(&origin, None);
        assert_eq!(received.get(), 0, "the expected origin with another source changes nothing");
        send(&origin, Some(&window));
        assert_eq!(received.get(), 1, "the same payload from the authorized parent is delivered");
    }

    fn obj(fields: &[(&str, JsValue)]) -> JsValue {
        object(&[], fields).expect("an object").into()
    }

    #[wasm_bindgen_test]
    fn attach_pdf_checks_page_rectangle_and_bytes() {
        let valid = || {
            obj(&[
                ("name", "../sheet.pdf".into()),
                ("bytes", buffer(3).into()),
                ("page", 2.into()),
                ("insertion", js_sys::Array::of3(&10.into(), &20.into(), &3.into()).into()),
                ("width", 100.into()),
                ("height", 80.into()),
            ])
        };
        let Some(Message::AttachPdf { name, bytes, page, insertion, width, height }) = read(Incoming::AttachPdf, &valid()) else {
            panic!("valid attachment must be delivered");
        };
        assert_eq!(name, "sheet.pdf");
        assert_eq!(bytes.length(), 3);
        assert_eq!((page, insertion, width, height), (2, [10.0, 20.0, 3.0], 100.0, 80.0));
        for (key, value) in [
            ("bytes", JsValue::NULL),
            ("page", 0.into()),
            ("page", 1.5.into()),
            ("page", f64::INFINITY.into()),
            ("page", 4294967296.0.into()),
            ("insertion", js_sys::Array::of2(&0.into(), &0.into()).into()),
            ("insertion", js_sys::Array::of3(&0.into(), &f64::NAN.into(), &0.into()).into()),
            ("width", 0.into()),
            ("height", (-1).into()),
            ("width", "100".into()),
        ] {
            let data = valid();
            js_sys::Reflect::set(&data, &key.into(), &value).unwrap();
            assert!(read(Incoming::AttachPdf, &data).is_none(), "invalid {key} must be dropped");
        }
    }

    fn index_list(entries: &[JsValue]) -> JsValue {
        let list = js_sys::Array::new();
        for entry in entries {
            list.push(entry);
        }
        obj(&[("indexes", list.into())])
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// What the app is handed for a `fonts` message: the indexes, or why the bridge refused the list.
    fn fonts(data: &JsValue) -> Result<Vec<FontIndex>, FontsProblem> {
        match read(Incoming::Fonts, data) {
            // A list the bridge took must copy; one it refuses is the bridge's problem, not the copy's.
            Some(Message::Fonts { indexes }) => Ok(indexes?.iter().map(|index| index.copy().expect("a list the bridge took copies")).collect()),
            _ => panic!("a fonts message always reaches the app"),
        }
    }

    /// `buffer` with its contents transferred away: it reads 0 bytes, and `Uint8Array::new` on it throws.
    fn detach(buffer: &js_sys::ArrayBuffer) {
        let global = js_sys::global();
        let clone = js_sys::Reflect::get(&global, &JsValue::from_str("structuredClone")).and_then(|f| f.dyn_into::<js_sys::Function>());
        let options = obj(&[("transfer", js_sys::Array::of1(buffer).into())]);
        assert!(matches!(clone, Ok(f) if f.call2(&global, buffer, &options).is_ok()), "structuredClone transfers it away");
    }

    fn good_url() -> JsValue {
        obj(&[("url", "https://fonts.example/i.json".into()), ("sha256", SHA.into())])
    }

    #[wasm_bindgen_test]
    fn fonts_take_both_forms_of_index() {
        let array = js_sys::Uint8Array::from(&[1u8, 2, 3][..]);
        let source = js_sys::Uint8Array::from(&[0u8, 1, 2, 3, 4, 5, 6, 7][..]);
        let window = js_sys::Uint8Array::new_with_byte_offset_and_length(&source.buffer(), 2, 3);
        let data = index_list(&[good_url(), obj(&[("bytes", array.into())]), obj(&[("bytes", buffer(2).into())]), obj(&[("bytes", window.into())])]);
        assert_eq!(
            fonts(&data),
            Ok(vec![
                FontIndex::Url { url: "https://fonts.example/i.json".into(), sha256: SHA.into() },
                FontIndex::Bytes(vec![1, 2, 3]),
                FontIndex::Bytes(vec![0, 0]),
                FontIndex::Bytes(vec![2, 3, 4]),
            ])
        );
    }

    #[wasm_bindgen_test]
    fn a_fonts_list_has_one_to_sixteen_entries() {
        for data in [obj(&[]), obj(&[("indexes", "https://fonts.example/i.json".into())]), obj(&[("indexes", obj(&[]))]), JsValue::NULL] {
            assert_eq!(fonts(&data), Err(FontsProblem::NotAList));
        }
        assert_eq!(fonts(&index_list(&[])), Err(FontsProblem::NoIndexes));
        let many: Vec<JsValue> = (0..=MAX_FONT_INDEXES).map(|_| good_url()).collect();
        assert_eq!(fonts(&index_list(&many)), Err(FontsProblem::TooMany));
        assert_eq!(fonts(&index_list(&many[1..])).map(|i| i.len()), Ok(MAX_FONT_INDEXES));
    }

    #[wasm_bindgen_test]
    fn a_fonts_message_with_one_bad_entry_is_refused_whole_with_its_problem() {
        let a = "https://a.example/i";
        let detached = buffer(8);
        detach(&detached);
        for (entry, problem) in [
            (JsValue::from_str("a string"), FontsProblem::BadEntry),
            (obj(&[]), FontsProblem::BadEntry),
            (obj(&[("bytes", buffer(1).into()), ("url", a.into()), ("sha256", SHA.into())]), FontsProblem::BadEntry),
            (obj(&[("bytes", buffer(1).into()), ("sha256", SHA.into())]), FontsProblem::BadEntry),
            (obj(&[("bytes", "text".into())]), FontsProblem::BadEntry),
            (obj(&[("bytes", buffer(0).into())]), FontsProblem::NoBytes),
            (obj(&[("bytes", js_sys::Uint8Array::new_with_length(0).into())]), FontsProblem::NoBytes),
            (obj(&[("bytes", js_sys::Uint8Array::new_with_length(crate::MAX_FONT_INDEX_BYTES as u32 + 1).into())]), FontsProblem::TooLarge),
            (obj(&[("bytes", detached.into())]), FontsProblem::NoBytes),
            (obj(&[("bytes", buffer(crate::MAX_FONT_INDEX_BYTES as u32 + 1).into())]), FontsProblem::TooLarge),
            (obj(&[("url", 3.into()), ("sha256", SHA.into())]), FontsProblem::BadUrl),
            (obj(&[("url", "http://a.example/i".into()), ("sha256", SHA.into())]), FontsProblem::BadUrl),
            (obj(&[("url", a.into())]), FontsProblem::BadSha256),
            (obj(&[("url", a.into()), ("sha256", SHA.to_uppercase().into())]), FontsProblem::BadSha256),
        ] {
            assert_eq!(fonts(&index_list(&[good_url(), entry])), Err(problem));
        }
    }

    #[wasm_bindgen_test]
    fn the_bytes_are_copied_only_when_the_app_takes_them() {
        let buffer = buffer(4);
        let Some(Message::Fonts { indexes: Ok(indexes) }) = read(Incoming::Fonts, &index_list(&[obj(&[("bytes", buffer.clone().into())])])) else {
            panic!("a valid list is read");
        };
        // Written after the message was read: the app's copy sees it, so nothing was copied before.
        js_sys::Uint8Array::new(&buffer).fill(7, 0, 4);
        assert_eq!(indexes[0].copy(), Ok(FontIndex::Bytes(vec![7; 4])));
        detach(&buffer);
        assert_eq!(indexes[0].copy(), Err(FontsProblem::NoBytes), "transferred away before the app took it");
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
