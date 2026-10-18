//! The XDG desktop portal's print dialog (`org.freedesktop.portal.Print`): the print path for
//! installs where no CUPS server is reachable (a sandboxed one), where `lpstat`/`lp` are
//! missing too. The dialog is the portal backend's own, so the printer and the job's options
//! are the user's to choose there; this side hands over the print-ready PDF and reports it.

use super::{Error, Job};
use std::collections::HashMap;
use std::io::{Seek as _, Write as _};
use std::os::fd::AsFd as _;
use std::sync::OnceLock;
use std::time::Duration;

/// The desktop portal service, whichever backend provides it.
const PORTAL_SERVICE: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_INTERFACE: &str = "org.freedesktop.portal.Print";

/// A backend that has taken the call must not hold the worker forever.
const PORTAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether the print dialog can be opened: the session bus connects. The probe is asked once;
/// a missing bus (a sandbox without one) is a normal `false`, not an error.
pub fn print_dialog_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| session_bus().is_ok())
}

/// Hand the print-ready PDF to the portal's print dialog; the returned message is the receipt
/// of the handoff. The job goes through a private temporary file (0600, a random name, this
/// user's temporary folder) whose file descriptor the portal receives: the file is deleted as
/// soon as the portal has its own copy of the descriptor, and it reads the job through it when
/// the dialog closes. The printer and the job's options are chosen in the dialog, not here.
pub fn print(pdf: &[u8], job: &Job) -> Result<String, Error> {
    let connection = session_bus()?;
    let portal = zbus::blocking::Proxy::new(&connection, PORTAL_SERVICE, PORTAL_PATH, PORTAL_INTERFACE)
        .map_err(|e| Error::Spool(format!("the print portal could not be reached: {e}")))?;
    let mut file = tempfile::NamedTempFile::new().map_err(|e| Error::Spool(format!("the print job's file could not be created: {e}")))?;
    file.write_all(pdf)
        .and_then(|()| file.flush())
        // The portal reads through the descriptor from the start, and a descriptor shares the
        // offset with this file.
        .and_then(|()| file.rewind())
        .map_err(|e| Error::Spool(format!("the print job could not be written: {e}")))?;
    // `Print` takes the parent window (none here), the dialog title, the job's file descriptor
    // and the options; the dialog's own defaults apply. The reply is the request's object
    // path, on which the portal later reports the dialog's outcome.
    let options: HashMap<String, zbus::zvariant::Value> = HashMap::new();
    let _request: zbus::zvariant::OwnedObjectPath = portal
        .call("Print", &("", job.title.as_str(), zbus::zvariant::Fd::Borrowed(file.as_fd()), &options))
        .map_err(|e| Error::Spool(format!("the print dialog could not be opened: {e}")))?;
    Ok("the print dialog was opened; choose a printer there".into())
}

/// The user's session bus, which the portal lives on.
fn session_bus() -> Result<zbus::blocking::Connection, Error> {
    zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.method_timeout(PORTAL_TIMEOUT).build())
        .map_err(|e| Error::Spool(format!("the session bus could not be reached: {e}")))
}
