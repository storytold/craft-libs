//! The portal path's tests: without a session bus, the dialog is a normal `false` and the job
//! a clean `Err` — checked in a child process, because the bus address is process state and
//! changing the environment here is forbidden.

use super::*;
use crate as spool;

/// Set on the child process when a test runs this binary as a print caller without a session
/// bus: the child's `DBUS_SESSION_BUS_ADDRESS` then names a socket that does not exist.
const NO_BUS: &str = "CRAFT_STAND_IN_NO_BUS";

#[test]
fn stand_in_without_a_session_bus() {
    if std::env::var_os(NO_BUS).is_none() {
        return;
    }
    // The bus address names a socket that does not exist: the session bus is simply missing.
    assert!(!spool::portal::print_dialog_available(), "a missing bus is false, not an error or a panic");
    let err = spool::portal::print(b"%PDF-1.7\n", &Job::default()).unwrap_err();
    assert!(err.to_string().contains("session bus"), "{err}");
}

#[test]
fn a_missing_session_bus_is_a_normal_false_and_a_clean_error() {
    let dead = std::env::temp_dir().join(format!("craft-print-no-session-bus-{}", std::process::id()));
    let _ = std::fs::remove_file(&dead);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::portal::stand_in_without_a_session_bus", "--nocapture"])
        .env(NO_BUS, "1")
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", dead.display()))
        .spawn()
        .unwrap();
    assert!(child.wait().unwrap().success(), "without a session bus, the portal path fails cleanly");
}
