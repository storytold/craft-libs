//! The X server's settings manager and window manager, on an X11 session.
//!
//! - GTK's `Gtk/DecorationLayout` from the settings manager, by the XSETTINGS specification (freedesktop.org, version
//!   0.5): the manager owns the `_XSETTINGS_S<screen>` selection, its window holds every setting in the binary
//!   `_XSETTINGS_SETTINGS` property, a change of the property is a change of the settings, and a new manager announces
//!   itself with a `MANAGER` client message on the root window. The selection's owner is looked up with the server
//!   grabbed, as the specification asks, so a manager that starts or ends meanwhile is not missed. When the manager's
//!   window goes away its settings go too, until a new manager takes the selection.
//! - The window manager's name, by the Extended Window Manager Hints: the root window's `_NET_SUPPORTING_WM_CHECK`
//!   names a window of the window manager's, which names itself in the same property (else the root's is stale) and
//!   carries the window manager's `_NET_WM_NAME`.
//!
//! The settings manager publishes GTK's settings, which desktops make from their own: it is a compatibility source,
//! after the window manager's own setting. A Flatpak with X11 access reads it like any client.

use super::super::{LayoutSource, RawLayout, Shared};
use craft_host::caption_layout::{LayoutReading, MAX_LAYOUT_BYTES};
use std::os::fd::AsFd;
use std::time::Duration;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, Window};
use x11rb::rust_connection::RustConnection;

/// How long connecting to the X server may take.
const CONNECT: Duration = Duration::from_millis(500);
/// The most of the settings property that is read, in 32-bit units: a few kilobytes is usual.
const SETTINGS_LONGS: u32 = 64 * 1024 / 4;
const SETTING: &[u8] = b"Gtk/DecorationLayout";
const SOURCE: LayoutSource = LayoutSource::XSettings;

/// A connection to the session's X server, with the atoms the sources use.
pub(in crate::appearance) struct X11 {
    conn: RustConnection,
    root: Window,
    /// `_XSETTINGS_S<screen>`, `_XSETTINGS_SETTINGS`, `MANAGER`, `_NET_SUPPORTING_WM_CHECK`, `_NET_WM_NAME`, `UTF8_STRING`.
    selection: u32,
    settings: u32,
    manager: u32,
    check: u32,
    wm_name: u32,
    utf8: u32,
}

impl X11 {
    /// Connects to `$DISPLAY`, with the running window manager's EWMH name. Both are asked on a short-lived thread and
    /// given up after `CONNECT`, so a stalled server can't hold the worker; that thread then ends when the server answers
    /// or the process exits.
    pub(super) async fn connect() -> Option<(X11, Option<String>)> {
        let (send, receive) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("desktop-x11-connect".into())
            .spawn(move || {
                let _ = send.send_blocking(Self::open().map(|x11| {
                    let wm = x11.window_manager();
                    (x11, wm)
                }));
            })
            .ok()?;
        futures_lite::future::race(async { receive.recv().await.ok().flatten() }, async {
            async_io::Timer::after(CONNECT).await;
            None
        })
        .await
    }

    fn open() -> Option<X11> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let names = [
            format!("_XSETTINGS_S{screen}"),
            "_XSETTINGS_SETTINGS".into(),
            "MANAGER".into(),
            "_NET_SUPPORTING_WM_CHECK".into(),
            "_NET_WM_NAME".into(),
            "UTF8_STRING".into(),
        ];
        let cookies: Vec<_> = names.iter().map(|name| conn.intern_atom(false, name.as_bytes())).collect::<Result<_, _>>().ok()?;
        let atoms: Vec<u32> = cookies.into_iter().map(|cookie| cookie.reply().map(|reply| reply.atom)).collect::<Result<_, _>>().ok()?;
        let [selection, settings, manager, check, wm_name, utf8] = atoms[..] else { return None };
        Some(X11 { conn, root, selection, settings, manager, check, wm_name, utf8 })
    }

    /// The running window manager's EWMH name; `None` without a conforming window manager.
    fn window_manager(&self) -> Option<String> {
        let window = |of: Window| -> Option<Window> {
            let reply = self.conn.get_property(false, of, self.check, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
            reply.value32()?.next()
        };
        let child = window(self.root)?;
        if window(child)? != child {
            return None;
        }
        let reply = self.conn.get_property(false, child, self.wm_name, self.utf8, 0, 256).ok()?.reply().ok()?;
        let name = String::from_utf8(reply.value).ok()?;
        Some(name.trim_end_matches('\0').to_owned()).filter(|name| !name.is_empty())
    }

    /// The settings manager's window, now: looked up with the server grabbed and watched for property changes and its
    /// destruction before the grab ends. `x11rb::NONE` when no manager runs.
    fn take_owner(&self) -> Option<Window> {
        let watch = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY);
        self.conn.grab_server().ok()?;
        let owner =
            self.conn.get_selection_owner(self.selection).ok().and_then(|cookie| cookie.reply().ok()).map_or(x11rb::NONE, |reply| reply.owner);
        if owner != x11rb::NONE {
            // A window that went away meanwhile answers with an error event, which `watch` ignores.
            let _ = self.conn.change_window_attributes(owner, &watch);
        }
        self.conn.ungrab_server().ok()?;
        self.conn.flush().ok()?;
        Some(owner)
    }

    /// Publishes the layout `owner`'s settings hold: none without a manager, a property that is missing, too big or
    /// malformed, or no `Gtk/DecorationLayout` string in it.
    fn publish(&self, shared: &Shared, owner: Window) {
        let layout = (owner != x11rb::NONE)
            .then(|| self.conn.get_property(false, owner, self.settings, AtomEnum::ANY, 0, SETTINGS_LONGS).ok()?.reply().ok())
            .flatten()
            .filter(|reply| reply.type_ == self.settings && reply.format == 8 && reply.bytes_after == 0)
            .and_then(|reply| decoration_layout(&reply.value).ok().flatten());
        let reading = layout.map(|layout| LayoutReading::new(SOURCE, RawLayout::Names(layout), "X settings manager, Gtk/DecorationLayout"));
        shared.publish_caption(SOURCE, reading);
    }
}

/// Follows the settings manager's layout until the connection ends or the watcher is dropped.
pub(super) async fn watch(shared: &Shared, x11: Option<X11>) {
    let Some(x11) = x11 else { return };
    let Some(readable) = x11.conn.stream().as_fd().try_clone_to_owned().ok().and_then(|fd| async_io::Async::new(fd).ok()) else { return };
    // The root window gets the `MANAGER` message of a manager that starts later.
    let root = ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY);
    if x11.conn.change_window_attributes(x11.root, &root).is_err() {
        return;
    }
    let Some(mut owner) = x11.take_owner() else { return };
    x11.publish(shared, owner);
    loop {
        // What x11rb has already read, then what the server sends.
        loop {
            match x11.conn.poll_for_event() {
                Ok(Some(event)) => owner = handle(&x11, shared, owner, event),
                Ok(None) => break,
                Err(_) => {
                    shared.publish_caption(SOURCE, None);
                    return;
                }
            }
        }
        if readable.readable().await.is_err() {
            shared.publish_caption(SOURCE, None);
            return;
        }
    }
}

/// One event: the manager's settings changed, its window went away (its settings with it), or a new manager took the
/// selection. The manager's window after it.
fn handle(x11: &X11, shared: &Shared, owner: Window, event: Event) -> Window {
    match event {
        Event::PropertyNotify(event) if event.window == owner && event.atom == x11.settings => {
            x11.publish(shared, owner);
            owner
        }
        Event::DestroyNotify(event) if event.window == owner && owner != x11rb::NONE => {
            shared.publish_caption(SOURCE, None);
            let owner = x11.take_owner().unwrap_or(x11rb::NONE);
            x11.publish(shared, owner);
            owner
        }
        Event::ClientMessage(event) if event.type_ == x11.manager && event.data.as_data32()[1] == x11.selection => {
            let owner = x11.take_owner().unwrap_or(x11rb::NONE);
            x11.publish(shared, owner);
            owner
        }
        _ => owner,
    }
}

/// The settings property was not one: truncated, with a length past its end, an unknown type or byte order.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Malformed;

/// The `Gtk/DecorationLayout` string of an `_XSETTINGS_SETTINGS` property's bytes, by XSETTINGS 0.5's format: a byte
/// order, the serial, the number of settings, then each setting's type, name (padded to four bytes), last-change serial
/// and value (an integer, a padded string or a colour). The whole property is read before it counts. The byte order is
/// the manager's, written as X11 does (`l` or 0 little-endian, `B` or 1 big-endian). `Ok(None)` when it has no such
/// string, or one longer than a layout holds.
pub(super) fn decoration_layout(bytes: &[u8]) -> Result<Option<String>, Malformed> {
    let little = match bytes.first() {
        Some(0 | b'l') => true,
        Some(1 | b'B') => false,
        _ => return Err(Malformed),
    };
    let take = |at: &mut usize, len: usize| -> Result<&[u8], Malformed> {
        let end = at.checked_add(len).filter(|end| *end <= bytes.len()).ok_or(Malformed)?;
        let taken = &bytes[*at..end];
        *at = end;
        Ok(taken)
    };
    let u32_at = |at: &mut usize| -> Result<u32, Malformed> {
        let raw: [u8; 4] = take(at, 4)?.try_into().map_err(|_| Malformed)?;
        Ok(if little { u32::from_le_bytes(raw) } else { u32::from_be_bytes(raw) })
    };
    let padded = |len: usize| len.checked_add(3).map(|len| len & !3).ok_or(Malformed);
    let mut at = 4;
    let _serial = u32_at(&mut at)?;
    let count = u32_at(&mut at)? as usize;
    // Each setting takes at least twelve bytes, so a count beyond that is no property's.
    if count > (bytes.len() - at) / 12 {
        return Err(Malformed);
    }
    let mut found = None;
    for _ in 0..count {
        let header = take(&mut at, 4)?;
        let kind = header[0];
        let name_len = usize::from(if little { u16::from_le_bytes([header[2], header[3]]) } else { u16::from_be_bytes([header[2], header[3]]) });
        let name = &take(&mut at, padded(name_len)?)?[..name_len];
        let _last_change = u32_at(&mut at)?;
        match kind {
            0 => {
                take(&mut at, 4)?;
            }
            1 => {
                let len = u32_at(&mut at)? as usize;
                let value = &take(&mut at, padded(len)?)?[..len];
                if name == SETTING {
                    let text = std::str::from_utf8(value).map_err(|_| Malformed)?;
                    found = (text.len() <= MAX_LAYOUT_BYTES).then(|| text.trim().to_owned());
                }
            }
            2 => {
                take(&mut at, 8)?;
            }
            _ => return Err(Malformed),
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::{Malformed, decoration_layout};

    /// One setting of an `_XSETTINGS_SETTINGS` property.
    enum Setting<'a> {
        Integer(&'a str, i32),
        Text(&'a str, &'a str),
        Colour(&'a str, [u16; 4]),
    }

    /// A property as a settings manager writes it, in its byte order: `little` writes `l`, else `B`.
    fn property(little: bool, settings: &[Setting<'_>]) -> Vec<u8> {
        let u32s = |value: u32| if little { value.to_le_bytes() } else { value.to_be_bytes() };
        let u16s = |value: u16| if little { value.to_le_bytes() } else { value.to_be_bytes() };
        let pad = |bytes: &mut Vec<u8>| bytes.resize(bytes.len().next_multiple_of(4), 0);
        let mut bytes = vec![if little { b'l' } else { b'B' }, 0, 0, 0];
        bytes.extend(u32s(7));
        bytes.extend(u32s(u32::try_from(settings.len()).unwrap_or(u32::MAX)));
        for setting in settings {
            let (kind, name) = match setting {
                Setting::Integer(name, _) => (0, name),
                Setting::Text(name, _) => (1, name),
                Setting::Colour(name, _) => (2, name),
            };
            bytes.extend([kind, 0]);
            bytes.extend(u16s(u16::try_from(name.len()).unwrap_or(u16::MAX)));
            bytes.extend(name.as_bytes());
            pad(&mut bytes);
            bytes.extend(u32s(3));
            match setting {
                Setting::Integer(_, value) => bytes.extend(u32s(*value as u32)),
                Setting::Text(_, value) => {
                    bytes.extend(u32s(u32::try_from(value.len()).unwrap_or(u32::MAX)));
                    bytes.extend(value.as_bytes());
                    pad(&mut bytes);
                }
                Setting::Colour(_, rgba) => rgba.iter().for_each(|channel| bytes.extend(u16s(*channel))),
            }
        }
        bytes
    }

    fn desktop(layout: &str) -> Vec<Setting<'_>> {
        vec![
            Setting::Integer("Net/DoubleClickTime", 400),
            Setting::Text("Gtk/DecorationLayout", layout),
            Setting::Colour("Gtk/Color", [1, 2, 3, 65535]),
            Setting::Text("Net/ThemeName", "Adwaita"),
        ]
    }

    // GTK's layout is found among the other settings in either byte order, after integers and colours, and is gone
    // when the manager no longer has it.
    #[test]
    fn the_decoration_layout_is_read_in_either_byte_order() {
        for little in [true, false] {
            assert_eq!(
                decoration_layout(&property(little, &desktop("menu:minimize,maximize,close"))),
                Ok(Some("menu:minimize,maximize,close".into()))
            );
            assert_eq!(decoration_layout(&property(little, &desktop(""))), Ok(Some(String::new())), "a layout naming no button is one");
            let removed = [Setting::Integer("Net/DoubleClickTime", 400), Setting::Text("Net/ThemeName", "Adwaita")];
            assert_eq!(decoration_layout(&property(little, &removed)), Ok(None), "no layout setting");
        }
        let mut zero_order = property(true, &desktop("close:"));
        zero_order[0] = 0;
        assert_eq!(decoration_layout(&zero_order), Ok(Some("close:".into())), "X11's LSBFirst written as 0");
    }

    // A property cut short anywhere, a length past its end, a count no property could hold, an unknown type or byte
    // order: malformed, never a guess and never a panic.
    #[test]
    fn a_truncated_or_inflated_property_is_malformed() {
        let whole = property(true, &desktop("close:minimize"));
        for end in 0..whole.len() {
            assert_eq!(decoration_layout(&whole[..end]), Err(Malformed), "cut at {end}");
        }
        let mut huge_name = whole.clone();
        huge_name[14..16].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(decoration_layout(&huge_name), Err(Malformed));
        let mut huge_count = whole.clone();
        huge_count[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decoration_layout(&huge_count), Err(Malformed));
        let mut huge_value = property(true, &[Setting::Text("Gtk/DecorationLayout", "close:")]);
        let value_len = 12 + 4 + "Gtk/DecorationLayout".len() + 4;
        huge_value[value_len..value_len + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decoration_layout(&huge_value), Err(Malformed));
        let mut unknown = whole.clone();
        unknown[12] = 9;
        assert_eq!(decoration_layout(&unknown), Err(Malformed));
        let mut order = whole;
        order[0] = b'x';
        assert_eq!(decoration_layout(&order), Err(Malformed));
        let long = "x".repeat(5000);
        assert_eq!(decoration_layout(&property(false, &desktop(&long))), Ok(None), "longer than a layout");
    }
}
