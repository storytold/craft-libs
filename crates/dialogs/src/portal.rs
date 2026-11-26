//! Save through the XDG desktop portal on Linux and the BSDs.
//!
//! The GTK file chooser behind the portal keeps the typed name when the user picks another file
//! type, and rfd 0.17 reads only the file name from the portal's answer, so `image.psd` with PNG
//! picked came back as `image.psd`. The answer does say which type was picked (`current_filter`),
//! so Save asks the portal itself and reports that type with the typed path; the service gives a
//! name the dialog left alone the type's extension ([`crate::service::with_chosen_type`]). rfd stays
//! the fallback (zenity, or no portal), and this can go once an rfd release reads the picked type
//! (PolyMeilex/rfd#341).
//!
//! A Save in flight can be stopped through its [`Cancel`] (its pick was forgotten): the wait for
//! the portal's answer races the cancel and the portal leaving the bus, and a cancel closes the
//! portal's request. How each answer reads is [`classify`]'s one rule.
//!
//! The dialog is attached to the window on X11. On Wayland it has no parent: naming the window to
//! the portal needs an xdg-foreign handle, which takes `unsafe` code the workspace forbids.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use raw_window_handle::RawWindowHandle;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::service::normal;
use crate::{FileDialog, Filter};

/// How soon an empty answer means the dialog never showed: a portal that fails answers at once
/// (#2999), and no user closes a file dialog this quickly.
pub(crate) const TOO_QUICK: Duration = Duration::from_millis(300);

/// The cause a pick answers when the system dialog closed at once, twice.
pub(crate) const CLOSED_AT_ONCE: &str = "the system file dialog closed at once";

/// The cause of a Save whose file type the portal did not report and the typed name does not
/// tell: no dialog can answer it, and a guessed type would write the wrong format.
const AMBIGUOUS: &str = "ambiguous file type";

/// The portal's bus name.
const PORTAL: &str = "org.freedesktop.portal.Desktop";

/// A Save dialog to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SaveRequest {
    /// The portal's parent window identifier (`x11:<xid>`), or empty.
    pub parent: String,
    /// The name the dialog shows: the stem of the suggested file (`test`), see
    /// [`crate::service::dialog_file_name`].
    pub name: Option<String>,
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
            folder: dialog.directory.clone(),
            filters: dialog.filters.clone(),
            current: current.filter(|_| !dialog.filters.is_empty()),
        }
    }
}

/// What the user chose: the path, as typed, and the label of the file type picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Saved {
    path: PathBuf,
    filter: Option<String>,
}

/// A flag that cancels a portal Save in flight. Clones share the flag: the dialog's owner keeps
/// one, the worker waiting for the portal another.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cancel(Arc<CancelState>);

#[derive(Debug, Default)]
struct CancelState {
    cancelled: AtomicBool,
    /// The task waiting in [`Cancel::cancelled`], woken by [`Cancel::cancel`].
    waiter: Mutex<Option<Waker>>,
}

impl Cancel {
    /// Asks the Save to stop.
    pub(crate) fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        if let Some(waker) = self.0.waiter.lock().unwrap_or_else(|e| e.into_inner()).take() {
            waker.wake();
        }
    }

    /// Whether [`cancel`](Self::cancel) was called.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Resolves once [`cancel`](Self::cancel) is called.
    fn cancelled(&self) -> impl Future<Output = ()> + '_ {
        std::future::poll_fn(move |cx| {
            if self.is_cancelled() {
                return Poll::Ready(());
            }
            *self.0.waiter.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
            // A cancel between the check and the store found no waker to wake: look again.
            if self.is_cancelled() { Poll::Ready(()) } else { Poll::Pending }
        })
    }
}

/// How a portal Save ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SaveOutcome {
    /// The user saved to this path, as typed, with the index of the file type picked (into the
    /// request's filters) when the portal reported one.
    Saved(PathBuf, Option<usize>),
    /// The user cancelled, or the Save was cancelled.
    Cancelled,
    /// The portal could not show the dialog: the caller shows rfd's.
    UseRfd(String),
    /// The file type picked is ambiguous: no dialog can answer.
    Unavailable(String),
}

/// What a portal response means for a Save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// The user saved: the path and the file type picked.
    Saved(Saved),
    /// The user cancelled, or the Save was cancelled.
    Cancelled,
    /// The answer came too quickly to be the user's: the dialog never showed, ask another way.
    Retry,
    /// No portal answered usefully; the text says why.
    Unavailable(String),
}

/// Reads one portal response into a [`Decision`]: the response `code`, how long the dialog was
/// open, whether a cancel was queued, whether the portal left the bus, and the `results`.
///
/// A queued cancel wins over any answer that arrived after it. Code 0 is a save with the first
/// `uris` entry; 1 is Cancel (KDE's Esc too). 2 is a dialog closed some other way: Esc in GNOME's
/// after a while, which is a cancel and no reason to show another dialog, or, within
/// [`TOO_QUICK`], a portal that failed to show it (#2999), which is a [`Decision::Retry`].
// pc: apps/photocraft/src/portal_save.rs:111 answer
pub(crate) fn classify(code: u32, elapsed: Duration, cancel_queued: bool, backend_gone: bool, results: &HashMap<String, OwnedValue>) -> Decision {
    if cancel_queued {
        return Decision::Cancelled;
    }
    if backend_gone {
        return Decision::Unavailable("the desktop portal left the session bus".into());
    }
    match code {
        0 => {}
        1 => return Decision::Cancelled,
        2 if elapsed < TOO_QUICK => return Decision::Retry,
        2 => return Decision::Cancelled,
        _ => return Decision::Unavailable(format!("the save dialog failed (portal response {code})")),
    }
    match saved(results) {
        Ok(saved) => Decision::Saved(saved),
        Err(why) => Decision::Unavailable(why),
    }
}

/// The file and type a code-0 response names.
fn saved(results: &HashMap<String, OwnedValue>) -> Result<Saved, String> {
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
    Ok(Saved { path, filter })
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

/// Shows the portal's Save dialog and waits for its answer, the portal leaving the bus or
/// `cancel`, whichever comes first. A cancel closes the portal's request. `Err` when the portal
/// couldn't be asked (no portal, no FileChooser).
// pc: apps/photocraft/src/portal_save.rs:310 save_file, :266 close_request, :333 NameOwnerChanged
fn save_file(conn: &zbus::blocking::Connection, req: &SaveRequest, cancel: &Cancel) -> Result<Decision, String> {
    use futures_lite::StreamExt;
    use futures_lite::future::{block_on, or};
    use zbus::blocking::Proxy;

    enum Event {
        Reply(Option<zbus::message::Message>),
        Gone,
        Cancelled,
    }

    let text = |e: zbus::Error| e.to_string();
    let unique = conn.unique_name().ok_or("Portal connection has no unique name")?.as_str().trim_start_matches(':').replace('.', "_");
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let token = format!("craft_dialogs_save_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");
    // Listen before asking, so a quick answer isn't missed: to the request's answer, and to the
    // portal leaving the bus, after which no answer comes.
    let request = Proxy::new(conn, PORTAL, path.as_str(), "org.freedesktop.portal.Request").map_err(text)?;
    let replies = block_on(request.inner().receive_signal("Response")).map_err(text)?;
    let bus = Proxy::new(conn, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").map_err(text)?;
    let mut owners = block_on(bus.inner().receive_signal_with_args("NameOwnerChanged", &[(0, PORTAL)])).map_err(text)?;
    let chooser = Proxy::new(conn, PORTAL, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser").map_err(text)?;
    let started = Instant::now();
    let handle: OwnedObjectPath = chooser.call("SaveFile", &(req.parent.as_str(), "Save As", options(req, &token))).map_err(text)?;
    // A portal older than 0.9 ignores `handle_token` and answers on a request path of its own,
    // which it returns: listen there instead, as the Request documentation asks.
    let (request, mut replies) = if handle.as_str() == path {
        (request, replies)
    } else {
        let request = Proxy::new(conn, PORTAL, handle.as_str(), "org.freedesktop.portal.Request").map_err(text)?;
        let replies = block_on(request.inner().receive_signal("Response")).map_err(text)?;
        (request, replies)
    };
    let event = block_on(or(
        async { Event::Reply(replies.next().await) },
        or(
            async {
                cancel.cancelled().await;
                Event::Cancelled
            },
            async {
                // NameOwnerChanged(name, old owner, new owner): no new owner is the portal gone.
                while let Some(change) = owners.next().await {
                    if change.body().deserialize::<(String, String, String)>().is_ok_and(|(_, _, new)| new.is_empty()) {
                        return Event::Gone;
                    }
                }
                Event::Gone
            },
        ),
    ));
    let elapsed = started.elapsed();
    let nothing = HashMap::new();
    // Without a response there is no code: the cancel and backend flags decide.
    Ok(match event {
        Event::Reply(Some(reply)) => {
            let (code, results): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(text)?;
            classify(code, elapsed, cancel.is_cancelled(), false, &results)
        }
        Event::Reply(None) | Event::Gone => classify(0, elapsed, cancel.is_cancelled(), true, &nothing),
        Event::Cancelled => {
            // The portal closes its dialog; a request it already answered has nothing to close.
            let _ = request.call_method("Close", &());
            classify(0, elapsed, true, false, &nothing)
        }
    })
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

/// Whether a Save answer with no file type picked leaves the format open: several types were
/// offered and the typed name's extension is none of theirs.
// pc: apps/wordcraft/src/portal_save.rs:446 is_ambiguous_filter
fn is_ambiguous(filters: &[Filter], path: &Path) -> bool {
    let listed = path
        .extension()
        .and_then(|e| e.to_str())
        .map(normal)
        .is_some_and(|ext| filters.iter().any(|f| f.extensions.iter().any(|e| normal(e) == ext)));
    filters.len() > 1 && !listed
}

/// Save on Linux and the BSDs: the portal's dialog, answering the typed path and the index of the
/// file type picked (into `req.filters`); the service settles the name from them. A portal that
/// can't be reached or can't show the dialog answers [`SaveOutcome::UseRfd`]; an answer whose file
/// type nothing tells answers [`SaveOutcome::Unavailable`], never a guess.
// pc: apps/photocraft/src/portal_save.rs:174 save_as, services.rs:244-280; apps/wordcraft/src/portal_save.rs:471 save_as
pub(crate) fn save_as(req: SaveRequest, cancel: &Cancel) -> SaveOutcome {
    let conn = match zbus::blocking::connection::Builder::session().and_then(|b| b.build()) {
        Ok(conn) => conn,
        Err(e) => return SaveOutcome::UseRfd(e.to_string()),
    };
    let saved = match save_file(&conn, &req, cancel) {
        Ok(Decision::Saved(saved)) => saved,
        Ok(Decision::Cancelled) => return SaveOutcome::Cancelled,
        Ok(Decision::Retry) => return SaveOutcome::UseRfd(CLOSED_AT_ONCE.into()),
        Ok(Decision::Unavailable(why)) | Err(why) => return SaveOutcome::UseRfd(why),
    };
    let picked = saved.filter.as_deref().and_then(|label| req.filters.iter().position(|f| f.name == label));
    // With one type offered the service takes that one; with several, the typed extension must name one.
    if picked.is_none() && is_ambiguous(&req.filters, &saved.path) {
        return SaveOutcome::Unavailable(AMBIGUOUS.into());
    }
    SaveOutcome::Saved(saved.path, picked)
}

#[cfg(test)]
pub(crate) mod tests;
