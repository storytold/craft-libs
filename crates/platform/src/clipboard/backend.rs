//! Clipboard backends: the OS clipboard and the plan for what a write puts where.

#[cfg(not(target_arch = "wasm32"))]
use std::borrow::Cow;
use std::path::PathBuf;

use super::*;

/// What a write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Written {
    /// The clipboard holds the new contents.
    Written,
    /// Dropped: the write came from synthetic input ([`Clipboard::set_synthetic`]), and the
    /// clipboard keeps what the user copied.
    Fenced,
}

/// Where clipboard bytes go: the OS clipboard, the browser's, or a test's.
///
/// `Send` so a [`ClipboardProbe`] can own one on its worker thread. Every method may block on a
/// slow clipboard owner (an X11 client that never answers); the UI thread calls only
/// [`Clipboard`]'s reads and writes, on a user's copy or paste.
pub trait ClipboardBackend: Send {
    /// Replaces the clipboard's contents with `flavors`, best first; none empties it. A backend
    /// holds fewer flavours than an app offers, so it answers the index in `flavors` of the one
    /// it wrote (`0` when it emptied the clipboard); [`Clipboard::holds_ours`] compares that one.
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError>;
    /// The first of `mimes` the clipboard holds, at most `limit` bytes of it (more is
    /// [`ClipboardError::TooLarge`]). [`BITMAP`] matches any bitmap, returned as an [`RGBA8`] flavor.
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError>;
    /// The clipboard's bitmap as RGBA8, at most `limit` bytes of pixels.
    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError>;
    /// Copied files (Finder, Explorer, GNOME Files), at most `limit` of them.
    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError>;
    /// Does the clipboard hold one of `mimes`? Cheap enough to ask a few times a second.
    fn has(&mut self, mimes: &[&str]) -> bool;
    /// The flavours of a paste the host delivered as an event (the browser's Paste). Only
    /// [`EventClipboard`] keeps them; the system clipboard ignores them.
    fn offer(&mut self, _flavors: Vec<Flavor>) {}
}

/// The desktop's clipboard through arboard: X11 and Wayland, macOS, Windows. Not on wasm.
///
/// The handle opens on first use and stays open (on X11 the last handle alive hands what the app
/// copied to the clipboard manager at exit). When no backend opens, every call answers
/// [`ClipboardError::Unavailable`] and [`unreadable`](Self::unreadable) turns true. On Wayland a
/// bitmap arboard can't read (it asks for `image/png` only; a screenshot tool may offer only
/// `image/jpeg` or `image/bmp`, storytold/photocraft#2546) reads as `None`.
///
/// arboard reaches text, HTML, one bitmap and copied files; other flavors ([`SVG`], [`PDF`],
/// [`RTF`]) are not written or read here.
#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub struct SystemClipboard {
    handle: Option<arboard::Clipboard>,
    unreadable: bool,
    decode: Option<Decode>,
    presence: presence::Presence,
}

/// The app's decoder for encoded bitmaps ([`SystemClipboard::decoder`]).
#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub(crate) type Decode = Box<dyn Fn(&[u8]) -> Option<ClipboardImage> + Send>;

#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
impl SystemClipboard {
    /// A clipboard that connects to the system clipboard on first use.
    pub fn new() -> Self {
        Self { handle: None, unreadable: false, decode: None, presence: presence::Presence::new() }
    }

    /// Decodes a [`PNG`] flavor to pixels for [`ClipboardBackend::write`], which hands the
    /// platform pixels only. Without one, a write offers no PNG.
    pub fn decoder(mut self, decode: impl Fn(&[u8]) -> Option<ClipboardImage> + Send + 'static) -> Self {
        self.decode = Some(Box::new(decode));
        self
    }

    /// True when the last attempt to open the clipboard found no backend at all.
    pub fn unreadable(&self) -> bool {
        self.unreadable
    }

    fn open(&mut self) -> Result<&mut arboard::Clipboard, ClipboardError> {
        if self.handle.is_none() {
            match arboard::Clipboard::new() {
                Ok(handle) => {
                    self.unreadable = false;
                    self.handle = Some(handle);
                }
                Err(error) => {
                    log::debug!("no system clipboard: {error}");
                    self.unreadable = true;
                }
            }
        }
        self.handle.as_mut().ok_or(ClipboardError::Unavailable)
    }
}

#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub(super) fn system_error(error: arboard::Error) -> ClipboardError {
    match error {
        arboard::Error::ClipboardOccupied => ClipboardError::Busy,
        arboard::Error::ClipboardNotSupported => ClipboardError::Unavailable,
        error => ClipboardError::Other(error.to_string()),
    }
}

/// A read's value, or `None` when the clipboard holds nothing in that format. `unconvertible`
/// also counts content the format can't be converted from (text that isn't UTF-8) as nothing.
#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub(super) fn present<T>(read: Result<T, arboard::Error>, unconvertible: bool) -> Result<Option<T>, ClipboardError> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(arboard::Error::ConversionFailure) if unconvertible => Ok(None),
        Err(error) => Err(system_error(error)),
    }
}

/// Whether the system clipboard offers a format, asked of the system without reading or decoding
/// any content: Windows `IsClipboardFormatAvailable`, macOS the pasteboard's type list, Wayland
/// the offer's MIME types (`wl_clipboard_rs::paste::get_mime_types`), X11 a `TARGETS` request.
///
/// The macOS and Windows code is not built on the Linux box; it is a few lines each and needs a
/// check on those systems.
#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
mod presence {
    #[cfg(any(windows, target_os = "macos"))]
    use super::{BITMAP, HTML, RGBA8, TEXT, URI_LIST};

    pub(super) struct Presence {
        #[cfg(any(windows, target_os = "macos"))]
        _system: (),
        #[cfg(not(any(windows, target_os = "macos")))]
        x11: Option<x11::Targets>,
    }

    #[cfg(windows)]
    impl Presence {
        pub(super) fn new() -> Self {
            Self { _system: () }
        }

        pub(super) fn any(&mut self, mimes: &[&str]) -> bool {
            use clipboard_win::{formats, is_format_avail, register_format};
            let registered = |name: &str| register_format(name).is_some_and(|format| is_format_avail(format.get()));
            mimes.iter().any(|&mime| match mime {
                TEXT => is_format_avail(formats::CF_UNICODETEXT),
                HTML => registered("HTML Format"),
                BITMAP | RGBA8 => registered("PNG") || is_format_avail(formats::CF_DIBV5) || is_format_avail(formats::CF_DIB),
                URI_LIST => is_format_avail(formats::CF_HDROP),
                _ => false,
            })
        }
    }

    #[cfg(target_os = "macos")]
    impl Presence {
        pub(super) fn new() -> Self {
            Self { _system: () }
        }

        pub(super) fn any(&mut self, mimes: &[&str]) -> bool {
            use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypeHTML, NSPasteboardTypeString, NSPasteboardTypeTIFF};
            // `types` lists the formats on offer; no item's data is fetched.
            let Some(types) = (unsafe { NSPasteboard::generalPasteboard().types() }) else { return false };
            mimes.iter().any(|&mime| {
                // Safety: the pasteboard type constants are valid for the life of the process.
                let wanted = unsafe {
                    match mime {
                        TEXT => NSPasteboardTypeString,
                        HTML => NSPasteboardTypeHTML,
                        BITMAP | RGBA8 => NSPasteboardTypeTIFF,
                        URI_LIST => NSPasteboardTypeFileURL,
                        _ => return false,
                    }
                };
                types.iter().any(|offered| offered.isEqualToString(wanted))
            })
        }
    }

    /// Linux and BSD. Wayland when `WAYLAND_DISPLAY` is set and the compositor has a data-control
    /// protocol, else X11: the order arboard picks its own backend in.
    #[cfg(not(any(windows, target_os = "macos")))]
    impl Presence {
        pub(super) fn new() -> Self {
            Self { x11: None }
        }

        pub(super) fn any(&mut self, mimes: &[&str]) -> bool {
            let offered = self.offered();
            mimes.iter().any(|&mime| super::offered_has(&offered, mime))
        }

        fn offered(&mut self) -> Vec<String> {
            use wl_clipboard_rs::paste::{ClipboardType, Error, Seat, get_mime_types};
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                match get_mime_types(ClipboardType::Regular, Seat::Unspecified) {
                    Ok(types) => return types.into_iter().collect(),
                    // Nothing is copied.
                    Err(Error::NoSeats | Error::ClipboardEmpty | Error::NoMimeType) => return Vec::new(),
                    Err(error) => log::debug!("no Wayland data control for the clipboard probe, trying X11: {error}"),
                }
            }
            if self.x11.is_none() {
                self.x11 = x11::Targets::connect();
            }
            self.x11.as_ref().map(x11::Targets::offered).unwrap_or_default()
        }
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    mod x11 {
        use std::time::{Duration, Instant};

        use x11rb::connection::Connection;
        use x11rb::protocol::Event;
        use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, WindowClass};
        use x11rb::rust_connection::RustConnection;
        use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

        /// How long a `TARGETS` request waits for the owner. The probe's worker waits, not the UI.
        const ANSWER_WAIT: Duration = Duration::from_millis(250);

        /// A connection and a window that receive the clipboard owner's `TARGETS` answer.
        pub(super) struct Targets {
            conn: RustConnection,
            window: u32,
            clipboard: Atom,
            targets: Atom,
            property: Atom,
        }

        impl Targets {
            pub(super) fn connect() -> Option<Self> {
                let (conn, screen) = RustConnection::connect(None).ok()?;
                let root = conn.setup().roots.get(screen)?.root;
                let window = conn.generate_id().ok()?;
                conn.create_window(
                    COPY_DEPTH_FROM_PARENT,
                    window,
                    root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .ok()?;
                let atom = |name: &[u8]| conn.intern_atom(false, name).ok()?.reply().ok().map(|reply| reply.atom);
                let (clipboard, targets, property) = (atom(b"CLIPBOARD")?, atom(b"TARGETS")?, atom(b"CRAFT_CLIPBOARD_TARGETS")?);
                Some(Self { conn, window, clipboard, targets, property })
            }

            /// The target names the CLIPBOARD owner offers; none when nobody owns it or answers.
            pub(super) fn offered(&self) -> Vec<String> {
                self.ask().unwrap_or_default()
            }

            fn ask(&self) -> Option<Vec<String>> {
                self.conn.delete_property(self.window, self.property).ok()?;
                self.conn.convert_selection(self.window, self.clipboard, self.targets, self.property, CURRENT_TIME).ok()?;
                self.conn.flush().ok()?;
                let end = Instant::now() + ANSWER_WAIT;
                while Instant::now() < end {
                    match self.conn.poll_for_event().ok()? {
                        Some(Event::SelectionNotify(event)) if event.selection == self.clipboard => {
                            if event.property == NONE {
                                return None;
                            }
                            let reply =
                                self.conn.get_property(true, self.window, self.property, AtomEnum::ATOM, 0, u32::MAX / 4).ok()?.reply().ok()?;
                            let atoms: Vec<Atom> = reply.value32()?.collect();
                            return Some(
                                atoms
                                    .into_iter()
                                    .filter_map(|atom| {
                                        let name = self.conn.get_atom_name(atom).ok()?.reply().ok()?.name;
                                        String::from_utf8(name).ok()
                                    })
                                    .collect(),
                            );
                        }
                        Some(_) => {}
                        None => std::thread::sleep(Duration::from_millis(1)),
                    }
                }
                None
            }
        }
    }
}

/// Whether the target or MIME names a clipboard `offered` contain a format that [`SystemClipboard`]
/// can read as `mime`. Text includes the X11 text targets; a bitmap is `image/png`, the only
/// bitmap arboard reads on Linux.
#[allow(dead_code, reason = "only the Linux and BSD probe maps names")]
pub(crate) fn offered_has(offered: &[String], mime: &str) -> bool {
    let any = |names: &[&str]| offered.iter().any(|offer| names.iter().any(|name| offer.eq_ignore_ascii_case(name)));
    match mime {
        TEXT => any(&["text/plain", "text/plain;charset=utf-8", "UTF8_STRING", "STRING", "TEXT"]),
        HTML => any(&["text/html"]),
        BITMAP | RGBA8 => any(&[PNG]),
        URI_LIST => any(&[URI_LIST]),
        _ => false,
    }
}

/// What a write puts on the system clipboard.
#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub(crate) enum Put {
    Clear,
    Image(ClipboardImage),
    Html { html: String, alt: Option<String> },
    Text(String),
}

/// The first flavor the system clipboard can hold, as its index in `flavors` and what to put
/// there.
#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub(crate) fn plan_write(flavors: &[Flavor], decode: Option<&Decode>) -> Result<(usize, Put), ClipboardError> {
    if flavors.is_empty() {
        return Ok((0, Put::Clear));
    }
    let text_of = |f: &Flavor| String::from_utf8_lossy(&f.data).into_owned();
    for (index, flavor) in flavors.iter().enumerate() {
        match &*flavor.mime {
            RGBA8 => {
                if let Some(image) = rgba8_image(&flavor.data).filter(ClipboardImage::is_whole) {
                    return Ok((index, Put::Image(image)));
                }
            }
            PNG => {
                if let Some(image) = decode.and_then(|decode| decode(&flavor.data)).filter(ClipboardImage::is_whole) {
                    return Ok((index, Put::Image(image)));
                }
            }
            HTML => return Ok((index, Put::Html { html: text_of(flavor), alt: flavors.iter().find(|f| f.mime == TEXT).map(text_of) })),
            TEXT => return Ok((index, Put::Text(text_of(flavor)))),
            _ => {}
        }
    }
    Err(ClipboardError::NoFormat)
}

#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
impl ClipboardBackend for SystemClipboard {
    // pc: apps/photocraft/src/services.rs:702 clipboard_set_image (open on first use, "clipboard is busy")
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        let (index, put) = plan_write(flavors, self.decode.as_ref())?;
        let handle = self.open()?;
        match put {
            Put::Clear => handle.clear(),
            Put::Image(image) => {
                handle.set_image(arboard::ImageData { width: image.width as usize, height: image.height as usize, bytes: Cow::Borrowed(&image.rgba) })
            }
            Put::Html { html, alt } => handle.set_html(html, alt),
            Put::Text(text) => handle.set_text(text),
        }
        .map_err(system_error)?;
        Ok(index)
    }

    // vectorcraft: crates/ui-egui/src/sysclip.rs:27 SystemClipboard::read
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        for &mime in mimes {
            let found = match mime {
                TEXT => present(self.open()?.get().text(), true)?.map(|text| Flavor::text(&text)),
                HTML => present(self.open()?.get().html(), true)?.map(|html| Flavor::new(HTML, html.into_bytes())),
                BITMAP | RGBA8 => self.read_image(limit)?.map(|image| rgba8_flavor(&image)),
                URI_LIST => {
                    let files = present(self.open()?.get().file_list(), false)?.filter(|files| !files.is_empty());
                    files.map(|files| Flavor::new(URI_LIST, uri_list(&files, cfg!(windows)).into_bytes()))
                }
                _ => None,
            };
            if let Some(flavor) = found {
                if flavor.data.len() as u64 > limit {
                    return Err(ClipboardError::TooLarge { limit });
                }
                return Ok(Some(flavor));
            }
        }
        Ok(None)
    }

    // pc: apps/photocraft/src/services.rs:714 clipboard_get_image, :536 wayland_clipboard_image, :601 clipboard_read
    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        let Some(image) = present(self.open()?.get_image(), false)? else { return Ok(None) };
        if image.bytes.len() as u64 > limit {
            return Err(ClipboardError::TooLarge { limit });
        }
        Ok(Some(ClipboardImage { width: image.width as u32, height: image.height as u32, rgba: image.bytes.into_owned() }))
    }

    // pc: apps/photocraft/src/services.rs:506 image_from_files (trailing '\r' of text/uri-list lines trimmed)
    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let Some(files) = present(self.open()?.get().file_list(), false)? else { return Ok(None) };
        // arboard splits a text/uri-list on LF only, so a path copied in GNOME Files keeps its CR.
        let files: Vec<PathBuf> = files.into_iter().map(trim_cr).collect();
        if files.len() > limit {
            return Err(ClipboardError::TooLarge { limit: limit as u64 });
        }
        Ok((!files.is_empty()).then_some(files))
    }

    // vectorcraft: crates/ui-egui/src/sysclip.rs:29 SystemClipboard::has
    fn has(&mut self, mimes: &[&str]) -> bool {
        self.presence.any(mimes)
    }
}
