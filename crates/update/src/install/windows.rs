//! Windows installs: the release MSI through Windows Installer.

use crate::Reason;
use crate::protocol::CANCELLED;

use super::{
    App, INSTALLER, JobPhase, Path, PathBuf, Stop, cancelled, msi_command, msi_folder_argument, msi_log, process, reinstall_command_ui, turn,
};

fn msiexec() -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join(r"System32\msiexec.exe")
}

fn explain(code: Option<i32>) -> Result<(), Reason> {
    match code {
        Some(0 | 3010 | 1641) => Ok(()),
        Some(1602) => Err(Reason::data("cancelled")),
        Some(1618) => Err(Reason::new("another installation is in progress. Wait for it to finish, then try again.", &[])),
        Some(1925 | 1303) => Err(Reason::new("Windows didn't allow the install (administrator permission is needed)", &[])),
        Some(c) => Err(Reason::new("the Windows Installer stopped with error {c}", &[("c", &(c).to_string())])),
        None => Err(Reason::new("the Windows Installer was interrupted", &[])),
    }
}

/// Runs an MSI with a progress window and no restart. Windows asks for permission itself.
pub fn run_msi(msi: &Path, destination: Option<&Path>, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), String> {
    run_msi_reason(msi, destination, stop, phase).map_err(|reason| reason.to_string())
}

pub(crate) fn run_msi_reason(msi: &Path, destination: Option<&Path>, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), Reason> {
    in_turn(stop, phase, JobPhase::Installing, || {
        use std::os::windows::process::CommandExt;
        let mut command = msi_command(&msiexec(), msi)?;
        if let Some(dir) = destination {
            command.raw_arg(msi_folder_argument(dir)?);
        }
        let status = command.status().map_err(|e| Reason::new("couldn't start the Windows Installer ({e})", &[("e", &(e).to_string())]))?;
        explain(status.code()).map_err(|e| match status.code() {
            Some(1602) | None => e,
            _ => e.context("{reason}. Its log is at {path}", &[("path", &msi_log(msi).display().to_string())]),
        })
    })
}

/// Runs an app's own new MSI for a self-update, with the same progress window and no restart
/// as [`run_msi`], and waits for it. `context` keeps the installed copy's Windows Installer
/// context; `folder`, when given, is where it goes. A failure names the installer's log,
/// which is the only place 1603 is explained.
pub fn reinstall(msi: &Path, context: &[&str], folder: Option<&Path>) -> Result<(), String> {
    reinstall_reason(msi, context, folder).map_err(|reason| reason.to_string())
}

pub(crate) fn reinstall_reason(msi: &Path, context: &[&str], folder: Option<&Path>) -> Result<(), Reason> {
    reinstall_with_ui_reason(msi, context, folder, false)
}

/// [`reinstall`]; `quiet` hides the progress window of a per-user install.
pub(crate) fn reinstall_with_ui_reason(msi: &Path, context: &[&str], folder: Option<&Path>, quiet: bool) -> Result<(), Reason> {
    use std::os::windows::process::CommandExt;
    let mut command = reinstall_command_ui(&msiexec(), msi, context, quiet)?;
    if let Some(dir) = folder {
        command.raw_arg(msi_folder_argument(dir)?);
    }
    let status = command.status().map_err(|e| Reason::new("couldn't start the Windows Installer ({e})", &[("e", &(e).to_string())]))?;
    explain(status.code()).map_err(|e| match status.code() {
        Some(1602) | None => e,
        _ => e.context("{reason}. Its log is at {path}", &[("path", &msi_log(msi).display().to_string())]),
    })
}

/// Runs `work` once no other Windows Installer job of ours is running (a second one would
/// stop with error 1618). Cancelled while it waits, it never runs.
fn in_turn(stop: &Stop, phase: &mut dyn FnMut(JobPhase), working: JobPhase, work: impl FnOnce() -> Result<(), Reason>) -> Result<(), Reason> {
    INSTALLER.alone(&cancelled(stop), &mut turn(phase, working), work).unwrap_or_else(|| Err(CANCELLED.into()))
}

/// Removes one installed product by its code, with a progress window and no restart, and waits.
/// A per-user product asks for no permission. `log` receives the installer's verbose log.
pub fn remove_product(product_code: &str, log: &Path) -> Result<(), String> {
    remove_product_reason(product_code, log).map_err(|reason| reason.to_string())
}

pub(crate) fn remove_product_reason(product_code: &str, log: &Path) -> Result<(), Reason> {
    if !crate::msi_context::valid_product_code(product_code) {
        return Err(Reason::new("{product_code} isn't a Windows Installer product code", &[("product_code", product_code)]));
    }
    let status = process::command(msiexec())
        .args(["/x", product_code, "/passive", "/norestart", "/l*v"])
        .arg(log)
        .status()
        .map_err(|e| Reason::new("couldn't start the Windows Installer ({e})", &[("e", &(e).to_string())]))?;
    explain(status.code()).map_err(|e| e.context("{reason}. Its log is at {path}", &[("path", &log.display().to_string())]))
}

/// Uninstalls every MSI install of `app` registered with the Windows Installer, one at a time, with a
/// progress window and no restart. Fails when none is registered.
pub fn uninstall_msi(app: &App, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), String> {
    uninstall_msi_reason(app, stop, phase).map_err(|reason| reason.to_string())
}

pub(crate) fn uninstall_msi_reason(app: &App, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), Reason> {
    let installs = crate::detect::windows::msi_installs(app);
    if installs.is_empty() {
        return Err(Reason::new("{arg0} isn't registered with the Windows Installer", &[("arg0", app.name)]));
    }
    in_turn(stop, phase, JobPhase::Removing, || {
        for m in installs {
            let status = process::command(msiexec())
                .args(["/x", &m.product_code, "/passive", "/norestart"])
                .status()
                .map_err(|e| Reason::new("couldn't start the Windows Installer ({e})", &[("e", &(e).to_string())]))?;
            explain(status.code())?;
        }
        Ok(())
    })
}
