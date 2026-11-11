//! Windows installs: the release MSI through Windows Installer.

use crate::protocol::CANCELLED;

use super::{App, INSTALLER, JobPhase, Path, PathBuf, Stop, cancelled, msi_command, msi_folder_argument, msi_log, process, reinstall_command, turn};

fn msiexec() -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join(r"System32\msiexec.exe")
}

fn explain(code: Option<i32>) -> Result<(), String> {
    match code {
        Some(0 | 3010 | 1641) => Ok(()),
        Some(1602) => Err("cancelled".into()),
        Some(1618) => Err("another installation is in progress. Wait for it to finish, then try again.".into()),
        Some(1925 | 1303) => Err("Windows didn't allow the install (administrator permission is needed)".into()),
        Some(c) => Err(format!("the Windows Installer stopped with error {c}")),
        None => Err("the Windows Installer was interrupted".into()),
    }
}

/// Runs an MSI with a progress window and no restart. Windows asks for permission itself.
pub fn run_msi(msi: &Path, destination: Option<&Path>, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), String> {
    in_turn(stop, phase, JobPhase::Installing, || {
        use std::os::windows::process::CommandExt;
        let mut command = msi_command(&msiexec(), msi)?;
        if let Some(dir) = destination {
            command.raw_arg(msi_folder_argument(dir)?);
        }
        let status = command.status().map_err(|e| format!("couldn't start the Windows Installer ({e})"))?;
        explain(status.code()).map_err(|e| match status.code() {
            Some(1602) | None => e,
            _ => format!("{e}. Its log is at {}", msi_log(msi).display()),
        })
    })
}

/// Runs an app's own new MSI for a self-update, with the same progress window and no restart
/// as [`run_msi`], and waits for it. `context` keeps the installed copy's Windows Installer
/// context; `folder`, when given, is where it goes. A failure names the installer's log,
/// which is the only place 1603 is explained.
pub fn reinstall(msi: &Path, context: &[&str], folder: Option<&Path>) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut command = reinstall_command(&msiexec(), msi, context)?;
    if let Some(dir) = folder {
        command.raw_arg(msi_folder_argument(dir)?);
    }
    let status = command.status().map_err(|e| format!("couldn't start the Windows Installer ({e})"))?;
    explain(status.code()).map_err(|e| match status.code() {
        Some(1602) | None => e,
        _ => format!("{e}. Its log is at {}", msi_log(msi).display()),
    })
}

/// Runs `work` once no other Windows Installer job of ours is running (a second one would
/// stop with error 1618). Cancelled while it waits, it never runs.
fn in_turn(stop: &Stop, phase: &mut dyn FnMut(JobPhase), working: JobPhase, work: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    INSTALLER.alone(&cancelled(stop), &mut turn(phase, working), work).unwrap_or_else(|| Err(CANCELLED.into()))
}

/// Removes one installed product by its code, with a progress window and no restart, and waits.
/// A per-user product asks for no permission. `log` receives the installer's verbose log.
pub fn remove_product(product_code: &str, log: &Path) -> Result<(), String> {
    if !crate::msi_context::valid_product_code(product_code) {
        return Err(format!("{product_code} isn't a Windows Installer product code"));
    }
    let status = process::command(msiexec())
        .args(["/x", product_code, "/passive", "/norestart", "/l*v"])
        .arg(log)
        .status()
        .map_err(|e| format!("couldn't start the Windows Installer ({e})"))?;
    explain(status.code()).map_err(|e| format!("{e}. Its log is at {}", log.display()))
}

pub fn uninstall_msi(app: &App, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), String> {
    let installs = crate::detect::windows::msi_installs(app);
    if installs.is_empty() {
        return Err(format!("{} isn't registered with the Windows Installer", app.name));
    }
    in_turn(stop, phase, JobPhase::Removing, || {
        for m in installs {
            let status = process::command(msiexec())
                .args(["/x", &m.product_code, "/passive", "/norestart"])
                .status()
                .map_err(|e| format!("couldn't start the Windows Installer ({e})"))?;
            explain(status.code())?;
        }
        Ok(())
    })
}
