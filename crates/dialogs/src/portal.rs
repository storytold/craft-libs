//! Save through the XDG desktop portal on Linux.
//!
//! The GTK file chooser behind the portal keeps the typed name when the user picks another file
//! type, and rfd 0.17 reads only the file name from the portal's answer, so `image.psd` with PNG
//! picked came back as `image.psd`. The answer does say which type was picked (`current_filter`),
//! so Save asks the portal itself and gives a name the dialog left alone that type's extension
//! ([`with_chosen_type`]). rfd stays the fallback (zenity, or no portal), and this can go once an
//! rfd release reads the picked type (PolyMeilex/rfd#341).
//!
//! The dialog is attached to the window on X11. On Wayland it has no parent: naming the window to
//! the portal needs an xdg-foreign handle, which takes `unsafe` code the workspace forbids.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use raw_window_handle::RawWindowHandle;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::{FileDialog, Filter};

/// A Save dialog to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SaveRequest {
    /// The portal's parent window identifier (`x11:<xid>`), or empty.
    pub parent: String,
    /// The name the dialog shows: the stem of the suggested file (`test`), see
    /// [`crate::service::dialog_file_name`].
    pub name: Option<String>,
    /// The suggested file's name (`test.psd`): a name that comes back with its extension still
    /// takes the extension of the type picked ([`with_chosen_type`]).
    pub suggested: Option<String>,
    /// The folder the dialog opens in.
    pub folder: Option<PathBuf>,
    /// The file types offered.
    pub filters: Vec<Filter>,
    /// The filter the dialog starts on.
    pub current: Option<usize>,
}

impl SaveRequest {
    /// The portal request for a Save `dialog` attached to the window `parent` names. The dialog
    /// starts on the type that lists the suggested name's extension, else the first, so a name
    /// left alone keeps its extension: the app lists the type it suggests among its filters.
    // pc: apps/photocraft/src/services.rs:244-280 show_file_dialog (the Linux Save request)
    pub(crate) fn new(dialog: &FileDialog, parent: String) -> Self {
        let suggested = dialog.file_name.as_deref().and_then(|n| Path::new(n).file_name()).map(|n| n.to_string_lossy().into_owned());
        let ext = suggested.as_deref().and_then(|n| Path::new(n).extension()).map(|e| e.to_string_lossy().to_ascii_lowercase());
        let current = ext.and_then(|ext| dialog.filters.iter().position(|f| f.extensions.iter().any(|e| normal(e) == ext))).or(Some(0));
        Self {
            parent,
            name: dialog.file_name.as_deref().and_then(|n| crate::service::dialog_file_name(n, true)),
            suggested,
            folder: dialog.directory.clone(),
            filters: dialog.filters.clone(),
            current: current.filter(|_| !dialog.filters.is_empty()),
        }
    }
}

/// What the user chose: the path, as typed, and the label of the file type picked.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    path: PathBuf,
    filter: Option<String>,
}

/// An extension as compared: without the dot, in lower case.
fn normal(ext: &str) -> String {
    ext.trim_start_matches('.').to_ascii_lowercase()
}

/// The portal's parent window identifier for `window`: `x11:<xid>` on X11, empty elsewhere.
// pc: apps/photocraft/src/portal_save.rs:47 parent_id
pub(crate) fn parent_id(window: Option<RawWindowHandle>) -> String {
    match window {
        Some(RawWindowHandle::Xlib(h)) => format!("x11:{:x}", h.window),
        Some(RawWindowHandle::Xcb(h)) => format!("x11:{:x}", h.window.get()),
        _ => String::new(),
    }
}

/// Shows the portal's Save dialog and waits for it. `Ok(None)` is Cancel; `Err` means the portal
/// couldn't show it (no portal, no FileChooser), so the caller falls back to rfd.
// pc: apps/photocraft/src/portal_save.rs:58 save_file
fn save_file(conn: &zbus::blocking::Connection, req: &SaveRequest) -> Result<Option<Saved>, String> {
    use futures_lite::StreamExt;
    use zbus::blocking::Proxy;
    let unique = conn.unique_name().ok_or("Portal connection has no unique name")?.as_str().trim_start_matches(':').replace('.', "_");
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let token = format!("craft_dialogs_save_{}_{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");
    // Listen before asking, so a quick answer isn't missed.
    let request = Proxy::new(conn, "org.freedesktop.portal.Desktop", path.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    let mut replies = futures_lite::future::block_on(request.inner().receive_signal("Response")).map_err(|e| e.to_string())?;
    let chooser = Proxy::new(conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser")
        .map_err(|e| e.to_string())?;
    let handle: OwnedObjectPath = chooser.call("SaveFile", &(req.parent.as_str(), "Save As", options(req, &token))).map_err(|e| e.to_string())?;
    // A portal older than 0.9 ignores `handle_token` and answers on a request path of its own,
    // which it returns: listen there instead, as the Request documentation asks.
    let reply = if handle.as_str() == path {
        futures_lite::future::block_on(replies.next())
    } else {
        let request =
            Proxy::new(conn, "org.freedesktop.portal.Desktop", handle.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
        let mut replies = futures_lite::future::block_on(request.inner().receive_signal("Response")).map_err(|e| e.to_string())?;
        futures_lite::future::block_on(replies.next())
    }
    .ok_or("The save dialog closed without an answer")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(|e| e.to_string())?;
    answer(code, &results)
}

/// A filter as the portal takes it: its label and one `*.ext` glob per pattern (the case-insensitive
/// patterns [`Filter`] gives rfd).
fn wire(filter: &Filter) -> (String, Vec<(u32, String)>) {
    (filter.name.clone(), filter.patterns().iter().map(|p| (0u32, format!("*.{p}"))).collect())
}

/// The `SaveFile` options for `req`.
// pc: apps/photocraft/src/portal_save.rs:86 options
fn options<'a>(req: &'a SaveRequest, token: &'a str) -> HashMap<&'static str, Value<'a>> {
    let mut o: HashMap<&'static str, Value<'a>> = HashMap::from([("handle_token", Value::from(token)), ("modal", Value::from(true))]);
    if let Some(name) = &req.name {
        o.insert("current_name", Value::from(name.as_str()));
    }
    if let Some(folder) = &req.folder {
        // A byte string with its terminating NUL, as the portal specifies.
        use std::os::unix::ffi::OsStrExt;
        let mut bytes = folder.as_os_str().as_bytes().to_vec();
        bytes.push(0);
        o.insert("current_folder", Value::from(bytes));
    }
    if !req.filters.is_empty() {
        o.insert("filters", Value::from(req.filters.iter().map(wire).collect::<Vec<_>>()));
    }
    if let Some(f) = req.current.and_then(|i| req.filters.get(i)) {
        o.insert("current_filter", Value::from(wire(f)));
    }
    o
}

/// Reads the portal's `Response`: code 0 with the first `uris` entry. 1 is Cancel, and 2 a dialog
/// closed some other way, such as with Esc in GNOME's: no file either, and not a reason to show
/// rfd's dialog instead (it did, so Esc opened a second Save As).
// pc: apps/photocraft/src/portal_save.rs:111 answer
fn answer(code: u32, results: &HashMap<String, OwnedValue>) -> Result<Option<Saved>, String> {
    match code {
        0 => {}
        1 | 2 => return Ok(None),
        _ => return Err(format!("the save dialog failed (portal response {code})")),
    }
    let uris: Vec<String> = results
        .get("uris")
        .ok_or("no file in the portal's answer")?
        .try_clone()
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|e: zbus::zvariant::Error| e.to_string())?;
    let uri = uris.first().ok_or("no file in the portal's answer")?;
    let path = file_uri_path(uri).ok_or_else(|| format!("not a local file: {uri}"))?;
    let filter = results.get("current_filter").and_then(|v| {
        let (label, _globs): (String, Vec<(u32, String)>) = v.try_clone().ok()?.try_into().ok()?;
        Some(label)
    });
    Ok(Some(Saved { path, filter }))
}

/// `file:///home/me/A%20b.psd` → `/home/me/A b.psd`; `None` for other schemes.
// pc: apps/photocraft/src/portal_save.rs:134 file_uri_path
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Only local files: `file:///…` or `file://localhost/…`.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Which file type's extensions belong on the name the dialog returned: the one it reported, or,
/// when the dialog offered only one type, that one. It reports no pick then, and the name it was
/// given is the stem, so a bare `Swatches` has to come back as `Swatches.aco`. `None` when the
/// answer names no type this dialog offered.
// pc: apps/photocraft/src/portal_save.rs:163 chosen_type
fn chosen_type(filters: &[Filter], picked: Option<usize>) -> Option<usize> {
    match picked {
        Some(i) if i < filters.len() => Some(i),
        None if filters.len() == 1 => Some(0),
        _ => None,
    }
}

/// `path` with the extension of the file type picked in the Save dialog (`chosen`, its extensions
/// without the dot), where the dialog left the name alone: the format follows the extension, and
/// the GTK file chooser keeps the typed name when another type is picked. Only a name that still
/// has the extension the dialog opened with (`opened_with`, `Untitled-1.psd` with PNG picked) or
/// none gets the type's first extension, as PolyMeilex/rfd#341 does. An extension of the picked
/// type stays (`.psb` for PSD), and so does one that any of the request's `filters` lists
/// (`art.png` with the PSD type untouched): the user typed a type the app saves. Any other
/// extension (`scan.2024`) is part of the name, and the type's extension is added after it. With
/// no type picked the path is as typed. Extensions compare in any letter case.
// pc: apps/photocraft/src/services.rs:161 with_chosen_type
pub(crate) fn with_chosen_type(path: &Path, opened_with: Option<&str>, chosen: &[String], filters: &[Filter]) -> PathBuf {
    let Some(first) = chosen.first() else { return path.to_path_buf() };
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let opened_ext = opened_with.and_then(|n| Path::new(n).extension()).and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let first = normal(first);
    let lists = |ext: &str| filters.iter().any(|f| f.extensions.iter().any(|e| normal(e) == ext));
    match ext {
        Some(ext) if chosen.iter().any(|e| normal(e) == ext) => path.to_path_buf(),
        Some(ext) if opened_ext.as_ref() == Some(&ext) => path.with_extension(first),
        Some(ext) if lists(&ext) => path.to_path_buf(),
        _ => {
            let mut name = path.as_os_str().to_owned();
            name.push(".");
            name.push(first);
            PathBuf::from(name)
        }
    }
}

/// Save on Linux: the portal's dialog, with the name given the extension of the file type picked.
/// When that changes the name to one that already exists, the dialog opens again under the new
/// name, so the portal asks before replacing it (it asked only about the typed name). Connects to
/// the session bus; `Err` when the portal can't be reached, and the caller shows rfd's dialog.
// pc: apps/photocraft/src/portal_save.rs:174 save_as, services.rs:244-280
pub(crate) fn save_as(mut req: SaveRequest) -> Result<Option<PathBuf>, String> {
    let conn = zbus::blocking::connection::Builder::session().and_then(|b| b.build()).map_err(|e| e.to_string())?;
    loop {
        let Some(saved) = save_file(&conn, &req)? else { return Ok(None) };
        let picked = saved.filter.as_deref().and_then(|label| req.filters.iter().position(|f| f.name == label));
        // A typed extension is the user's own: `with_chosen_type` keeps one it recognises.
        let exts: &[String] = match chosen_type(&req.filters, picked) {
            Some(i) => &req.filters[i].extensions,
            None => &[],
        };
        let path = with_chosen_type(&saved.path, req.suggested.as_deref(), exts, &req.filters);
        if path == saved.path || !path.exists() {
            return Ok(Some(path));
        }
        req.name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        req.suggested = req.name.clone();
        req.folder = path.parent().map(Path::to_path_buf);
        req.current = picked;
    }
}

#[cfg(test)]
mod tests;
