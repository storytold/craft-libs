//! Windows subprocesses for read-only spooler queries and native printer preferences.
//! Query workers accept only discovery and duplex reads, never preferences or jobs.

use std::io::Write as _;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::Printer;
use crate::command::OUTPUT_CAP;

const SWITCH: &str = "--craft-printer-query";
const LIST_LIMIT: Duration = Duration::from_secs(10);
const LIMIT: Duration = Duration::from_secs(60);

pub(super) fn preferences_command(printer: &str) -> std::io::Result<Command> {
    use std::os::windows::process::CommandExt as _;
    if printer.is_empty() || printer.contains(['"', '\0']) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a printer name"));
    }
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut command = Command::new(std::path::Path::new(&root).join("System32").join("rundll32.exe"));
    command.raw_arg("printui.dll,PrintUIEntry").raw_arg("/e").raw_arg("/n").raw_arg(format!("\"{printer}\""));
    Ok(command)
}

pub(super) fn preferences(printer: &str) -> std::io::Result<()> {
    let status = preferences_command(printer)?.status()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other("the printer's preferences window failed")) }
}

fn query(operation: &str, printer: &str, cancelled: &AtomicBool, limit: Duration) -> std::io::Result<Vec<u8>> {
    let output = crate::command::output(Command::new(std::env::current_exe()?).args([SWITCH, operation, printer]), cancelled, limit)?;
    if output.status.success() { Ok(output.stdout) } else { Err(std::io::Error::other("the Windows printer query stopped unexpectedly")) }
}

pub(super) fn printers() -> Vec<Printer> {
    query("list", "", &AtomicBool::new(false), LIST_LIMIT)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<(String, bool)>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(name, default)| Printer { name, default, description: String::new(), two_sided: None })
        .collect()
}

pub(super) fn two_sided(printer: &str, cancelled: &AtomicBool) -> Option<bool> {
    query("two-sided", printer, cancelled, LIMIT).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).flatten()
}

/// Handle the private read-only worker invocation before CLI or desktop startup. Returns
/// `None` for a normal launch. Handled queries terminate this process after writing their answer.
/// If `Some` is returned, the host must immediately exit using its value as the success status.
/// Worker arguments cannot select a file, submit a job, or start a GUI.
pub fn worker() -> Option<bool> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(SWITCH)) {
        return None;
    }
    let operation = args.next().and_then(|arg| arg.into_string().ok());
    let printer = args.next().and_then(|arg| arg.into_string().ok());
    let valid = args.next().is_none()
        && match (operation.as_deref(), printer.as_deref()) {
            (Some("list"), Some("")) => true,
            (Some("two-sided"), Some(name)) => !name.is_empty(),
            _ => false,
        };
    if !valid {
        return Some(false);
    }
    let limit = if operation.as_deref() == Some("list") { LIST_LIMIT } else { LIMIT };
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("Windows printer query".into()).spawn(move || {
        let value = if operation.as_deref() == Some("list") {
            let list: Vec<_> = crate::printer_queues().into_iter().map(|printer| (printer.name, printer.default)).collect();
            serde_json::to_vec(&list)
        } else {
            serde_json::to_vec(&printer.as_deref().and_then(crate::two_sided))
        };
        let _ = sender.send(value);
    });
    if spawned.is_err() {
        return Some(false);
    }
    let success = match receiver.recv_timeout(limit) {
        Ok(Ok(bytes)) if bytes.len() as u64 <= OUTPUT_CAP => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&bytes).and_then(|()| stdout.flush()).is_ok()
        }
        _ => false,
    };
    // ExitProcess can deadlock in driver DLL detach while another thread holds a driver lock.
    let _ = winsafe::HPROCESS::GetCurrentProcess().TerminateProcess(u32::from(!success));
    Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_preference_names_are_rejected_without_starting_a_window() {
        for printer in ["", "bad\"name", "bad\0name"] {
            assert_eq!(preferences_command(printer).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        }
    }
}
