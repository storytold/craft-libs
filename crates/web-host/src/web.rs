//! The bridge an app's web shell calls: find the host, hear its messages, post the app's.
//!
//! ```ignore
//! if let Some(host) = craft_web_host::web::Host::connect("pdfcraft", &canvas) {
//!     host.listen(move |message| inbox.push(message));
//!     host.post_ready(env!("CARGO_PKG_VERSION"), false);
//! }
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use web_sys::js_sys;

use crate::{Incoming, Mode, Outgoing, VERSION, file_name, message_type};

/// A file the host passes in, not yet copied into the app's memory: check its size first.
pub enum FileData {
    /// `bytes`: an `ArrayBuffer` or `Uint8Array`, viewed in place.
    Bytes(js_sys::Uint8Array),
    /// `file` + `size`: a `File` or `Blob`, for the app to read in slices.
    Blob { blob: web_sys::Blob, size: f64 },
}

impl FileData {
    /// The file's length in bytes.
    pub fn size(&self) -> f64 {
        match self {
            Self::Bytes(array) => f64::from(array.length()),
            Self::Blob { size, .. } => *size,
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

    /// Hear the host's messages for this app. The listener lives as long as the page.
    pub fn listen(&self, handler: impl FnMut(Message) + 'static) {
        let app = self.app;
        let handler = Rc::new(RefCell::new(handler));
        match &self.transport {
            Transport::Parent { window, parent, origin } => {
                let (parent, origin, handler) = (parent.clone(), origin.clone(), handler.clone());
                let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                    if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                        return;
                    }
                    let data = event.data();
                    let Some(kind) = string(&data, "type").and_then(|t| Incoming::parse(app, &t)) else { return };
                    if let Some(message) = read(kind, &data) {
                        (handler.borrow_mut())(message);
                    }
                });
                if window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
                    on_message.forget();
                }
            }
            Transport::Page { canvas } => {
                for kind in Incoming::ALL {
                    let handler = handler.clone();
                    let on_event = Closure::<dyn FnMut(web_sys::CustomEvent)>::new(move |event: web_sys::CustomEvent| {
                        if let Some(message) = read(kind, &event.detail()) {
                            (handler.borrow_mut())(message);
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
    pub fn post_dirty(&self, dirty: bool) {
        if self.dirty.get() != Some(dirty) && self.post(Outgoing::Dirty, &[("dirty", dirty.into())], None).is_ok() {
            self.dirty.set(Some(dirty));
        }
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

/// `file` (a `Blob`, with `size`) or `bytes` (an `ArrayBuffer` or `Uint8Array`), not copied.
fn file_data(data: &JsValue) -> Option<FileData> {
    let file = get(data, "file");
    if let Some(blob) = file.dyn_ref::<web_sys::Blob>() {
        let size = get(data, "size").as_f64().unwrap_or_else(|| blob.size());
        return Some(FileData::Blob { blob: blob.clone(), size });
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
