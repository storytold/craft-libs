//! The offer on Linux and the BSDs.
//!
//! Wayland when `WAYLAND_DISPLAY` is set and the compositor has a data-control protocol, else
//! X11: the order arboard picks its own backend in. On Wayland a write is one offer from
//! `wl_clipboard_rs::copy` with the owner marker in it; on X11 arboard owns the selection, and
//! ownership is the selection owner being the window arboard set.

use std::io::Read;

use wl_clipboard_rs::copy::{self, MimeSource, Options, Source};
use wl_clipboard_rs::paste::{self, ClipboardType, Seat, get_contents, get_mime_types_ordered};

use super::{Plan, Primary, arboard_write};
use crate::clipboard::types::{BITMAP, ClipboardError, HTML, MATHML, PNG, RGBA8, RTF, TEXT, URI_LIST};

/// The Wayland text type the copy's text goes under; wl-clipboard-rs serves the plain-text
/// aliases from the first text type offered.
const UTF8_TEXT: &str = "text/plain;charset=utf-8";
/// The MIME type of the owner marker in a Wayland offer.
const OWNER_MIME: &str = "application/x-craft-clipboard-owner";
/// RTF types a clipboard may offer, best first.
const RTF_MIMES: [&str; 3] = ["text/rtf", "application/rtf", "text/richtext"];
/// Most formats looked at, so a hostile owner's list can't make the app allocate without end.
const MAX_TYPES: usize = 128;

/// How this process's last write can be recognised later.
#[derive(Clone, Copy)]
enum Mark {
    /// Nothing this process wrote is on offer.
    Nothing,
    /// A write that carries no marker and recorded no selection owner, so ownership can't be told.
    Unmarked,
    /// An X11 write; the window that owned the selection after it.
    X11(u32),
}

/// The cached X11 connection for `TARGETS` and the selection owner, and how the last write
/// can be recognised.
pub(super) struct State {
    x11: Option<x11::Targets>,
    mark: Mark,
}

impl State {
    pub(super) fn new() -> Self {
        Self { x11: None, mark: Mark::Nothing }
    }

    /// Nothing this process wrote is on offer any more.
    pub(super) fn forget(&mut self) {
        self.mark = Mark::Nothing;
    }

    fn targets(&mut self) -> Option<&x11::Targets> {
        if self.x11.is_none() {
            self.x11 = x11::Targets::connect();
        }
        self.x11.as_ref()
    }
}

/// Whether the target or MIME names in `offered` contain a format that can be read as `mime`.
/// Text includes the X11 text targets; a bitmap is `image/png`, the only bitmap arboard reads
/// on Linux.
pub(super) fn offered_has(offered: &[String], mime: &str) -> bool {
    let any = |names: &[&str]| offered.iter().any(|offer| names.iter().any(|name| offer.eq_ignore_ascii_case(name)));
    match mime {
        TEXT => any(&["text/plain", UTF8_TEXT, "UTF8_STRING", "STRING", "TEXT"]),
        HTML => any(&[HTML]),
        RTF => rtf_target(offered).is_some(),
        MATHML => any(&[MATHML]),
        BITMAP | RGBA8 => any(&[PNG]),
        URI_LIST => any(&[URI_LIST]),
        _ => false,
    }
}

/// The target that carries RTF among `offered`.
pub(super) fn rtf_target(offered: &[String]) -> Option<&'static str> {
    RTF_MIMES.into_iter().find(|mime| offered.iter().any(|offer| offer == mime))
}

/// Whether a read of `len` bytes with `remaining` left under `cap` is complete.
pub(super) fn complete(len: usize, remaining: usize, cap: usize) -> bool {
    remaining == 0 && len <= cap
}

/// The Wayland offer's MIME types, best first; `None` outside Wayland or without a data-control
/// protocol, where X11 answers.
fn wayland_types() -> Option<Vec<String>> {
    std::env::var_os("WAYLAND_DISPLAY")?;
    match get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified) {
        Ok(mut types) => {
            types.truncate(MAX_TYPES);
            Some(types)
        }
        // Nothing is copied.
        Err(paste::Error::NoSeats | paste::Error::ClipboardEmpty | paste::Error::NoMimeType) => Some(Vec::new()),
        Err(error) => {
            log::debug!("no Wayland data control for the clipboard, using X11: {error}");
            None
        }
    }
}

/// One Wayland MIME type's bytes, at most `limit` of them.
fn wayland_read(mime: &str, limit: u64) -> Result<Option<Vec<u8>>, ClipboardError> {
    let (pipe, _) = match get_contents(ClipboardType::Regular, Seat::Unspecified, paste::MimeType::Specific(mime)) {
        Ok(contents) => contents,
        Err(paste::Error::NoSeats | paste::Error::ClipboardEmpty | paste::Error::NoMimeType) => return Ok(None),
        Err(error) => return Err(ClipboardError::Other(error.to_string())),
    };
    let mut out = Vec::new();
    pipe.take(limit.saturating_add(1)).read_to_end(&mut out).map_err(|error| ClipboardError::Other(error.to_string()))?;
    if out.len() as u64 > limit {
        return Err(ClipboardError::TooLarge { limit });
    }
    Ok(Some(out))
}

/// The plan as one Wayland offer with the owner marker; `None` when the plan needs arboard (a
/// bitmap given as pixels only: this crate has no PNG encoder, arboard has) or offers nothing.
fn wayland_write(plan: &Plan<'_>, owner: &[u8]) -> Result<Option<usize>, ClipboardError> {
    let text = |part: &Option<super::Part<'_>>, mime: &'static str| part.as_ref().map(|part| (part.index, mime, part.text.as_bytes().to_vec()));
    let parts: Vec<(usize, &str, Vec<u8>)> = match plan.primary() {
        Some(Primary::Picture) => {
            let Some((index, png)) = plan.picture.as_ref().and_then(|picture| Some((picture.index, picture.png?))) else { return Ok(None) };
            [Some((index, PNG, png.to_vec())), text(&plan.html, HTML), text(&plan.text, UTF8_TEXT)].into_iter().flatten().collect()
        }
        // arboard offers a file list with the names file managers read (`text/uri-list` and GNOME's own).
        Some(Primary::Files) => return Ok(None),
        // The text first: the plain-text aliases serve the first text type offered.
        _ => [text(&plan.text, UTF8_TEXT), text(&plan.html, HTML), text(&plan.rtf, RTF), text(&plan.mathml, MATHML)].into_iter().flatten().collect(),
    };
    let Some(first) = parts.iter().map(|&(index, _, _)| index).min() else { return Ok(None) };
    let source = |mime: &str, data: Vec<u8>| MimeSource { source: Source::Bytes(data.into()), mime_type: copy::MimeType::Specific(mime.into()) };
    let mut sources: Vec<MimeSource> = parts.into_iter().map(|(_, mime, data)| source(mime, data)).collect();
    sources.push(source(OWNER_MIME, owner.to_vec()));
    Options::new().copy_multi(sources).map_err(|error| ClipboardError::Other(error.to_string()))?;
    Ok(Some(first))
}

pub(super) fn write(handle: &mut arboard::Clipboard, state: &mut State, plan: &Plan<'_>, owner: &[u8]) -> Result<usize, ClipboardError> {
    let wayland = wayland_types().is_some();
    if wayland && let Some(index) = wayland_write(plan, owner)? {
        state.forget();
        return Ok(index);
    }
    let (index, _) = arboard_write(handle, plan)?;
    // On Wayland arboard's offer carries no marker, and on X11 the owner may be unreadable:
    // `owns` can't tell then, and the caller compares content.
    let recorded = if wayland { None } else { state.targets().and_then(x11::Targets::owner) };
    state.mark = recorded.map_or(Mark::Unmarked, Mark::X11);
    Ok(index)
}

pub(super) fn rtf(state: &mut State, limit: u64) -> Result<Option<Vec<u8>>, ClipboardError> {
    if let Some(types) = wayland_types() {
        return match rtf_target(&types) {
            Some(mime) => wayland_read(mime, limit),
            None => Ok(None),
        };
    }
    let offered = state.targets().map(x11::Targets::offered).unwrap_or_default();
    match rtf_target(&offered) {
        Some(target) => x11::read(target, limit),
        None => Ok(None),
    }
}

pub(super) fn formats(state: &mut State) -> Vec<String> {
    if let Some(types) = wayland_types() {
        return types;
    }
    state.targets().map(x11::Targets::offered).unwrap_or_default()
}

pub(super) fn owns(state: &mut State, owner: &[u8]) -> Option<bool> {
    if let Some(types) = wayland_types() {
        if matches!(state.mark, Mark::Unmarked) {
            return None;
        }
        if !types.iter().any(|mime| mime == OWNER_MIME) {
            return Some(false);
        }
        return match wayland_read(OWNER_MIME, owner.len() as u64) {
            Ok(marker) => Some(marker.is_some_and(|marker| marker == owner)),
            // A marker longer than ours is another process's.
            Err(ClipboardError::TooLarge { .. }) => Some(false),
            Err(_) => None,
        };
    }
    let recorded = match state.mark {
        Mark::Nothing => return Some(false),
        Mark::Unmarked => return None,
        Mark::X11(recorded) => recorded,
    };
    let current = state.targets()?.owner();
    Some(current == Some(recorded))
}

/// The X11 CLIPBOARD selection: its owner's `TARGETS` on a cached connection, and its content
/// read into a property of a hidden window, neither of which arboard offers.
mod x11 {
    use std::time::{Duration, Instant};

    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, GetPropertyReply, Window, WindowClass};
    use x11rb::rust_connection::RustConnection;
    use x11rb::{COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE};

    use crate::clipboard::types::ClipboardError;

    /// How long a `TARGETS` request waits for the owner. The probe's worker waits, not the UI.
    const ANSWER_WAIT: Duration = Duration::from_millis(250);
    /// How long a content read waits for the selection's owner, in all.
    const READ_WAIT: Duration = Duration::from_secs(1);
    /// Most targets looked at: a longer list is cut short, not refused.
    const MAX_TARGETS: u32 = super::MAX_TYPES as u32;

    fn atom(conn: &RustConnection, name: &[u8]) -> Option<Atom> {
        conn.intern_atom(false, name).ok()?.reply().ok().map(|reply| reply.atom)
    }

    /// A connection and a window that receive the clipboard owner's `TARGETS` answer.
    pub(super) struct Targets {
        conn: RustConnection,
        window: Window,
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
            let (clipboard, targets, property) = (atom(&conn, b"CLIPBOARD")?, atom(&conn, b"TARGETS")?, atom(&conn, b"CRAFT_CLIPBOARD_TARGETS")?);
            Some(Self { conn, window, clipboard, targets, property })
        }

        /// The target names the CLIPBOARD owner offers; none when nobody owns it or answers.
        pub(super) fn offered(&self) -> Vec<String> {
            self.ask().unwrap_or_default()
        }

        /// The window that owns the CLIPBOARD selection; `None` when nobody does.
        pub(super) fn owner(&self) -> Option<Window> {
            let reply = self.conn.get_selection_owner(self.clipboard).ok()?.reply().ok()?;
            (reply.owner != NONE).then_some(reply.owner)
        }

        fn ask(&self) -> Option<Vec<String>> {
            self.conn.delete_property(self.window, self.property).ok()?;
            self.conn.convert_selection(self.window, self.clipboard, self.targets, self.property, CURRENT_TIME).ok()?;
            self.conn.flush().ok()?;
            let end = Instant::now() + ANSWER_WAIT;
            while Instant::now() < end {
                match self.conn.poll_for_event().ok()? {
                    Some(Event::SelectionNotify(event)) if event.requestor == self.window && event.selection == self.clipboard => {
                        if event.property == NONE {
                            return None;
                        }
                        let reply = self.conn.get_property(true, self.window, self.property, AtomEnum::ATOM, 0, MAX_TARGETS).ok()?.reply().ok()?;
                        // The name requests go out together; their answers are collected after.
                        let cookies: Vec<_> =
                            reply.value32()?.take(MAX_TARGETS as usize).filter_map(|atom| self.conn.get_atom_name(atom).ok()).collect();
                        return Some(
                            cookies
                                .into_iter()
                                .filter_map(|cookie| cookie.reply().ok())
                                .filter_map(|name| String::from_utf8(name.name).ok())
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

    /// A hidden window the selection's owner writes a conversion into.
    struct Reader {
        conn: RustConnection,
        window: Window,
        clipboard: Atom,
        property: Atom,
        deadline: Instant,
    }

    impl Reader {
        fn new() -> Option<Self> {
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
                WindowClass::INPUT_ONLY,
                COPY_FROM_PARENT,
                &CreateWindowAux::new(),
            )
            .ok()?;
            let (clipboard, property) = (atom(&conn, b"CLIPBOARD")?, atom(&conn, b"CRAFT_CLIPBOARD_PASTE")?);
            Some(Self { conn, window, clipboard, property, deadline: Instant::now() + READ_WAIT })
        }

        /// The selection converted to `target`, read as at most one unit past `cap` bytes so a
        /// larger value shows in its length or `bytes_after`; `None` when the owner refuses or
        /// doesn't answer in time.
        fn convert(&self, target: Atom, cap: usize) -> Option<GetPropertyReply> {
            self.conn.convert_selection(self.window, self.clipboard, target, self.property, CURRENT_TIME).ok()?;
            self.conn.flush().ok()?;
            loop {
                // Checked every time round, so a stream of other events can't keep us here.
                if Instant::now() >= self.deadline {
                    return None;
                }
                match self.conn.poll_for_event().ok()? {
                    Some(Event::SelectionNotify(event)) if event.requestor == self.window => {
                        if event.property == NONE {
                            return None;
                        }
                        break;
                    }
                    Some(_) => {}
                    None => std::thread::sleep(Duration::from_millis(5)),
                }
            }
            // In 32-bit units.
            let units = u32::try_from(cap / 4 + 1).unwrap_or(u32::MAX);
            self.conn.get_property(true, self.window, self.property, AtomEnum::ANY, 0, units).ok()?.reply().ok()
        }
    }

    impl Drop for Reader {
        fn drop(&mut self) {
            let _ = self.conn.destroy_window(self.window);
            let _ = self.conn.flush();
        }
    }

    /// The CLIPBOARD selection as `target`, at most `limit` bytes. Copies too large to send at
    /// once arrive in pieces (INCR), which aren't read: nothing then.
    pub(super) fn read(target: &str, limit: u64) -> Result<Option<Vec<u8>>, ClipboardError> {
        let cap = usize::try_from(limit).unwrap_or(usize::MAX);
        let Some(reader) = Reader::new() else { return Ok(None) };
        let (Some(target), Some(incr)) = (atom(&reader.conn, target.as_bytes()), atom(&reader.conn, b"INCR")) else { return Ok(None) };
        let Some(reply) = reader.convert(target, cap) else { return Ok(None) };
        if reply.type_ == incr {
            log::debug!("the clipboard owner sends its content in pieces (INCR), which isn't read");
            return Ok(None);
        }
        if !super::complete(reply.value.len(), usize::try_from(reply.bytes_after).unwrap_or(usize::MAX), cap) {
            return Err(ClipboardError::TooLarge { limit });
        }
        Ok(Some(reply.value))
    }
}
